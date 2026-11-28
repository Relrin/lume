//! The configuration as the application uses it and as its pages are told it.
//!
//! The configuration crate reads the user's files and resolves them. What a page needs
//! is one step further on: the settings, every design token's value under the theme in
//! use, and the keys in effect, all under one revision, so that a page never draws with
//! the tokens of one configuration and the keys of another. That is a view, and this
//! service makes it.
//!
//! A view changes when a file changes, when a setting is written through the
//! application, and when the operating system changes between its light and dark
//! schemes. Each of these ends in the same place: the view is made again and, if it
//! came out different, every page is told.
//!
//! Plugins add to the configuration too: themes to choose from, keys between the
//! application's and the user's, and the tables of their own settings, which become a
//! part of the schema the settings window draws from. Those come from the plugins'
//! manager whenever what is installed changes, and make the view again like any other
//! change.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use lume_config::layouts::Layouts;
use lume_config::schema::TitleBar;
use lume_config::source::split_path;
use lume_config::{
    ColorScheme, ConfigBody, ConfigError, ConfigStore, Dirs, Dropped, Keymap, KeymapView,
    LayoutEntry, ListedProfile, Platform, PluginBinding, Problem, Resolved, RexHostEntry,
    SshProfile, Theme, ThemeChoice, Themes, TokenValue, WatchEvent, Watcher, WslProfile,
    check_value, plugin_setting_problems, resolve_keymap_with, resolve_tokens,
};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::background;

/// The token that names the picture behind the panes.
const BACKGROUND_IMAGE: &str = "background.image";

/// One of the themes there are to choose from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ThemeEntry {
    /// What the settings call it.
    pub id: String,
    /// What a person calls it.
    pub name: String,
    pub scheme: ColorScheme,
    /// Whether it comes with the application, not from a file of the user's.
    pub builtin: bool,
    /// The plugin it comes from, by the id it is installed under, if it does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
}

/// What the plugins add to the configuration.
#[derive(Clone, Debug, Default)]
pub struct PluginConfig {
    pub themes: Vec<Theme>,
    pub keys: Vec<PluginBinding>,
    /// The schema of each one's table of settings, by install.
    pub fragments: BTreeMap<String, serde_json::Value>,
    /// The configuration's schema with the plugins' parts in it.
    pub schema: Option<Arc<Value>>,
    /// The installs that are turned on, whose settings' problems are said.
    pub enabled: BTreeSet<String>,
}

/// Takes what pages are to be told.
type Tell = Arc<dyn Fn(Value) + Send + Sync>;

/// The configuration in use, and the way to tell pages about it.
pub struct ConfigService {
    store: Arc<ConfigStore>,
    platform: Platform,
    /// The colour scheme of the operating system.
    scheme: Mutex<ColorScheme>,
    /// Counts the views pages have been told of.
    revision: AtomicU64,
    /// The view pages were last told, without its revision.
    told: Mutex<Value>,
    tell: OnceLock<Tell>,
    watcher: Mutex<Option<Watcher>>,
    plugins: Mutex<PluginConfig>,
    /// Counts the schemas the settings window has been given: it reads the schema again
    /// when this changes.
    schema_revision: AtomicU64,
    /// The plugins' keys that were left out when the keys were last worked out.
    dropped: Mutex<Vec<Dropped>>,
    /// The layout files as they were when their follower was last told of them.
    layouts_seen: Mutex<Layouts>,
    /// Who is told when the layout files change: the layouts before and after.
    layouts_follower: OnceLock<LayoutsFollower>,
}

/// Something that is told the layout files before and after they changed.
type LayoutsFollower = Box<dyn Fn(Layouts, Layouts) + Send + Sync>;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The paths at which two JSON values differ. Objects are compared key by key; anything
/// else is one value.
fn differing(path: &str, before: &Value, after: &Value, found: &mut Vec<String>) {
    match (before, after) {
        (Value::Object(before), Value::Object(after)) => {
            let mut keys: Vec<&String> = before.keys().chain(after.keys()).collect();
            keys.sort();
            keys.dedup();
            for key in keys {
                let inner = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                differing(
                    &inner,
                    before.get(key).unwrap_or(&Value::Null),
                    after.get(key).unwrap_or(&Value::Null),
                    found,
                );
            }
        }
        _ if before != after => found.push(path.to_owned()),
        _ => {}
    }
}

