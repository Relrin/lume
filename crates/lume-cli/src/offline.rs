//! The commands on the configuration: print its schema, check it, list the keys that
//! are bound, and make a theme from another terminal's colour scheme.
//!
//! None of them talks to the application. They read the same files the application
//! reads, with the same code, so what they say about a file is what the application
//! would make of it.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use lume_config::generate::pretty_json;
use lume_config::theme::Themes;
use lume_config::{
    ColorScheme, Config, ConfigStore, Dirs, LayerInput, LayerKind, Platform, Problem,
    config_schema, import_ghostty, parse_theme, plugin_setting_problems, resolve_keymap,
    resolve_layers, resolve_tokens, write_when,
};
use serde_json::{Value, json};

use crate::args::{IntegrationShell, Offline};

/// The exit status when the command did what it was asked and found nothing wrong.
const DONE: u8 = 0;

/// The exit status when it could not, or when a check found something wrong.
const FAILED: u8 = 1;

/// The folders whose files are not `config.toml` but are checked all the same.
const WORKSPACES_FOLDER: &str = "workspaces";
const THEMES_FOLDER: &str = "themes";

const NO_DIRECTORY: &str = "the directory for the configuration could not be found";

/// Carries out a command. `dirs` is where the user's files are, if that is known.
pub fn run(command: Offline, dirs: Option<&Dirs>, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let result = match command {
        Offline::Schema => write!(out, "{}", pretty_json(config_schema())).map(|()| DONE),
        Offline::Path => match dirs {
            Some(dirs) => writeln!(out, "{}", dirs.config_file().display()).map(|()| DONE),
            None => writeln!(err, "lume: {NO_DIRECTORY}").map(|()| FAILED),
        },
        Offline::Check { file } => check(file.as_deref(), dirs, out, err),
        Offline::Keymap {
            platform,
            workspace,
            json,
        } => keymap(
            platform.unwrap_or_else(Platform::current),
            workspace.as_deref(),
            json,
            dirs,
            out,
            err,
        ),
        Offline::ThemeImport { file, name } => import(&file, name.as_deref(), dirs, out, err),
        Offline::ShellIntegration { shell } => {
            write!(out, "{}", integration_script(shell)).map(|()| DONE)
        }
    };
    // Output that cannot be written (a closed pipe) is nothing to report, and there
    // would be nowhere to report it.
    result.unwrap_or(FAILED)
}

/// The script that has `shell` report its directory and its commands. It is the one
/// the application loads into shells it starts itself, which is why it is safe to put
/// in a start-up file: loaded a second time, it does nothing.
fn integration_script(shell: IntegrationShell) -> &'static str {
    match shell {
        IntegrationShell::Bash => {
            include_str!("../../../resources/shell-integration/lume.bash")
        }
        IntegrationShell::Zsh => include_str!("../../../resources/shell-integration/lume.zsh"),
        IntegrationShell::Fish => {
            include_str!("../../../resources/shell-integration/lume.fish")
        }
        IntegrationShell::PowerShell => {
            include_str!("../../../resources/shell-integration/lume.ps1")
        }
    }
}

/// What is wrong with the look and the keys under some settings: themes that are not
/// there, tokens and bindings that cannot be read.
fn derived_problems(
    config: &Config,
    sources: &BTreeMap<String, lume_config::Source>,
    themes: &Themes,
) -> Vec<Problem> {
    let mut problems = Vec::new();
    for scheme in [ColorScheme::Dark, ColorScheme::Light] {
        for problem in resolve_tokens(config, sources, themes, scheme).1 {
            if !problems.contains(&problem) {
                problems.push(problem);
            }
        }
    }
    problems.extend(resolve_keymap(config, sources, Platform::current()).problems);
    problems
}

