//! `lume plugin`: installing plugins, and changing what is installed.
//!
//! These commands work on the plugins folder alone and need no application running. A
//! running application sees what they change in the lock file and follows it.
//!
//! Installing asks first. It shows what the plugin may do, in plain words, and installs
//! nothing until the user says yes; `--yes` says yes ahead, for scripts.

use std::io::{BufRead, Write};
use std::path::Path;

use lume_config::Dirs;
use lume_plugin::order::State;
use lume_plugin::permissions::{self, Item};
use lume_plugin::store::{Change, Check, Direction, InstallOptions, Position, Prepared};
use lume_plugin::{Installed, Store};
use serde_json::{Value, json};

/// The exit status when the command did what it was asked.
const DONE: u8 = 0;

/// The exit status when it was refused or failed.
const FAILED: u8 = 1;

/// How many lines of a log are shown when not told.
const LOG_LINES_DEFAULT: usize = 40;

/// What `lume plugin` was asked to do.
#[derive(Debug, PartialEq, Eq)]
pub struct PluginCommand {
    pub action: Action,
    /// Print one JSON object instead of words.
    pub json: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Install {
        source: String,
        local: Option<String>,
        name: Option<String>,
        yes: bool,
    },
    List,
    Remove {
        local: String,
        purge: bool,
    },
    Enable {
        local: String,
        on: bool,
    },
    Rename {
        local: String,
        name: Option<String>,
    },
    Bind {
        local: String,
        wanted: String,
        target: Option<String>,
    },
    Order {
        local: String,
        position: Position,
    },
    Audit {
        local: String,
    },
    Logs {
        local: String,
        lines: usize,
    },
}

/// Reads the arguments after `lume plugin`.
pub fn parse(words: impl Iterator<Item = String>) -> Result<PluginCommand, String> {
    let mut words = words.peekable();
    let mut json = false;
    let mut rest = Vec::new();
    for word in words.by_ref() {
        if word == "--json" {
            json = true;
        } else {
            rest.push(word);
        }
    }
    let mut words = rest.into_iter();
    let command = words
        .next()
        .ok_or("lume plugin needs a command: install, list, remove, enable, disable, rename, bind, order, audit or logs")?;
    let id = |words: &mut std::vec::IntoIter<String>, what: &str| -> Result<String, String> {
        words
            .next()
            .filter(|word| !word.starts_with("--"))
            .ok_or_else(|| format!("lume plugin {command} needs {what}"))
    };
    let action = match command.as_str() {
        "install" => {
            let source = id(&mut words, "the folder or the .tar.gz archive to install")?;
            let mut local = None;
            let mut name = None;
            let mut yes = false;
            while let Some(word) = words.next() {
                match word.as_str() {
                    "--as" => local = Some(words.next().ok_or("--as needs an id")?),
                    "--name" => name = Some(words.next().ok_or("--name needs a name")?),
                    "--yes" | "-y" => yes = true,
                    other => return Err(format!("lume plugin install does not take {other}")),
                }
            }
            Action::Install {
                source,
                local,
                name,
                yes,
            }
        }
        "list" => Action::List,
        "remove" => {
            let local = id(&mut words, "the id of the plugin to remove")?;
            let purge = match words.next().as_deref() {
                None => false,
                Some("--purge") => true,
                Some(other) => return Err(format!("lume plugin remove does not take {other}")),
            };
            Action::Remove { local, purge }
        }
        "enable" | "disable" => Action::Enable {
            local: id(&mut words, "a plugin's id")?,
            on: command == "enable",
        },
        "rename" => {
            let local = id(&mut words, "a plugin's id")?;
            let name = match words.next() {
                Some(word) if word == "--reset" => None,
                Some(name) => Some(name),
                None => return Err("lume plugin rename needs the new name, or --reset".into()),
            };
            Action::Rename { local, name }
        }
        "bind" => {
            let local = id(&mut words, "a plugin's id")?;
            let binding = words.next().ok_or(
                "lume plugin bind needs <dependency>=<installed id>, or <dependency>= to undo",
            )?;
            let (wanted, target) = binding
                .split_once('=')
                .ok_or("lume plugin bind needs <dependency>=<installed id>")?;
            Action::Bind {
                local,
                wanted: wanted.to_owned(),
                target: (!target.is_empty()).then(|| target.to_owned()),
            }
        }
        "order" => {
            let local = id(&mut words, "a plugin's id")?;
            let position = match words.next().as_deref() {
                Some("--first") => Position::First,
                Some("--last") => Position::Last,
                Some("--before") => Position::Before(words.next().ok_or("--before needs an id")?),
                Some("--after") => Position::After(words.next().ok_or("--after needs an id")?),
                _ => {
                    return Err(
                        "lume plugin order needs --first, --last, --before <id> or --after <id>"
                            .into(),
                    );
                }
            };
            Action::Order { local, position }
        }
        "audit" => Action::Audit {
            local: id(&mut words, "a plugin's id")?,
        },
        "logs" => {
            let local = id(&mut words, "a plugin's id")?;
            let lines = match words.next().as_deref() {
                None => LOG_LINES_DEFAULT,
                Some("--lines") => words
                    .next()
                    .and_then(|count| count.parse().ok())
                    .ok_or("--lines needs a number")?,
                Some(other) => return Err(format!("lume plugin logs does not take {other}")),
            };
            Action::Logs { local, lines }
        }
        other => {
            return Err(format!(
                "{other} is not something lume plugin does (install, list, remove, enable, \
                 disable, rename, bind, order, audit, logs)"
            ));
        }
    };
    if let Some(extra) = words.next() {
        return Err(format!("lume plugin {command} does not take {extra}"));
    }
    Ok(PluginCommand { action, json })
}