impl ConfigService {
    /// Reads the configuration, with the file of `workspace` laid over the user's.
    pub fn load(dirs: Dirs, workspace: Option<&str>) -> Arc<Self> {
        let store = Arc::new(ConfigStore::load(dirs, workspace));
        let service = Self {
            store,
            platform: Platform::current(),
            // Until a window can say otherwise.
            scheme: Mutex::new(ColorScheme::Dark),
            revision: AtomicU64::new(1),
            told: Mutex::new(Value::Null),
            tell: OnceLock::new(),
            watcher: Mutex::new(None),
            plugins: Mutex::new(PluginConfig::default()),
            schema_revision: AtomicU64::new(1),
            dropped: Mutex::new(Vec::new()),
            layouts_seen: Mutex::new(Layouts::default()),
            layouts_follower: OnceLock::new(),
        };
        *lock(&service.layouts_seen) = service.store.current().layouts.clone();
        *lock(&service.told) = service.body();
        Arc::new(service)
    }

    /// The configuration as it stands.
    pub fn current(&self) -> Arc<Resolved> {
        self.store.current()
    }

    /// Where the user's files are.
    pub fn dirs(&self) -> &Dirs {
        self.store.dirs()
    }

    /// The themes there are: the configuration's and the plugins'.
    fn all_themes(&self, resolved: &Resolved) -> Themes {
        let plugins = lock(&self.plugins);
        if plugins.themes.is_empty() {
            return resolved.themes.clone();
        }
        let mut themes = resolved.themes.clone();
        for theme in &plugins.themes {
            themes.insert(theme.clone());
        }
        themes
    }

    /// The keys in effect: the application's, the plugins', the user's.
    fn keymap(&self, resolved: &Resolved) -> Keymap {
        let keys = lock(&self.plugins).keys.clone();
        let (keymap, dropped) =
            resolve_keymap_with(&resolved.config, &resolved.sources, self.platform, &keys);
        *lock(&self.dropped) = dropped;
        keymap
    }

    /// What is wrong with the settings of plugins that are turned on: a key a plugin
    /// does not have, or a value it does not take. An installed plugin's table is used
    /// for what is right in it; a table of a plugin that is not installed is kept as it
    /// is, and nothing is said of it.
    fn plugin_problems(&self, resolved: &Resolved) -> Vec<Problem> {
        let plugins = lock(&self.plugins);
        plugin_setting_problems(
            &resolved.config.plugins,
            &resolved.sources,
            &plugins.fragments,
            &|local| plugins.enabled.contains(local),
        )
    }

    /// The colour behind the panes under the theme in use: what a window is filled with
    /// before its page has drawn anything.
    pub fn surface_color(&self) -> Option<lume_config::Color> {
        let resolved = self.store.current();
        let scheme = *lock(&self.scheme);
        let themes = self.all_themes(&resolved);
        let (tokens, _) = resolve_tokens(&resolved.config, &resolved.sources, &themes, scheme);
        tokens.color("color.surface")
    }

    /// A view without its revision, as the page's generated `ConfigView` type says it.
    fn body(&self) -> Value {
        self.view_body().to_value()
    }

    /// A view without its revision.
    fn view_body(&self) -> ConfigBody {
        let resolved = self.store.current();
        let scheme = *lock(&self.scheme);
        let themes = self.all_themes(&resolved);
        let (tokens, token_problems) =
            resolve_tokens(&resolved.config, &resolved.sources, &themes, scheme);
        let keymap = self.keymap(&resolved);
        let plugin_problems = self.plugin_problems(&resolved);
        let problems: Vec<&Problem> = resolved
            .problems
            .iter()
            .chain(&token_problems)
            .chain(&keymap.problems)
            .chain(&plugin_problems)
            .collect();
        let title_bar = resolved.config.window.title_bar_on(self.platform);
        ConfigBody {
            platform: self.platform,
            title_bar,
            scheme,
            theme: ThemeChoice {
                id: tokens.theme.clone(),
                scheme: tokens.scheme,
            },
            config: resolved.config.clone(),
            sources: resolved.sources.clone(),
            problems: problems.into_iter().cloned().collect(),
            tokens: tokens.to_json(),
            keymap: KeymapView {
                bindings: keymap.bindings,
                removed: keymap.removed,
            },
            layouts: resolved
                .layouts
                .iter()
                .map(|layout| LayoutEntry {
                    id: layout.id.clone(),
                    name: layout.name.clone(),
                    description: layout.description.clone(),
                    tabs: layout.tabs.len(),
                    file: layout.file.display().to_string(),
                })
                .collect(),
            schema_revision: self.schema_revision.load(Ordering::Relaxed),
        }
    }