/// The schema of each installed plugin's table of settings, by install: of the plugins
/// that can be used, as the application would find them. Empty where the folders are
/// not known or the installed plugins cannot be read.
fn plugin_fragments(dirs: Option<&Dirs>) -> BTreeMap<String, Value> {
    let Some(dirs) = dirs else {
        return BTreeMap::new();
    };
    let store = lume_plugin::Store::of(dirs);
    match store.resolve(true) {
        Ok((installed, resolution)) => {
            crate::plugin::contributions(&store, &installed, &resolution)
                .settings
                .fragments
        }
        Err(_) => BTreeMap::new(),
    }
}

/// What is wrong with the plugins' settings under some settings.
fn plugin_problems(
    config: &Config,
    sources: &BTreeMap<String, lume_config::Source>,
    fragments: &BTreeMap<String, Value>,
) -> Vec<Problem> {
    plugin_setting_problems(&config.plugins, sources, fragments, &|_| true)
}

/// The name of the folder a file is in.
fn folder(file: &Path) -> Option<&str> {
    file.parent()?.file_name()?.to_str()
}

fn stem(file: &Path) -> String {
    file.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The problems of one file, read as the kind of file its folder says it is.
fn check_file(file: &Path, dirs: Option<&Dirs>) -> Result<Vec<Problem>, String> {
    let text =
        std::fs::read_to_string(file).map_err(|error| format!("{}: {error}", file.display()))?;
    if folder(file) == Some(THEMES_FOLDER) {
        return Ok(match parse_theme(&stem(file), &text, Some(file)) {
            Ok((_, problems)) => problems,
            Err(problem) => vec![problem],
        });
    }
    let kind = if folder(file) == Some(WORKSPACES_FOLDER) {
        LayerKind::Workspace { name: stem(file) }
    } else {
        LayerKind::User
    };
    let layered = resolve_layers(&[LayerInput {
        kind,
        file: file.to_owned(),
        text,
    }]);
    // The user's own themes are known to the file if their folder can be found.
    let themes = dirs.map_or_else(Themes::builtin, |dirs| Themes::load(&dirs.themes_dir()).0);
    let mut problems = layered.fatal;
    problems.extend(layered.problems);
    problems.extend(derived_problems(&layered.config, &layered.sources, &themes));
    problems.extend(plugin_problems(
        &layered.config,
        &layered.sources,
        &plugin_fragments(dirs),
    ));
    Ok(problems)
}

/// The problems of the whole configuration: the user's file, the themes, and each
/// workspace's file laid over the user's.
fn check_all(dirs: &Dirs) -> Vec<Problem> {
    let store = ConfigStore::load(dirs.clone(), None);
    let current = store.current();
    let mut problems = current.problems.clone();
    problems.extend(derived_problems(
        &current.config,
        &current.sources,
        &current.themes,
    ));
    let fragments = plugin_fragments(Some(dirs));
    problems.extend(plugin_problems(
        &current.config,
        &current.sources,
        &fragments,
    ));

    let mut workspaces: Vec<PathBuf> = std::fs::read_dir(dirs.workspaces_dir())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "toml")
        })
        .collect();
    workspaces.sort();
    for file in workspaces {
        let with = store.set_workspace(Some(&stem(&file)));
        let mut found = with.problems.clone();
        found.extend(derived_problems(&with.config, &with.sources, &with.themes));
        found.extend(plugin_problems(&with.config, &with.sources, &fragments));
        // Only what this file adds: the rest has been said.
        for problem in found {
            if !problems.contains(&problem) {
                problems.push(problem);
            }
        }
    }
    problems
}