/// Where answers to questions come from, and whether anyone is there to give them.
pub struct Asker<'a> {
    pub input: &'a mut dyn BufRead,
    pub interactive: bool,
}

/// Carries out a command. `dirs` is where the user's files are, if that is known.
pub fn run(
    command: PluginCommand,
    dirs: Option<&Dirs>,
    asker: &mut Asker<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let Some(dirs) = dirs else {
        let _ = writeln!(
            err,
            "lume: the directory for the application's state could not be found"
        );
        return FAILED;
    };
    let store = Store::of(dirs);
    let json = command.json;
    let result = match command.action {
        Action::Install {
            source,
            local,
            name,
            yes,
        } => install(
            &store,
            Path::new(&source),
            InstallOptions { local, name },
            yes,
            json,
            asker,
            out,
        ),
        Action::List => list(&store, json, out),
        Action::Remove { local, purge } => store
            .remove(&local, purge)
            .map(|entry| {
                let effect = if purge {
                    format!(
                        "{local} {} was removed, with its data and log",
                        entry.version
                    )
                } else {
                    format!("{local} {} was removed; its data is kept", entry.version)
                };
                Outcome::done(effect)
            })
            .map_err(|error| error.to_string()),
        Action::Enable { local, on } => store
            .set_enabled(&local, on)
            .map(|()| {
                Outcome::done(format!(
                    "{local} is turned {}",
                    if on { "on" } else { "off" }
                ))
            })
            .map_err(|error| error.to_string()),
        Action::Rename { local, name } => {
            let effect = match &name {
                Some(name) => format!("{local} is called {name}"),
                None => format!("{local} is called what its manifest calls it"),
            };
            store
                .rename(&local, name)
                .map(|()| Outcome::done(effect))
                .map_err(|error| error.to_string())
        }
        Action::Bind {
            local,
            wanted,
            target,
        } => {
            let effect = match &target {
                Some(target) => format!("{local} uses {target} for {wanted}"),
                None => format!("{local} uses the plugin installed as {wanted}"),
            };
            store
                .bind(&local, &wanted, target.as_deref())
                .map(|()| Outcome::done(effect))
                .map_err(|error| error.to_string())
        }
        Action::Order { local, position } => store
            .move_to(&local, position)
            .and_then(|()| store.resolve(false))
            .map(|(_, resolution)| {
                let mut outcome = Outcome::done(format!(
                    "the plugins load in this order: {}",
                    resolution.order.join(", ")
                ));
                outcome
                    .extra
                    .insert("order".to_owned(), json!(resolution.order));
                outcome
            })
            .map_err(|error| error.to_string()),
        Action::Audit { local } => audit(&store, &local, json, out),
        Action::Logs { local, lines } => logs(&store, &local, lines, out),
    };
    match result {
        Ok(outcome) => {
            if json {
                let mut object = json!({ "ok": true, "effect": outcome.effect });
                if let Some(map) = object.as_object_mut() {
                    map.extend(outcome.extra);
                }
                let _ = writeln!(out, "{object}");
            } else if !outcome.effect.is_empty() {
                let _ = writeln!(out, "{}", outcome.effect);
            }
            outcome.status
        }
        Err(problem) => {
            if json {
                let _ = writeln!(out, "{}", json!({ "ok": false, "error": problem }));
            } else {
                let _ = writeln!(err, "lume: {problem}");
            }
            FAILED
        }
    }
}