    fn with_revision(&self, body: &Value) -> Value {
        let mut view = body.as_object().cloned().unwrap_or_else(Map::new);
        view.insert(
            "revision".to_owned(),
            json!(self.revision.load(Ordering::Relaxed)),
        );
        Value::Object(view)
    }

    /// The configuration as a page is told it.
    pub fn view(&self) -> Value {
        self.with_revision(&lock(&self.told))
    }

    /// Makes the view again and tells the pages if it came out different.
    fn announce(&self) {
        // A change made through the application reads the layout files too, and the
        // watcher then finds nothing new.
        self.layouts_read(&self.store.current().layouts);
        let notice = {
            let mut told = lock(&self.told);
            let body = self.body();
            if body == *told {
                return;
            }
            let mut changed = Vec::new();
            differing("", &told["config"], &body["config"], &mut changed);
            // What is not a setting but changes what a page draws.
            for part in [
                "scheme",
                "theme",
                "tokens",
                "keymap",
                "layouts",
                "problems",
                "schema_revision",
            ] {
                if told[part] != body[part] {
                    changed.push(part.to_owned());
                }
            }
            *told = body;
            let revision = self.revision.fetch_add(1, Ordering::Relaxed) + 1;
            json!({
                "type": "config_changed",
                "revision": revision,
                "changed": changed,
                "view": self.with_revision(&told),
            })
        };
        if let Some(tell) = self.tell.get() {
            tell(notice);
        }
    }

    /// Records the operating system's colour scheme, which chooses between the light
    /// and the dark theme.
    pub fn set_scheme(&self, scheme: ColorScheme) {
        *lock(&self.scheme) = scheme;
        self.announce();
    }

    /// Starts telling pages of changes through `tell`, and watching the user's files
    /// for them. Without the watch the configuration still changes when it is written
    /// through the application.
    pub fn watch(self: &Arc<Self>, tell: impl Fn(Value) + Send + Sync + 'static) {
        if self.tell.set(Arc::new(tell)).is_err() {
            return;
        }
        let service = Arc::downgrade(self);
        let watching = Watcher::spawn(Arc::clone(&self.store), move |event| {
            let Some(service) = service.upgrade() else {
                return;
            };
            match event {
                WatchEvent::Reloaded(now) => {
                    service.layouts_read(&now.layouts);
                    service.announce();
                }
                WatchEvent::Failed(problems) => {
                    if let Some(tell) = service.tell.get() {
                        tell(json!({ "type": "config_error", "problems": problems }));
                    }
                }
            }
        });
        match watching {
            Ok(watcher) => *lock(&self.watcher) = Some(watcher),
            Err(error) => tracing::error!(%error, "the configuration is not watched for changes"),
        }
    }

    /// Starts telling `follower` of every change to the layout files: what they were
    /// and what they are now. A change the files' list shows also reaches the pages.
    pub fn follow_layouts(&self, follower: impl Fn(Layouts, Layouts) + Send + Sync + 'static) {
        let _ = self.layouts_follower.set(Box::new(follower));
    }

    /// Notes the layout files as they were read, and tells their follower if they
    /// changed.
    fn layouts_read(&self, now: &Layouts) {
        let before = {
            let mut seen = lock(&self.layouts_seen);
            if *seen == *now {
                return;
            }
            std::mem::replace(&mut *seen, now.clone())
        };
        if let Some(follower) = self.layouts_follower.get() {
            follower(before, now.clone());
        }
    }

    /// Whether the windows are drawn without the system's title bar, the page drawing
    /// its own.
    pub fn title_bar_custom(&self) -> bool {
        self.current().config.window.title_bar_on(self.platform) == TitleBar::Custom
    }

