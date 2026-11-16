//! The workspaces there are, and which of them is on show.
//!
//! The application shows one workspace at a time. Showing another puts the one on show
//! away first: it is written to its file as it stands and its sessions are ended, since
//! a session nobody reads would stop its program as soon as its output filled up. Then
//! the other workspace is read, or begun, and takes the first one's place in the
//! layout, and every window is told the whole of it.
//!
//! A workspace is two files with one name: what the user wrote about it, among the
//! configuration, and how its windows were arranged, among the application's state.
//! Renaming or deleting a workspace does both files, and leaves both as they were if
//! either cannot be done.
//!
//! One more file says which workspace was last on show, for the next start.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lume_config::{Dirs, LayerInput, LayerKind, resolve_layers};
use lume_layout::{
    Intent, Loaded, NewContent, SpawnRequest, Store, StoreError, Workspace, WorkspaceId, apply,
    valid_name,
};
use lume_proto::app::WorkspaceEntry;
use serde::{Deserialize, Serialize};

use crate::carrier::SessionKey;
use crate::error::CommandError;
use crate::panes::{Panes, now_unix_ms};
use crate::tuning::{APP_FILE, WORKSPACE_NAME};

/// What the application remembers from one run to the next, apart from the workspaces
/// themselves.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct AppFile {
    /// The workspace that was last on show.
    #[serde(default)]
    last_workspace: Option<String>,
}

fn app_file(dirs: &Dirs) -> PathBuf {
    dirs.state.join(APP_FILE)
}

/// The workspace that was last on show, if that is known and still a name.
pub fn last_workspace(dirs: &Dirs) -> Option<String> {
    let text = std::fs::read_to_string(app_file(dirs)).ok()?;
    let file: AppFile = serde_json::from_str(&text).ok()?;
    file.last_workspace.filter(|name| valid_name(name))
}

/// Remembers which workspace is on show. Failing to is not worth a word to the user:
/// the next start then shows the workspace before this one.
pub fn remember_workspace(dirs: &Dirs, name: &str) {
    let file = AppFile {
        last_workspace: Some(name.to_owned()),
    };
    let written = serde_json::to_string_pretty(&file)
        .map_err(std::io::Error::other)
        .and_then(|text| {
            std::fs::create_dir_all(&dirs.state)?;
            // Written beside the file and moved over it, as a workspace's file is.
            let beside = dirs.state.join(format!("{APP_FILE}.tmp"));
            std::fs::write(&beside, text + "\n")?;
            std::fs::rename(&beside, app_file(dirs))
        });
    if let Err(error) = written {
        tracing::debug!(%error, "the workspace on show could not be remembered");
    }
}

/// The workspace a start shows: the one asked for on the command line, else the one
/// the configuration names, else the one that was last on show, else the usual one.
/// `configured` is the setting: a workspace's name, or `last`.
pub fn start_workspace(asked: Option<&str>, configured: &str, last: Option<String>) -> String {
    if let Some(name) = asked {
        return name.to_owned();
    }
    let configured = configured.trim();
    if configured != "last" && valid_name(configured) {
        return configured.to_owned();
    }
    last.unwrap_or_else(|| WORKSPACE_NAME.to_owned())
}

/// The folder a workspace's file gives it, for a workspace that is not on show.
fn root_of(dirs: &Dirs, name: &str) -> Option<String> {
    let file = dirs.workspace_file(name);
    let text = std::fs::read_to_string(&file).ok()?;
    let layered = resolve_layers(&[LayerInput {
        kind: LayerKind::Workspace {
            name: name.to_owned(),
        },
        file,
        text,
    }]);
    layered.workspace.root
}

/// An id for a workspace that has none yet: sixteen hexadecimal digits nobody chose.
fn fresh_workspace_id() -> WorkspaceId {
    use std::hash::{BuildHasher, Hasher};

    // The standard library's hasher is keyed afresh in every process, which is all the
    // chance this needs: the id tells workspaces apart, it guards nothing.
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(now_unix_ms());
    hasher.write_u32(std::process::id());
    WorkspaceId::new(hasher.finish() | 1).expect("an odd number is not nought")
}

/// A workspace read from its file, or begun.
pub struct Opened {
    pub workspace: Workspace,
    /// Whether the workspace was begun here: it has no file yet.
    pub fresh: bool,
    /// What to tell the user about the reading, if it did not go well.
    pub warning: Option<String>,
}

