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

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use lume_config::schema::TitleBar;
use lume_config::{
    ColorScheme, ConfigError, ConfigStore, Dirs, Platform, Problem, Resolved, TokenValue,
    WatchEvent, Watcher,
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
}

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
        };
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

    /// A view without its revision.
    fn body(&self) -> Value {
        let resolved = self.store.current();
        let scheme = *lock(&self.scheme);
        let (tokens, token_problems) = resolved.tokens(scheme);
        let keymap = resolved.keymap(self.platform);
        let problems: Vec<&Problem> = resolved
            .problems
            .iter()
            .chain(&token_problems)
            .chain(&keymap.problems)
            .collect();
        let title_bar = resolved.config.window.title_bar_on(self.platform);
        json!({
            "platform": self.platform.as_str(),
            "title_bar": title_bar,
            "scheme": scheme,
            "theme": { "id": tokens.theme, "scheme": tokens.scheme },
            "config": resolved.config,
            "sources": resolved.sources,
            "problems": problems,
            "tokens": tokens.to_json(),
            "keymap": { "bindings": keymap.bindings },
        })
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
        let notice = {
            let mut told = lock(&self.told);
            let body = self.body();
            if body == *told {
                return;
            }
            let mut changed = Vec::new();
            differing("", &told["config"], &body["config"], &mut changed);
            // What is not a setting but changes what a page draws.
            for part in ["scheme", "theme", "tokens", "keymap", "problems"] {
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
                WatchEvent::Reloaded(_) => service.announce(),
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

    /// Whether the windows are drawn without the system's title bar, the page drawing
    /// its own.
    pub fn title_bar_custom(&self) -> bool {
        self.current().config.window.title_bar_on(self.platform) == TitleBar::Custom
    }

    /// The workspace whose file is laid over the user's.
    #[cfg(test)]
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

    /// Gives a user who has no `config.toml` one that shows every setting.
    pub fn ensure_default_file(&self) {
        if let Err(error) = self.store.ensure_default_file() {
            tracing::warn!(%error, "the default configuration file could not be written");
        }
    }

    /// Sets one setting in the user's file.
    pub fn set(&self, path: &str, value: &Value) -> Result<(), ConfigError> {
        self.store.set(path, value)?;
        self.announce();
        Ok(())
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
        self.current()
            .themes
            .iter()
            .map(|theme| ThemeEntry {
                id: theme.id.clone(),
                name: theme.name.clone(),
                scheme: theme.scheme,
                builtin: theme.file.is_none(),
            })
            .collect()
    }

    /// The file of the picture shown behind the panes, if the theme in use or the
    /// settings name one.
    pub fn background_file(&self) -> Option<PathBuf> {
        let (tokens, _) = self.current().tokens(*lock(&self.scheme));
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
            _ => None,
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
        assert_eq!(ids, ["ember", "lume-dark", "lume-light"]);
        assert_eq!(listed[0].name, "Ember");
        assert!(!listed[0].builtin);
        assert!(listed[1].builtin);
        assert_eq!(listed[2].scheme, ColorScheme::Light);
        assert_eq!(
            serde_json::to_value(&listed[2]).unwrap(),
            json!({ "id": "lume-light", "name": listed[2].name, "scheme": "light", "builtin": true })
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