    /// The workspace whose file is laid over the user's.
    pub fn workspace(&self) -> Option<String> {
        self.store.workspace()
    }

    /// Makes `workspace` the one whose file is laid over the user's.
    pub fn set_workspace(&self, workspace: Option<&str>) {
        self.store.set_workspace(workspace);
        self.announce();
    }

    /// Sets or clears the folder of the workspace called `name`, in that workspace's
    /// own file.
    pub fn set_workspace_root(&self, name: &str, root: Option<&str>) -> Result<(), ConfigError> {
        self.store.set_workspace_root(name, root)?;
        self.announce();
        Ok(())
    }

    /// Writes a Rex server into the user's file, in place of the one labelled `was` if
    /// one is named.
    pub fn put_rex_host(&self, was: Option<&str>, entry: &RexHostEntry) -> Result<(), ConfigError> {
        self.store.put_rex_host(was, entry)?;
        self.announce();
        Ok(())
    }

    /// Takes a Rex server out of the user's file.
    pub fn remove_rex_host(&self, label: &str) -> Result<(), ConfigError> {
        self.store.remove_rex_host(label)?;
        self.announce();
        Ok(())
    }

    /// Writes a profile for a machine reached over SSH into the user's file, in place of
    /// the one called `id` if one is named. Gives its id.
    pub fn put_ssh_profile(
        &self,
        id: Option<&str>,
        profile: &SshProfile,
    ) -> Result<String, ConfigError> {
        let id = self.store.put_ssh_profile(id, profile)?;
        self.announce();
        Ok(id)
    }

    /// Writes a profile for a Linux distribution into the user's file, in place of the
    /// one called `id` if one is named. Gives its id.
    pub fn put_wsl_profile(
        &self,
        id: Option<&str>,
        profile: &WslProfile,
    ) -> Result<String, ConfigError> {
        let id = self.store.put_wsl_profile(id, profile)?;
        self.announce();
        Ok(id)
    }

    /// Takes a profile out of the user's file.
    pub fn remove_profile(&self, id: &str) -> Result<(), ConfigError> {
        self.store.remove_profile(id)?;
        self.announce();
        Ok(())
    }

    /// Says of profiles that were found whether they are listed and where they start.
    pub fn list_profiles(&self, changes: &[ListedProfile]) -> Result<(), ConfigError> {
        self.store.list_profiles(changes)?;
        self.announce();
        Ok(())
    }

    /// Gives a user who has no `config.toml` one that shows every setting.
    pub fn ensure_default_file(&self) {
        if let Err(error) = self.store.ensure_default_file() {
            tracing::warn!(%error, "the default configuration file could not be written");
        }
    }

    /// Sets one setting in the user's file. A setting of an installed plugin is held
    /// to what the plugin's manifest says of it.
    pub fn set(&self, path: &str, value: &Value) -> Result<(), ConfigError> {
        self.check_plugin_setting(path, value)?;
        self.store.set(path, value)?;
        self.announce();
        Ok(())
    }

    fn check_plugin_setting(&self, path: &str, value: &Value) -> Result<(), ConfigError> {
        let Ok(segments) = split_path(path) else {
            return Ok(());
        };
        let [section, local, key] = segments.as_slice() else {
            return Ok(());
        };
        if section != "plugins" {
            return Ok(());
        }
        let plugins = lock(&self.plugins);
        let Some(fragment) = plugins.fragments.get(local) else {
            return Ok(());
        };
        let Some(schema) = fragment.pointer(&format!("/properties/{key}")) else {
            return Err(ConfigError::UnknownKey {
                path: path.to_owned(),
                suggestion: None,
            });
        };
        check_value(value, schema).map_err(|message| ConfigError::InvalidValue {
            path: path.to_owned(),
            message,
        })
    }

