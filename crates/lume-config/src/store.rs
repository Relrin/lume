//! The configuration in use: read from the user's files, kept, and read again when
//! they change.
//!
//! The store holds one resolved configuration at a time and hands out shared copies of
//! it. Reading the files again either replaces it whole or, when a file cannot be used,
//! leaves it exactly as it was and says why: a mistake made while editing never takes
//! the settings away from a running application.
//!
//! At start there is no earlier configuration to keep. A file that cannot be used then
//! contributes nothing, and what is wrong with it is among the problems.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use lume_proto::ColorScheme;
use serde_json::Value;

use crate::error::ConfigError;
use crate::keymap::{Keymap, resolve_keymap};
use crate::keys::KeySequence;
use crate::layers::{LayerInput, LayerKind, check, entry_schema, is_table, resolve_layers};
use crate::paths::{Dirs, Platform, check_workspace_name};
use crate::schema::{Config, WorkspaceSettings, config_schema, default_config_text};
use crate::source::{Problem, ProblemKind, Source, join_path, split_path};
use crate::suggest::nearest;
use crate::theme::{Themes, is_group, meant, read_limited, resolve_tokens};
use crate::tokens::{PALETTE, ResolvedTokens, TokenValue, token_def};
use crate::write;

/// The configuration as it stands: the settings, where each came from, and the themes
/// there are.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub config: Config,
    /// The layer each setting came from, by the setting's path.
    pub sources: BTreeMap<String, Source>,
    /// What was wrong in the files and was passed over.
    pub problems: Vec<Problem>,
    /// What the active workspace's file says besides settings.
    pub workspace: WorkspaceSettings,
    /// The built-in themes and the user's.
    pub themes: Themes,
    /// Counts the changes: higher than that of every configuration before this one.
    pub revision: u64,
}

impl Resolved {
    /// Every token's value under these settings. `system` is the colour scheme of the
    /// operating system, which chooses between the light and the dark theme.
    pub fn tokens(&self, system: ColorScheme) -> (ResolvedTokens, Vec<Problem>) {
        resolve_tokens(&self.config, &self.sources, &self.themes, system)
    }

    /// The keys bound on `platform` under these settings.
    pub fn keymap(&self, platform: Platform) -> Keymap {
        resolve_keymap(&self.config, &self.sources, platform)
    }

    /// Whether this is the same configuration as `other`, whatever their revisions.
    fn same_as(&self, other: &Self) -> bool {
        self.config == other.config
            && self.sources == other.sources
            && self.problems == other.problems
            && self.workspace == other.workspace
            && self.themes == other.themes
    }
}

/// A problem with a whole file.
fn file_problem(kind: ProblemKind, file: &Path, message: String) -> Problem {
    Problem {
        kind,
        path: String::new(),
        file: Some(file.to_owned()),
        line: 0,
        column: 0,
        message,
        suggestion: None,
    }
}

/// Reads the files and resolves them. The second value is what made a file unusable.
fn read(dirs: &Dirs, workspace: Option<&str>) -> (Resolved, Vec<Problem>) {
    let mut fatal = Vec::new();
    let mut inputs = Vec::new();
    let mut layer = |kind: LayerKind, file: std::path::PathBuf| {
        if !file.exists() {
            // A file that is not there sets nothing, which is how most users start.
            return;
        }
        match read_limited(&file) {
            Ok(text) => inputs.push(LayerInput { kind, file, text }),
            Err(error) => fatal.push(file_problem(ProblemKind::Io, &file, error)),
        }
    };
    layer(LayerKind::User, dirs.config_file());
    if let Some(name) = workspace.filter(|name| check_workspace_name(name).is_ok()) {
        layer(
            LayerKind::Workspace {
                name: name.to_owned(),
            },
            dirs.workspace_file(name),
        );
    }

    let layered = resolve_layers(&inputs);
    fatal.extend(layered.fatal);
    let (themes, found) = Themes::load(&dirs.themes_dir());
    let mut problems = layered.problems;
    for problem in found {
        if problem.is_fatal() {
            fatal.push(problem);
        } else {
            problems.push(problem);
        }
    }
    let resolved = Resolved {
        config: layered.config,
        sources: layered.sources,
        problems,
        workspace: layered.workspace,
        themes,
        revision: 0,
    };
    (resolved, fatal)
}

struct State {
    workspace: Option<String>,
    current: Arc<Resolved>,
}

/// The configuration in use, and the way to read it again or change it.
pub struct ConfigStore {
    dirs: Dirs,
    state: Mutex<State>,
}

