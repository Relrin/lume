//! The plugins as the settings window manages them: what is installed and how each
//! fares, turning one on or off, removing it, moving it in the order plugins load in,
//! and installing one from a folder or from a repository, which is fetched with git
//! and can be looked at again later for something newer.
//!
//! Installing goes as `lume plugin install` does: the package is copied and checked
//! first, and the user is shown what it would be able to do before anything is
//! installed. The same plugin installed already is offered to be replaced; another
//! package under the same id is refused, with an id to install it beside the first.
//! Every change goes to the lock file, which the application follows.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use lume_plugin::git::GitSource;
use lume_plugin::id::overlap;
use lume_plugin::order::State;
use lume_plugin::permissions;
use lume_plugin::store::{
    Change, Check, Direction, InstallOptions, PackageSource, Position, Prepared, StoreError,
};
use serde_json::{Value, json};

use super::{BREACH_WINDOW, Plugins};
use crate::error::CommandError;

/// How many lines of a plugin's log, and of its audit entries, the settings window
/// shows.
const TAIL_LINES: usize = 20;

/// Packages copied and checked, waiting for the user's word, by the token the settings
/// window holds.
#[derive(Default)]
pub(super) struct Waiting {
    prepared: Mutex<HashMap<u64, Prepared>>,
}

fn store_error(error: StoreError) -> CommandError {
    match error {
        StoreError::NotInstalled(_) => CommandError::not_found(error.to_string()),
        other => CommandError::new("failed", other.to_string()),
    }
}

fn joined<T>(
    result: Result<Result<T, StoreError>, tokio::task::JoinError>,
) -> Result<T, CommandError> {
    result
        .map_err(|error| CommandError::internal(error.to_string()))?
        .map_err(store_error)
}

/// What a package would be able to do, in words with their warnings.
fn described(items: &[permissions::Item]) -> Vec<Value> {
    permissions::describe_items(items)
        .into_iter()
        .map(|description| json!({ "what": description.what, "warning": description.warning }))
        .collect()
}

/// The newest audit entries of each plugin, with the length and the time of the file
/// they were read from.
#[derive(Default)]
pub(super) struct Audited {
    length: u64,
    written: Option<std::time::SystemTime>,
    entries: Arc<BTreeMap<String, Vec<Value>>>,
}

impl Plugins {
    /// The newest audit entries of each plugin. The file is read again only when it
    /// has been written to since: the settings window asks every few seconds.
    async fn audited(&self) -> Arc<BTreeMap<String, Vec<Value>>> {
        let file = self.store.audit_file();
        let held = {
            let audited = self.audited.lock().unwrap_or_else(PoisonError::into_inner);
            (
                audited.length,
                audited.written,
                Arc::clone(&audited.entries),
            )
        };
        let read = tokio::task::spawn_blocking(move || {
            let (length, written) = std::fs::metadata(&file)
                .map(|metadata| (metadata.len(), metadata.modified().ok()))
                .unwrap_or((0, None));
            if (length, written) == (held.0, held.1) {
                return None;
            }
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            let mut entries: BTreeMap<String, Vec<Value>> = BTreeMap::new();
            for line in text.lines().rev() {
                let Ok(entry) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                let Some(plugin) = entry["plugin"].as_str() else {
                    continue;
                };
                let held = entries.entry(plugin.to_owned()).or_default();
                if held.len() < TAIL_LINES {
                    held.push(entry);
                }
            }
            Some((length, written, Arc::new(entries)))
        })
        .await
        .ok()
        .flatten();
        match read {
            None => held.2,
            Some((length, written, entries)) => {
                *self.audited.lock().unwrap_or_else(PoisonError::into_inner) = Audited {
                    length,
                    written,
                    entries: Arc::clone(&entries),
                };
                entries
            }
        }
    }