    /// Takes in what the plugins add, and tells the pages if the view changes.
    pub fn set_plugins(&self, config: PluginConfig) {
        {
            let mut plugins = lock(&self.plugins);
            let schema_changed = plugins.schema != config.schema;
            *plugins = config;
            if schema_changed {
                self.schema_revision.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.announce();
    }

    /// The schema of the settings, with the plugins' parts in it.
    pub fn schema(&self) -> Value {
        match &lock(&self.plugins).schema {
            Some(schema) => (**schema).clone(),
            None => lume_config::config_schema().clone(),
        }
    }

    /// The plugins' keys that were left out, and why.
    pub fn dropped_keys(&self) -> Vec<Dropped> {
        lock(&self.dropped).clone()
    }

    /// A plugin's settings as they stand: what the user set where it is a value the
    /// plugin takes, over the defaults its manifest gives.
    pub fn plugin_settings(&self, local: &str) -> Value {
        let fragment = lock(&self.plugins).fragments.get(local).cloned();
        let Some(fragment) = fragment else {
            return Value::Object(Map::new());
        };
        let mut settings = Map::new();
        let properties = fragment
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let current = self.current();
        let set = current.config.plugins.get(local).and_then(Value::as_object);
        for (key, schema) in &properties {
            let chosen = set
                .and_then(|set| set.get(key))
                .filter(|value| check_value(value, schema).is_ok())
                .or_else(|| schema.get("default"));
            if let Some(value) = chosen {
                settings.insert(key.clone(), value.clone());
            }
        }
        Value::Object(settings)
    }

    /// Takes one setting out of the user's file, so its default holds again.
    pub fn unset(&self, path: &str) -> Result<(), ConfigError> {
        self.store.unset(path)?;
        self.announce();
        Ok(())
    }

    /// Reads the user's files again.
    pub fn reload(&self) -> Result<(), ConfigError> {
        self.store.reload().map_err(ConfigError::Rejected)?;
        self.announce();
        Ok(())
    }

    /// The themes there are: the ones that come with the application and the user's
    /// own, in the order of their ids.
    pub fn themes(&self) -> Vec<ThemeEntry> {
        let resolved = self.current();
        let plugins: BTreeMap<String, String> = lock(&self.plugins)
            .themes
            .iter()
            .filter_map(|theme| {
                let (local, _) = theme.id.split_once('/')?;
                Some((theme.id.clone(), local.to_owned()))
            })
            .collect();
        self.all_themes(&resolved)
            .iter()
            .map(|theme| ThemeEntry {
                id: theme.id.clone(),
                name: theme.name.clone(),
                scheme: theme.scheme,
                builtin: theme.file.is_none(),
                plugin: plugins.get(&theme.id).cloned(),
            })
            .collect()
    }

    /// The look of a terminal drawn with the theme `id` in place of the one the settings
    /// choose: every token's value, the user's own settings still laid over, and the
    /// theme's colour scheme, as a page is told them. `None` when there is no such theme.
    pub fn look_with_theme(&self, id: &str) -> Option<Value> {
        let resolved = self.current();
        let themes = self.all_themes(&resolved);
        themes.get(id)?;
        let mut config = resolved.config.clone();
        config.appearance.theme = Some(id.to_owned());
        let scheme = *lock(&self.scheme);
        let (tokens, _) = resolve_tokens(&config, &resolved.sources, &themes, scheme);
        Some(json!({ "scheme": tokens.scheme, "tokens": tokens.to_json() }))
    }

    /// The file of the picture shown behind the panes, if the theme in use or the
    /// settings name one.
    pub fn background_file(&self) -> Option<PathBuf> {
        let resolved = self.current();
        let themes = self.all_themes(&resolved);
        let (tokens, _) = resolve_tokens(
            &resolved.config,
            &resolved.sources,
            &themes,
            *lock(&self.scheme),
        );
        match tokens.get(BACKGROUND_IMAGE)? {
            TokenValue::Image(Some(written)) => {
                Some(background::resolve(&self.dirs().config, written))
            }
            _ => None,
        }
    }

    /// The file a page may ask to have opened: the user's `config.toml`, or the file of
    /// the workspace in use.
    pub fn file(&self, which: &str) -> Option<PathBuf> {
        match which {
            "config" => Some(self.dirs().config_file()),
            "workspace" => Some(self.dirs().workspace_file(&self.store.workspace()?)),
            _ => {
                // A layout file, as `layout:<id>`, or a theme of the user's, as
                // `theme:<id>`.
                if let Some(id) = which.strip_prefix("layout:") {
                    return lume_config::layouts::valid_id(id).then(|| self.dirs().layout_file(id));
                }
                let id = which.strip_prefix("theme:")?;
                lume_config::layouts::valid_id(id)
                    .then(|| self.dirs().themes_dir().join(format!("{id}.toml")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;
    use crate::testing::TempDir;

    /// How long a change may take to reach the pages before a test gives up on it.
    const PATIENCE: Duration = Duration::from_secs(10);

    fn service(dir: &TempDir) -> (Arc<ConfigService>, mpsc::Receiver<Value>) {
        let service = ConfigService::load(dir.dirs(), None);
        let (told, notices) = mpsc::channel();
        service.watch(move |notice| {
            let _ = told.send(notice);
        });
        (service, notices)
    }

    #[test]
    fn the_view_is_the_json_it_was_before_it_had_a_type() {
        let dir = TempDir::new("view-json");
        std::fs::create_dir_all(dir.dirs().config_file().parent().unwrap()).unwrap();
        std::fs::write(
            dir.dirs().config_file(),
            "[terminal]
cursor-style = \"bar\"
not-a-setting = 1
",
        )
        .unwrap();
        let service = ConfigService::load(dir.dirs(), None);
        let typed = service.view_body();
        // The view as it was written by hand before the page's type was generated.
        let by_hand = json!({
            "platform": typed.platform.as_str(),
            "title_bar": typed.title_bar,
            "scheme": typed.scheme,
            "theme": { "id": typed.theme.id, "scheme": typed.theme.scheme },
            "config": typed.config,
            "sources": typed.sources,
            "problems": typed.problems,
            "tokens": typed.tokens,
            "keymap": { "bindings": typed.keymap.bindings, "removed": typed.keymap.removed },
            "layouts": typed.layouts,
            "schema_revision": typed.schema_revision,
        });
        assert!(!typed.problems.is_empty(), "the unknown key is a problem");
        assert_eq!(service.body(), by_hand);
    }

    #[test]
    fn the_themes_listed_are_the_built_in_ones_and_the_users_own() {
        let dir = TempDir::new("themes");
        let themes = dir.dirs().themes_dir();
        std::fs::create_dir_all(&themes).unwrap();
        std::fs::write(
            themes.join("ember.toml"),
            "name = \"Ember\"\nscheme = \"dark\"\ninherits = \"lume-dark\"\n",
        )
        .unwrap();
        let service = ConfigService::load(dir.dirs(), None);

        let listed = service.themes();
        let ids: Vec<&str> = listed.iter().map(|theme| theme.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "ember",
                "lume-dark",
                "lume-daylight",
                "lume-light",
                "lume-midnight"
            ]
        );
        assert_eq!(listed[0].name, "Ember");
        assert!(!listed[0].builtin);
        assert!(listed[1..].iter().all(|theme| theme.builtin));
        assert_eq!(listed[2].scheme, ColorScheme::Light);
        assert_eq!(listed[4].scheme, ColorScheme::Dark);
        assert_eq!(
            serde_json::to_value(&listed[3]).unwrap(),
            json!({ "id": "lume-light", "name": listed[3].name, "scheme": "light", "builtin": true })
        );
    }

    #[test]
    fn what_plugins_add_is_in_the_view_and_their_settings_are_held_to_their_schema() {
        let dir = TempDir::new("plugins");
        std::fs::create_dir_all(&dir.dirs().config).unwrap();
        std::fs::write(
            dir.dirs().config_file(),
            "[plugins.\"acme.meters\"]\nrate = 9\ncpu = false\nbogus = 1\n\n[plugins.\"gone.x\"]\nkept = true\n",
        )
        .unwrap();
        let service = ConfigService::load(dir.dirs(), None);
        let fragment = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "cpu": { "type": "boolean", "default": true },
                "rate": { "type": "integer", "default": 2, "enum": [1, 2, 5] },
            },
        });
        let (theme, _) = lume_config::parse_theme(
            "acme.meters/night",
            "name = \"Night\"\ninherits = \"lume-dark\"\n",
            None,
        )
        .unwrap();
        let mut fragments = BTreeMap::new();
        fragments.insert("acme.meters".to_owned(), fragment);
        let before = service.view()["schema_revision"].clone();
        service.set_plugins(PluginConfig {
            themes: vec![theme],
            keys: vec![PluginBinding {
                plugin: "acme.meters".to_owned(),
                keys: "ctrl+shift+y".to_owned(),
                macos: None,
                command: "acme.meters.refresh".to_owned(),
                args: Value::Null,
                when: None,
            }],
            fragments,
            schema: Some(Arc::new(json!({ "composed": true }))),
            enabled: ["acme.meters".to_owned()].into_iter().collect(),
        });
        let view = service.view();
        assert_ne!(view["schema_revision"], before);
        assert_eq!(service.schema(), json!({ "composed": true }));
        assert!(
            view["keymap"]["bindings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|binding| binding["command"] == "acme.meters.refresh")
        );
        let problems: Vec<&str> = view["problems"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|problem| problem["path"].as_str())
            .collect();
        assert_eq!(
            problems,
            [
                "plugins.\"acme.meters\".bogus",
                "plugins.\"acme.meters\".rate"
            ]
        );
        // A value the plugin does not take falls back to its default.
        assert_eq!(
            service.plugin_settings("acme.meters"),
            json!({ "cpu": false, "rate": 2 })
        );
        assert!(
            service
                .themes()
                .iter()
                .any(|theme| theme.id == "acme.meters/night"
                    && theme.plugin.as_deref() == Some("acme.meters"))
        );
        assert!(matches!(
            service.set("plugins.\"acme.meters\".rate", &json!(3)),
            Err(ConfigError::InvalidValue { .. })
        ));
        assert!(matches!(
            service.set("plugins.\"acme.meters\".nope", &json!(3)),
            Err(ConfigError::UnknownKey { .. })
        ));
        service
            .set("plugins.\"acme.meters\".rate", &json!(5))
            .unwrap();
        assert_eq!(service.plugin_settings("acme.meters")["rate"], 5);
    }

    #[test]
    fn a_plugins_table_setting_is_merged_from_the_users_file_and_the_workspaces() {
        let dir = TempDir::new("plugin-tables");
        std::fs::create_dir_all(dir.dirs().workspaces_dir()).unwrap();
        std::fs::write(
            dir.dirs().config_file(),
            "[plugins.\"acme.agents\".profiles.fast]\nagent = \"one\"\nmodel = \"small\"\n\n\
             [plugins.\"acme.agents\".profiles.deep]\nagent = \"two\"\n\n\
             [plugins.\"acme.agents\"]\nlimits = 3\n",
        )
        .unwrap();
        std::fs::write(
            dir.dirs().workspace_file("work"),
            "[plugins.\"acme.agents\".profiles.fast]\nmodel = \"large\"\n\n\
             [plugins.\"acme.agents\".profiles.local]\nagent = \"three\"\n",
        )
        .unwrap();
        let service = ConfigService::load(dir.dirs(), None);
        let mut fragments = BTreeMap::new();
        fragments.insert(
            "acme.agents".to_owned(),
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "profiles": { "type": "object", "default": {} },
                    "limits": { "type": "object", "default": { "runs": 2 } },
                },
            }),
        );
        service.set_plugins(PluginConfig {
            fragments,
            enabled: ["acme.agents".to_owned()].into_iter().collect(),
            ..PluginConfig::default()
        });
        assert_eq!(
            service.plugin_settings("acme.agents"),
            json!({
                "profiles": {
                    "fast": { "agent": "one", "model": "small" },
                    "deep": { "agent": "two" },
                },
                // Not a table: the default stands in for it.
                "limits": { "runs": 2 },
            })
        );
        let view = service.view();
        let problems = view["problems"].as_array().unwrap();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0]["path"], "plugins.\"acme.agents\".limits");
        assert_eq!(problems[0]["line"], 9);
        assert!(
            problems[0]["message"]
                .as_str()
                .unwrap()
                .contains("expected a table"),
            "{problems:?}"
        );