/// What a command did.
struct Outcome {
    effect: String,
    extra: serde_json::Map<String, Value>,
    status: u8,
}

impl Outcome {
    fn done(effect: String) -> Self {
        Self {
            effect,
            extra: serde_json::Map::new(),
            status: DONE,
        }
    }
}

/// One line for each thing granted.
fn granted_lines(items: &[Item]) -> Vec<String> {
    permissions::describe_items(items)
        .into_iter()
        .map(|description| description.what)
        .collect()
}

/// What installing a prepared package would do, in words.
fn describe_install(prepared: &Prepared) -> String {
    let manifest = &prepared.manifest;
    let shown = prepared
        .name
        .clone()
        .unwrap_or_else(|| manifest.name.clone());
    let mut text = String::new();
    let heading = match &prepared.change {
        Change::New => format!("Install {shown} {}", manifest.version),
        Change::Replace {
            from,
            direction: Direction::Newer,
        } => format!("Update {shown} from {from} to {}", manifest.version),
        Change::Replace {
            direction: Direction::Same,
            ..
        } => format!("Install {shown} {} again", manifest.version),
        Change::Replace {
            from,
            direction: Direction::Older,
        } => format!("Go back from {shown} {from} to {}", manifest.version),
    };
    let under = if prepared.local == manifest.id {
        format!(" ({})", prepared.local)
    } else {
        format!(" ({}, installed as {})", manifest.id, prepared.local)
    };
    text.push_str(&format!(
        "{heading}{under}, from {}.\n",
        prepared.source.path
    ));
    if let Some(description) = &manifest.description {
        text.push_str(&format!("{description}\n"));
    }
    text.push('\n');
    let lines = granted_lines(&prepared.new_permissions);
    match (&prepared.change, lines.is_empty()) {
        (Change::New, true) => text.push_str("It asks for no permissions.\n"),
        (Change::New, false) => text.push_str("It will be able to:\n"),
        (Change::Replace { .. }, true) => text.push_str("It asks for nothing new.\n"),
        (Change::Replace { .. }, false) => text.push_str("It will also be able to:\n"),
    }
    for line in lines {
        text.push_str(&format!("  - {line}\n"));
    }
    if !manifest.requested.optional.is_empty() {
        text.push_str("\nIt may ask you later to:\n");
        for optional in &manifest.requested.optional {
            text.push_str(&format!(
                "  - {}\n",
                permissions::describe_optional(optional)
            ));
        }
    }
    for warning in &prepared.warnings {
        text.push_str(&format!("\nNote: {warning}\n"));
    }
    if !prepared.left_out.is_empty() {
        text.push_str(&format!(
            "\nLeft out of the package: {}\n",
            prepared.left_out.join(", ")
        ));
    }
    text
}

