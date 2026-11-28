//! The workspaces there are, and which of them are open.
//!
//! The layout is held by one workspace: the one the application started with, whose
//! file has the windows. Other workspaces are opened beside it. Opening one reads its
//! file and puts all its tabs after the last tab of the window the user works in,
//! marked as its own, so the tab strip shows the tabs of every open workspace. No
//! window closes and no session ends. Each open workspace is written back to its own
//! file, and closing one takes its tabs out of the strip and ends its sessions, unless
//! the settings keep them.
//!
//! The current workspace is the one of the tab on show in the window the user works
//! in. Its settings are in effect, so new terminals start where it says, and new tabs
//! belong to it.
//!
//! While a server's layout is on show, a workspace of the application's own takes its
//! place whole, in the same window.
//!
//! A workspace is two files with one name: what the user wrote about it, among the
//! configuration, and how its windows were arranged, among the application's state.
//! Renaming or deleting a workspace does both files, and leaves both as they were if
//! either cannot be done.
//!
//! One more file says which workspace holds the layout and which others are open, for
//! the next start.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lume_config::schema::OnSwitch;
use lume_config::{Dirs, LayerInput, LayerKind, resolve_layers};
use lume_layout::{
    Group, GroupId, Intent, Loaded, NewContent, PaneContent, Shelved, SpawnRequest, Store,
    StoreError, Tab, Workspace, WorkspaceId, all_tabs, apply, check, keep_window_ids, valid_name,
};
use lume_proto::app::WorkspaceEntry;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::carrier::SessionKey;
use crate::error::CommandError;
use crate::panes::{Panes, lock, now_unix_ms};
use crate::tuning::{APP_FILE, WORKSPACE_NAME};

/// What the application remembers from one run to the next, apart from the workspaces
/// themselves.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct AppFile {
    /// The workspace that last held the layout.
    #[serde(default)]
    last_workspace: Option<String>,
    /// The workspaces that were open beside it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    open: Vec<String>,
    /// The workspace that was current, when that was one open beside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    current: Option<String>,
}

fn app_file(dirs: &Dirs) -> PathBuf {
    dirs.state.join(APP_FILE)
}