impl ConfigStore {
    /// Reads the configuration: the user's file, and the file of `workspace` if one is
    /// named. Files that are not there set nothing.
    pub fn load(dirs: Dirs, workspace: Option<&str>) -> Self {
        let (mut resolved, fatal) = read(&dirs, workspace);
        resolved.problems.extend(fatal);
        resolved.revision = 1;
        Self {
            dirs,
            state: Mutex::new(State {
                workspace: workspace.map(str::to_owned),
                current: Arc::new(resolved),
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Where the files are.
    pub fn dirs(&self) -> &Dirs {
        &self.dirs
    }

    /// The workspace whose file is laid over the user's.
    pub fn workspace(&self) -> Option<String> {
        self.state().workspace.clone()
    }

    /// The configuration as it stands.
    pub fn current(&self) -> Arc<Resolved> {
        Arc::clone(&self.state().current)
    }

    /// Reads the files again.
    ///
    /// If a file cannot be used, the configuration stays as it was and the error says
    /// what is wrong. If nothing came out different, the configuration returned is the
    /// very one there was, revision and all.
    pub fn reload(&self) -> Result<Arc<Resolved>, Vec<Problem>> {
        let mut state = self.state();
        let (mut resolved, fatal) = read(&self.dirs, state.workspace.as_deref());
        if !fatal.is_empty() {
            return Err(fatal);
        }
        if resolved.same_as(&state.current) {
            return Ok(Arc::clone(&state.current));
        }
        resolved.revision = state.current.revision + 1;
        state.current = Arc::new(resolved);
        Ok(Arc::clone(&state.current))
    }

    /// Makes `workspace` the one whose file is laid over the user's, and reads the
    /// files. There is no keeping the old configuration here: it was another
    /// workspace's. A file that cannot be used contributes nothing and is among the
    /// problems.
    pub fn set_workspace(&self, workspace: Option<&str>) -> Arc<Resolved> {
        let mut state = self.state();
        state.workspace = workspace.map(str::to_owned);
        let (mut resolved, fatal) = read(&self.dirs, workspace);
        resolved.problems.extend(fatal);
        if !resolved.same_as(&state.current) {
            resolved.revision = state.current.revision + 1;
            state.current = Arc::new(resolved);
        }
        Arc::clone(&state.current)
    }

    /// Sets one setting in the user's file and reads the files again. `path` is the
    /// setting's dotted path, such as `font.size`. Everything else in the file stays.
    pub fn set(&self, path: &str, value: &Value) -> Result<Arc<Resolved>, ConfigError> {
        let segments = split_path(path).map_err(ConfigError::BadPath)?;
        check_setting(&segments, value)?;
        self.edit(|text| write::set_value(text, &segments, value))
    }

    /// Takes one setting out of the user's file, so that its default holds again, and
    /// reads the files again.
    pub fn unset(&self, path: &str) -> Result<Arc<Resolved>, ConfigError> {
        let segments = split_path(path).map_err(ConfigError::BadPath)?;
        setting_schema(&segments)?;
        self.edit(|text| write::unset_value(text, &segments))
    }

    fn edit(
        &self,
        change: impl FnOnce(&str) -> Result<String, String>,
    ) -> Result<Arc<Resolved>, ConfigError> {
        let file = self.dirs.config_file();
        let text = if file.exists() {
            read_limited(&file)
                .map_err(|error| ConfigError::io(&file, std::io::Error::other(error)))?
        } else {
            String::new()
        };
        let changed = change(&text).map_err(|message| ConfigError::Syntax {
            path: file.clone(),
            message,
        })?;
        if changed != text {
            write::write_atomic(&file, &changed).map_err(|error| ConfigError::io(&file, error))?;
        }
        self.reload().map_err(ConfigError::Rejected)
    }

    /// Sets or clears the folder of the workspace called `name`, in that workspace's
    /// own file, and reads the files again if it is the active one.
    pub fn set_workspace_root(&self, name: &str, root: Option<&str>) -> Result<(), ConfigError> {
        check_workspace_name(name).map_err(ConfigError::BadName)?;
        let file = self.dirs.workspace_file(name);
        let text = if file.exists() {
            read_limited(&file)
                .map_err(|error| ConfigError::io(&file, std::io::Error::other(error)))?
        } else {
            String::new()
        };
        let changed =
            write::set_workspace_root(&text, root).map_err(|message| ConfigError::Syntax {
                path: file.clone(),
                message,
            })?;
        if changed != text {
            write::write_atomic(&file, &changed).map_err(|error| ConfigError::io(&file, error))?;
        }
        if self.workspace().as_deref() == Some(name) {
            self.reload().map_err(ConfigError::Rejected)?;
        }
        Ok(())
    }

    /// Gives a user who has no `config.toml` one that shows every setting at its
    /// default, commented out. Returns whether a file was written.
    pub fn ensure_default_file(&self) -> Result<bool, ConfigError> {
        let file = self.dirs.config_file();
        if file.exists() {
            return Ok(false);
        }
        write::write_atomic(&file, default_config_text())
            .map_err(|error| ConfigError::io(&file, error))?;
        Ok(true)
    }
}

/// The schema of the setting at `path`, if there is such a setting.
fn setting_schema(path: &[String]) -> Result<&'static Value, ConfigError> {
    let mut schema = config_schema();
    for (depth, segment) in path.iter().enumerate() {
        schema = entry_schema(schema, segment).ok_or_else(|| {
            let known = schema.get("properties").and_then(Value::as_object);
            ConfigError::UnknownKey {
                path: join_path(&path[..=depth]),
                suggestion: known
                    .and_then(|known| nearest(segment, known.keys().map(String::as_str)))
                    .map(str::to_owned),
            }
        })?;
    }
    Ok(schema)
}

/// Checks that `value` is one the setting at `path` takes, before it is written.
pub fn check_setting(path: &[String], value: &Value) -> Result<(), ConfigError> {
    let schema = setting_schema(path)?;
    let joined = join_path(path);
    if is_table(schema) && !value.is_object() {
        return Err(ConfigError::NotWritable(format!(
            "`{joined}` is a section; set one of the settings in it"
        )));
    }
    check(value, schema).map_err(|message| ConfigError::InvalidValue {
        path: joined.clone(),
        message,
    })?;

    // What the schema leaves open is held to its own rules.
    match path
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["appearance", "tokens", token @ ..] if !token.is_empty() => {
            let name = token.join(".");
            match token_def(&name) {
                Some(token) => {
                    TokenValue::parse(token.kind, value).map_err(|message| {
                        ConfigError::InvalidValue {
                            path: joined,
                            message,
                        }
                    })?;
                }
                None if name == PALETTE || is_group(&name) => {}
                None => {
                    return Err(ConfigError::UnknownKey {
                        path: joined,
                        suggestion: meant(&name),
                    });
                }
            }
        }
        // A whole table of one platform's bindings; each is checked when it is read.
        ["keybindings", "windows" | "macos" | "linux"] => {}
        ["keybindings", "windows" | "macos" | "linux", key] | ["keybindings", key] => {
            KeySequence::parse(key).map_err(|message| ConfigError::InvalidValue {
                path: joined,
                message,
            })?;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::schema::CursorStyle;
    use crate::testing::TempDir;

    #[test]
    fn an_accent_is_a_preset_or_a_colour_and_anything_else_is_refused() {
        let path = |key: &str| vec!["appearance".to_owned(), key.to_owned()];
        assert!(check_setting(&path("accent"), &json!("lilac")).is_ok());
        assert!(check_setting(&path("accent"), &json!("#a0b1c2")).is_ok());
        let refused = check_setting(&path("accent"), &json!("#a0b1c2ff")).unwrap_err();
        assert!(
            refused.to_string().contains("`appearance.accent`"),
            "{refused}"
        );
        assert!(refused.to_string().contains("`mint`"), "{refused}");
    }

    #[test]
    fn with_no_files_the_configuration_is_the_defaults_and_nothing_is_written() {
        let dir = TempDir::new("store-empty");
        let store = ConfigStore::load(dir.dirs(), Some("work"));
        let current = store.current();
        assert_eq!(current.config, Config::default());
        assert_eq!(current.problems, vec![]);
        assert_eq!(current.revision, 1);
        assert_eq!(store.workspace().as_deref(), Some("work"));
        assert!(!dir.dirs().config.exists());
    }

    #[test]
    fn the_users_file_and_the_workspaces_are_laid_over_the_defaults() {
        let dir = TempDir::new("store-layers");
        dir.write("config/config.toml", "[font]\nsize = 16\n");
        dir.write(
            "config/workspaces/work.toml",
            "root = \"D:/code\"\n[font]\nsize = 12\n",
        );
        let store = ConfigStore::load(dir.dirs(), Some("work"));
        assert_eq!(store.current().config.font.size, 12.0);
        assert_eq!(store.current().workspace.root.as_deref(), Some("D:/code"));

        // Another workspace has no file, so the user's size holds.
        let other = store.set_workspace(Some("play"));
        assert_eq!(other.config.font.size, 16.0);
        assert_eq!(other.workspace.root, None);
        assert_eq!(other.revision, 2);
        let none = store.set_workspace(None);
        assert_eq!(none.revision, 2, "nothing came out different");
    }

    #[test]
    fn reading_again_takes_a_change_and_keeps_the_old_when_a_file_is_broken() {
        let dir = TempDir::new("store-reload");
        let file = dir.write("config/config.toml", "[font]\nsize = 16\n");
        let store = ConfigStore::load(dir.dirs(), None);
        let first = store.current();

        // Nothing changed: the very same configuration comes back.
        assert!(Arc::ptr_eq(&store.reload().unwrap(), &first));

        std::fs::write(&file, "[font]\nsize = 18\n").unwrap();
        let second = store.reload().unwrap();
        assert_eq!(second.config.font.size, 18.0);
        assert_eq!(second.revision, 2);

        std::fs::write(&file, "[font\nsize = 20\n").unwrap();
        let problems = store.reload().unwrap_err();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].kind, ProblemKind::Syntax);
        assert_eq!(problems[0].line, 1);
        assert!(Arc::ptr_eq(&store.current(), &second));

        // A mistake in a value is not a broken file: the rest of it counts.
        std::fs::write(&file, "[font]\nsize = 20\nweight = \"heavy\"\n").unwrap();
        let third = store.reload().unwrap();
        assert_eq!(third.config.font.size, 20.0);
        assert_eq!(third.problems.len(), 1);
        assert_eq!(third.revision, 3);
    }

    #[test]
    fn a_file_that_is_broken_at_the_start_contributes_nothing_and_says_so() {
        let dir = TempDir::new("store-broken");
        dir.write("config/config.toml", "[font\n");
        dir.write("config/themes/bad.toml", "[color\n");
        let store = ConfigStore::load(dir.dirs(), None);
        let current = store.current();
        assert_eq!(current.config, Config::default());
        assert_eq!(current.problems.len(), 2);
        assert!(current.problems.iter().all(Problem::is_fatal));
    }

    #[test]
    fn a_theme_in_the_users_folder_can_be_chosen_and_a_change_to_it_is_a_change() {
        let dir = TempDir::new("store-themes");
        dir.write("config/config.toml", "[appearance]\ntheme = \"sea\"\n");
        let theme = dir.write(
            "config/themes/sea.toml",
            "scheme = \"dark\"\n[color]\naccent = \"#00aaff\"\n",
        );
        let store = ConfigStore::load(dir.dirs(), None);
        let (tokens, problems) = store.current().tokens(ColorScheme::Light);
        assert_eq!(problems, vec![]);
        assert_eq!(tokens.theme, "sea");
        assert_eq!(tokens.color("color.accent").unwrap().to_string(), "#00aaff");

        std::fs::write(&theme, "scheme = \"dark\"\n[color]\naccent = \"#ff00aa\"\n").unwrap();
        let changed = store.reload().unwrap();
        assert_eq!(changed.revision, 2);
        let (tokens, _) = changed.tokens(ColorScheme::Light);
        assert_eq!(tokens.color("color.accent").unwrap().to_string(), "#ff00aa");
    }

    #[test]
    fn setting_a_value_writes_the_users_file_and_keeps_what_else_is_in_it() {
        let dir = TempDir::new("store-set");
        let file = dir.write("config/config.toml", "# Mine.\n[font]\nsize = 16  # big\n");
        let store = ConfigStore::load(dir.dirs(), None);

        let set = store.set("font.size", &json!(13)).unwrap();
        assert_eq!(set.config.font.size, 13.0);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "# Mine.\n[font]\nsize = 13  # big\n"
        );

        let set = store.set("terminal.cursor-style", &json!("bar")).unwrap();
        assert_eq!(set.config.terminal.cursor_style, CursorStyle::Bar);
        let set = store
            .set("appearance.tokens.color.accent", &json!("#ff8800"))
            .unwrap();
        let (tokens, _) = set.tokens(ColorScheme::Dark);
        assert_eq!(tokens.color("color.accent").unwrap().to_string(), "#ff8800");
        let set = store
            .set("keybindings.\"ctrl+shift+y\"", &json!("lume.tab.new"))
            .unwrap();
        assert!(set.config.keybindings.all.contains_key("ctrl+shift+y"));
        assert!(matches!(
            set.sources["font.size"],
            Source::User { line: 3, .. }
        ));

        let unset = store.unset("font.size").unwrap();
        assert_eq!(unset.config.font.size, 14.0);
        assert_eq!(unset.sources["font.size"], Source::Default);
        assert!(
            std::fs::read_to_string(&file)
                .unwrap()
                .starts_with("# Mine.\n[font]\n")
        );
    }

    #[test]
    fn setting_a_value_makes_the_file_when_there_is_none() {
        let dir = TempDir::new("store-set-new");
        let store = ConfigStore::load(dir.dirs(), None);
        store.set("font.size", &json!(15)).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.dirs().config_file()).unwrap(),
            "[font]\nsize = 15\n"
        );
        // Unsetting what is not set changes nothing.
        let before = store.current();
        assert!(Arc::ptr_eq(&store.unset("font.weight").unwrap(), &before));
    }

    #[test]
    fn what_cannot_be_a_setting_or_a_value_is_refused_before_anything_is_written() {
        let dir = TempDir::new("store-refuse");
        let store = ConfigStore::load(dir.dirs(), None);
        let refused = |path: &str, value: Value| store.set(path, &value).unwrap_err().to_string();

        assert_eq!(
            refused("font.sizee", json!(1)),
            "there is no setting called `font.sizee` (did you mean `size`?)"
        );
        assert_eq!(
            refused("font.size", json!(500)),
            "`font.size` cannot be set to that: must be at most 72"
        );
        assert!(refused("font.size", json!("big")).contains("expected a number"));
        assert!(refused("font", json!(3)).contains("is a section"));
        assert!(refused("font..size", json!(3)).contains("empty key"));
        assert!(
            refused("appearance.tokens.color.acent", json!("#000000"))
                .contains("did you mean `color.accent`")
        );
        assert!(
            refused("appearance.tokens.color.accent", json!("blue")).contains("expected a colour")
        );
        assert!(
            refused("keybindings.\"ctrl+shift\"", json!("lume.tab.new")).contains("only modifiers")
        );
        assert!(
            refused("keybindings.\"ctrl+shift+y\"", json!(7)).contains("cannot be set to that")
        );
        assert!(store.unset("nope.key").is_err());

        assert!(!dir.dirs().config_file().exists());
        assert_eq!(store.current().revision, 1);
    }

    #[test]
    fn a_file_that_is_not_toml_is_not_written_into() {
        let dir = TempDir::new("store-set-broken");
        let file = dir.write("config/config.toml", "[font\n");
        let store = ConfigStore::load(dir.dirs(), None);
        let error = store.set("font.size", &json!(15)).unwrap_err();
        assert!(matches!(error, ConfigError::Syntax { .. }), "{error}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "[font\n");
    }

    #[test]
    fn a_workspaces_folder_is_written_to_its_own_file() {
        let dir = TempDir::new("store-root");
        let store = ConfigStore::load(dir.dirs(), Some("work"));
        store
            .set_workspace_root("work", Some("/home/u/project"))
            .unwrap();
        assert_eq!(
            store.current().workspace.root.as_deref(),
            Some("/home/u/project")
        );
        assert_eq!(
            std::fs::read_to_string(dir.dirs().workspace_file("work")).unwrap(),
            "root = \"/home/u/project\"\n"
        );
        // Another workspace's folder is written without touching the active one.
        store.set_workspace_root("play", Some("/tmp")).unwrap();
        assert_eq!(
            store.current().workspace.root.as_deref(),
            Some("/home/u/project")
        );
        store.set_workspace_root("work", None).unwrap();
        assert_eq!(store.current().workspace.root, None);
        assert!(store.set_workspace_root("a/b", Some("/tmp")).is_err());
    }

    #[test]
    fn a_new_user_is_given_a_file_that_shows_the_settings_and_changes_none() {
        let dir = TempDir::new("store-default-file");
        let store = ConfigStore::load(dir.dirs(), None);
        assert!(store.ensure_default_file().unwrap());
        assert!(!store.ensure_default_file().unwrap());
        let read = store.reload().unwrap();
        assert_eq!(read.config, Config::default());
        assert_eq!(read.problems, vec![]);
    }

    #[test]
    fn the_keys_come_from_the_settings() {
        let dir = TempDir::new("store-keys");
        dir.write(
            "config/config.toml",
            "[keybindings]\n\"ctrl+shift+k\" = \"unbound\"\n",
        );
        let store = ConfigStore::load(dir.dirs(), None);
        let keymap = store.current().keymap(Platform::Windows);
        assert_eq!(keymap.removed.len(), 1);
        assert_eq!(keymap.problems, vec![]);
    }
}