/// Reads the workspace called `name`, or begins it. The workspace has at least one
/// window afterwards; the pane of a window made here shows `first`, or a shell.
pub fn open_workspace(store: &Store, name: &str, first: Option<SpawnRequest>) -> Opened {
    let (mut workspace, fresh, warning) = match store.load(name, now_unix_ms()) {
        Ok(Loaded::Workspace(workspace)) => (*workspace, false, None),
        Ok(Loaded::Missing) => (Workspace::new(fresh_workspace_id(), name), true, None),
        Ok(Loaded::SetAside { to, reason }) => {
            tracing::warn!(file = %to.display(), reason, "a workspace's file was set aside");
            let warning = format!(
                "The saved workspace could not be read ({reason}) and a new one was begun. \
                 The file is kept as {}.",
                to.display()
            );
            (
                Workspace::new(fresh_workspace_id(), name),
                true,
                Some(warning),
            )
        }
        Err(error) => {
            // Nothing was moved or lost; the workspace is simply not saved from here if
            // the same fault stops the writing too.
            tracing::error!(%error, "a workspace's file could not be read");
            (Workspace::new(fresh_workspace_id(), name), true, None)
        }
    };
    if workspace.windows.is_empty() {
        // A window that is only being opened has no page yet to say how large its pane
        // is, so its pane gets its session once the page is there.
        let opened = apply(
            &mut workspace,
            Intent::WindowNew {
                content: NewContent::Terminal(first.unwrap_or_default()),
                bounds: None,
            },
            None,
        );
        if let Err(error) = opened {
            tracing::error!(%error, "a workspace's first window could not be made");
        }
    }
    Opened {
        workspace,
        fresh,
        warning,
    }
}

fn store_error(error: StoreError) -> CommandError {
    match &error {
        StoreError::Name(_) => CommandError::bad_request(error.to_string()),
        StoreError::Exists(_) => CommandError::new("exists", error.to_string()),
        StoreError::Missing(_) => CommandError::not_found(error.to_string()),
        StoreError::Io { .. } | StoreError::Encode(_) => CommandError::internal(error.to_string()),
    }
}

fn check_name(name: &str) -> Result<(), CommandError> {
    if valid_name(name) && lume_config::check_workspace_name(name).is_ok() {
        return Ok(());
    }
    Err(CommandError::bad_request(format!(
        "`{name}` cannot be a workspace's name: a name begins with a letter or a digit and \
         goes on with letters, digits, spaces, dots, underscores and hyphens"
    )))
}

/// Moves a file, if it is there. Says whether it was.
fn move_file(from: &Path, to: &Path) -> std::io::Result<bool> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// How a workspace that is asked for is to be found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Find {
    /// It must be there already.
    Existing,
    /// It must not be there yet, and is begun.
    New,
    /// It is shown if it is there and begun if it is not.
    Either,
}

impl Panes {
    /// The workspaces there are, in alphabetical order, each with its folder.
    pub fn workspaces(&self) -> Result<(String, Vec<WorkspaceEntry>), CommandError> {
        let active = self.workspace().name;
        let mut names = self.store.list().map_err(store_error)?;
        // The one on show may not have been written yet.
        if !names.iter().any(|name| name == &active) {
            names.push(active.clone());
            names.sort_by_key(|name| name.to_ascii_lowercase());
        }
        let dirs = self.config.dirs();
        let workspaces = names
            .into_iter()
            .map(|name| {
                let root = if name == active {
                    self.config.current().workspace.root.clone()
                } else {
                    root_of(dirs, &name)
                };
                WorkspaceEntry { name, root }
            })
            .collect();
        Ok((active, workspaces))
    }

