//! `lume plugin`: installing plugins, and changing what is installed.
//!
//! These commands work on the plugins folder alone and need no application running. A
//! running application sees what they change in the lock file and follows it.
//!
//! Installing asks first. It shows what the plugin may do, in plain words, and installs
//! nothing until the user says yes; `--yes` says yes ahead, for scripts.
//!
//! A package is a folder or an archive on this machine, or a repository fetched with
//! git, which is then pinned to the commit that was fetched. `update` fetches such a
//! repository again; an update that would let the plugin do more than the user agreed
//! to is always asked about, whatever was said ahead. `link` uses a folder where it
//! lies, for a plugin being worked on.

use std::io::{BufRead, Write};
use std::path::Path;

use lume_config::Dirs;
use lume_plugin::git::{GitSource, looks_like_git};
use lume_plugin::order::State;
use lume_plugin::permissions::{self, Item};
use lume_plugin::store::{
    Change, Check, Direction, InstallOptions, PackageSource, Position, Prepared,
};
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
        /// The folder of a repository the package is in.
        path: Option<String>,
        /// The branch or tag of a repository to take.
        reference: Option<String>,
        local: Option<String>,
        name: Option<String>,
        yes: bool,
    },
    Update {
        /// The install to update; none is every install that follows a repository.
        local: Option<String>,
        yes: bool,
    },
    Link {
        folder: String,
        local: Option<String>,
        name: Option<String>,
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
        .ok_or("lume plugin needs a command: install, update, link, list, remove, enable, disable, rename, bind, order, audit or logs")?;
    let id = |words: &mut std::vec::IntoIter<String>, what: &str| -> Result<String, String> {
        words
            .next()
            .filter(|word| !word.starts_with("--"))
            .ok_or_else(|| format!("lume plugin {command} needs {what}"))
    };
    let action = match command.as_str() {
        "install" => {
            let source = id(
                &mut words,
                "the folder, the .tar.gz archive or the repository to install",
            )?;
            let mut path = None;
            let mut reference = None;
            let mut local = None;
            let mut name = None;
            let mut yes = false;
            while let Some(word) = words.next() {
                match word.as_str() {
                    "--as" => local = Some(words.next().ok_or("--as needs an id")?),
                    "--name" => name = Some(words.next().ok_or("--name needs a name")?),
                    "--path" => {
                        path = Some(
                            words
                                .next()
                                .ok_or("--path needs a folder of the repository")?,
                        );
                    }
                    "--ref" => {
                        reference = Some(words.next().ok_or("--ref needs a branch or a tag")?);
                    }
                    "--yes" | "-y" => yes = true,
                    other => return Err(format!("lume plugin install does not take {other}")),
                }
            }
            Action::Install {
                source,
                path,
                reference,
                local,
                name,
                yes,
            }
        }
        "update" => {
            let mut local = None;
            let mut yes = false;
            for word in words.by_ref() {
                match word.as_str() {
                    "--yes" | "-y" => yes = true,
                    other if other.starts_with("--") || local.is_some() => {
                        return Err(format!("lume plugin update does not take {other}"));
                    }
                    other => local = Some(other.to_owned()),
                }
            }
            Action::Update { local, yes }
        }
        "link" => {
            let folder = id(&mut words, "the folder of the plugin being worked on")?;
            let mut local = None;
            let mut name = None;
            while let Some(word) = words.next() {
                match word.as_str() {
                    "--as" => local = Some(words.next().ok_or("--as needs an id")?),
                    "--name" => name = Some(words.next().ok_or("--name needs a name")?),
                    other => return Err(format!("lume plugin link does not take {other}")),
                }
            }
            Action::Link {
                folder,
                local,
                name,
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
                "{other} is not something lume plugin does (install, update, link, list, \
                 remove, enable, disable, rename, bind, order, audit, logs)"
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
            path,
            reference,
            local,
            name,
            yes,
        } => package_source(&source, path.as_deref(), reference.as_deref()).and_then(|source| {
            install(
                &store,
                source,
                InstallOptions { local, name },
                yes,
                json,
                asker,
                out,
            )
        }),
        Action::Update { local, yes } => update(&store, local.as_deref(), yes, json, asker, out),
        Action::Link {
            folder,
            local,
            name,
        } => store
            .link(Path::new(&folder), InstallOptions { local, name })
            .map(|installed| {
                let mut outcome = Outcome::done(format!(
                    "{} is linked to {}, and is given what its manifest asks for. Lume \
                     follows the folder: a change to its files is loaded as it is saved.",
                    installed.entry.local, installed.entry.source.path
                ));
                outcome
                    .extra
                    .insert("plugin".to_owned(), describe_installed(&installed, None));
                outcome
            })
            .map_err(|error| error.to_string()),
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
    if let Some(commit) = &prepared.source.commit {
        text.push_str(&format!("Commit {commit}.\n"));
    }
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

/// What the user named as a package: a folder or an archive on this machine, or a
/// repository. `--path` and `--ref` belong to a repository only.
fn package_source(
    source: &str,
    path: Option<&str>,
    reference: Option<&str>,
) -> Result<PackageSource, String> {
    if looks_like_git(source) {
        return GitSource::parse(source, reference, path)
            .map(PackageSource::Git)
            .map_err(|error| error.to_string());
    }
    if path.is_some() || reference.is_some() {
        return Err(format!(
            "--path and --ref go with a repository, and {source} is on this machine"
        ));
    }
    Ok(PackageSource::Path(Path::new(source).to_owned()))
}

/// Asks whether to go ahead with what `description` says. Nobody to ask is a refusal
/// that says how to be asked.
fn agreed(
    description: &str,
    what: &str,
    json: bool,
    asker: &mut Asker<'_>,
    out: &mut dyn Write,
) -> Result<bool, String> {
    if json || !asker.interactive {
        return Err(format!(
            "{what} asks first; run it in a terminal, or pass --yes to agree ahead"
        ));
    }
    let _ = write!(out, "{description}\nGo ahead? [y/N] ");
    let _ = out.flush();
    let mut answer = String::new();
    let _ = asker.input.read_line(&mut answer);
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// Fetches the repositories installs follow again, and installs what is new there.
/// Saying yes ahead covers an update that asks for nothing new; one that would let the
/// plugin do more is asked about, and is left alone where there is nobody to ask.
fn update(
    store: &Store,
    only: Option<&str>,
    yes: bool,
    json: bool,
    asker: &mut Asker<'_>,
    out: &mut dyn Write,
) -> Result<Outcome, String> {
    let locals: Vec<String> = match only {
        Some(local) => {
            store.git_source(local).map_err(|error| error.to_string())?;
            vec![local.to_owned()]
        }
        None => store
            .lock()
            .map_err(|error| error.to_string())?
            .plugins
            .iter()
            .filter(|entry| entry.source.is_git())
            .map(|entry| entry.local.clone())
            .collect(),
    };
    if locals.is_empty() {
        return Ok(Outcome::done(
            "No plugin was installed from a repository, so there is nothing to update.".to_owned(),
        ));
    }
    let mut results = Vec::new();
    let mut lines = Vec::new();
    let mut failed = false;
    for local in locals {
        let (word, line) = match update_one(store, &local, yes, json, asker, out) {
            Ok(done) => done,
            Err(problem) => {
                failed = true;
                ("failed", format!("{local}: {problem}"))
            }
        };
        results.push(json!({ "id": local, "result": word, "effect": line }));
        lines.push(line);
    }
    let mut outcome = Outcome::done(lines.join("\n"));
    outcome.extra.insert("updates".to_owned(), json!(results));
    if failed {
        outcome.status = FAILED;
    }
    Ok(outcome)
}

fn update_one(
    store: &Store,
    local: &str,
    yes: bool,
    json: bool,
    asker: &mut Asker<'_>,
    out: &mut dyn Write,
) -> Result<(&'static str, String), String> {
    let prepared = store
        .prepare_update(local)
        .map_err(|error| error.to_string())?;
    if prepared.unchanged {
        return Ok((
            "current",
            format!(
                "{local} {} is what its repository has",
                prepared.manifest.version
            ),
        ));
    }
    let widens = !prepared.new_permissions.is_empty();
    if widens || !yes {
        let description = describe_install(&prepared);
        let asked = if widens && yes {
            agreed(
                &description,
                "an update that asks for more",
                false,
                asker,
                out,
            )
            .map_err(|_| {
                format!(
                    "the update would let it do more than you agreed to ({}); run lume \
                         plugin update {local} in a terminal to see it and agree",
                    granted_lines(&prepared.new_permissions).join("; ")
                )
            })?
        } else {
            agreed(&description, "updating", json, asker, out)?
        };
        if !asked {
            return Ok(("declined", format!("{local} was left as it is")));
        }
    }
    let commit = prepared.source.commit.clone().unwrap_or_default();
    let installed = store.commit(prepared).map_err(|error| error.to_string())?;
    Ok((
        "updated",
        format!(
            "{local} is updated to {} (commit {})",
            installed.entry.version,
            commit.get(..12).unwrap_or(&commit)
        ),
    ))
}

fn install(
    store: &Store,
    source: PackageSource,
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
    if !yes && !agreed(&description, "installing", json, asker, out)? {
        let mut outcome = Outcome::done("Nothing was installed.".to_owned());
        outcome.status = FAILED;
        return Ok(outcome);
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
        "kind": one.entry.source.kind,
        "dir": one.entry.dir,
    });
    if let Some(commit) = &one.entry.source.commit {
        value["commit"] = json!(commit);
    }
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
        let from = if one.entry.is_link() {
            format!("  [dev, linked to {}]", one.entry.source.path)
        } else if let Some(commit) = &one.entry.source.commit {
            format!(
                "  [{} at {}]",
                one.entry.source.path,
                commit.get(..12).unwrap_or(commit)
            )
        } else {
            String::new()
        };
        let _ = writeln!(
            out,
            "  {local}{alias}  {} {}  {state}{from}",
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
                    path: None,
                    reference: None,
                    local: Some("me.x".to_owned()),
                    name: Some("Mine".to_owned()),
                    yes: true
                },
                json: true
            })
        );
        assert_eq!(parsed(&["list"]).unwrap().action, Action::List);
        assert_eq!(
            parsed(&["install", "acme/tools#v1", "--path", "plugins/x", "-y"])
                .unwrap()
                .action,
            Action::Install {
                source: "acme/tools#v1".to_owned(),
                path: Some("plugins/x".to_owned()),
                reference: None,
                local: None,
                name: None,
                yes: true
            }
        );
        assert_eq!(
            parsed(&["update"]).unwrap().action,
            Action::Update {
                local: None,
                yes: false
            }
        );
        assert_eq!(
            parsed(&["update", "a.x", "--yes"]).unwrap().action,
            Action::Update {
                local: Some("a.x".to_owned()),
                yes: true
            }
        );
        assert_eq!(
            parsed(&["link", "./work", "--as", "me.x"]).unwrap().action,
            Action::Link {
                folder: "./work".to_owned(),
                local: Some("me.x".to_owned()),
                name: None
            }
        );
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
            &["update", "a.x", "b.x"],
            &["update", "--force"],
            &["link"],
            &["link", "./work", "--yes"],
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

        // --path and --ref go with a repository.
        let (status, _, err) = run_with(
            &dirs,
            &["install", &source, "--path", "x", "--yes"],
            "",
            false,
        );
        assert_eq!(status, FAILED);
        assert!(err.contains("go with a repository"), "{err}");
    }

    fn git(folder: &std::path::Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(folder)
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.org"])
            .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn an_update_that_lets_the_plugin_do_more_is_asked_about_whatever_was_said_ahead() {
        let dir = scratch("update");
        let dirs = Dirs {
            config: dir.0.join("config"),
            state: dir.0.join("state"),
        };
        let origin = dir.0.join("src");
        let manifest = |version: &str, permissions: &str| {
            std::fs::write(
                origin.join("plugin.toml"),
                format!(
                    "schema = 1\nid = \"acme.metrics\"\nname = \"Metrics\"\n\
                     version = \"{version}\"\napi = \"1\"\n[permissions]\n{permissions}"
                ),
            )
            .unwrap();
        };
        manifest("1.0.0", "\"system.metrics\" = true\n");
        git(&origin, &["init", "--quiet"]);
        git(&origin, &["add", "-A"]);
        git(&origin, &["commit", "--quiet", "-m", "one"]);
        let address = format!(
            "file:///{}",
            origin
                .display()
                .to_string()
                .replace('\\', "/")
                .trim_start_matches('/')
        );

        let (status, out, err) = run_with(&dirs, &["install", &address, "--yes"], "", false);
        assert_eq!(status, DONE, "{out}{err}");
        let (_, out, _) = run_with(&dirs, &["list", "--json"], "", false);
        let listed: Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(listed["plugins"][0]["kind"], "git");
        assert_eq!(listed["plugins"][0]["commit"].as_str().unwrap().len(), 40);

        let (status, out, _) = run_with(&dirs, &["update"], "", false);
        assert_eq!(status, DONE);
        assert!(out.contains("is what its repository has"), "{out}");

        // A newer version that asks for nothing new goes in on a yes said ahead.
        manifest("1.1.0", "\"system.metrics\" = true\n");
        git(&origin, &["commit", "--quiet", "-am", "two"]);
        let (status, out, _) = run_with(&dirs, &["update", "acme.metrics"], "", false);
        assert_eq!(status, FAILED);
        assert!(out.contains("--yes"), "{out}");
        let (status, out, _) = run_with(&dirs, &["update", "acme.metrics", "--yes"], "", false);
        assert_eq!(status, DONE, "{out}");
        assert!(out.contains("is updated to 1.1.0"), "{out}");

        // One that asks for more does not: with nobody to ask it is left alone, and
        // asked, it shows what is new.
        manifest(
            "1.2.0",
            "\"system.metrics\" = true\n\"clipboard.write\" = true\n",
        );
        git(&origin, &["commit", "--quiet", "-am", "three"]);
        let (status, out, _) = run_with(&dirs, &["update", "--yes"], "", false);
        assert_eq!(status, FAILED, "{out}");
        assert!(out.contains("more than you agreed to"), "{out}");
        let (_, out, _) = run_with(&dirs, &["list", "--json"], "", false);
        let listed: Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(listed["plugins"][0]["version"], "1.1.0");

        let (status, out, _) = run_with(&dirs, &["update", "--yes"], "n\n", true);
        assert_eq!(status, DONE, "{out}");
        assert!(out.contains("Update Metrics from 1.1.0 to 1.2.0"), "{out}");
        assert!(out.contains("It will also be able to:"), "{out}");
        assert!(out.contains("was left as it is"), "{out}");
        let (status, out, _) = run_with(&dirs, &["update", "--yes"], "y\n", true);
        assert_eq!(status, DONE, "{out}");
        assert!(out.contains("is updated to 1.2.0"), "{out}");

        // A linked folder is listed as one, and has nothing to update from.
        let work = dir.0.join("work");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(
            work.join("plugin.toml"),
            "schema = 1\nid = \"me.dev\"\nname = \"Dev\"\nversion = \"0.1.0\"\napi = \"1\"\n",
        )
        .unwrap();
        let work = work.display().to_string();
        let (status, out, err) = run_with(&dirs, &["link", &work], "", false);
        assert_eq!(status, DONE, "{out}{err}");
        assert!(out.contains("me.dev is linked to"), "{out}");
        let (_, out, _) = run_with(&dirs, &["list"], "", false);
        assert!(out.contains("[dev, linked to"), "{out}");
        let (status, _, err) = run_with(&dirs, &["update", "me.dev"], "", false);
        assert_eq!(status, FAILED);
        assert!(err.contains("not installed from a repository"), "{err}");
    }
}