fn check(
    file: Option<&str>,
    dirs: Option<&Dirs>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> std::io::Result<u8> {
    let problems = match (file, dirs) {
        (Some(file), _) => match check_file(Path::new(file), dirs) {
            Ok(problems) => problems,
            Err(problem) => {
                writeln!(err, "lume: {problem}")?;
                return Ok(FAILED);
            }
        },
        (None, Some(dirs)) => check_all(dirs),
        (None, None) => {
            writeln!(err, "lume: {NO_DIRECTORY}")?;
            return Ok(FAILED);
        }
    };
    for problem in &problems {
        writeln!(out, "{problem}")?;
    }
    match problems.len() {
        0 => writeln!(out, "no problems")?,
        1 => writeln!(out, "1 problem")?,
        many => writeln!(out, "{many} problems")?,
    }
    Ok(if problems.is_empty() { DONE } else { FAILED })
}

fn keymap(
    platform: Platform,
    workspace: Option<&str>,
    as_json: bool,
    dirs: Option<&Dirs>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> std::io::Result<u8> {
    // Without a directory there are no files, and the keys are the defaults.
    let keymap = match dirs {
        Some(dirs) => ConfigStore::load(dirs.clone(), workspace)
            .current()
            .keymap(platform),
        None => resolve_keymap(&Config::default(), &BTreeMap::new(), platform),
    };
    for problem in &keymap.problems {
        writeln!(err, "{problem}")?;
    }
    let status = if keymap.problems.is_empty() {
        DONE
    } else {
        FAILED
    };

    if as_json {
        let all = json!({
            "platform": platform.as_str(),
            "bindings": keymap.bindings,
            "removed": keymap.removed,
        });
        write!(out, "{}", pretty_json(&all))?;
        return Ok(status);
    }

    let command = |command: &str, args: &Value| match args {
        Value::Null => command.to_owned(),
        args => format!("{command} {args}"),
    };
    let rows: Vec<[String; 4]> = keymap
        .bindings
        .iter()
        .map(|binding| {
            [
                binding.keys.display_for(platform),
                command(&binding.command, &binding.args),
                write_when(&binding.when),
                binding.source.to_string(),
            ]
        })
        .collect();
    let heading = ["KEY", "COMMAND", "WHERE", "FROM"];
    let width = |column: usize| {
        rows.iter()
            .map(|row| row[column].chars().count())
            .chain([heading[column].len()])
            .max()
            .unwrap_or(0)
    };
    let widths = [width(0), width(1), width(2)];
    let mut line = |row: &[&str; 4]| {
        writeln!(
            out,
            "{:<a$}  {:<b$}  {:<c$}  {}",
            row[0],
            row[1],
            row[2],
            row[3],
            a = widths[0],
            b = widths[1],
            c = widths[2],
        )
        .map(|()| ())
    };
    line(&heading)?;
    for row in &rows {
        line(&[&row[0], &row[1], &row[2], &row[3]])?;
    }
    if !keymap.removed.is_empty() {
        writeln!(out, "\nTaken away:")?;
        for removed in &keymap.removed {
            writeln!(
                out,
                "{}  {}  by {}",
                removed.binding.keys.display_for(platform),
                command(&removed.binding.command, &removed.binding.args),
                removed.by
            )?;
        }
    }
    Ok(status)
}

fn import(
    file: &str,
    name: Option<&str>,
    dirs: Option<&Dirs>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> std::io::Result<u8> {
    let file = Path::new(file);
    let fail =
        |err: &mut dyn Write, problem: String| writeln!(err, "lume: {problem}").map(|()| FAILED);
    let source = match std::fs::read_to_string(file) {
        Ok(source) => source,
        Err(error) => return fail(err, format!("{}: {error}", file.display())),
    };
    let name = name.map_or_else(|| stem(file), str::to_owned);
    let imported = match import_ghostty(&source, &name) {
        Ok(imported) => imported,
        Err(problem) => return fail(err, format!("{}: {problem}", file.display())),
    };
    let Some(dirs) = dirs else {
        return fail(err, NO_DIRECTORY.to_owned());
    };
    let theme = dirs.themes_dir().join(format!("{}.toml", imported.id));
    if theme.exists() {
        return fail(
            err,
            format!(
                "there is already a theme called `{}` ({}); give this one another name \
                 with --name",
                imported.id,
                theme.display()
            ),
        );
    }
    if let Err(error) = lume_config::write::write_atomic(&theme, &imported.text) {
        return fail(err, format!("{}: {error}", theme.display()));
    }
    for note in &imported.notes {
        writeln!(err, "note: {note}")?;
    }
    writeln!(out, "wrote {}", theme.display())?;
    writeln!(
        out,
        "to use it, set `theme = \"{}\"` under [appearance] in config.toml",
        imported.id
    )?;
    Ok(DONE)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    static NEXT: AtomicU32 = AtomicU32::new(1);

    /// Directories of a test's own, removed when the test is over. The commands are
    /// never pointed at the user's.
    struct Sandbox {
        root: PathBuf,
        dirs: Dirs,
    }

    impl Sandbox {
        fn new(what: &str) -> Self {
            let number = NEXT.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "lume-cli-test-{}-{number}-{what}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            let dirs = Dirs {
                config: root.join("config"),
                state: root.join("state"),
            };
            std::fs::create_dir_all(&dirs.config).unwrap();
            Self { root, dirs }
        }

        fn write(&self, relative: &str, text: &str) -> PathBuf {
            let path = self.root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
            path
        }

        /// Runs a command here and returns its status, what it printed and what it
        /// said was wrong.
        fn run(&self, command: Offline) -> (u8, String, String) {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let status = run(command, Some(&self.dirs), &mut out, &mut err);
            (
                status,
                String::from_utf8(out).unwrap(),
                String::from_utf8(err).unwrap(),
            )
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn check(file: Option<&Path>) -> Offline {
        Offline::Check {
            file: file.map(|file| file.to_string_lossy().into_owned()),
        }
    }

    #[test]
    fn the_schema_is_printed_as_json_and_the_path_is_the_config_files() {
        let sandbox = Sandbox::new("schema");
        let (status, out, err) = sandbox.run(Offline::Schema);
        assert_eq!((status, err.as_str()), (DONE, ""));
        let schema: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(schema["title"], "Lume configuration");
        assert!(schema["properties"]["font"]["properties"]["size"].is_object());

        let (status, out, _) = sandbox.run(Offline::Path);
        assert_eq!(status, DONE);
        assert_eq!(
            out.trim_end(),
            sandbox.dirs.config_file().display().to_string()
        );

        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(run(Offline::Path, None, &mut out, &mut err), FAILED);
        assert!(String::from_utf8(err).unwrap().contains(NO_DIRECTORY));
    }

    #[test]
    fn checking_a_file_says_what_is_wrong_with_the_line_and_what_was_meant() {
        let sandbox = Sandbox::new("check-file");
        let file = sandbox.write(
            "elsewhere/config.toml",
            "[font]\nsizee = 15\nweight = \"heavy\"\n\n[appearance]\ntheme = \"lume-drak\"\n\n\
             [keybindings]\n\"ctrl+shift\" = \"lume.tab.new\"\n",
        );
        let (status, out, err) = sandbox.run(check(Some(&file)));
        assert_eq!((status, err.as_str()), (FAILED, ""));
        let shown = file.display().to_string();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 5, "{out}");
        assert_eq!(
            lines[0],
            format!("{shown}:2:1: unknown key `font.sizee` (did you mean `size`?)")
        );
        assert_eq!(
            lines[1],
            format!("{shown}:3:1: `font.weight`: expected a whole number, not a string")
        );
        assert!(
            lines[2].starts_with(&format!("{shown}:6: ")),
            "{}",
            lines[2]
        );
        assert!(
            lines[2].ends_with("(did you mean `lume-dark`?)"),
            "{}",
            lines[2]
        );
        assert!(
            lines[3].starts_with(&format!("{shown}:9: ")),
            "{}",
            lines[3]
        );
        assert!(lines[3].contains("only modifiers"));
        assert_eq!(lines[4], "4 problems");
    }

    #[test]
    fn a_file_without_fault_passes_and_one_that_is_not_there_fails() {
        let sandbox = Sandbox::new("check-good");
        let file = sandbox.write("config/config.toml", "[font]\nsize = 15\n");
        let (status, out, _) = sandbox.run(check(Some(&file)));
        assert_eq!((status, out.as_str()), (DONE, "no problems\n"));

        let (status, out, err) = sandbox.run(check(Some(&sandbox.root.join("nope.toml"))));
        assert_eq!((status, out.as_str()), (FAILED, ""));
        assert!(err.starts_with("lume: "), "{err}");
    }

    #[test]
    fn workspace_files_and_themes_are_checked_as_what_they_are() {
        let sandbox = Sandbox::new("check-kinds");
        let workspace = sandbox.write(
            "config/workspaces/work.toml",
            "root = \"/home/u\"\n[app]\nkeep-running = true\n",
        );
        let (status, out, _) = sandbox.run(check(Some(&workspace)));
        assert_eq!(status, FAILED);
        assert!(out.contains("`app` is set in config.toml"), "{out}");
        assert!(!out.contains("root"), "{out}");

        let theme = sandbox.write("config/themes/sea.toml", "[color]\nacent = \"#00aaff\"\n");
        let (status, out, _) = sandbox.run(check(Some(&theme)));
        assert_eq!(status, FAILED);
        assert!(
            out.contains("unknown token `color.acent` (did you mean `color.accent`?)"),
            "{out}"
        );
    }

    #[test]
    fn checking_everything_covers_the_users_file_the_themes_and_each_workspace() {
        let sandbox = Sandbox::new("check-all");
        let (status, out, _) = sandbox.run(check(None));
        assert_eq!((status, out.as_str()), (DONE, "no problems\n"));

        sandbox.write("config/config.toml", "[font]\nsizee = 15\n");
        sandbox.write("config/themes/sea.toml", "[color]\nacent = \"#00aaff\"\n");
        sandbox.write(
            "config/workspaces/work.toml",
            "[terminal]\ncursor-style = \"beam\"\n",
        );
        sandbox.write("config/workspaces/play.toml", "[font]\nsize = 12\n");
        let (status, out, _) = sandbox.run(check(None));
        assert_eq!(status, FAILED);
        assert!(out.contains("unknown key `font.sizee`"), "{out}");
        assert!(out.contains("unknown token `color.acent`"), "{out}");
        assert!(
            out.contains("`terminal.cursor-style`: expected one of"),
            "{out}"
        );
        assert!(out.ends_with("3 problems\n"), "{out}");
    }

    #[test]
    fn checking_holds_an_installed_plugins_settings_to_what_its_manifest_says() {
        let sandbox = Sandbox::new("check-plugins");
        sandbox.write(
            "package/plugin.toml",
            "schema = 1\nid = \"acme.agents\"\nname = \"Agents\"\nversion = \"1.0.0\"\n\
             api = \"1\"\n\n[[contributes.settings]]\nkey = \"profiles\"\ntype = \"table\"\n\
             default = {}\n\n[[contributes.settings]]\nkey = \"shown\"\ntype = \"boolean\"\n\
             default = true\n",
        );
        let store = lume_plugin::Store::of(&sandbox.dirs);
        let prepared = store
            .prepare(
                sandbox.root.join("package").as_path(),
                lume_plugin::store::InstallOptions::default(),
            )
            .unwrap();
        store.commit(prepared).unwrap();

        let file = sandbox.write(
            "config/config.toml",
            "[plugins.\"acme.agents\".profiles.fast]\nagent = \"one\"\n\n\
             [plugins.\"gone.x\"]\nanything = 1\n",
        );
        let (status, out, _) = sandbox.run(check(None));
        assert_eq!((status, out.as_str()), (DONE, "no problems\n"));

        sandbox.write(
            "config/config.toml",
            "[plugins.\"acme.agents\"]\nprofiles = 3\nshown = \"yes\"\nmore = 1\n",
        );
        let (status, out, _) = sandbox.run(check(None));
        assert_eq!(status, FAILED);
        assert!(
            out.contains("`plugins.\"acme.agents\".profiles`: expected a table"),
            "{out}"
        );
        assert!(out.contains(":2:"), "{out}");
        assert!(
            out.contains("`plugins.\"acme.agents\".shown`: expected"),
            "{out}"
        );
        assert!(
            out.contains("unknown key `plugins.\"acme.agents\".more`"),
            "{out}"
        );
        assert!(out.ends_with("3 problems\n"), "{out}");
        // One file alone is held to the same.
        let (status, out, _) = sandbox.run(check(Some(&file)));
        assert_eq!(status, FAILED);
        assert!(out.ends_with("3 problems\n"), "{out}");

        // A workspace's file is checked as it lies over the user's.
        sandbox.write("config/config.toml", "");
        sandbox.write(
            "config/workspaces/work.toml",
            "[plugins.\"acme.agents\"]\nprofiles = \"none\"\n",
        );
        let (status, out, _) = sandbox.run(check(None));
        assert_eq!(status, FAILED);
        assert!(out.contains("work.toml"), "{out}");
        assert!(out.ends_with("1 problem\n"), "{out}");
    }

    #[test]
    fn the_keys_are_listed_with_where_they_hold_and_where_they_come_from() {
        let sandbox = Sandbox::new("keymap");
        sandbox.write(
            "config/config.toml",
            "[keybindings]\n\"ctrl+shift+k\" = \"unbound\"\n\"ctrl+a c\" = \"lume.tab.new\"\n",
        );
        let (status, out, err) = sandbox.run(Offline::Keymap {
            platform: Some(Platform::Linux),
            workspace: None,
            json: false,
        });
        assert_eq!((status, err.as_str()), (DONE, ""));
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].starts_with("KEY"), "{out}");
        let copy = lines
            .iter()
            .find(|line| line.starts_with("ctrl+shift+c "))
            .unwrap();
        let fields: Vec<&str> = copy.split_whitespace().collect();
        assert_eq!(
            fields,
            ["ctrl+shift+c", "lume.terminal.copy", "terminal", "default"]
        );
        assert!(lines.iter().any(|line| {
            line.starts_with("ctrl+alt+3 ") && line.contains("lume.tab.activate {\"index\":3}")
        }));
        let own = lines
            .iter()
            .find(|line| line.starts_with("ctrl+a c "))
            .unwrap();
        assert!(
            own.contains("lume.tab.new") && own.contains("user:"),
            "{own}"
        );
        assert!(own.trim_end().ends_with(":3"), "{own}");
        assert!(
            !lines
                .iter()
                .any(|line| line.starts_with("ctrl+shift+k ") && line.contains("default"))
        );
        let taken = lines
            .iter()
            .position(|line| *line == "Taken away:")
            .unwrap();
        assert!(lines[taken + 1].starts_with("ctrl+shift+k  lume.terminal.clear  by user:"));
    }

    #[test]
    fn another_platforms_keys_can_be_listed_and_as_json() {
        let sandbox = Sandbox::new("keymap-json");
        let (status, out, _) = sandbox.run(Offline::Keymap {
            platform: Some(Platform::Macos),
            workspace: None,
            json: true,
        });
        assert_eq!(status, DONE);
        let all: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(all["platform"], "macos");
        let bindings = all["bindings"].as_array().unwrap();
        assert!(bindings.iter().any(|binding| {
            binding["keys"] == json!(["super+c"]) && binding["command"] == "lume.terminal.copy"
        }));
        assert_eq!(all["removed"], json!([]));

        // As a person reads them, the logo key has the platform's name.
        let (_, out, _) = sandbox.run(Offline::Keymap {
            platform: Some(Platform::Macos),
            workspace: None,
            json: false,
        });
        assert!(out.lines().any(|line| line.starts_with("cmd+c ")), "{out}");
    }

    #[test]
    fn a_workspaces_bindings_are_laid_over_the_users_and_a_bad_one_is_said() {
        let sandbox = Sandbox::new("keymap-workspace");
        sandbox.write(
            "config/workspaces/work.toml",
            "[keybindings]\n\"ctrl+shift+y\" = \"lume.window.new\"\n\"hyper+y\" = \"lume.window.new\"\n",
        );
        let list = |workspace: Option<&str>| {
            sandbox.run(Offline::Keymap {
                platform: Some(Platform::Windows),
                workspace: workspace.map(str::to_owned),
                json: false,
            })
        };
        let (status, out, err) = list(Some("work"));
        assert_eq!(status, FAILED);
        assert!(
            out.lines()
                .any(|line| line.starts_with("ctrl+shift+y ") && line.contains("workspace work:")),
            "{out}"
        );
        assert!(err.contains("`hyper` is not a modifier"), "{err}");

        let (status, out, _) = list(None);
        assert_eq!(status, DONE);
        assert!(!out.contains("ctrl+shift+y"));
    }

    const SCHEME: &str =
        "background = #1a1b26\nforeground = #c0caf5\npalette = 4=#7aa2f7\nfont-size = 12\n";

    #[test]
    fn a_colour_scheme_is_imported_into_the_themes_folder_and_can_then_be_chosen() {
        let sandbox = Sandbox::new("import");
        let scheme = sandbox.write("downloads/Tokyo Night", SCHEME);
        let import = |name: Option<&str>| {
            sandbox.run(Offline::ThemeImport {
                file: scheme.to_string_lossy().into_owned(),
                name: name.map(str::to_owned),
            })
        };
        let (status, out, err) = import(None);
        assert_eq!(status, DONE, "{err}");
        let theme = sandbox.dirs.themes_dir().join("tokyo-night.toml");
        assert!(out.contains(&format!("wrote {}", theme.display())), "{out}");
        assert!(out.contains("theme = \"tokyo-night\""), "{out}");
        assert!(err.contains("note: line 4: `font-size`"), "{err}");

        // The theme is one the configuration can now name.
        sandbox.write(
            "config/config.toml",
            "[appearance]\ntheme = \"tokyo-night\"\n",
        );
        let (status, out, _) = sandbox.run(check(None));
        assert_eq!((status, out.as_str()), (DONE, "no problems\n"));

        // It is not written over; another name puts another beside it.
        let (status, _, err) = import(None);
        assert_eq!(status, FAILED);
        assert!(
            err.contains("already a theme called `tokyo-night`"),
            "{err}"
        );
        let (status, _, _) = import(Some("Night Two"));
        assert_eq!(status, DONE);
        assert!(sandbox.dirs.themes_dir().join("night-two.toml").is_file());
    }

    #[test]
    fn what_is_not_a_colour_scheme_is_not_imported() {
        let sandbox = Sandbox::new("import-bad");
        let not_a_scheme = sandbox.write("downloads/notes.txt", "hello\n");
        let (status, out, err) = sandbox.run(Offline::ThemeImport {
            file: not_a_scheme.to_string_lossy().into_owned(),
            name: None,
        });
        assert_eq!((status, out.as_str()), (FAILED, ""));
        assert!(err.contains("does not look like a Ghostty theme"), "{err}");
        assert!(!sandbox.dirs.themes_dir().exists());

        let (status, _, err) = sandbox.run(Offline::ThemeImport {
            file: sandbox.root.join("missing").to_string_lossy().into_owned(),
            name: None,
        });
        assert_eq!(status, FAILED);
        assert!(err.starts_with("lume: "));
    }

    #[test]
    fn the_script_of_each_shell_is_printed_as_it_is() {
        let sandbox = Sandbox::new("integration");
        for shell in [
            IntegrationShell::Bash,
            IntegrationShell::Zsh,
            IntegrationShell::Fish,
            IntegrationShell::PowerShell,
        ] {
            let (status, out, err) = sandbox.run(Offline::ShellIntegration { shell });
            assert_eq!((status, err.as_str()), (DONE, ""), "{shell:?}");
            assert_eq!(out, integration_script(shell));
            // Each reports the directory and marks the prompt, whatever its language.
            assert!(out.contains("file://"), "{shell:?} reports no directory");
            assert!(out.contains("133;"), "{shell:?} marks no prompt");
            // A script that is to be read by a Linux shell has Linux line endings.
            assert!(!out.contains('\r'), "{shell:?} has carriage returns");
        }
    }
}