    /// Shows the workspace called `name` in place of the one on show. Showing the one
    /// that is on show already changes nothing.
    pub async fn switch(&self, name: &str, find: Find) -> Result<(), CommandError> {
        check_name(name)?;
        let _one_at_a_time = self.switching.lock().await;
        let old = self.workspace();
        let kept = self.store.contains(name).map_err(store_error)?;
        let on_show = old.name.eq_ignore_ascii_case(name);
        match find {
            Find::New if kept || on_show => {
                return Err(CommandError::new(
                    "exists",
                    format!("there is already a workspace called `{name}`"),
                ));
            }
            Find::Existing if !kept && !on_show => {
                return Err(CommandError::not_found(format!(
                    "there is no workspace called `{name}`"
                )));
            }
            _ => {}
        }
        if on_show {
            return Ok(());
        }

        // The one on show is written as it stands, with every pane still in it.
        self.save().await;
        let store = Arc::clone(&self.store);
        let wanted = name.to_owned();
        let opened = tokio::task::spawn_blocking(move || open_workspace(&store, &wanted, None))
            .await
            .map_err(|error| CommandError::internal(error.to_string()))?;
        // A file that differs from the name asked for only in the case of its letters
        // is the same workspace, and the layout takes the name it was read under.
        let name = opened.workspace.name.clone();

        // The settings of the workspace are in effect before its panes are, so the
        // sessions of its panes start where it says.
        self.config.set_workspace(Some(&name));
        self.layout.replace(opened.workspace);
        remember_workspace(self.config.dirs(), &name);
        if let Some(warning) = opened.warning {
            self.notice("warning", warning);
        }

        // The sessions of the workspace that was put away end with it.
        if let Some(state) = self.state.upgrade() {
            for (pane, session) in old.sessions() {
                let Some(found) = old.pane(pane) else {
                    continue;
                };
                let lume_layout::PaneContent::Terminal(terminal) = &found.content else {
                    continue;
                };
                let key = SessionKey {
                    backend: self.backend_of(terminal),
                    session,
                };
                let state = Arc::clone(&state);
                state.runtime().clone().spawn(async move {
                    if let Err(error) = state.end(&key).await {
                        tracing::debug!(session = %key.session, %error, "a session of the workspace that was put away could not be ended");
                    }
                });
            }
        }
        self.sync_tab_panel().await;
        Ok(())
    }

    /// Gives the workspace on show another name: its two files are renamed and the
    /// layout follows. If either file cannot be renamed both keep their names.
    pub async fn rename(&self, to: &str) -> Result<(), CommandError> {
        check_name(to)?;
        let _one_at_a_time = self.switching.lock().await;
        let from = self.workspace().name;
        if from == to {
            return Ok(());
        }
        // Its file is there to be renamed, and is not written again under the name it
        // is losing while that is done.
        self.save().await;
        let _not_written_meanwhile = self.hold_saving().await;
        let dirs = self.config.dirs().clone();
        let store = Arc::clone(&self.store);
        let (old, new) = (from.clone(), to.to_owned());
        tokio::task::spawn_blocking(move || -> Result<(), CommandError> {
            if let Some(taken) =
                store.list().map_err(store_error)?.into_iter().find(|kept| {
                    kept.eq_ignore_ascii_case(&new) && !kept.eq_ignore_ascii_case(&old)
                })
            {
                return Err(CommandError::new(
                    "exists",
                    format!("there is already a workspace called `{taken}`"),
                ));
            }
            // The user's file first: it is the one that may not be there at all.
            let (settings_from, settings_to) =
                (dirs.workspace_file(&old), dirs.workspace_file(&new));
            let moved = move_file(&settings_from, &settings_to).map_err(|error| {
                CommandError::internal(format!("{}: {error}", settings_from.display()))
            })?;
            if let Err(error) = store.rename(&old, &new) {
                if moved {
                    let _ = std::fs::rename(&settings_to, &settings_from);
                }
                return Err(store_error(error));
            }
            Ok(())
        })
        .await
        .map_err(|error| CommandError::internal(error.to_string()))??;

        self.layout
            .apply_core(Intent::WorkspaceRename {
                name: to.to_owned(),
            })
            .await?;
        self.config.set_workspace(Some(to));
        remember_workspace(self.config.dirs(), to);
        Ok(())
    }