fn read_app_file(dirs: &Dirs) -> AppFile {
    std::fs::read_to_string(app_file(dirs))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// The workspace that last held the layout, if that is known and still a name.
pub fn last_workspace(dirs: &Dirs) -> Option<String> {
    read_app_file(dirs)
        .last_workspace
        .filter(|name| valid_name(name))
}

/// The workspaces that were open beside the one that held the layout, those that are
/// still names.
pub fn open_workspaces(dirs: &Dirs) -> Vec<String> {
    read_app_file(dirs)
        .open
        .into_iter()
        .filter(|name| valid_name(name))
        .collect()
}

/// Remembers which workspace holds the layout. Failing to is not worth a word to the
/// user: the next start then shows the workspace before this one.
pub fn remember_workspace(dirs: &Dirs, name: &str) {
    let mut file = read_app_file(dirs);
    if file.last_workspace.as_deref() != Some(name) {
        // Those open beside another are not open beside this one.
        file.open.clear();
    }
    file.last_workspace = Some(name.to_owned());
    write_app_file(dirs, &file);
}

/// The workspace that was current, if it was one open beside the one that held the
/// layout.
pub fn current_workspace(dirs: &Dirs) -> Option<String> {
    read_app_file(dirs).current.filter(|name| valid_name(name))
}

/// Remembers which workspace is current, `None` for the one that holds the layout.
pub fn remember_current(dirs: &Dirs, current: Option<&str>) {
    let mut file = read_app_file(dirs);
    if file.current.as_deref() == current {
        return;
    }
    file.current = current.map(str::to_owned);
    write_app_file(dirs, &file);
}

/// Remembers which workspace holds the layout and which are open beside it.
pub fn remember_open(dirs: &Dirs, holds: &str, open: Vec<String>) {
    let mut file = read_app_file(dirs);
    file.last_workspace = Some(holds.to_owned());
    file.open = open;
    write_app_file(dirs, &file);
}

fn write_app_file(dirs: &Dirs, file: &AppFile) {
    let written = serde_json::to_string_pretty(file)
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

/// What the configuration calls the workspace that was last on show.
pub const LAST: &str = "last";

/// The workspace a start shows: the one asked for on the command line, else the one
/// the configuration names, else the one that was last on show, else the usual one.
/// `configured` is the setting: a workspace's name, or `last`.
pub fn start_workspace(asked: Option<&str>, configured: &str, last: Option<String>) -> String {
    if let Some(name) = asked {
        return name.to_owned();
    }
    let configured = configured.trim();
    if configured != LAST && valid_name(configured) {
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

/// One new tab with a shell, started in `root` if there is one, as a workspace that
/// has none is given.
fn fresh_tabs(root: Option<String>) -> (Vec<Tab>, Vec<Group>) {
    let mut scratch = Workspace::new(fresh_workspace_id(), WORKSPACE_NAME);
    let made = apply(
        &mut scratch,
        Intent::WindowNew {
            content: NewContent::Terminal(SpawnRequest {
                cwd: root,
                ..SpawnRequest::default()
            }),
            bounds: None,
        },
        None,
    );
    if let Err(error) = made {
        tracing::error!(%error, "a new tab could not be made");
    }
    all_tabs(&scratch)
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
        // is: its session is started at the size the pane is expected to have, and
        // resized once the page is there.
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

/// A group of a workspace and its tabs, as it is moved to another.
fn group_of(workspace: &Workspace, group: GroupId) -> Option<Shelved> {
    let window = workspace
        .windows
        .iter()
        .find(|window| window.centre.group(group).is_some())?;
    Some(Shelved {
        group: window.centre.group(group)?.clone(),
        tabs: window
            .centre
            .tabs
            .iter()
            .filter(|tab| tab.group == Some(group))
            .cloned()
            .collect(),
    })
}

/// What a group is called where the user reads it.
fn shown_name(shelved: &Shelved) -> String {
    shelved
        .group
        .name
        .clone()
        .unwrap_or_else(|| "Unnamed group".to_owned())
}

/// Puts a group at the end of the window of a workspace the user last worked in, and
/// says what it is called there.
fn graft(workspace: &mut Workspace, shelved: Shelved) -> Result<GroupId, CommandError> {
    let window = workspace
        .focused_window
        .or_else(|| workspace.windows.first().map(|window| window.id))
        .ok_or_else(|| CommandError::bad_request("that workspace has no window to put it in"))?;
    let (_, created) = apply(
        workspace,
        Intent::GroupGraft {
            window,
            group: Box::new(shelved),
        },
        None,
    )?;
    created
        .group
        .ok_or_else(|| CommandError::internal("the group was put in and has no id"))
}

impl Panes {
    /// Reads a workspace that is not on show, to change it in its file.
    async fn read_put_away(&self, name: &str) -> Result<Workspace, CommandError> {
        let store = Arc::clone(&self.store);
        let wanted = name.to_owned();
        let loaded = tokio::task::spawn_blocking(move || store.load(&wanted, now_unix_ms()))
            .await
            .map_err(|error| CommandError::internal(error.to_string()))?
            .map_err(store_error)?;
        match loaded {
            Loaded::Workspace(workspace) => Ok(*workspace),
            Loaded::Missing => Err(CommandError::not_found(format!(
                "there is no workspace called `{name}`"
            ))),
            Loaded::SetAside { reason, .. } => Err(CommandError::internal(format!(
                "the workspace `{name}` could not be read ({reason})"
            ))),
        }
    }

    /// Writes a workspace that is not on show to its file.
    async fn write_put_away(&self, workspace: Workspace) -> Result<(), CommandError> {
        let store = Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || {
            store.save(&workspace, env!("CARGO_PKG_VERSION"), now_unix_ms())
        })
        .await
        .map_err(|error| CommandError::internal(error.to_string()))?
        .map_err(store_error)
    }

    /// Moves a group of the workspace on show, with all its tabs, to the end of another
    /// workspace's window. Its sessions end here; its panes start anew there. A notice
    /// offers to bring it back.
    pub async fn move_group(&self, group: GroupId, to: &str) -> Result<(), CommandError> {
        check_name(to)?;
        self.own_workspace("moving a group to another workspace")?;
        let _one_at_a_time = self.switching.lock().await;
        let here = self.workspace();
        let shelved = group_of(&here, group)
            .ok_or_else(|| CommandError::not_found(format!("there is no group {group}")))?;
        let belongs = shelved
            .tabs
            .first()
            .and_then(|tab| tab.workspace.clone())
            .unwrap_or_else(|| here.name.clone());
        if belongs.eq_ignore_ascii_case(to) {
            return Err(CommandError::bad_request(format!(
                "the group is in `{to}` already"
            )));
        }
        // A workspace that is open has its tabs in the strip already: the group stays
        // where it is and belongs to that workspace from now on.
        if let Some(open) = self.open_as(to) {
            self.apply_core(Intent::GroupRetag {
                group,
                workspace: open,
            })
            .await?;
            self.follow_current_workspace();
            return Ok(());
        }
        let take = Intent::GroupTake { group };
        check(&here, &take)?;

        let mut there = self.read_put_away(to).await?;
        let name = shown_name(&shelved);
        let moved = graft(&mut there, shelved)?;
        let to = there.name.clone();
        self.write_put_away(there).await?;
        self.apply_core(take).await?;
        self.notice_with(
            "info",
            format!("Moved {name} to {to}"),
            Some(json!({
                "label": "Undo",
                "command": "lume.group.bring-back",
                "args": { "workspace": to, "group": moved.to_string() },
            })),
        );
        Ok(())
    }

    /// Takes a group out of a workspace that is not on show and puts it at the end of
    /// a window of the one on show.
    pub async fn bring_group_back(
        &self,
        from: &str,
        group: GroupId,
        window: lume_layout::WindowId,
    ) -> Result<(), CommandError> {
        self.own_workspace("bringing a group from another workspace")?;
        check_name(from)?;
        let _one_at_a_time = self.switching.lock().await;
        let here = self.workspace();
        if here.name.eq_ignore_ascii_case(from) {
            return Err(CommandError::bad_request(format!(
                "`{from}` is the workspace on show"
            )));
        }
        let mut there = self.read_put_away(from).await?;
        let shelved = group_of(&there, group)
            .ok_or_else(|| CommandError::not_found(format!("`{from}` has no group {group}")))?;
        let put = Intent::GroupGraft {
            window,
            group: Box::new(shelved),
        };
        check(&here, &put)?;
        apply(&mut there, Intent::GroupTake { group }, None)?;
        self.write_put_away(there).await?;
        self.apply_core(put).await?;
        Ok(())
    }

    /// The workspaces there are, in alphabetical order, each with its folder.
    pub fn workspaces(&self) -> Result<(String, Vec<WorkspaceEntry>), CommandError> {
        let shown = self.workspace();
        let own = shown.backend.is_none();
        let active = if own {
            self.current_workspace()
        } else {
            shown.name.clone()
        };
        let mut open = if own { self.open_beside() } else { Vec::new() };
        if own {
            open.push(shown.name.clone());
        }
        let mut names = self.store.list().map_err(store_error)?;
        // Those that are open may not have been written yet. A server's is never
        // written, and is not one of the application's workspaces.
        for name in &open {
            if !names.iter().any(|kept| kept == name) {
                names.push(name.clone());
            }
        }
        names.sort_by_key(|name| name.to_ascii_lowercase());
        let dirs = self.config.dirs();
        let workspaces = names
            .into_iter()
            .map(|name| {
                let root = if name == active && own {
                    self.config.current().workspace.root.clone()
                } else {
                    root_of(dirs, &name)
                };
                let is_open = open.contains(&name);
                let holds = own && name == shown.name;
                WorkspaceEntry {
                    name,
                    root,
                    open: is_open,
                    holds,
                }
            })
            .collect();
        Ok((active, workspaces))
    }

    /// The workspace an open one is called by, as it was asked for: `Some(None)` for the
    /// one that holds the layout, `Some(Some(name))` for one open beside it, and `None`
    /// for one that is not open. Names that differ only in the case of their letters
    /// name the same workspace.
    pub(crate) fn open_as(&self, name: &str) -> Option<Option<String>> {
        let home = self.workspace();
        if home.backend.is_none() && home.name.eq_ignore_ascii_case(name) {
            return Some(None);
        }
        lock(&self.open)
            .keys()
            .find(|open| open.eq_ignore_ascii_case(name))
            .map(|open| Some(open.clone()))
    }

    /// The names of the workspaces open beside the one that holds the layout.
    pub(crate) fn open_beside(&self) -> Vec<String> {
        let mut names: Vec<String> = lock(&self.open).keys().cloned().collect();
        names.sort_by_key(|name| name.to_ascii_lowercase());
        names
    }

    /// The current workspace: the one of the tab on show in the window the user works
    /// in, or the one that holds the layout.
    pub fn current_workspace(&self) -> String {
        let layout = self.workspace();
        let focused = layout
            .focused_window
            .and_then(|window| layout.window(window))
            .or_else(|| layout.windows.first());
        focused
            .and_then(|window| window.centre.active_tab())
            .and_then(|tab| tab.workspace.clone())
            .filter(|name| lock(&self.open).contains_key(name))
            .unwrap_or(layout.name)
    }

    /// Puts the settings of the current workspace in effect, when another is current
    /// than before.
    pub(crate) fn follow_current_workspace(&self) {
        if self.served_by().is_some() {
            return;
        }
        let current = self.current_workspace();
        if self.config.workspace().as_deref() != Some(current.as_str()) {
            self.config.set_workspace(Some(&current));
            let beside = lock(&self.open)
                .contains_key(&current)
                .then_some(current.as_str());
            remember_current(self.config.dirs(), beside);
        }
    }

    /// Remembers which workspaces are open, for the next start.
    fn remember_open(&self) {
        remember_open(
            self.config.dirs(),
            &self.workspace().name,
            self.open_beside(),
        );
    }

    /// Shows a workspace: the tabs of one that is open, or one that is not, opened
    /// beside those that are. While a server's layout is on show, the workspace takes
    /// its place whole.
    pub async fn switch(&self, name: &str, find: Find) -> Result<(), CommandError> {
        check_name(name)?;
        let _one_at_a_time = self.switching.lock().await;
        if self.served_by().is_some() {
            return self.show_whole(name, find).await;
        }
        let kept = self.store.contains(name).map_err(store_error)?;
        let open = self.open_as(name);
        match find {
            Find::New if kept || open.is_some() => {
                return Err(CommandError::new(
                    "exists",
                    format!("there is already a workspace called `{name}`"),
                ));
            }
            Find::Existing if !kept && open.is_none() => {
                return Err(CommandError::not_found(format!(
                    "there is no workspace called `{name}`"
                )));
            }
            _ => {}
        }
        if let Some(open) = open {
            return self.show_open(open.as_deref()).await;
        }
        self.open_beside_layout(name).await
    }

    /// Begins a workspace, its terminals starting in `root` if one is given, and opens
    /// it.
    pub async fn begin(&self, name: &str, root: Option<&str>) -> Result<(), CommandError> {
        check_name(name)?;
        if self.store.contains(name).map_err(store_error)? || self.open_as(name).is_some() {
            return Err(CommandError::new(
                "exists",
                format!("there is already a workspace called `{name}`"),
            ));
        }
        if let Some(root) = root {
            if !Path::new(root).is_dir() {
                return Err(CommandError::not_found(format!(
                    "`{root}` is not a folder that is there"
                )));
            }
            // Written first, so its first terminal starts there.
            let config = Arc::clone(&self.config);
            let (wanted, root) = (name.to_owned(), root.to_owned());
            tokio::task::spawn_blocking(move || config.set_workspace_root(&wanted, Some(&root)))
                .await
                .map_err(|error| CommandError::internal(error.to_string()))?
                .map_err(CommandError::from)?;
        }
        self.switch(name, Find::New).await
    }

    /// Shows a tab of a workspace that is open, in the window the user works in if that
    /// has one. A workspace with no tab left is given a new one.
    async fn show_open(&self, belongs: Option<&str>) -> Result<(), CommandError> {
        let layout = self.workspace();
        let focused = layout.focused_window;
        let mut windows: Vec<&lume_layout::Window> = layout.windows.iter().collect();
        windows.sort_by_key(|window| Some(window.id) != focused);
        let found = windows.into_iter().find_map(|window| {
            // The tab of the workspace a window shows, else its first one there.
            let tabs = &window.centre.tabs;
            window
                .centre
                .active_tab()
                .filter(|tab| tab.workspace.as_deref() == belongs)
                .or_else(|| tabs.iter().find(|tab| tab.workspace.as_deref() == belongs))
                .map(|tab| (window.id, tab.id))
        });
        match found {
            Some((window, tab)) => {
                if Some(window) != focused {
                    self.apply_core(Intent::WindowFocus { window }).await?;
                }
                self.apply_core(Intent::TabActivate {
                    target: lume_layout::TabTarget::Id { tab },
                })
                .await?;
            }
            None => {
                let name = belongs.map_or_else(|| layout.name.clone(), str::to_owned);
                let root = if belongs.is_none() {
                    self.config.current().workspace.root.clone()
                } else {
                    root_of(self.config.dirs(), &name)
                };
                self.config.set_workspace(Some(&name));
                let (tabs, groups) = fresh_tabs(root);
                self.adopt(belongs.map(str::to_owned), tabs, groups).await?;
            }
        }
        self.follow_current_workspace();
        Ok(())
    }

    /// Puts tabs in the window the user works in, as a workspace's.
    async fn adopt(
        &self,
        belongs: Option<String>,
        tabs: Vec<lume_layout::Tab>,
        groups: Vec<lume_layout::Group>,
    ) -> Result<(), CommandError> {
        let layout = self.workspace();
        let window = layout
            .focused_window
            .or_else(|| layout.windows.first().map(|window| window.id))
            .ok_or_else(|| CommandError::internal("there is no window to open it in"))?;
        self.apply_core(Intent::TabsAdopt {
            window,
            workspace: belongs,
            tabs,
            groups,
        })
        .await
        .map(drop)
    }

    /// Reads a workspace, or begins it, and opens it beside those that are open: its
    /// tabs, from every window it had, join the window the user works in.
    async fn open_beside_layout(&self, name: &str) -> Result<(), CommandError> {
        let store = Arc::clone(&self.store);
        let wanted = name.to_owned();
        let mut opened = tokio::task::spawn_blocking(move || open_workspace(&store, &wanted, None))
            .await
            .map_err(|error| CommandError::internal(error.to_string()))?;
        // Its programs may have been kept running while it was closed.
        self.keep_live_sessions(&mut opened.workspace).await;
        // A file that differs from the name asked for only in the case of its letters
        // is the same workspace, and its tabs take the name it was read under.
        let name = opened.workspace.name.clone();
        let (mut tabs, mut groups) = all_tabs(&opened.workspace);
        if tabs.is_empty() {
            (tabs, groups) = fresh_tabs(root_of(self.config.dirs(), &name));
        }

        // The settings of the workspace are in effect before its panes are, so the
        // sessions of its panes start where it says.
        self.config.set_workspace(Some(&name));
        lock(&self.open).insert(name.clone(), opened.workspace);
        if let Err(error) = self.adopt(Some(name.clone()), tabs, groups).await {
            lock(&self.open).remove(&name);
            self.follow_current_workspace();
            return Err(error);
        }
        self.remember_open();
        remember_current(self.config.dirs(), Some(&name));
        if let Some(warning) = opened.warning {
            self.notice("warning", warning);
        }
        Ok(())
    }

    /// Opens the workspaces that were open beside the one that holds the layout when
    /// the application last ran. One that cannot be read is passed over.
    pub async fn reopen(&self, names: Vec<String>) {
        let _one_at_a_time = self.switching.lock().await;
        let was_current = current_workspace(self.config.dirs());
        for name in names {
            if check_name(&name).is_err() || self.open_as(&name).is_some() {
                continue;
            }
            if !self.store.contains(&name).unwrap_or(false) {
                continue;
            }
            if let Err(error) = self.open_beside_layout(&name).await {
                tracing::warn!(workspace = name, %error, "a workspace that was open could not be opened again");
            }
        }
        // The workspace that holds the layout may have had no tab of its own left. Its
        // window is not left empty.
        let layout = self.workspace();
        let empty = layout
            .windows
            .iter()
            .all(|window| window.centre.tabs.is_empty() && window.floating.is_empty());
        if empty {
            let root = self.config.current().workspace.root.clone();
            let (tabs, groups) = fresh_tabs(root);
            if let Err(error) = self.adopt(None, tabs, groups).await {
                tracing::warn!(%error, "the empty window could not be given a tab");
            }
        }
        // What was current is on show again: the one that holds the layout, unless
        // another was.
        let current = was_current.and_then(|name| self.open_as(&name)).flatten();
        if let Err(error) = self.show_open(current.as_deref()).await {
            tracing::debug!(%error, "the workspace that was current could not be shown again");
        }
        self.follow_current_workspace();
        self.remember_open();
    }

    /// Closes a workspace that is open beside the one that holds the layout: it is
    /// written to its file, its tabs leave the strip, and its sessions end unless the
    /// settings keep them. Without a name, the current workspace is closed.
    pub async fn close_workspace(&self, name: Option<&str>) -> Result<(), CommandError> {
        self.own_workspace("closing a workspace")?;
        let _one_at_a_time = self.switching.lock().await;
        let name = name.map_or_else(|| self.current_workspace(), str::to_owned);
        let belongs = match self.open_as(&name) {
            None => {
                return Err(CommandError::not_found(format!("`{name}` is not open")));
            }
            Some(None) => {
                return Err(CommandError::bad_request(format!(
                    "`{name}` holds the windows and is closed with them"
                )));
            }
            Some(Some(open)) => open,
        };

        // Nothing is written while its tabs leave, so its file keeps them.
        self.save().await;
        let _not_written_meanwhile = self.hold_saving().await;
        let layout = self.workspace();
        let theirs = |tab: &lume_layout::Tab| tab.workspace.as_deref() == Some(belongs.as_str());
        let every_tab_is_theirs = layout
            .windows
            .iter()
            .flat_map(|window| &window.centre.tabs)
            .all(theirs);
        if every_tab_is_theirs {
            // A window is not left without a tab: the workspace that holds the layout
            // is given one.
            let root = self.layout_root();
            let (tabs, groups) = fresh_tabs(root);
            self.adopt(None, tabs, groups).await?;
        }
        let keep = self.config.current().config.workspace.on_switch == OnSwitch::Keep;
        if keep {
            let layout = self.workspace();
            let mut kept = lock(&self.kept_running);
            for window in &layout.windows {
                for tab in window.centre.tabs.iter().filter(|tab| theirs(tab)) {
                    for pane in tab.root.panes() {
                        let PaneContent::Terminal(terminal) = &pane.content else {
                            continue;
                        };
                        if let Some(session) = terminal.session {
                            kept.insert(SessionKey {
                                backend: self.backend_of(terminal),
                                session,
                            });
                        }
                    }
                }
            }
        }
        lock(&self.open).remove(&belongs);
        self.apply_core(Intent::TabsTake {
            workspace: belongs.clone(),
        })
        .await?;
        drop(_not_written_meanwhile);
        self.remember_open();
        self.follow_current_workspace();
        self.save().await;
        Ok(())
    }

    /// The folder the workspace that holds the layout gives its terminals.
    fn layout_root(&self) -> Option<String> {
        let home = self.workspace().name;
        if self.config.workspace().as_deref() == Some(home.as_str()) {
            return self.config.current().workspace.root.clone();
        }
        root_of(self.config.dirs(), &home)
    }

    /// Shows the workspace called `name` in place of the layout on show, whole, in the
    /// window the user works in: what is done when a server's layout is on show.
    async fn show_whole(&self, name: &str, find: Find) -> Result<(), CommandError> {
        let old = self.workspace();
        let kept = self.store.contains(name).map_err(store_error)?;
        match find {
            Find::New if kept => {
                return Err(CommandError::new(
                    "exists",
                    format!("there is already a workspace called `{name}`"),
                ));
            }
            Find::Existing if !kept => {
                return Err(CommandError::not_found(format!(
                    "there is no workspace called `{name}`"
                )));
            }
            _ => {}
        }

        // What becomes of its programs is the say of the workspace being put away.
        let keep = self.config.current().config.workspace.on_switch == OnSwitch::Keep;
        // The one on show is written as it stands, with every pane still in it.
        self.save().await;
        let store = Arc::clone(&self.store);
        let wanted = name.to_owned();
        let mut opened = tokio::task::spawn_blocking(move || open_workspace(&store, &wanted, None))
            .await
            .map_err(|error| CommandError::internal(error.to_string()))?;
        // Its programs may have been kept running while it was put away.
        self.keep_live_sessions(&mut opened.workspace).await;
        // The window the user works in stays open on the desktop.
        keep_window_ids(&old, &mut opened.workspace);
        let name = opened.workspace.name.clone();

        // The settings of the workspace are in effect before its panes are, so the
        // sessions of its panes start where it says.
        self.config.set_workspace(Some(&name));
        let left = self.leave_server();
        lock(&self.open).clear();
        self.layout.replace(opened.workspace);
        if let Some(left) = left {
            self.forget_unshown(&left);
        }
        let reopen = if last_workspace(self.config.dirs()).as_deref() == Some(name.as_str()) {
            open_workspaces(self.config.dirs())
        } else {
            Vec::new()
        };
        remember_workspace(self.config.dirs(), &name);
        if let Some(warning) = opened.warning {
            self.notice("warning", warning);
        }
        self.end_sessions_put_away(&old, keep);
        self.sync_tab_panel().await;
        // Back to the workspace that held the layout before: those that were open
        // beside it are opened again.
        for other in reopen {
            if other.eq_ignore_ascii_case(&name) || !self.store.contains(&other).unwrap_or(false) {
                continue;
            }
            if let Err(error) = self.open_beside_layout(&other).await {
                tracing::warn!(workspace = other, %error, "a workspace that was open could not be opened again");
            }
        }
        self.follow_current_workspace();
        Ok(())
    }

    /// Ends the sessions of a workspace that was put away, unless they are to be kept
    /// for when it is shown again: its file names them. The sessions in a copy of a
    /// server's layout are the server's, and are left alone.
    pub(crate) fn end_sessions_put_away(&self, old: &Workspace, keep: bool) {
        if keep || old.backend.is_some() {
            return;
        }
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
    }

    /// Gives the current workspace another name: its two files are renamed and the
    /// layout follows. If either file cannot be renamed both keep their names.
    pub async fn rename(&self, to: &str) -> Result<(), CommandError> {
        check_name(to)?;
        self.own_workspace("renaming the workspace")?;
        let _one_at_a_time = self.switching.lock().await;
        let from = self.current_workspace();
        if from == to {
            return Ok(());
        }
        let holds_layout = self.workspace().name == from;
        if !holds_layout && self.open_as(to).is_some() {
            return Err(CommandError::new(
                "exists",
                format!("there is already a workspace called `{to}`"),
            ));
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

        if holds_layout {
            self.layout
                .apply_core(Intent::WorkspaceRename {
                    name: to.to_owned(),
                })
                .await?;
            self.config.set_workspace(Some(to));
            // Still the same workspaces open beside it.
            let open = self.open_beside();
            remember_open(self.config.dirs(), to, open);
        } else {
            {
                let mut open = lock(&self.open);
                if let Some(mut rest) = open.remove(&from) {
                    rest.name = to.to_owned();
                    open.insert(to.to_owned(), rest);
                }
            }
            self.layout
                .apply_core(Intent::TabsRetag {
                    from: Some(from),
                    to: Some(to.to_owned()),
                })
                .await?;
            self.config.set_workspace(Some(to));
            self.remember_open();
        }
        Ok(())
    }

    /// Removes a workspace that is not on show: both of its files.
    pub async fn delete(&self, name: &str) -> Result<(), CommandError> {
        check_name(name)?;
        let _one_at_a_time = self.switching.lock().await;
        if self.open_as(name).is_some() {
            return Err(CommandError::bad_request(format!(
                "`{name}` is open; close it before deleting it"
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

    /// Sets the folder the new terminals of the current workspace start in, or takes it
    /// away.
    pub async fn set_root(&self, root: Option<&str>) -> Result<(), CommandError> {
        self.own_workspace("giving the workspace a folder")?;
        let root = root.map(str::trim).filter(|root| !root.is_empty());
        if let Some(root) = root
            && !Path::new(root).is_dir()
        {
            return Err(CommandError::not_found(format!(
                "`{root}` is not a folder that is there"
            )));
        }
        let name = self.current_workspace();
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