fn install(
    store: &Store,
    source: &Path,
    options: InstallOptions,
    yes: bool,
    json: bool,
    asker: &mut Asker<'_>,
    out: &mut dyn Write,
) -> Result<Outcome, String> {
    let prepared = store
        .prepare(source, options)
        .map_err(|error| error.to_string())?;
    let description = describe_install(&prepared);
    if !yes {
        if json || !asker.interactive {
            return Err(
                "installing asks first; run lume plugin install in a terminal, or pass --yes \
                 to agree ahead"
                    .to_owned(),
            );
        }
        let _ = write!(out, "{description}\nGo ahead? [y/N] ");
        let _ = out.flush();
        let mut answer = String::new();
        let _ = asker.input.read_line(&mut answer);
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            let mut outcome = Outcome::done("Nothing was installed.".to_owned());
            outcome.status = FAILED;
            return Ok(outcome);
        }
    }
    let local = prepared.local.clone();
    let installed = store.commit(prepared).map_err(|error| error.to_string())?;
    let mut outcome = Outcome::done(format!(
        "{local} {} is installed{}",
        installed.entry.version,
        if installed.entry.enabled {
            ""
        } else {
            " (turned off, as it was)"
        }
    ));
    if json {
        outcome
            .extra
            .insert("description".to_owned(), json!(description));
        outcome
            .extra
            .insert("plugin".to_owned(), describe_installed(&installed, None));
    }
    Ok(outcome)
}

fn check_word(check: &Check) -> &'static str {
    match check {
        Check::Intact => "intact",
        Check::Modified => "modified",
        Check::Missing => "missing",
        Check::Unreadable(_) => "unreadable",
        Check::Unchecked => "unchecked",
    }
}

fn describe_installed(one: &Installed, state: Option<&State>) -> Value {
    let manifest = one.manifest.as_ref().ok();
    let mut value = json!({
        "id": one.entry.local,
        "manifest_id": one.entry.manifest_id,
        "name": one.name(),
        "version": one.entry.version,
        "enabled": one.entry.enabled,
        "check": check_word(&one.check),
        "source": one.entry.source.path,
        "dir": one.entry.dir,
    });
    if let Some(manifest) = manifest {
        value["permissions"] = json!(
            manifest
                .requested
                .required
                .iter()
                .map(|request| permissions::describe(request).what)
                .collect::<Vec<_>>()
        );
    }
    if !one.entry.bindings.is_empty() {
        value["bindings"] = json!(one.entry.bindings);
    }
    match state {
        Some(State::Ready) => value["state"] = json!("ready"),
        Some(State::Inactive(why)) => {
            value["state"] = json!("inactive");
            value["reason"] = json!(why);
        }
        None => {}
    }
    value
}

/// The application's commands, as the command line knows them: by name only. Whether a
/// plugin may run one of them is the running application's to say, so here every one
/// is taken to be there and open.
struct Unchecked;

impl lume_plugin::contributions::Commands for Unchecked {
    fn builtin(&self, id: &str) -> Option<(String, bool)> {
        Some((id.to_owned(), true))
    }
}

/// What could not be placed or read of what the plugins that can be used add, by
/// install, as the application would find it.
fn contribution_warnings(
    store: &Store,
    installed: &[Installed],
    resolution: &lume_plugin::order::Resolution,
) -> Vec<(String, String)> {
    let empty = std::collections::BTreeMap::new();
    let installs: Vec<lume_plugin::contributions::Install<'_>> = resolution
        .order
        .iter()
        .filter(|local| resolution.is_ready(local))
        .filter_map(|local| installed.iter().find(|one| one.entry.local == *local))
        .filter_map(|one| {
            let local = one.entry.local.as_str();
            Some(lume_plugin::contributions::Install {
                local,
                name: one.name(),
                manifest: one.manifest.as_ref().ok()?,
                dir: store.package_dir(&one.entry),
                content: &one.entry.content_hash,
                rank: resolution
                    .rank(local)
                    .and_then(|rank| i64::try_from(rank).ok())
                    .unwrap_or(i64::MAX),
                bound: resolution.bound.get(local).unwrap_or(&empty),
            })
        })
        .collect();
    lume_plugin::contributions::build(
        &installs,
        &Unchecked,
        &mut lume_plugin::contributions::Read::default(),
    )
    .warnings
}