    /// Removes a workspace that is not on show: both of its files.
    pub async fn delete(&self, name: &str) -> Result<(), CommandError> {
        check_name(name)?;
        let _one_at_a_time = self.switching.lock().await;
        if self.workspace().name.eq_ignore_ascii_case(name) {
            return Err(CommandError::bad_request(format!(
                "`{name}` is the workspace on show; show another before deleting it"
            )));
        }
        let dirs = self.config.dirs().clone();
        let store = Arc::clone(&self.store);
        let name = name.to_owned();
        tokio::task::spawn_blocking(move || -> Result<(), CommandError> {
            store.delete(&name).map_err(store_error)?;
            let settings = dirs.workspace_file(&name);
            match std::fs::remove_file(&settings) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(CommandError::internal(format!(
                    "{}: {error}",
                    settings.display()
                ))),
            }
        })
        .await
        .map_err(|error| CommandError::internal(error.to_string()))?
    }

    /// Sets the folder the new terminals of the workspace on show start in, or takes
    /// it away.
    pub async fn set_root(&self, root: Option<&str>) -> Result<(), CommandError> {
        let root = root.map(str::trim).filter(|root| !root.is_empty());
        if let Some(root) = root
            && !Path::new(root).is_dir()
        {
            return Err(CommandError::not_found(format!(
                "`{root}` is not a folder that is there"
            )));
        }
        let name = self.workspace().name;
        let config = Arc::clone(&self.config);
        let root = root.map(str::to_owned);
        tokio::task::spawn_blocking(move || config.set_workspace_root(&name, root.as_deref()))
            .await
            .map_err(|error| CommandError::internal(error.to_string()))?
            .map_err(CommandError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn a_start_shows_what_was_asked_for_then_what_is_configured_then_the_last() {
        let last = || Some("site".to_owned());
        assert_eq!(start_workspace(Some("play"), "work", last()), "play");
        assert_eq!(start_workspace(None, "work", last()), "work");
        assert_eq!(start_workspace(None, "last", last()), "site");
        assert_eq!(start_workspace(None, "last", None), WORKSPACE_NAME);
        // A setting that is no name is passed over.
        assert_eq!(start_workspace(None, "../x", last()), "site");
        assert_eq!(start_workspace(None, "", None), WORKSPACE_NAME);
    }

    #[test]
    fn the_layout_and_the_configuration_agree_on_what_a_workspace_may_be_called() {
        // A workspace is a file among the layouts and a file among the settings, and
        // each side checks the name for itself. A name one takes and the other refuses
        // would be a workspace that can be begun and not configured, or the reverse.
        let long = "a".repeat(64);
        let too_long = "a".repeat(65);
        let names = [
            "default",
            "site v2",
            "a.b_c-d",
            "9 lives",
            long.as_str(),
            too_long.as_str(),
            "",
            " lead",
            "trail ",
            "trail.",
            ".hidden",
            "-dash",
            "_under",
            "a/b",
            "a\\b",
            "..",
            "con",
            "CON",
            "nul.txt",
            "com1",
            "lpt9.old",
            "aux",
            "caf\u{e9}",
            "tab\there",
            "a:b",
            "a*b",
        ];
        for name in names {
            assert_eq!(
                valid_name(name),
                lume_config::check_workspace_name(name).is_ok(),
                "{name:?}"
            );
        }
    }

    #[test]
    fn the_workspace_on_show_is_remembered_between_runs() {
        let dir = TempDir::new("appfile");
        let dirs = dir.dirs();
        assert_eq!(last_workspace(&dirs), None);
        remember_workspace(&dirs, "site v2");
        assert_eq!(last_workspace(&dirs), Some("site v2".to_owned()));

        // A file that says something that is no name says nothing.
        std::fs::write(app_file(&dirs), r#"{"last_workspace":"../x"}"#).unwrap();
        assert_eq!(last_workspace(&dirs), None);
        std::fs::write(app_file(&dirs), "not json").unwrap();
        assert_eq!(last_workspace(&dirs), None);
    }

    #[test]
    fn a_workspace_that_is_begun_has_one_window_whose_pane_shows_what_was_asked() {
        let dir = TempDir::new("begin");
        let store = Store::new(dir.dirs().state_workspaces_dir());
        let first = SpawnRequest {
            cwd: Some("/work".to_owned()),
            ..SpawnRequest::default()
        };
        let opened = open_workspace(&store, "play", Some(first));
        assert!(opened.fresh);
        assert_eq!(opened.workspace.name, "play");
        assert_eq!(opened.workspace.windows.len(), 1);
        let panes = opened.workspace.panes();
        let lume_layout::PaneContent::Terminal(terminal) = &panes[0].content else {
            panic!("the first pane is not a terminal");
        };
        assert_eq!(terminal.cwd.as_deref(), Some("/work"));
        assert_eq!(terminal.session, None);
    }

    #[test]
    fn a_workspaces_folder_is_read_from_its_own_file() {
        let dir = TempDir::new("root");
        let dirs = dir.dirs();
        std::fs::create_dir_all(dirs.workspaces_dir()).unwrap();
        std::fs::write(dirs.workspace_file("site"), "root = \"/srv/site\"\n").unwrap();
        assert_eq!(root_of(&dirs, "site"), Some("/srv/site".to_owned()));
        assert_eq!(root_of(&dirs, "none"), None);
    }
}