        // The workspace's file is laid over the user's key by key, at every depth.
        service.set_workspace(Some("work"));
        assert_eq!(
            service.plugin_settings("acme.agents")["profiles"],
            json!({
                "fast": { "agent": "one", "model": "large" },
                "deep": { "agent": "two" },
                "local": { "agent": "three" },
            })
        );
    }

    #[test]
    fn the_picture_behind_the_panes_is_the_file_the_settings_name() {
        let dir = TempDir::new("background-file");
        let service = ConfigService::load(dir.dirs(), None);
        assert_eq!(service.background_file(), None);

        std::fs::create_dir_all(&dir.dirs().config).unwrap();
        service
            .set("appearance.background-image", &json!("wall.png"))
            .unwrap();
        assert_eq!(
            service.background_file(),
            Some(dir.dirs().config.join("wall.png"))
        );
        // No picture again once the setting is emptied.
        service
            .set("appearance.background-image", &json!(""))
            .unwrap();
        assert_eq!(service.background_file(), None);
    }

    #[test]
    fn a_view_holds_the_settings_the_tokens_and_the_keys_under_one_revision() {
        let dir = TempDir::new("view");
        let service = ConfigService::load(dir.dirs(), None);
        let view = service.view();
        assert_eq!(view["revision"], 1);
        assert_eq!(view["scheme"], "dark");
        let expected = if cfg!(target_os = "linux") {
            "system"
        } else {
            "custom"
        };
        assert_eq!(view["title_bar"], expected);
        assert_eq!(view["theme"]["id"], "lume-dark");
        assert_eq!(view["config"]["font"]["family"], "JetBrains Mono");
        assert!(view["tokens"]["color.surface"].is_string());
        assert!(view["tokens"]["radius.card"].is_number());
        let bindings = view["keymap"]["bindings"].as_array().unwrap();
        assert!(
            bindings
                .iter()
                .any(|binding| binding["command"] == "lume.tab.new")
        );
        assert_eq!(view["problems"], json!([]));
    }

    #[test]
    fn a_setting_written_through_the_service_is_told_with_what_changed() {
        let dir = TempDir::new("set");
        let (service, notices) = service(&dir);
        service.set("appearance.gap", &json!(12)).unwrap();

        let notice = notices.recv_timeout(PATIENCE).unwrap();
        assert_eq!(notice["type"], "config_changed");
        assert_eq!(notice["revision"], 2);
        let changed: Vec<&str> = notice["changed"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(changed.contains(&"appearance.gap"), "{changed:?}");
        assert!(changed.contains(&"tokens"), "{changed:?}");
        assert_eq!(notice["view"]["tokens"]["space.gap"].as_f64(), Some(12.0));
        assert_eq!(notice["view"]["revision"], 2);
        assert_eq!(service.view(), notice["view"]);
    }

    #[test]
    fn an_edit_of_the_file_reaches_the_pages_and_a_broken_file_changes_nothing() {
        let dir = TempDir::new("edit");
        let (service, notices) = service(&dir);
        let file = service.dirs().config_file();

        std::fs::write(&file, "[appearance]\ncard-radius = 4\n").unwrap();
        let notice = notices.recv_timeout(PATIENCE).unwrap();
        assert_eq!(notice["type"], "config_changed");
        assert_eq!(notice["view"]["tokens"]["radius.card"].as_f64(), Some(4.0));

        std::fs::write(&file, "[appearance\ncard-radius = 9\n").unwrap();
        let notice = notices.recv_timeout(PATIENCE).unwrap();
        assert_eq!(notice["type"], "config_error");
        assert!(!notice["problems"].as_array().unwrap().is_empty());
        assert_eq!(service.view()["tokens"]["radius.card"].as_f64(), Some(4.0));
    }

    #[test]
    fn the_systems_scheme_chooses_the_theme_and_the_same_view_is_not_told_twice() {
        let dir = TempDir::new("scheme");
        let (service, notices) = service(&dir);

        service.set_scheme(ColorScheme::Light);
        let notice = notices.recv_timeout(PATIENCE).unwrap();
        assert_eq!(notice["view"]["scheme"], "light");
        assert_eq!(notice["view"]["theme"]["id"], "lume-light");
        assert!(
            notice["changed"]
                .as_array()
                .unwrap()
                .contains(&json!("scheme"))
        );

        service.set_scheme(ColorScheme::Light);
        assert!(notices.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn only_the_users_own_files_can_be_asked_for() {
        let dir = TempDir::new("files");
        let service = ConfigService::load(dir.dirs(), Some("default"));
        assert_eq!(service.file("config"), Some(service.dirs().config_file()));
        assert_eq!(
            service.file("workspace"),
            Some(service.dirs().workspace_file("default"))
        );
        assert_eq!(service.file("../secrets"), None);
    }
}