fn list(store: &Store, json: bool, out: &mut dyn Write) -> Result<Outcome, String> {
    let (installed, mut resolution) = store.resolve(true).map_err(|error| error.to_string())?;
    let added = contribution_warnings(store, &installed, &resolution);
    resolution.warnings.extend(added);
    let by_id = |local: &str| installed.iter().find(|one| one.entry.local == local);
    if json {
        let plugins: Vec<Value> = resolution
            .order
            .iter()
            .filter_map(|local| by_id(local))
            .map(|one| {
                let mut value = describe_installed(one, resolution.states.get(&one.entry.local));
                let warnings: Vec<&str> = resolution
                    .warnings
                    .iter()
                    .filter(|(local, _)| *local == one.entry.local)
                    .map(|(_, warning)| warning.as_str())
                    .collect();
                if !warnings.is_empty() {
                    value["warnings"] = json!(warnings);
                }
                value
            })
            .collect();
        let mut outcome = Outcome::done(format!("{} installed", plugins.len()));
        outcome.extra.insert("plugins".to_owned(), json!(plugins));
        return Ok(outcome);
    }
    if installed.is_empty() {
        return Ok(Outcome::done("No plugins are installed.".to_owned()));
    }
    let _ = writeln!(out, "In the order they load in:");
    for local in &resolution.order {
        let Some(one) = by_id(local) else { continue };
        let state = match resolution.states.get(local) {
            Some(State::Ready) => "ready".to_owned(),
            Some(State::Inactive(why)) => format!("not used: {why}"),
            None => String::new(),
        };
        let alias = if one.entry.local == one.entry.manifest_id {
            String::new()
        } else {
            format!(" (from {})", one.entry.manifest_id)
        };
        let _ = writeln!(
            out,
            "  {local}{alias}  {} {}  {state}",
            one.name(),
            one.entry.version
        );
        for (wanted, target) in &one.entry.bindings {
            let _ = writeln!(out, "      uses {target} for {wanted}");
        }
        for (_, warning) in resolution.warnings.iter().filter(|(of, _)| of == local) {
            let _ = writeln!(out, "      note: {warning}");
        }
    }
    Ok(Outcome::done(String::new()))
}

fn audit(store: &Store, local: &str, json: bool, out: &mut dyn Write) -> Result<Outcome, String> {
    let text = match std::fs::read_to_string(store.audit_file()) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("{}: {error}", store.audit_file().display())),
    };
    let entries: Vec<Value> = text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry["plugin"] == local)
        .collect();
    if json {
        let mut outcome = Outcome::done(format!("{} entries", entries.len()));
        outcome.extra.insert("entries".to_owned(), json!(entries));
        return Ok(outcome);
    }
    if entries.is_empty() {
        return Ok(Outcome::done(format!("Nothing is recorded for {local}.")));
    }
    for entry in &entries {
        let _ = writeln!(
            out,
            "{}  {}  {}  {}",
            entry["time"].as_str().unwrap_or(""),
            entry["capability"].as_str().unwrap_or(""),
            entry["scope"].as_str().unwrap_or(""),
            entry["outcome"].as_str().unwrap_or("")
        );
    }
    Ok(Outcome::done(String::new()))
}