    /// Every install, in the order plugins load in, with what the settings window shows
    /// of it: its permissions in words, how it fares, its log and its audit entries.
    pub async fn addons(&self) -> Value {
        let entries = self.audited().await;
        let inner = self.inner();
        let mut order = inner.resolution.order.clone();
        for local in inner.loaded.keys() {
            if !order.contains(local) {
                order.push(local.clone());
            }
        }
        let plugins: Vec<Value> = order
            .iter()
            .filter_map(|local| inner.loaded.get(local).map(|loaded| (local, loaded)))
            .map(|(local, loaded)| {
                let manifest = loaded.manifest();
                let entry = &loaded.installed.entry;
                let state = match &loaded.state {
                    State::Ready => json!({ "ready": true, "reason": null }),
                    State::Inactive(why) => json!({ "ready": false, "reason": why }),
                };
                let bound = inner.resolution.bound.get(local);
                let dependencies: Vec<Value> = manifest
                    .map(|manifest| {
                        manifest
                            .dependencies
                            .iter()
                            .map(|(wanted, dependency)| {
                                let bound_to = bound.and_then(|bound| bound.get(wanted));
                                json!({
                                    "id": wanted,
                                    "optional": dependency.optional(),
                                    "bound": bound_to,
                                    "rebound": entry.bindings.contains_key(wanted),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let granted: Vec<Value> = described(
                    &permissions::items(&loaded.grants.required)
                        .into_iter()
                        .collect::<Vec<_>>(),
                );
                let later: Vec<String> = manifest
                    .map(|manifest| {
                        manifest
                            .requested
                            .optional
                            .iter()
                            .map(permissions::describe_optional)
                            .collect()
                    })
                    .unwrap_or_default();
                let recent = loaded
                    .health
                    .breaches
                    .iter()
                    .filter(|at| at.elapsed() < BREACH_WINDOW)
                    .count();
                let log: Vec<&String> = loaded.log.iter().rev().take(TAIL_LINES).rev().collect();
                json!({
                    "id": local,
                    "manifestId": entry.manifest_id,
                    "name": loaded.installed.name(),
                    "version": entry.version,
                    "description": manifest.and_then(|manifest| manifest.description.clone()),
                    "enabled": entry.enabled,
                    "state": state,
                    "modified": loaded.installed.check == Check::Modified,
                    "missing": loaded.installed.check == Check::Missing,
                    "source": entry.source.path,
                    "sourceKind": entry.source.kind,
                    "commit": entry.source.commit,
                    "dev": entry.is_link(),
                    "dependencies": dependencies,
                    "permissions": granted,
                    "later": later,
                    "health": {
                        "running": loaded.running.is_some(),
                        "starts": loaded.health.starts,
                        "breaches": recent,
                        "lastProblem": loaded.health.last_problem,
                        "warnings": loaded.health.warnings,
                    },
                    "log": log,
                    "audit": entries.get(local).cloned().unwrap_or_default(),
                })
            })
            .collect();
        json!({ "plugins": plugins })
    }

    /// Turns an install on or off.
    pub async fn set_addon_enabled(&self, local: &str, enabled: bool) -> Result<(), CommandError> {
        let store = self.store.clone();
        let owned = local.to_owned();
        joined(tokio::task::spawn_blocking(move || store.set_enabled(&owned, enabled)).await)
    }

    /// Removes an install. Its data and log are kept, as `lume plugin remove` keeps them.
    pub async fn remove_addon(&self, local: &str) -> Result<(), CommandError> {
        let store = self.store.clone();
        let owned = local.to_owned();
        joined(tokio::task::spawn_blocking(move || store.remove(&owned, false).map(|_| ())).await)
    }

    /// Moves an install one place earlier or later in the order the user wants.
    pub async fn move_addon(&self, local: &str, earlier: bool) -> Result<(), CommandError> {
        let store = self.store.clone();
        let owned = local.to_owned();
        joined(
            tokio::task::spawn_blocking(move || {
                let order = store.lock()?.full_order();
                let at = order
                    .iter()
                    .position(|other| *other == owned)
                    .ok_or_else(|| StoreError::NotInstalled(owned.clone()))?;
                let position = if earlier {
                    match at.checked_sub(1).and_then(|before| order.get(before)) {
                        Some(before) => Position::Before(before.clone()),
                        None => return Ok(()),
                    }
                } else {
                    match order.get(at + 1) {
                        Some(after) => Position::After(after.clone()),
                        None => return Ok(()),
                    }
                };
                store.move_to(&owned, position)
            })
            .await,
        )
    }

    /// Copies and checks the package in `folder`, and says what installing it would do:
    /// what it would be able to do, and what it would replace. A package whose id is
    /// another's is not copied: the answer says so, with an id it could be installed
    /// under beside it.
    pub async fn inspect_addon(
        &self,
        folder: String,
        install_as: Option<String>,
    ) -> Result<Value, CommandError> {
        self.inspect(PackageSource::Path(PathBuf::from(folder)), install_as)
            .await
    }

    /// The same for a package in a repository, which is fetched with git: `source` is
    /// its address, with `#<ref>` after it for a branch or a tag, and `path` the folder
    /// of the repository the package is in, if not its top.
    pub async fn inspect_addon_git(
        &self,
        source: String,
        path: Option<String>,
        install_as: Option<String>,
    ) -> Result<Value, CommandError> {
        let path = path.filter(|path| !path.trim().is_empty());
        let source = GitSource::parse(source.trim(), None, path.as_deref())
            .map_err(|error| CommandError::bad_request(error.to_string()))?;
        self.inspect(PackageSource::Git(source), install_as).await
    }

    /// Fetches the repository an install was taken from again, and says what installing
    /// what is there now would do; `current` when it is what is installed.
    pub async fn check_addon_update(&self, local: String) -> Result<Value, CommandError> {
        let store = self.store.clone();
        let prepared =
            joined(tokio::task::spawn_blocking(move || store.prepare_update(&local)).await)?;
        if prepared.unchanged {
            return Ok(json!({ "current": true, "version": prepared.manifest.version }));
        }
        Ok(self.wait_for_word(prepared))
    }

    async fn inspect(
        &self,
        source: PackageSource,
        install_as: Option<String>,
    ) -> Result<Value, CommandError> {
        let store = self.store.clone();
        let prepared = tokio::task::spawn_blocking(move || {
            store.prepare(
                source,
                InstallOptions {
                    local: install_as,
                    name: None,
                },
            )
        })
        .await
        .map_err(|error| CommandError::internal(error.to_string()))?;
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(StoreError::Taken { local, existing }) => {
                let installed: Vec<String> = self.inner().loaded.keys().cloned().collect();
                let suggestion =
                    (2..100)
                        .map(|number| format!("{local}-{number}"))
                        .find(|candidate| {
                            lume_plugin::id::check_plugin_id(candidate).is_ok()
                                && !installed.iter().any(|other| overlap(other, candidate))
                        });
                return Ok(json!({
                    "taken": { "local": local, "existing": existing, "suggestion": suggestion },
                }));
            }
            Err(error) => return Err(store_error(error)),
        };
        Ok(self.wait_for_word(prepared))
    }

    /// Keeps a package that was copied and checked until the user says whether to
    /// install it, and says what installing it would do, with the token to say so by.
    fn wait_for_word(&self, prepared: Prepared) -> Value {
        let manifest = &prepared.manifest;
        let (change, from) = match &prepared.change {
            Change::New => ("new", None),
            Change::Replace {
                from,
                direction: Direction::Newer,
            } => ("newer", Some(from.clone())),
            Change::Replace {
                from,
                direction: Direction::Same,
            } => ("same", Some(from.clone())),
            Change::Replace {
                from,
                direction: Direction::Older,
            } => ("older", Some(from.clone())),
        };
        let answer = json!({
            "id": manifest.id,
            "local": prepared.local,
            "name": prepared.name.clone().unwrap_or_else(|| manifest.name.clone()),
            "version": manifest.version,
            "description": manifest.description,
            "change": change,
            "from": from,
            "permissions": described(&prepared.new_permissions),
            "later": manifest
                .requested
                .optional
                .iter()
                .map(permissions::describe_optional)
                .collect::<Vec<_>>(),
            "warnings": prepared.warnings,
            "leftOut": prepared.left_out,
            "source": prepared.source.path,
            "commit": prepared.source.commit,
        });
        let token = self.next();
        self.waiting
            .prepared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(token, prepared);
        let mut answer = answer;
        answer["token"] = json!(token);
        answer
    }

    /// Installs a package the user agreed to, by the token its inspection gave.
    pub async fn install_addon(&self, token: u64) -> Result<Value, CommandError> {
        let prepared = self
            .waiting
            .prepared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&token)
            .ok_or_else(|| {
                CommandError::not_found("that package is no longer waiting to be installed")
            })?;
        let store = self.store.clone();
        let installed = joined(tokio::task::spawn_blocking(move || store.commit(prepared)).await)?;
        Ok(json!({ "id": installed.entry.local }))
    }

    /// Lets go of a package the user did not install.
    pub fn cancel_addon(&self, token: u64) {
        self.waiting
            .prepared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&token);
    }
}