fn logs(store: &Store, local: &str, lines: usize, out: &mut dyn Write) -> Result<Outcome, String> {
    let path = store.log_file(local);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Outcome::done(format!("{local} has logged nothing.")));
        }
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let all: Vec<&str> = text.lines().collect();
    for line in &all[all.len().saturating_sub(lines)..] {
        let _ = writeln!(out, "{line}");
    }
    Ok(Outcome::done(String::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Result<PluginCommand, String> {
        parse(args.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn the_commands_are_read() {
        assert_eq!(
            parsed(&[
                "install", "./p", "--as", "me.x", "--name", "Mine", "--yes", "--json"
            ]),
            Ok(PluginCommand {
                action: Action::Install {
                    source: "./p".to_owned(),
                    local: Some("me.x".to_owned()),
                    name: Some("Mine".to_owned()),
                    yes: true
                },
                json: true
            })
        );
        assert_eq!(parsed(&["list"]).unwrap().action, Action::List);
        assert_eq!(
            parsed(&["disable", "a.x"]).unwrap().action,
            Action::Enable {
                local: "a.x".to_owned(),
                on: false
            }
        );
        assert_eq!(
            parsed(&["bind", "a.x", "core.x="]).unwrap().action,
            Action::Bind {
                local: "a.x".to_owned(),
                wanted: "core.x".to_owned(),
                target: None
            }
        );
        assert_eq!(
            parsed(&["order", "a.x", "--after", "b.x"]).unwrap().action,
            Action::Order {
                local: "a.x".to_owned(),
                position: Position::After("b.x".to_owned())
            }
        );
        assert_eq!(
            parsed(&["rename", "a.x", "--reset"]).unwrap().action,
            Action::Rename {
                local: "a.x".to_owned(),
                name: None
            }
        );
        for bad in [
            &["install"][..],
            &["remove"],
            &["order", "a.x"],
            &["bind", "a.x", "core.x"],
            &["list", "extra"],
            &["update", "a.x"],
            &[],
        ] {
            assert!(parsed(bad).is_err(), "{bad:?}");
        }
    }

    struct Scratch(std::path::PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(what: &str) -> Scratch {
        let path =
            std::env::temp_dir().join(format!("lume-cli-plugin-{}-{what}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("src")).unwrap();
        std::fs::write(
            path.join("src/plugin.toml"),
            "schema = 1\nid = \"acme.metrics\"\nname = \"Metrics\"\nversion = \"1.0.0\"\n\
             api = \"1\"\n[permissions]\n\"system.metrics\" = true\n\"process.run\" = [\"git\"]\n",
        )
        .unwrap();
        Scratch(path)
    }

    fn run_with(
        dirs: &Dirs,
        args: &[&str],
        answer: &str,
        interactive: bool,
    ) -> (u8, String, String) {
        let mut input = std::io::Cursor::new(answer.as_bytes().to_vec());
        let mut asker = Asker {
            input: &mut input,
            interactive,
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let status = run(
            parsed(args).unwrap(),
            Some(dirs),
            &mut asker,
            &mut out,
            &mut err,
        );
        (
            status,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn installing_shows_what_the_plugin_may_do_and_waits_for_a_yes() {
        let dir = scratch("install");
        let dirs = Dirs {
            config: dir.0.join("config"),
            state: dir.0.join("state"),
        };
        let source = dir.0.join("src").display().to_string();

        let (status, out, _) = run_with(&dirs, &["install", &source], "n\n", true);
        assert_eq!(status, FAILED);
        assert!(
            out.contains("Install Metrics 1.0.0 (acme.metrics)"),
            "{out}"
        );
        assert!(out.contains("See how busy the processor"), "{out}");
        assert!(
            out.contains("Run these programs, with any arguments: git"),
            "{out}"
        );
        assert!(
            out.contains("Note: A program such as a shell or git"),
            "{out}"
        );
        assert!(out.contains("Nothing was installed."), "{out}");

        let (status, _, err) = run_with(&dirs, &["install", &source], "", false);
        assert_eq!(status, FAILED);
        assert!(err.contains("--yes"), "{err}");

        let (status, out, _) = run_with(&dirs, &["install", &source], "y\n", true);
        assert_eq!(status, DONE, "{out}");
        assert!(out.contains("acme.metrics 1.0.0 is installed"), "{out}");

        let (status, out, _) = run_with(&dirs, &["list", "--json"], "", false);
        assert_eq!(status, DONE);
        let listed: Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(listed["plugins"][0]["id"], "acme.metrics");
        assert_eq!(listed["plugins"][0]["state"], "ready");
        assert_eq!(listed["plugins"][0]["check"], "intact");

        let (status, out, _) = run_with(&dirs, &["install", &source, "--yes"], "", false);
        assert_eq!(status, DONE, "{out}");

        let (status, out, _) = run_with(&dirs, &["disable", "acme.metrics", "--json"], "", false);
        assert_eq!(status, DONE);
        assert!(out.contains("\"ok\":true"), "{out}");
        let (_, out, _) = run_with(&dirs, &["list"], "", false);
        assert!(out.contains("not used: it is turned off"), "{out}");

        let (status, _, err) = run_with(&dirs, &["remove", "nobody.here"], "", false);
        assert_eq!(status, FAILED);
        assert!(
            err.contains("no plugin is installed as nobody.here"),
            "{err}"
        );
        let (status, _, _) = run_with(&dirs, &["remove", "acme.metrics", "--purge"], "", false);
        assert_eq!(status, DONE);
    }
}
