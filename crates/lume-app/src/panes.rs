//! The panes of the application's windows, and the sessions in them.
//!
//! The layout says which panes there are; this says what follows from that. A terminal
//! pane has a session for as long as the pane exists: the session is started when the
//! pane is made, or when a page first knows how large the pane is, and is ended when
//! the pane closes. A page that is reloaded changes nothing here. It reads the layout
//! and attaches to the sessions it finds in it.
//!
//! One task follows the layout's changes. It tells every page, ends the sessions of
//! panes that have closed, opens and closes the windows the layout gains and loses,
//! closes a window whose last tab has gone, and writes the workspace to its file a
//! moment after it last changed.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use lume_config::schema::{CloseOnExit, NewPaneCwd, TabPlacement, TerminalConfig};
use lume_config::{Profile, ShellIntegration};
use lume_layout::{
    Applied, Area, ChangeStream, Intent, LayoutAuthority, LayoutChange, LayoutEvent,
    LayoutSnapshot, LocalLayout, NewContent, Pane, PaneContent, PaneId, PanelContent, PanelId,
    SpawnFailure, SpawnRequest, Spawner, Started, Store, UNITS, WindowId, Workspace, needs,
};
use lume_proto::app::{AppReplyBody, PageRequestBody};
use lume_session::start_dir_from_report;
use lume_session::{
    BackendId, ExitStatus, Launcher, SessionId, SessionInfo, SpawnSpec, TermSize, default_shell,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::carrier::SessionKey;
use crate::configsvc::ConfigService;
use crate::control::Shell;
use crate::error::CommandError;
use crate::profiles::ProfileService;
use crate::state::AppState;
use crate::tuning::{
    DEFAULT_SPAWN_COLS, DEFAULT_SPAWN_ROWS, ESTIMATE_TIMEOUT, PENDING_NOTICES_MAX, SAVE_DEBOUNCE,
    SAVE_MAX_DELAY, TAB_LIST_PANEL,
};
use crate::window::{label_of, window_of};
use crate::workspaces::open_workspace;

/// A pane a page asks to have a session started for, and the grid the pane has.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneStart {
    pub pane: PaneId,
    pub size: TermSize,
}

/// A pane and the session that now runs in it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StartedPane {
    pub pane: PaneId,
    pub backend: BackendId,
    pub session: SessionId,
}

/// A pane that got no session, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FailedPane {
    pub pane: PaneId,
    pub code: String,
    pub message: String,
}

/// What starting the sessions of some panes came to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SessionsStarted {
    pub started: Vec<StartedPane>,
    pub failed: Vec<FailedPane>,
}

/// The grid a session is started with when nobody could say how large its pane is.
fn default_size() -> TermSize {
    TermSize::new(DEFAULT_SPAWN_COLS, DEFAULT_SPAWN_ROWS).expect("the default grid is a valid size")
}

pub(crate) fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// The directory a session starts in. `asked` is where its pane was last started; a
/// directory that is no longer there is passed over.
fn start_dir(asked: Option<&str>, root: Option<&str>, new_panes: NewPaneCwd) -> Option<String> {
    let there = |dir: &&str| Path::new(dir).is_dir();
    if let Some(dir) = asked.filter(there) {
        return Some(dir.to_owned());
    }
    // A pane that had a directory belongs to the workspace's work, whatever new panes
    // are set to start in.
    let in_root = asked.is_some() || new_panes != NewPaneCwd::Home;
    if let Some(dir) = root.filter(|_| in_root).filter(there) {
        return Some(dir.to_owned());
    }
    std::env::home_dir().map(|home| home.to_string_lossy().into_owned())
}

/// The program a session runs when its pane names none: the configured shell, or the
/// user's own.
fn start_launcher(shell: &str, args: &[String]) -> Launcher {
    if !shell.trim().is_empty() {
        return Launcher::Shell {
            program: shell.to_owned(),
            args: args.to_vec(),
            login: false,
        };
    }
    match default_shell() {
        Launcher::Shell { program, login, .. } if !args.is_empty() => Launcher::Shell {
            program,
            args: args.to_vec(),
            login,
        },
        launcher => launcher,
    }
}

/// What a pane's session is started with, once its profile and the settings have
/// been consulted.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Start {
    launcher: Launcher,
    cwd: Option<String>,
    env: Vec<(String, String)>,
}

/// A directory as a profile writes it, with a leading `~` standing for the user's
/// home directory.
fn expand_home(dir: &str) -> String {
    let rest = match dir.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => rest,
        _ => return dir.to_owned(),
    };
    match std::env::home_dir() {
        Some(home) => {
            let rest = rest.trim_start_matches(['/', '\\']);
            // Joining nothing would still add a separator.
            let dir = if rest.is_empty() {
                home
            } else {
                home.join(rest)
            };
            dir.to_string_lossy().into_owned()
        }
        None => dir.to_owned(),
    }
}

/// The directory given to a Linux distribution's shell. A Linux path is taken on
/// trust, since only the distribution can say whether it is there; a path of this
/// machine has to be a folder. Anything else leaves the shell in its home directory.
fn distro_dir(asked: Option<&str>) -> Option<String> {
    let dir = asked?;
    let linux = dir.starts_with('/') || dir == "~" || dir.starts_with("~/");
    (linux || Path::new(dir).is_dir()).then(|| dir.to_owned())
}

/// What a pane runs. A pane that names its program runs it as named. Any other runs
/// its profile; a profile that names no program runs the user's shell with the
/// profile's arguments. With neither, it is the shell of the settings.
fn planned_launcher(
    named: Option<&Launcher>,
    profile: Option<&Profile>,
    terminal: &TerminalConfig,
) -> Launcher {
    match (named, profile) {
        (Some(launcher), _) => launcher.clone(),
        (None, Some(profile)) => match &profile.launcher {
            Launcher::Shell {
                program,
                args,
                login,
            } if program.trim().is_empty() => match start_launcher("", args) {
                Launcher::Shell {
                    program,
                    args,
                    login: own,
                } => Launcher::Shell {
                    program,
                    args,
                    login: *login || own,
                },
                other => other,
            },
            launcher => launcher.clone(),
        },
        (None, None) => start_launcher(&terminal.shell, &terminal.shell_args),
    }
}

/// Whether two sessions are in the same place, so that a directory of one is a
/// directory of the other: both on this machine, or both in the same Linux
/// distribution as the same user.
fn same_place(one: &Launcher, other: &Launcher) -> bool {
    match (one, other) {
        (Launcher::Shell { .. }, Launcher::Shell { .. }) => true,
        (
            Launcher::Wsl { distro, user },
            Launcher::Wsl {
                distro: other_distro,
                user: other_user,
            },
        ) => distro.eq_ignore_ascii_case(other_distro) && user == other_user,
        _ => false,
    }
}

/// Works out what a pane's session is started with: what it runs, where, and with
/// which variables.
fn plan_start(
    request: &SpawnRequest,
    profile: Option<&Profile>,
    terminal: &TerminalConfig,
    root: Option<&str>,
    new_panes: NewPaneCwd,
) -> Start {
    let profile = profile.filter(|_| request.launcher.is_none());
    let launcher = planned_launcher(request.launcher.as_ref(), profile, terminal);

    let from_profile = profile.and_then(|profile| profile.cwd.as_deref());
    let cwd = match &launcher {
        Launcher::Wsl { .. } => distro_dir(request.cwd.as_deref().or(from_profile)),
        // The directory is the other machine's business.
        Launcher::Ssh { .. } => None,
        _ => {
            let asked = request
                .cwd
                .clone()
                .or_else(|| from_profile.map(expand_home));
            start_dir(asked.as_deref(), root, new_panes)
        }
    };

    // The pane's own variables go last, so they are the ones that hold.
    let mut env: Vec<(String, String)> = profile
        .map(|profile| {
            profile
                .env
                .iter()
                .filter(|(name, _)| !request.env.iter().any(|(set, _)| set == *name))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default();
    env.extend(request.env.iter().cloned());

    Start { launcher, cwd, env }
}

/// Starts and ends the sessions of panes on the application's backends.
struct PaneSpawner {
    state: Weak<AppState>,
    config: Arc<ConfigService>,
    profiles: Arc<ProfileService>,
    /// Sessions that were running already when a pane was made to show them. A pane
    /// that is not made after all leaves such a session as it was.
    adopted: Mutex<HashSet<SessionKey>>,
}

impl PaneSpawner {
    fn failure(error: impl Into<CommandError>) -> SpawnFailure {
        let error = error.into();
        SpawnFailure {
            code: error.code.into_owned(),
            message: error.message,
        }
    }

    /// What a session that was started before this application ran, or that no pane
    /// shows, most likely runs: what the pane's `request` would start, or, when it
    /// says nothing, the program the backend lists.
    async fn launcher_of(
        &self,
        request: &SpawnRequest,
        info: &SessionInfo,
    ) -> (Launcher, Option<String>) {
        if request.launcher.is_none() && request.profile.is_none() && request.session.is_some() {
            return (start_launcher(&info.program, &[]), None);
        }
        let resolved = self.config.current();
        let wanted = match &request.launcher {
            Some(_) => None,
            None => request
                .profile
                .clone()
                .or_else(|| self.profiles.default_id()),
        };
        let profile = match &wanted {
            Some(id) => self.profiles.resolve(id).await,
            None => None,
        };
        let Start { launcher, .. } = plan_start(
            request,
            profile.as_ref(),
            &resolved.config.terminal,
            resolved.workspace.root.as_deref(),
            resolved.config.layout.new_pane_cwd,
        );
        (launcher, profile.map(|profile| profile.id))
    }

    /// Takes in a session that runs already, as a pane's: what it runs is noted as if
    /// it had been started here.
    async fn remember(
        &self,
        state: &AppState,
        key: &SessionKey,
        request: &SpawnRequest,
        info: &SessionInfo,
    ) {
        if state.reports().launched(key).is_none() {
            let (launcher, profile) = self.launcher_of(request, info).await;
            state.reports().started(key.clone(), launcher, profile);
        }
        if let Some(pid) = info.pid {
            state.reports().set_pid(key, pid);
        }
    }

    /// A pane is to show a session that runs already.
    async fn adopt(
        &self,
        state: &AppState,
        backend: BackendId,
        session: SessionId,
        request: &SpawnRequest,
    ) -> Result<Started, SpawnFailure> {
        let host = state.router().get(&backend).map_err(Self::failure)?;
        let listed = host.list().await.map_err(Self::failure)?;
        let info = listed
            .into_iter()
            .find(|info| info.id == session)
            .ok_or_else(|| SpawnFailure {
                code: "not_found".to_owned(),
                message: format!("there is no session {session}"),
            })?;
        let key = SessionKey {
            backend: backend.clone(),
            session,
        };
        self.remember(state, &key, request, &info).await;
        lock(&self.adopted).insert(key);
        Ok(Started {
            backend,
            session,
            cwd: None,
        })
    }

    /// Goes over the sessions a workspace read from its file names: those that still
    /// run are kept, and their panes show them again; the others have ended since the
    /// file was written, or ran on a host that has gone, and their panes are left
    /// without, to be started afresh.
    async fn keep_live_sessions(&self, state: &AppState, workspace: &mut Workspace) {
        let saved = workspace.sessions();
        if saved.is_empty() {
            return;
        }
        let mut listed: HashMap<BackendId, Vec<SessionInfo>> = HashMap::new();
        for (pane, session) in saved {
            let Some(terminal) = workspace.pane(pane).and_then(terminal_of).cloned() else {
                continue;
            };
            let backend = terminal
                .backend
                .clone()
                .unwrap_or_else(lume_session::Router::local_id);
            if !listed.contains_key(&backend) {
                let sessions = match state.router().get(&backend) {
                    Ok(host) => host.list().await.unwrap_or_default(),
                    Err(_) => Vec::new(),
                };
                listed.insert(backend.clone(), sessions);
            }
            let running = listed[&backend]
                .iter()
                .find(|info| info.id == session && info.exit.is_none())
                .cloned();
            match running {
                Some(info) => {
                    let key = SessionKey { backend, session };
                    let request = SpawnRequest {
                        backend: terminal.backend.clone(),
                        profile: terminal.profile.clone(),
                        launcher: terminal.launcher.clone(),
                        cwd: terminal.cwd.clone(),
                        env: terminal.env.clone(),
                        size: None,
                        session: None,
                    };
                    self.remember(state, &key, &request, &info).await;
                    tracing::info!(%pane, session = %key.session, "a pane shows its session again");
                }
                None => {
                    if let Some(Pane {
                        content: PaneContent::Terminal(terminal),
                        ..
                    }) = workspace.pane_mut(pane)
                    {
                        terminal.session = None;
                    }
                }
            }
        }
    }
}

#[async_trait]
impl Spawner for PaneSpawner {
    async fn spawn(&self, request: &SpawnRequest) -> Result<Started, SpawnFailure> {
        let state = self.state.upgrade().ok_or_else(|| SpawnFailure {
            code: "disconnected".to_owned(),
            message: "the application is closing".to_owned(),
        })?;
        let resolved = self.config.current();
        let backend = request
            .backend
            .clone()
            .unwrap_or_else(|| state.default_backend());
        if let Some(session) = request.session {
            return self.adopt(&state, backend, session, request).await;
        }
        // A pane that names its program needs no profile, and is not kept waiting for
        // one.
        let wanted = match &request.launcher {
            Some(_) => None,
            None => request
                .profile
                .clone()
                .or_else(|| self.profiles.default_id()),
        };
        let profile = match &wanted {
            Some(id) => {
                let profile = self.profiles.resolve(id).await;
                if profile.is_none() {
                    tracing::warn!(profile = %id, "a pane names a profile that is not there");
                    state.broadcast(&json!({
                        "type": "notice",
                        "level": "warning",
                        "message": format!(
                            "there is no profile called `{id}`, so the default shell was started"
                        ),
                    }));
                }
                profile
            }
            None => None,
        };
        let Start { launcher, cwd, env } = plan_start(
            request,
            profile.as_ref(),
            &resolved.config.terminal,
            resolved.workspace.root.as_deref(),
            resolved.config.layout.new_pane_cwd,
        );
        // A shell is asked to report its directory and its commands unless the
        // profile it was started from says not to.
        let shell_integration = profile
            .as_ref()
            .is_none_or(|profile| profile.shell_integration == ShellIntegration::Auto);
        let spec = SpawnSpec {
            launcher: launcher.clone(),
            cwd: cwd.clone(),
            env,
            size: request.size.unwrap_or_else(default_size),
            shell_integration,
            scrollback_lines: Some(resolved.config.terminal.scrollback_lines),
        };
        let host = state.router().get(&backend).map_err(Self::failure)?;
        let session = host.spawn(spec).await.map_err(Self::failure)?;
        state.marks().spawned();
        let key = SessionKey {
            backend: backend.clone(),
            session,
        };
        // What the session runs decides later what a directory it reports is good for,
        // and how its pane is shown.
        state
            .reports()
            .started(key.clone(), launcher, profile.map(|profile| profile.id));
        // Which process is the shell tells it from a program it runs. Nothing waits
        // for that.
        let reports = Arc::clone(state.reports());
        tokio::spawn(async move {
            let listed = host.list().await;
            let pid = listed.ok().and_then(|sessions| {
                sessions
                    .into_iter()
                    .find(|info| info.id == key.session)
                    .and_then(|info| info.pid)
            });
            if let Some(pid) = pid {
                reports.set_pid(&key, pid);
            }
        });
        Ok(Started {
            backend,
            session,
            cwd,
        })
    }

    async fn discard(&self, started: &Started) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let key = SessionKey {
            backend: started.backend.clone(),
            session: started.session,
        };
        // A session that ran before the pane was to show it runs on without it.
        if lock(&self.adopted).remove(&key) {
            return;
        }
        if let Err(error) = state.end(&key).await {
            tracing::debug!(session = %key.session, %error, "a session nobody needs could not be ended");
        }
    }
}

/// Whether a change gives a pane a session: a pane whose session was set, or a tab
/// that comes with its panes.
fn shows_a_session(change: &LayoutChange) -> bool {
    let has = |pane: &Pane| terminal_of(pane).is_some_and(|terminal| terminal.session.is_some());
    match change {
        LayoutChange::Pane { pane } => has(pane),
        LayoutChange::TabAdded { tab, .. } => tab.root.panes().into_iter().any(has),
        LayoutChange::TabTree { root, .. } => root.panes().into_iter().any(has),
        _ => false,
    }
}

/// The terminal a pane shows, if it shows one.
fn terminal_of(pane: &Pane) -> Option<&lume_layout::TerminalPane> {
    match &pane.content {
        PaneContent::Terminal(terminal) => Some(terminal),
        PaneContent::PluginView { .. } | PaneContent::Placeholder { .. } => None,
    }
}

/// The session request inside an intent that makes a terminal pane.
fn request_mut(intent: &mut Intent) -> Option<&mut SpawnRequest> {
    match intent {
        Intent::Split {
            content: NewContent::Terminal(request),
            ..
        }
        | Intent::TabNew {
            content: NewContent::Terminal(request),
            ..
        }
        | Intent::ScratchShow {
            content: Some(NewContent::Terminal(request)),
            ..
        } => Some(request),
        _ => None,
    }
}

/// Shares of a split that are as equal as whole units allow and add up to a whole.
pub fn equal_ratios(count: usize) -> Vec<u32> {
    let count = u32::try_from(count.max(1)).unwrap_or(u32::MAX);
    let share = UNITS / count;
    let mut ratios = vec![share; count as usize];
    // What dividing left over goes to the first child.
    ratios[0] += UNITS - share * count;
    ratios
}

/// The panes of the application's windows and the sessions in them.
pub struct Panes {
    pub(crate) state: Weak<AppState>,
    pub(crate) layout: Arc<LocalLayout>,
    spawner: Arc<PaneSpawner>,
    pub(crate) config: Arc<ConfigService>,
    pub(crate) profiles: Arc<ProfileService>,
    pub(crate) store: Arc<Store>,
    /// The layout's changes from before any page could connect. The task that follows
    /// them takes this.
    changes: Mutex<Option<ChangeStream>>,
    /// Held while sessions are started for panes that have none, so two pages asking
    /// for the same pane get the same session.
    starting: tokio::sync::Mutex<()>,
    /// What the next page to connect is to be told: things that happened before there
    /// was a page.
    pending: Mutex<Vec<Value>>,
    /// Set when the application is closing. Sessions that end from then on were ended
    /// by that, and their panes stay in the workspace's file.
    closing: AtomicBool,
    /// Sessions that are being ended for a reason that keeps their panes: their panes
    /// stay, whatever the settings say about a program that ends.
    ending_on_purpose: Mutex<HashSet<SessionKey>>,
    /// Held while another workspace is put on show, or the one on show is renamed or
    /// one is deleted: these go one at a time.
    pub(crate) switching: tokio::sync::Mutex<()>,
    /// The server whose layout is on show, while one is.
    pub(crate) served: Mutex<Option<crate::served::Served>>,
    /// Held while the workspace is written to its file, and while its file is being
    /// renamed, so that it is not written under the name it is just losing.
    saving: tokio::sync::Mutex<()>,
    /// Who is told when a pane leaves the layout, whatever it showed.
    closed: Mutex<Vec<PaneListener>>,
}

/// Something that is told of a pane, by its id.
type PaneListener = Box<dyn Fn(PaneId) + Send + Sync>;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Panes {
    /// Reads the workspace called `name` from `store`, or begins one, and takes charge
    /// of its panes. The workspace has at least one window afterwards. The first pane
    /// of a workspace that is begun here shows `first`, or a shell; whether it was
    /// begun here is returned.
    pub async fn open(
        state: &Arc<AppState>,
        config: Arc<ConfigService>,
        store: Store,
        name: &str,
        first: Option<SpawnRequest>,
    ) -> (Arc<Self>, bool) {
        let mut opened = open_workspace(&store, name, first);
        let pending = opened
            .warning
            .iter()
            .map(|warning| json!({ "type": "notice", "level": "warning", "message": warning }))
            .collect();
        let profiles = ProfileService::new(Arc::clone(&config));
        let spawner = Arc::new(PaneSpawner {
            state: Arc::downgrade(state),
            config: Arc::clone(&config),
            profiles: Arc::clone(&profiles),
            adopted: Mutex::new(HashSet::new()),
        });
        spawner
            .keep_live_sessions(state, &mut opened.workspace)
            .await;
        let layout = LocalLayout::new(opened.workspace, 1, spawner.clone());
        // Taken before anything can change the layout, so the task that follows it
        // misses nothing.
        let changes = layout.changes();
        let panes = Arc::new(Self {
            state: Arc::downgrade(state),
            layout,
            spawner,
            config,
            profiles,
            store: Arc::new(store),
            changes: Mutex::new(Some(changes)),
            starting: tokio::sync::Mutex::new(()),
            pending: Mutex::new(pending),
            closing: AtomicBool::new(false),
            ending_on_purpose: Mutex::new(HashSet::new()),
            switching: tokio::sync::Mutex::new(()),
            served: Mutex::new(None),
            saving: tokio::sync::Mutex::new(()),
            closed: Mutex::new(Vec::new()),
        });
        panes.sync_tab_panel().await;
        (panes, opened.fresh)
    }

    /// Says that these sessions are about to be ended with their panes kept: the panes
    /// stay when their programs end.
    pub(crate) fn expect_ends(&self, keys: &[SessionKey]) {
        lock(&self.ending_on_purpose).extend(keys.iter().cloned());
    }

    /// Leaves the panes of a workspace read from its file showing the sessions they
    /// showed when it was written, where those still run, and without one otherwise.
    pub(crate) async fn keep_live_sessions(&self, workspace: &mut Workspace) {
        if let Some(state) = self.state.upgrade() {
            self.spawner.keep_live_sessions(&state, workspace).await;
        }
    }

    /// The layout as it is now.
    pub fn snapshot(&self) -> LayoutSnapshot {
        self.layout.snapshot()
    }

    pub fn workspace(&self) -> Workspace {
        self.layout.workspace()
    }

    /// The layout as the page of `window` is given it, with what the page needs to
    /// know of this machine to show where a shell says it is: the machine's name, which
    /// a report from another machine does not carry, and the user's home folder.
    pub fn view(&self, window: WindowId) -> Value {
        let snapshot = self.snapshot();
        let connected = self
            .state
            .upgrade()
            .is_none_or(|state| crate::host::link_up(&state));
        json!({
            "window": window,
            "epoch": snapshot.epoch,
            "revision": snapshot.revision,
            "workspace": snapshot.workspace,
            "host": { "connected": connected },
            "machine": {
                "name": lume_session::cwd::machine_name(),
                "home": std::env::home_dir().map(|home| home.to_string_lossy().into_owned()),
            },
        })
    }

    /// What a page that has just connected is to be told, once.
    pub fn take_pending(&self) -> Vec<Value> {
        std::mem::take(&mut *lock(&self.pending))
    }

    /// Tells every page something, or keeps it for the first page if there is none yet.
    pub fn notice(&self, level: &str, message: impl Into<String>) {
        self.notice_with(level, message, None);
    }

    /// Tells every page something, with a command the page offers to run from the
    /// notice: `{ label, command, args }`.
    pub fn notice_with(&self, level: &str, message: impl Into<String>, action: Option<Value>) {
        let mut notice = json!({ "type": "notice", "level": level, "message": message.into() });
        if let Some(action) = action {
            notice["action"] = action;
        }
        let links = self.state.upgrade().map(|state| state.links());
        match links {
            Some(links) if !links.is_empty() => {
                for (_, link) in links {
                    link.notify(&notice);
                }
            }
            _ => {
                let mut pending = lock(&self.pending);
                if pending.len() < PENDING_NOTICES_MAX {
                    pending.push(notice);
                }
            }
        }
    }

    /// The window the user is working in: the one that last had the keyboard, or the
    /// first there is.
    pub fn focused_window(&self) -> Option<WindowId> {
        let workspace = self.workspace();
        workspace
            .focused_window
            .filter(|window| workspace.window(*window).is_some())
            .or_else(|| workspace.windows.first().map(|window| window.id))
    }

    /// The pane that has the keyboard in a window.
    pub fn focused_pane(&self, window: WindowId) -> Option<PaneId> {
        self.workspace().window(window)?.focus
    }

    /// The terminal a pane shows: what it runs and how it was asked for.
    pub fn terminal(&self, pane: PaneId) -> Option<lume_layout::TerminalPane> {
        self.workspace().pane(pane).and_then(terminal_of).cloned()
    }

    /// The session a pane shows, if it is a terminal that has one.
    pub fn session_of(&self, pane: PaneId) -> Option<SessionKey> {
        let workspace = self.workspace();
        let terminal = terminal_of(workspace.pane(pane)?)?;
        Some(SessionKey {
            backend: self.backend_of(terminal),
            session: terminal.session?,
        })
    }

    pub(crate) fn backend_of(&self, terminal: &lume_layout::TerminalPane) -> BackendId {
        terminal
            .backend
            .clone()
            .unwrap_or_else(lume_session::Router::local_id)
    }

    /// The pane that shows a session, if one does: the first of them, if several do.
    pub fn pane_of(&self, key: &SessionKey) -> Option<PaneId> {
        self.panes_of(key).into_iter().next()
    }

    /// Every pane that shows a session. A session may be shown in several panes, of
    /// one window or of several.
    pub fn panes_of(&self, key: &SessionKey) -> Vec<PaneId> {
        let workspace = self.workspace();
        workspace
            .panes()
            .into_iter()
            .filter_map(|pane| {
                let terminal = terminal_of(pane)?;
                (terminal.session == Some(key.session) && self.backend_of(terminal) == key.backend)
                    .then_some(pane.id)
            })
            .collect()
    }

    /// Whether something runs in a pane that closing it would end.
    pub fn busy(&self, pane: PaneId) -> bool {
        let Some(state) = self.state.upgrade() else {
            return false;
        };
        self.session_of(pane)
            .is_some_and(|key| state.reports().busy(&key, state.ended(&key)))
    }

    /// Whether something runs in a pane of `window`: whether closing the window would
    /// end something.
    pub fn runs_something(&self, window: WindowId) -> bool {
        let workspace = self.workspace();
        let Some(window) = workspace.window(window) else {
            return false;
        };
        window
            .panes()
            .into_iter()
            .any(|pane| terminal_of(pane).is_some() && self.busy(pane.id))
    }

    /// Whether a session is in a pane of `window`: whether that window's page may
    /// attach to it.
    pub fn owns(&self, window: WindowId, key: &SessionKey) -> bool {
        let workspace = self.workspace();
        self.panes_of(key)
            .into_iter()
            .any(|pane| workspace.window_of(pane) == Some(window))
    }

    /// Applies an intent that someone other than the application asked for.
    ///
    /// An intent that makes a terminal pane and does not say how large the pane will
    /// be has the window's page asked, since only the page knows the window's pixels.
    ///
    /// Where a server owns the layout on show, what is asked of its tabs and panes is
    /// asked of the server, and shows once the server says its layout changed.
    pub async fn apply(&self, intent: Intent) -> Result<Applied, CommandError> {
        if let Some(answer) = self.ask_server(&intent).await {
            return answer;
        }
        let intent = self.sized(intent).await;
        Ok(self.layout.apply(intent).await?)
    }

    /// Applies an intent on the application's own behalf.
    pub async fn apply_core(&self, intent: Intent) -> Result<Applied, CommandError> {
        if let Some(answer) = self.ask_server_as_core(&intent).await {
            return answer;
        }
        Ok(self.layout.apply_core(intent).await?)
    }

    /// Gives the session request of an intent a size, if it needs one and has none.
    async fn sized(&self, mut intent: Intent) -> Intent {
        let workspace = self.workspace();
        let unsized_window = match needs(&workspace, &intent) {
            Some(request) if request.size.is_none() => match &intent {
                Intent::Split { pane, .. } => workspace.window_of(*pane),
                Intent::TabNew { window, .. } | Intent::ScratchShow { window, .. } => Some(*window),
                _ => None,
            },
            _ => return intent,
        };
        let size = match unsized_window {
            Some(window) => self.estimate(window, &intent).await,
            None => None,
        };
        if let Some(request) = request_mut(&mut intent) {
            request.size = Some(size.unwrap_or_else(default_size));
        }
        intent
    }

    /// Asks the page of `window` how large the pane an intent would make will be.
    async fn estimate(&self, window: WindowId, intent: &Intent) -> Option<TermSize> {
        let link = self.state.upgrade()?.page_of(&label_of(window))?;
        let intent = serde_json::to_value(intent).ok()?;
        match link
            .ask(PageRequestBody::Estimate { intent }, ESTIMATE_TIMEOUT)
            .await
        {
            Ok(AppReplyBody::Estimated { size }) => Some(size),
            Ok(_) => None,
            Err(error) => {
                tracing::debug!(%error, "the page did not say how large the new pane will be");
                None
            }
        }
    }

    /// Whether a session is still there and its program still running.
    async fn alive(&self, key: &SessionKey) -> bool {
        let Some(state) = self.state.upgrade() else {
            return false;
        };
        let Ok(backend) = state.router().get(&key.backend) else {
            return false;
        };
        backend.list().await.is_ok_and(|sessions| {
            sessions
                .iter()
                .any(|info| info.id == key.session && info.exit.is_none())
        })
    }

    /// Starts sessions for panes of `window` that have none: panes read from a file,
    /// the pane of a window that has just opened, and panes whose program has ended.
    /// A pane whose session is running keeps it and is answered with it.
    pub async fn start_sessions(&self, window: WindowId, asked: Vec<PaneStart>) -> SessionsStarted {
        let _one_at_a_time = self.starting.lock().await;
        let mut outcome = SessionsStarted::default();
        for PaneStart { pane, size } in asked {
            match self.start_one(window, pane, size).await {
                Ok(key) => outcome.started.push(StartedPane {
                    pane,
                    backend: key.backend,
                    session: key.session,
                }),
                Err(error) => outcome.failed.push(FailedPane {
                    pane,
                    code: error.code.into_owned(),
                    message: error.message,
                }),
            }
        }
        outcome
    }

    /// Puts the last window away while the application keeps running in the
    /// background: hidden, to be shown by the next launch, its panes given fresh shells
    /// unless a host of its own keeps their programs for the next start anyway.
    pub fn keep_ready(&self, window: WindowId, shell: &dyn Shell) {
        shell.hide_window(window);
        let Some(state) = self.state.upgrade() else {
            return;
        };
        if state.locals().iter().any(|local| local.is_daemon()) {
            return;
        }
        let Some(panes) = state.panes().cloned() else {
            return;
        };
        state
            .runtime()
            .spawn(async move { panes.refresh_window(window).await });
    }

    /// Gives every terminal pane of `window` a fresh session of what it runs, at the
    /// size its session has, as the next start of the application would; the sessions
    /// they had are ended once they show the new ones, so no pane is closed by its old
    /// program ending. This is what closing the last window does while the application
    /// keeps running in the background.
    pub async fn refresh_window(&self, window: WindowId) {
        let _one_at_a_time = self.starting.lock().await;
        let workspace = self.workspace();
        let Some(shown) = workspace.window(window) else {
            return;
        };
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let mut sizes: HashMap<BackendId, Vec<SessionInfo>> = HashMap::new();
        for pane in shown.panes() {
            let Some(terminal) = terminal_of(pane) else {
                continue;
            };
            let backend = self.backend_of(terminal);
            let old = terminal.session.map(|session| SessionKey {
                backend: backend.clone(),
                session,
            });
            let mut size = None;
            if let Some(old) = &old {
                if !sizes.contains_key(&backend) {
                    let listed = match state.router().get(&backend) {
                        Ok(host) => host.list().await.unwrap_or_default(),
                        Err(_) => Vec::new(),
                    };
                    sizes.insert(backend.clone(), listed);
                }
                size = sizes[&backend]
                    .iter()
                    .find(|info| info.id == old.session)
                    .map(|info| info.size);
            }
            let request = SpawnRequest {
                backend: terminal.backend.clone(),
                profile: terminal.profile.clone(),
                launcher: terminal.launcher.clone(),
                cwd: terminal.cwd.clone(),
                env: terminal.env.clone(),
                size,
                session: None,
            };
            let started = match self.spawner.spawn(&request).await {
                Ok(started) => started,
                Err(failure) => {
                    tracing::warn!(pane = %pane.id, message = %failure.message,
                        "a pane's fresh session could not be started");
                    continue;
                }
            };
            let recorded = self
                .layout
                .apply_core(Intent::SessionSet {
                    pane: pane.id,
                    session: Some(started.session),
                    backend: Some(started.backend.clone()),
                    cwd: started.cwd.clone(),
                })
                .await;
            if recorded.is_err() {
                self.spawner.discard(&started).await;
                continue;
            }
            if let Some(old) = old {
                state.reports().forget(&old);
                self.spawner
                    .discard(&Started {
                        backend: old.backend,
                        session: old.session,
                        cwd: None,
                    })
                    .await;
            }
        }
    }

    async fn start_one(
        &self,
        window: WindowId,
        pane: PaneId,
        size: TermSize,
    ) -> Result<SessionKey, CommandError> {
        let workspace = self.workspace();
        if workspace.window_of(pane) != Some(window) {
            return Err(CommandError::not_found(format!(
                "window {window} has no pane {pane}"
            )));
        }
        let terminal = workspace
            .pane(pane)
            .and_then(terminal_of)
            .ok_or_else(|| CommandError::bad_request(format!("pane {pane} is not a terminal")))?;
        if let Some(session) = terminal.session {
            let key = SessionKey {
                backend: self.backend_of(terminal),
                session,
            };
            if self.alive(&key).await {
                return Ok(key);
            }
            // What the program that ended said about itself is not true of the next.
            if let Some(state) = self.state.upgrade() {
                state.reports().forget(&key);
            }
        }
        let request = SpawnRequest {
            backend: terminal.backend.clone(),
            profile: terminal.profile.clone(),
            launcher: terminal.launcher.clone(),
            cwd: terminal.cwd.clone(),
            env: terminal.env.clone(),
            size: Some(size),
            session: None,
        };
        let started = self
            .spawner
            .spawn(&request)
            .await
            .map_err(|failure| CommandError::new(failure.code, failure.message))?;
        let recorded = self
            .layout
            .apply_core(Intent::SessionSet {
                pane,
                session: Some(started.session),
                backend: Some(started.backend.clone()),
                cwd: started.cwd.clone(),
            })
            .await;
        match recorded {
            Ok(_) => Ok(SessionKey {
                backend: started.backend,
                session: started.session,
            }),
            // The pane went away while its session was starting.
            Err(error) => {
                self.spawner.discard(&started).await;
                Err(error.into())
            }
        }
    }

    /// The directory a terminal opened from `source` starts in, if it is to start
    /// where that pane's shell is: the settings say new panes do, the shell has said
    /// where it is, the new terminal (which runs `profile`, or the default one) will
    /// be in the same place as the pane, and the directory checks out.
    pub async fn inherited_dir(&self, source: PaneId, profile: Option<&str>) -> Option<String> {
        let resolved = self.config.current();
        if resolved.config.layout.new_pane_cwd != NewPaneCwd::Inherit {
            return None;
        }
        let key = self.session_of(source)?;
        let (running, url) = self.state.upgrade()?.reports().reported_dir(&key)?;
        let wanted = profile
            .map(str::to_owned)
            .or_else(|| self.profiles.default_id());
        let profile = match &wanted {
            Some(id) => Some(self.profiles.resolve(id).await?),
            None => None,
        };
        let planned = planned_launcher(None, profile.as_ref(), &resolved.config.terminal);
        if !same_place(&running, &planned) {
            return None;
        }
        // Looking at a directory may take a moment, in a distribution above all.
        tokio::task::spawn_blocking(move || start_dir_from_report(&running, &url))
            .await
            .ok()
            .flatten()
    }

    /// The directory the shell in `pane` is in, as a path of this machine or of the
    /// pane's distribution, with what the pane runs. `None` where the shell has not
    /// said, or what it said does not hold up.
    pub async fn live_dir(&self, pane: PaneId) -> Option<(Launcher, String)> {
        let key = self.session_of(pane)?;
        let (running, url) = self.state.upgrade()?.reports().reported_dir(&key)?;
        tokio::task::spawn_blocking(move || {
            let dir = start_dir_from_report(&running, &url)?;
            Some((running, dir))
        })
        .await
        .ok()
        .flatten()
    }

    /// Starts the program of a pane again, once it has ended: the same profile, in the
    /// same pane. For a pane whose connection to another machine was lost this is
    /// connecting again.
    pub async fn restart(&self, pane: PaneId) -> Result<SessionKey, CommandError> {
        let _one_at_a_time = self.starting.lock().await;
        let window = self
            .workspace()
            .window_of(pane)
            .ok_or_else(|| CommandError::not_found(format!("there is no pane {pane}")))?;
        // The new program gets the grid the old one had, which is the pane's.
        let mut size = None;
        if let Some(key) = self.session_of(pane)
            && let Some(state) = self.state.upgrade()
            && let Ok(backend) = state.router().get(&key.backend)
            && let Ok(sessions) = backend.list().await
            && let Some(info) = sessions.iter().find(|info| info.id == key.session)
        {
            if info.exit.is_none() {
                return Err(CommandError::bad_request(format!(
                    "the program in pane {pane} is still running"
                )));
            }
            size = Some(info.size);
        }
        self.start_one(window, pane, size.unwrap_or_else(default_size))
            .await
    }

    /// Acts on the end of a session's program: its pane closes, or stays to say how
    /// the program ended, as the configuration has it.
    pub fn exited(self: &Arc<Self>, key: &SessionKey, status: ExitStatus) {
        if self.closing.load(Ordering::Relaxed) || lock(&self.ending_on_purpose).remove(key) {
            return;
        }
        // What becomes of a pane whose program ended is the say of whoever owns the
        // layout.
        if self.served_by().is_some() {
            return;
        }
        let panes = self.panes_of(key);
        let Some(&pane) = panes.first() else {
            return;
        };
        // A profile may say otherwise than the settings do.
        let of_profile = self
            .workspace()
            .pane(pane)
            .and_then(terminal_of)
            .and_then(|terminal| terminal.profile.as_deref())
            .and_then(|id| self.profiles.find(id))
            .and_then(|profile| profile.close_on_exit);
        let rule = of_profile.unwrap_or(self.config.current().config.terminal.close_on_exit);
        let close = match rule {
            CloseOnExit::Always => true,
            CloseOnExit::Clean => status == ExitStatus::Code(0),
            CloseOnExit::Never => false,
        };
        let Some(state) = self.state.upgrade() else {
            return;
        };
        if close {
            // Every pane that showed it goes.
            let this = Arc::clone(self);
            state.runtime().spawn(async move {
                for pane in panes {
                    if let Err(error) = this.layout.apply_core(Intent::Close { pane }).await {
                        tracing::debug!(%pane, %error, "the pane of a program that ended could not be closed");
                    }
                }
            });
        }
    }

    /// Starts the task that follows the layout's changes. `shell` is how it closes
    /// windows and the application.
    pub fn run(self: &Arc<Self>, shell: Arc<dyn Shell>) {
        let Some(changes) = lock(&self.changes).take() else {
            return;
        };
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let panes = Arc::clone(self);
        state.runtime().spawn(panes.follow(changes, shell));
    }

    async fn follow(self: Arc<Self>, mut changes: ChangeStream, shell: Arc<dyn Shell>) {
        // When the workspace first and last changed since it was written.
        let mut unsaved: Option<(Instant, Instant)> = None;
        loop {
            // Which session a pane shows is written at once when sessions outlive the
            // application: an application that is ended a moment later finds its
            // panes' sessions again at its next start, not new ones beside them.
            let mut names_a_session = false;
            let event = match unsaved {
                Some((first, last)) => {
                    let due = (last + SAVE_DEBOUNCE).min(first + SAVE_MAX_DELAY);
                    tokio::select! {
                        event = changes.next() => event,
                        () = tokio::time::sleep_until(due) => {
                            self.save().await;
                            unsaved = None;
                            continue;
                        }
                    }
                }
                None => changes.next().await,
            };
            let Some(event) = event else {
                return;
            };
            let Some(state) = self.state.upgrade() else {
                return;
            };
            match event {
                LayoutEvent::Changed {
                    epoch,
                    revision,
                    changes,
                } => {
                    let notice = json!({
                        "type": "layout_changed",
                        "epoch": epoch,
                        "revision": revision,
                        "changes": changes,
                    });
                    for (_, link) in state.links() {
                        link.notify(&notice);
                    }
                    for change in &changes {
                        if let LayoutChange::PaneClosed { pane, .. } = change {
                            for told in lock(&self.closed).iter() {
                                told(*pane);
                            }
                        }
                        self.act_on(&state, change, shell.as_ref()).await;
                    }
                    names_a_session = changes.iter().any(shows_a_session);
                }
                LayoutEvent::Reset(snapshot) => {
                    // Another workspace is on show. The windows it has are opened
                    // before the ones it has not are closed, so the application is
                    // never without a window.
                    let open = shell.layout_windows();
                    let wanted: Vec<String> = snapshot
                        .workspace
                        .windows
                        .iter()
                        .map(|window| label_of(window.id))
                        .collect();
                    for window in &snapshot.workspace.windows {
                        if !open.contains(&label_of(window.id)) {
                            shell.open_window(window.id, window.bounds);
                        }
                    }
                    for label in open.iter().filter(|label| !wanted.contains(label)) {
                        shell.close_window(label);
                    }
                    for (label, link) in state.links() {
                        let Some(window) = window_of(&label) else {
                            continue;
                        };
                        if !wanted.contains(&label) {
                            continue;
                        }
                        link.notify(&json!({
                            "type": "layout_reset",
                            "window": window,
                            "epoch": snapshot.epoch,
                            "revision": snapshot.revision,
                            "workspace": snapshot.workspace,
                        }));
                    }
                }
            }
            let now = Instant::now();
            let kept = state.locals().iter().any(|local| local.is_daemon());
            if names_a_session && kept {
                self.save().await;
                unsaved = None;
            } else {
                unsaved = Some((unsaved.map_or(now, |(first, _)| first), now));
            }
            // Whoever waits for a pane to have a session looks again.
            state.wake();
        }
    }

    /// Has `told` called with each pane that leaves the layout from now on. It is called
    /// between two changes to the layout, and must not wait for anything.
    pub fn when_pane_closed(&self, told: impl Fn(PaneId) + Send + Sync + 'static) {
        lock(&self.closed).push(Box::new(told));
    }

    /// Does what one change to the layout calls for outside the layout.
    async fn act_on(&self, state: &Arc<AppState>, change: &LayoutChange, shell: &dyn Shell) {
        match change {
            LayoutChange::PaneClosed {
                content: PaneContent::Terminal(terminal),
                ..
            } => {
                let Some(session) = terminal.session else {
                    return;
                };
                let key = SessionKey {
                    backend: self.backend_of(terminal),
                    session,
                };
                if self.pane_of(&key).is_some() {
                    // Another pane still shows it.
                    return;
                }
                // A pane that leaves a server's layout may only have been moved out of
                // sight there. Its session is the server's to end.
                if self.served_by().is_some() {
                    return;
                }
                // Not waited for: a session that is slow to end must not hold up what
                // the pages are told next.
                let state = Arc::clone(state);
                state.runtime().clone().spawn(async move {
                    if let Err(error) = state.end(&key).await {
                        // Usually the program had already ended.
                        tracing::debug!(session = %key.session, %error, "a closed pane's session could not be ended");
                    }
                });
            }
            LayoutChange::Tabs { window, order, .. } if order.is_empty() => {
                self.close_if_empty(*window, shell).await;
            }
            LayoutChange::Floating { window, floating } if floating.is_empty() => {
                self.close_if_empty(*window, shell).await;
            }
            LayoutChange::WindowOpened { window } => {
                shell.open_window(window.id, window.bounds);
                // A new window has the panels the configuration gives every window.
                self.sync_tab_panel().await;
            }
            LayoutChange::WindowClosed { window } => {
                shell.close_window(&label_of(*window));
                if self.workspace().windows.is_empty() {
                    shell.quit();
                }
            }
            _ => {}
        }
    }

    /// Closes a window that has nothing left in it: no tab, and no pane that floats
    /// over where the tabs were or is put away at its corner. The last window of an
    /// application that keeps running in the background is given a new tab and hidden
    /// instead, ready for the next launch.
    async fn close_if_empty(&self, window: WindowId, shell: &dyn Shell) {
        if self.closing.load(Ordering::Relaxed)
            || !self.config.current().config.window.close_with_last_tab
        {
            return;
        }
        // A server's layout with nothing in it is still the server's: the window stays
        // to show what comes next.
        if self.served_by().is_some() {
            return;
        }
        let empty = self
            .workspace()
            .window(window)
            .is_some_and(|found| found.centre.tabs.is_empty() && found.floating.is_empty());
        if !empty {
            return;
        }
        let keep_running = self.config.current().config.app.keep_running;
        if keep_running && self.workspace().windows.len() == 1 {
            let opened = self
                .layout
                .apply_core(Intent::TabNew {
                    window,
                    content: NewContent::Terminal(SpawnRequest::default()),
                    index: None,
                    activate: true,
                    group: None,
                })
                .await;
            match opened {
                Ok(_) => shell.hide_window(window),
                Err(error) => {
                    tracing::debug!(%window, %error, "the last window could not be made ready")
                }
            }
            return;
        }
        let closed = self.layout.apply_core(Intent::WindowClose { window }).await;
        if let Err(error) = closed {
            tracing::debug!(%window, %error, "a window with nothing in it could not be closed");
        }
    }

    /// The directories the shells in the panes are in now, as far as they have said
    /// and the say-so holds up: for each such pane, where its session would be started
    /// to be back where it is.
    fn live_dirs(&self) -> Vec<(PaneId, Launcher, String)> {
        let Some(state) = self.state.upgrade() else {
            return Vec::new();
        };
        let workspace = self.workspace();
        workspace
            .panes()
            .into_iter()
            .filter_map(|pane| {
                let terminal = terminal_of(pane)?;
                let key = SessionKey {
                    backend: self.backend_of(terminal),
                    session: terminal.session?,
                };
                let (launcher, url) = state.reports().reported_dir(&key)?;
                Some((pane.id, launcher, url))
            })
            .collect()
    }

    /// The workspace as it is written to its file: each pane with the directory its
    /// shell is in now, where that is known, in place of the one it was started in.
    /// Looks at the file system.
    fn to_save(mut workspace: Workspace, live: Vec<(PaneId, Launcher, String)>) -> Workspace {
        for (pane, launcher, url) in live {
            let Some(dir) = start_dir_from_report(&launcher, &url) else {
                continue;
            };
            if let Some(Pane {
                content: PaneContent::Terminal(terminal),
                ..
            }) = workspace.pane_mut(pane)
            {
                terminal.cwd = Some(dir);
            }
        }
        workspace
    }

    /// Writes the workspace to its file.
    pub(crate) async fn save(&self) {
        let _one_at_a_time = self.saving.lock().await;
        let workspace = self.workspace();
        // A copy of a server's layout is the server's to keep.
        if workspace.backend.is_some() {
            return;
        }
        let live = self.live_dirs();
        let store = Arc::clone(&self.store);
        let written = tokio::task::spawn_blocking(move || {
            let workspace = Self::to_save(workspace, live);
            store.save(&workspace, env!("CARGO_PKG_VERSION"), now_unix_ms())
        })
        .await;
        let failed = match written {
            Ok(Ok(())) => return,
            Ok(Err(error)) => error.to_string(),
            Err(error) => error.to_string(),
        };
        tracing::error!(error = failed, "the workspace could not be saved");
        self.notice(
            "error",
            format!("The workspace could not be saved: {failed}"),
        );
    }

    /// Writes the workspace to its file for the last time, and stops acting on
    /// sessions that end: from here on they end because the application is closing.
    pub fn close(&self) {
        if self.closing.swap(true, Ordering::Relaxed) {
            return;
        }
        if self.workspace().backend.is_some() {
            return;
        }
        let workspace = Self::to_save(self.workspace(), self.live_dirs());
        let saved = self
            .store
            .save(&workspace, env!("CARGO_PKG_VERSION"), now_unix_ms());
        if let Err(error) = saved {
            tracing::error!(%error, "the workspace could not be saved");
        }
    }

    /// Holds off writing the workspace to its file for as long as the guard lives.
    pub(crate) async fn hold_saving(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.saving.lock().await
    }

    /// Shows a plugin's panel in a window: where it is, if the window has it, or else at
    /// the end of `area`, the first time. The dock area it is in is opened.
    pub async fn show_plugin_panel(
        &self,
        window: WindowId,
        id: &str,
        plugin: &str,
        view: &str,
        area: Area,
    ) -> Result<(), CommandError> {
        let panel = PanelId::new(id)
            .map_err(|why| CommandError::bad_request(format!("`{id}` is not a panel id: {why}")))?;
        let workspace = self.workspace();
        let dock = &workspace
            .window(window)
            .ok_or_else(|| CommandError::not_found(format!("there is no window {window}")))?
            .dock;
        let have = [Area::Left, Area::Right, Area::Bottom]
            .into_iter()
            .find(|side| {
                dock.area(*side)
                    .panels
                    .iter()
                    .any(|found| found.id == panel)
            });
        let area = match have {
            Some(side) => side,
            None => {
                let index = u32::try_from(dock.area(area).panels.len()).unwrap_or(u32::MAX);
                self.layout
                    .apply_core(Intent::PanelSet {
                        window,
                        panel,
                        content: PanelContent::PluginView {
                            plugin: plugin.to_owned(),
                            view: view.to_owned(),
                            state: serde_json::Value::Null,
                        },
                        area: Some(area),
                        index,
                    })
                    .await?;
                area
            }
        };
        self.layout
            .apply_core(Intent::PanelToggle {
                window,
                area,
                on: Some(true),
            })
            .await?;
        Ok(())
    }

    /// Takes a plugin's panel out of a window. The list of tabs and the top bar's
    /// places are not taken out this way.
    pub async fn close_panel(&self, window: WindowId, id: &str) -> Result<(), CommandError> {
        let panel = PanelId::new(id)
            .map_err(|why| CommandError::bad_request(format!("`{id}` is not a panel id: {why}")))?;
        let workspace = self.workspace();
        let found = workspace.window(window).and_then(|window| {
            [Area::Left, Area::Right, Area::Bottom]
                .into_iter()
                .find_map(|side| {
                    window
                        .dock
                        .area(side)
                        .panels
                        .iter()
                        .find(|found| found.id == panel)
                        .cloned()
                })
        });
        let Some(found) = found else {
            return Ok(());
        };
        if !matches!(found.content, PanelContent::PluginView { .. }) {
            return Err(CommandError::bad_request(
                "only a plugin's panel is closed this way",
            ));
        }
        self.layout
            .apply_core(Intent::PanelSet {
                window,
                panel,
                content: found.content,
                area: None,
                index: 0,
            })
            .await?;
        Ok(())
    }

    /// Puts the list of tabs where the configuration says it goes, in every window: in
    /// a panel at the left or the right, or in no panel when the tabs are in the bar at
    /// the top.
    pub async fn sync_tab_panel(&self) {
        let want = match self.config.current().config.layout.tab_placement {
            TabPlacement::Top => None,
            TabPlacement::Left => Some(Area::Left),
            TabPlacement::Right => Some(Area::Right),
        };
        let Ok(panel) = PanelId::new(TAB_LIST_PANEL) else {
            return;
        };
        for window in self.workspace().windows {
            let have = [Area::Left, Area::Right, Area::Bottom]
                .into_iter()
                .find(|area| {
                    let panels = &window.dock.area(*area).panels;
                    panels.iter().any(|found| found.id == panel)
                });
            if have == want {
                continue;
            }
            let placed = self
                .layout
                .apply_core(Intent::PanelSet {
                    window: window.id,
                    panel: panel.clone(),
                    content: PanelContent::TabList {},
                    area: want,
                    index: 0,
                })
                .await;
            if let Err(error) = placed {
                tracing::debug!(window = %window.id, %error, "the list of tabs could not be placed");
                continue;
            }
            // A list that was just asked for is not left folded away.
            if let Some(area) = want.filter(|area| window.dock.area(*area).collapsed) {
                let _ = self
                    .layout
                    .apply_core(Intent::PanelToggle {
                        window: window.id,
                        area,
                        on: Some(true),
                    })
                    .await;
            }
        }
    }

    /// Whether a pane floats over a window.
    pub fn is_floating(&self, pane: PaneId) -> bool {
        self.workspace()
            .windows
            .iter()
            .any(|window| window.floating_index(pane).is_some())
    }

    /// Whether a pane is put away at the corner of its window.
    pub fn is_minimised(&self, pane: PaneId) -> bool {
        self.workspace().windows.iter().any(|window| {
            let floating = &window.floating;
            floating
                .iter()
                .any(|floating| floating.pane.id == pane && floating.minimised.is_some())
        })
    }

    /// Whether the workspace's scratch terminal is on show over `window`.
    pub fn scratch_shown_in(&self, window: WindowId) -> bool {
        let workspace = self.workspace();
        workspace.scratch.as_ref().is_some_and(|scratch| {
            scratch.parked.is_none() && workspace.window_of(scratch.pane) == Some(window)
        })
    }

    /// The dock area a command that names none means: the one that holds the list of
    /// tabs, or the first that holds any panel.
    pub fn dock_to_toggle(&self, window: WindowId) -> Option<Area> {
        let workspace = self.workspace();
        let dock = &workspace.window(window)?.dock;
        let sides = [Area::Left, Area::Right, Area::Bottom];
        let holds_tabs = |area: &Area| {
            let panels = &dock.area(*area).panels;
            panels
                .iter()
                .any(|panel| panel.id.as_str() == TAB_LIST_PANEL)
        };
        sides.into_iter().find(holds_tabs).or_else(|| {
            sides
                .into_iter()
                .find(|area| !dock.area(*area).panels.is_empty())
        })
    }

    /// The splits of the tab on show in a window, each with how many children it has.
    pub fn splits_on_show(&self, window: WindowId) -> Vec<(lume_layout::SplitId, usize)> {
        let workspace = self.workspace();
        let Some(tab) = workspace
            .window(window)
            .and_then(|window| window.centre.active_tab())
        else {
            return Vec::new();
        };
        tab.root
            .splits()
            .into_iter()
            .map(|split| (split.id, split.children.len()))
            .collect()
    }

    /// Whether a pane is the only thing its tab shows for now.
    pub fn is_zoomed(&self, pane: PaneId) -> bool {
        let workspace = self.workspace();
        workspace
            .windows
            .iter()
            .flat_map(|window| &window.centre.tabs)
            .any(|tab| tab.zoomed == Some(pane))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use lume_layout::{Dir, FocusTarget};
    use lume_proto::{Body, Message};
    use lume_session::{HostConfig, LocalBackend, Router};
    use tokio::runtime::Handle;
    use tokio::time::{sleep, timeout};

    use super::*;
    use crate::marks::Marks;
    use crate::testing::{
        CREDIT, FakeShell, RecordingCarrier, STEP, TempDir, client_frames, facts, size,
    };
    use crate::tuning::WORKSPACE_NAME;
    use crate::workspaces::Find;

    struct Running {
        state: Arc<AppState>,
        panes: Arc<Panes>,
        shell: Arc<FakeShell>,
        window: WindowId,
        _dir: TempDir,
    }

    fn profile(launcher: Launcher) -> Profile {
        Profile {
            id: "p".to_owned(),
            name: "P".to_owned(),
            icon: None,
            launcher,
            cwd: None,
            env: std::collections::BTreeMap::new(),
            theme: None,
            font_size: None,
            shell_integration: lume_config::ShellIntegration::Auto,
            close_on_exit: None,
            hidden: false,
            origin: lume_config::ProfileOrigin::User,
            detail: None,
        }
    }

    fn shell(program: &str, args: &[&str]) -> Launcher {
        Launcher::Shell {
            program: program.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            login: false,
        }
    }

    fn planned(request: &SpawnRequest, profile: Option<&Profile>) -> Start {
        let terminal = TerminalConfig {
            shell: "settings-shell".to_owned(),
            shell_args: vec!["-s".to_owned()],
            ..TerminalConfig::default()
        };
        plan_start(request, profile, &terminal, None, NewPaneCwd::Home)
    }

    #[test]
    fn a_pane_runs_its_own_program_then_its_profile_then_the_settings_shell() {
        let named = SpawnRequest {
            launcher: Some(shell("htop", &[])),
            ..SpawnRequest::default()
        };
        let nu = profile(shell("nu", &["--login"]));
        // A pane that names its program is not changed by any profile.
        assert_eq!(planned(&named, Some(&nu)).launcher, shell("htop", &[]));
        assert_eq!(
            planned(&SpawnRequest::default(), Some(&nu)).launcher,
            shell("nu", &["--login"])
        );
        assert_eq!(
            planned(&SpawnRequest::default(), None).launcher,
            shell("settings-shell", &["-s"])
        );
    }

    #[test]
    fn a_profile_that_names_no_program_runs_the_users_shell_with_its_arguments() {
        let bare = profile(shell("", &["-x"]));
        let Launcher::Shell { program, args, .. } =
            planned(&SpawnRequest::default(), Some(&bare)).launcher
        else {
            panic!("not a shell");
        };
        let Launcher::Shell { program: own, .. } = default_shell() else {
            panic!("the default is not a shell");
        };
        assert_eq!(program, own);
        assert_eq!(args, ["-x"]);
    }

    #[test]
    fn a_shell_starts_in_a_folder_that_is_there_and_the_profiles_tilde_is_home() {
        let here = std::env::current_dir().unwrap();
        let here = here.to_string_lossy();
        let home = std::env::home_dir().unwrap();

        let mut cmd = profile(shell("cmd", &[]));
        cmd.cwd = Some(here.to_string());
        let start = planned(&SpawnRequest::default(), Some(&cmd));
        assert_eq!(start.cwd.as_deref(), Some(&*here));

        cmd.cwd = Some("~".to_owned());
        let start = planned(&SpawnRequest::default(), Some(&cmd));
        assert_eq!(start.cwd.as_deref(), Some(&*home.to_string_lossy()));

        // The pane's own directory goes before the profile's.
        cmd.cwd = Some("~".to_owned());
        let asked = SpawnRequest {
            cwd: Some(here.to_string()),
            ..SpawnRequest::default()
        };
        assert_eq!(planned(&asked, Some(&cmd)).cwd.as_deref(), Some(&*here));

        // A folder that is not there is passed over for the home directory.
        cmd.cwd = Some(format!("{here}/no-such-folder-here"));
        let start = planned(&SpawnRequest::default(), Some(&cmd));
        assert_eq!(start.cwd.as_deref(), Some(&*home.to_string_lossy()));
    }

    #[test]
    fn a_distribution_takes_a_linux_path_on_trust_and_another_machine_takes_none() {
        let mut ubuntu = profile(Launcher::Wsl {
            distro: "Ubuntu".to_owned(),
            user: None,
        });
        assert_eq!(planned(&SpawnRequest::default(), Some(&ubuntu)).cwd, None);
        ubuntu.cwd = Some("/srv/www".to_owned());
        assert_eq!(
            planned(&SpawnRequest::default(), Some(&ubuntu))
                .cwd
                .as_deref(),
            Some("/srv/www")
        );
        ubuntu.cwd = Some("~/work".to_owned());
        assert_eq!(
            planned(&SpawnRequest::default(), Some(&ubuntu))
                .cwd
                .as_deref(),
            Some("~/work")
        );
        // Not a Linux path and not a folder of this machine: the shell's home.
        ubuntu.cwd = Some("no-such-folder-here".to_owned());
        assert_eq!(planned(&SpawnRequest::default(), Some(&ubuntu)).cwd, None);

        let mut remote = profile(Launcher::Ssh {
            host: "work".to_owned(),
            user: None,
            port: None,
            extra_args: vec![],
            client: None,
        });
        remote.cwd = Some("/srv".to_owned());
        let asked = SpawnRequest {
            cwd: Some("/tmp".to_owned()),
            ..SpawnRequest::default()
        };
        assert_eq!(planned(&asked, Some(&remote)).cwd, None);
    }

    #[test]
    fn the_panes_own_variables_hold_over_the_profiles() {
        let mut nu = profile(shell("nu", &[]));
        nu.env.insert("A".to_owned(), "profile".to_owned());
        nu.env.insert("B".to_owned(), "profile".to_owned());
        let asked = SpawnRequest {
            env: vec![("B".to_owned(), "pane".to_owned())],
            ..SpawnRequest::default()
        };
        assert_eq!(
            planned(&asked, Some(&nu)).env,
            [
                ("A".to_owned(), "profile".to_owned()),
                ("B".to_owned(), "pane".to_owned())
            ]
        );
    }

    /// A configuration whose shell is one that starts fast and keeps no history.
    fn plain_shell(dir: &TempDir, more: &str) {
        let (shell, args) = if cfg!(windows) {
            ("cmd.exe", "[\"/d\"]")
        } else {
            ("/bin/sh", "[]")
        };
        let text = format!("[terminal]\nshell = \"{shell}\"\nshell-args = {args}\n{more}");
        std::fs::create_dir_all(&dir.dirs().config).unwrap();
        std::fs::write(dir.dirs().config_file(), text).unwrap();
    }

    async fn running_in(dir: TempDir) -> Running {
        let local = Arc::new(
            LocalBackend::in_process(HostConfig::default())
                .await
                .unwrap(),
        );
        let router = Router::new();
        router.insert(Router::local_id(), local.clone());
        let state = AppState::new(
            Handle::current(),
            router,
            vec![local],
            Duration::from_secs(600),
            Arc::new(Marks::start()),
        );
        let config = ConfigService::load(dir.dirs(), Some(WORKSPACE_NAME));
        let store = Store::new(dir.dirs().state_workspaces_dir());
        let (panes, _) = Panes::open(&state, config, store, WORKSPACE_NAME, None).await;
        state.set_panes(Arc::clone(&panes));
        let shell = Arc::new(FakeShell::default());
        panes.run(shell.clone());
        let window = panes.focused_window().unwrap();
        Running {
            state,
            panes,
            shell,
            window,
            _dir: dir,
        }
    }

    async fn running(name: &str, config: &str) -> Running {
        let dir = TempDir::new(name);
        plain_shell(&dir, config);
        running_in(dir).await
    }

    async fn until(what: &str, mut done: impl FnMut() -> bool) {
        let waiting = async {
            while !done() {
                sleep(Duration::from_millis(10)).await;
            }
        };
        if timeout(STEP, waiting).await.is_err() {
            panic!("{what} never happened");
        }
    }

    fn only_pane(app: &Running) -> PaneId {
        app.panes.workspace().panes()[0].id
    }

    /// Opens a carrier for the window as its page would.
    async fn page(app: &Running) -> (Arc<RecordingCarrier>, u32) {
        let carrier = RecordingCarrier::new();
        let id = app.state.open(&label_of(app.window), carrier.clone()).await;
        (carrier, id)
    }

    async fn start(app: &Running, pane: PaneId) -> SessionKey {
        let outcome = app
            .panes
            .start_sessions(
                app.window,
                vec![PaneStart {
                    pane,
                    size: size(80, 24),
                }],
            )
            .await;
        assert_eq!(outcome.failed, vec![]);
        let started = &outcome.started[0];
        SessionKey {
            backend: started.backend.clone(),
            session: started.session,
        }
    }

    async fn attach(app: &Running, carrier: u32, key: &SessionKey, stream: u32) {
        app.state
            .attach(
                &label_of(app.window),
                carrier,
                &key.backend,
                key.session,
                stream,
                facts(),
            )
            .await
            .unwrap();
        let credit = client_frames(&[Message::new(stream, Body::Credit(CREDIT))]);
        app.state
            .send(&label_of(app.window), carrier, &credit)
            .await
            .unwrap();
    }

    async fn type_into(app: &Running, carrier: u32, stream: u32, text: &str) {
        let input = client_frames(&[Message::new(stream, Body::Input(text.as_bytes().to_vec()))]);
        app.state
            .send(&label_of(app.window), carrier, &input)
            .await
            .unwrap();
    }

    async fn listed(app: &Running) -> usize {
        app.state.list_all().await.unwrap().len()
    }

    fn notices(carrier: &RecordingCarrier, kind: &str) -> Vec<Value> {
        carrier
            .requests()
            .into_iter()
            .filter_map(|message| message.get("notify").cloned())
            .filter(|notice| notice["type"] == kind)
            .collect()
    }

    fn split(pane: PaneId) -> Intent {
        Intent::Split {
            pane,
            dir: Dir::Right,
            ratio: UNITS / 2,
            content: NewContent::Terminal(SpawnRequest::default()),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_new_workspace_has_one_window_with_one_pane_and_no_session_yet() {
        let app = running("new", "").await;
        let workspace = app.panes.workspace();
        assert_eq!(workspace.name, WORKSPACE_NAME);
        assert_eq!(workspace.windows.len(), 1);
        assert_eq!(workspace.panes().len(), 1);
        assert_eq!(app.panes.session_of(only_pane(&app)), None);
        assert_eq!(listed(&app).await, 0);
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_page_starts_the_session_of_a_pane_once() {
        let app = running("start", "").await;
        let pane = only_pane(&app);
        let key = start(&app, pane).await;
        assert_eq!(app.panes.session_of(pane), Some(key.clone()));
        assert_eq!(app.panes.pane_of(&key), Some(pane));

        // Asked again, the pane keeps the session it has.
        assert_eq!(start(&app, pane).await, key);
        assert_eq!(listed(&app).await, 1);

        // A pane of no window of this page is refused.
        let outcome = app
            .panes
            .start_sessions(
                WindowId::new(999),
                vec![PaneStart {
                    pane,
                    size: size(80, 24),
                }],
            )
            .await;
        assert_eq!(outcome.started, vec![]);
        assert_eq!(outcome.failed[0].code, "not_found");
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_split_starts_a_session_and_closing_the_pane_ends_it() {
        let app = running("split", "").await;
        let first = only_pane(&app);
        let applied = app.panes.apply(split(first)).await.unwrap();
        let second = applied.created.pane.unwrap();
        let key = app.panes.session_of(second).unwrap();
        assert_eq!(listed(&app).await, 1);

        app.panes
            .apply(Intent::Close { pane: second })
            .await
            .unwrap();
        let state = Arc::clone(&app.state);
        let waiting = async {
            while !state.list_all().await.unwrap().is_empty() {
                sleep(Duration::from_millis(20)).await;
            }
        };
        timeout(STEP, waiting)
            .await
            .expect("the closed pane's session was never ended");
        assert_eq!(app.panes.pane_of(&key), None);
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_session_that_cannot_be_started_leaves_no_pane() {
        let dir = TempDir::new("nospawn");
        std::fs::create_dir_all(&dir.dirs().config).unwrap();
        std::fs::write(
            dir.dirs().config_file(),
            "[terminal]\nshell = \"lume-no-such-program-k7q\"\n",
        )
        .unwrap();
        let app = running_in(dir).await;
        let before = app.panes.snapshot();

        let refused = app.panes.apply(split(only_pane(&app))).await.unwrap_err();
        assert_ne!(refused.code, "invalid_intent");
        assert_eq!(app.panes.snapshot(), before);
        assert_eq!(listed(&app).await, 0);
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_program_that_ends_cleanly_closes_its_pane_and_a_failed_one_stays() {
        let app = running("exit", "").await;
        let first = only_pane(&app);
        let (_carrier, id) = page(&app).await;
        let second = app
            .panes
            .apply(split(first))
            .await
            .unwrap()
            .created
            .pane
            .unwrap();
        let failing = start(&app, first).await;
        let clean = app.panes.session_of(second).unwrap();
        attach(&app, id, &failing, 1).await;
        attach(&app, id, &clean, 2).await;

        type_into(&app, id, 1, "exit 3\r").await;
        type_into(&app, id, 2, "exit 0\r").await;
        let panes = Arc::clone(&app.panes);
        until("the clean pane to close", || {
            panes.workspace().pane(second).is_none()
        })
        .await;
        sleep(Duration::from_millis(300)).await;
        // The other pane is still there, with the session whose program failed.
        assert_eq!(app.panes.session_of(first), Some(failing.clone()));

        // Started again, it gets a new session.
        let again = start(&app, first).await;
        assert_ne!(again, failing);
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_window_runs_something_while_a_program_other_than_a_shell_at_its_prompt_runs() {
        let app = running("runs", "").await;
        let pane = only_pane(&app);
        // A pane whose session has not been started runs nothing.
        assert!(!app.panes.runs_something(app.window));

        let (_carrier, id) = page(&app).await;
        let session = start(&app, pane).await;
        // Nothing is known yet of what has the terminal, so something may run.
        assert!(app.panes.runs_something(app.window));

        attach(&app, id, &session, 1).await;
        let panes = Arc::clone(&app.panes);
        let window = app.window;
        until("the shell to be seen at its prompt", || {
            !panes.runs_something(window)
        })
        .await;

        // A program the shell runs is something closing would end.
        let program = if cfg!(windows) {
            "ping -n 6 127.0.0.1 >nul\r"
        } else {
            "sleep 5\r"
        };
        type_into(&app, id, 1, program).await;
        until("the program to be seen", || panes.runs_something(window)).await;
        assert!(app.panes.busy(pane));
        until("the shell to be back at its prompt", || {
            !panes.runs_something(window)
        })
        .await;

        // A program that has ended runs nothing, and its pane stays.
        type_into(&app, id, 1, "exit 3\r").await;
        let state = Arc::clone(&app.state);
        until("the program to have ended", || state.ended(&session)).await;
        assert!(!app.panes.runs_something(app.window));
        assert!(!app.panes.busy(pane));
        assert!(app.panes.workspace().pane(pane).is_some());
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_last_tab_closing_closes_the_window_and_the_application() {
        let app = running("last", "").await;
        let pane = only_pane(&app);
        app.panes.apply(Intent::Close { pane }).await.unwrap();
        let shell = app.shell.clone();
        until("the application to be told to close", || shell.quits() == 1).await;
        assert_eq!(app.shell.closed(), vec![label_of(app.window)]);
        assert!(app.panes.workspace().windows.is_empty());
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_window_stays_while_a_pane_floats_over_it_or_is_put_away() {
        let app = running("floats", "").await;
        let pane = only_pane(&app);
        // The only pane floats: the window has no tab, and is not empty.
        app.panes
            .apply(Intent::Float { pane, rect: None })
            .await
            .unwrap();
        app.panes
            .apply(Intent::Minimise { pane, on: true })
            .await
            .unwrap();
        assert!(app.panes.is_minimised(pane));
        sleep(Duration::from_millis(300)).await;
        assert_eq!(app.shell.quits(), 0);
        assert_eq!(app.panes.workspace().windows.len(), 1);

        // Once that pane is closed there is nothing left, and the window goes.
        app.panes.apply(Intent::Close { pane }).await.unwrap();
        let shell = app.shell.clone();
        until("the application to be told to close", || shell.quits() == 1).await;
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn kept_running_the_last_window_is_given_a_new_tab_and_hidden() {
        let app = running("ready", "[app]\nkeep-running = true\n").await;
        let pane = only_pane(&app);
        app.panes.apply(Intent::Close { pane }).await.unwrap();
        let shell = app.shell.clone();
        let window = app.window;
        until("the window to be hidden", || shell.hidden() == vec![window]).await;
        assert_eq!(app.shell.quits(), 0);
        assert!(app.shell.closed().is_empty());
        let workspace = app.panes.workspace();
        assert_eq!(workspace.windows.len(), 1);
        let fresh = only_pane(&app);
        assert_ne!(fresh, pane, "the window has a new pane");
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_window_may_be_kept_when_its_last_tab_closes() {
        let app = running("keep", "[window]\nclose-with-last-tab = false\n").await;
        app.panes
            .apply(Intent::Close {
                pane: only_pane(&app),
            })
            .await
            .unwrap();
        sleep(Duration::from_millis(300)).await;
        assert_eq!(app.shell.quits(), 0);
        assert_eq!(app.panes.workspace().windows.len(), 1);
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_reloaded_page_finds_every_session_still_running() {
        let app = running("reload", "").await;
        let first = only_pane(&app);
        let (_old, old) = page(&app).await;
        let one = start(&app, first).await;
        let second = app
            .panes
            .apply(split(first))
            .await
            .unwrap()
            .created
            .pane
            .unwrap();
        let two = app.panes.session_of(second).unwrap();
        attach(&app, old, &one, 1).await;
        attach(&app, old, &two, 2).await;

        // The page reloads: a new carrier opens and attaches to what the layout holds.
        let (carrier, id) = page(&app).await;
        assert_eq!(listed(&app).await, 2);
        attach(&app, id, &one, 1).await;
        attach(&app, id, &two, 2).await;
        type_into(
            &app,
            id,
            2,
            std::str::from_utf8(crate::testing::PRINT_MARK).unwrap(),
        )
        .await;
        until("output on the new page", || {
            carrier.messages().iter().any(|message| {
                matches!(&message.body, Body::Data(bytes)
                    if message.stream == 2 && String::from_utf8_lossy(bytes).contains("lume-"))
            })
        })
        .await;
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_page_may_attach_only_to_sessions_in_its_own_windows_panes() {
        let app = running("foreign", "").await;
        let (_carrier, id) = page(&app).await;
        // A session that is in no pane: started behind the layout's back.
        let stray = app
            .state
            .router()
            .get(&Router::local_id())
            .unwrap()
            .spawn(crate::testing::shell())
            .await
            .unwrap();
        let refused = app
            .state
            .attach(
                &label_of(app.window),
                id,
                &Router::local_id(),
                stray,
                1,
                facts(),
            )
            .await
            .unwrap_err();
        assert_eq!(refused.code, "forbidden");

        // Nor may another window's page attach to this window's sessions.
        let key = start(&app, only_pane(&app)).await;
        let other = app.state.open("w-999", RecordingCarrier::new()).await;
        let refused = app
            .state
            .attach("w-999", other, &key.backend, key.session, 1, facts())
            .await
            .unwrap_err();
        assert_eq!(refused.code, "forbidden");
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pages_are_told_of_changes_in_the_order_they_happened() {
        let app = running("order", "").await;
        let (carrier, _id) = page(&app).await;
        let at = app.panes.snapshot().revision;
        let pane = only_pane(&app);
        for number in 0..5 {
            app.panes
                .apply(Intent::PaneRename {
                    pane,
                    title: Some(format!("name {number}")),
                })
                .await
                .unwrap();
        }
        // A change from before the page connected may still be on its way to it; a
        // page passes over what its view of the layout already holds.
        let since = || -> Vec<Value> {
            notices(&carrier, "layout_changed")
                .into_iter()
                .filter(|notice| notice["revision"].as_u64().unwrap() > at)
                .collect()
        };
        until("the notices", || since().len() == 5).await;
        let revisions: Vec<u64> = since()
            .iter()
            .map(|notice| notice["revision"].as_u64().unwrap())
            .collect();
        assert_eq!(revisions, (at + 1..=at + 5).collect::<Vec<u64>>());
        let last = &since()[4];
        assert_eq!(last["epoch"], 1);
        assert_eq!(last["changes"][0]["type"], "pane");
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_page_is_asked_how_large_a_new_pane_will_be() {
        let app = running("estimate", "").await;
        let (carrier, id) = page(&app).await;
        let first = only_pane(&app);

        let state = Arc::clone(&app.state);
        let label = label_of(app.window);
        let answering = tokio::spawn(async move {
            until("the request to the page", || {
                carrier
                    .requests()
                    .iter()
                    .any(|message| message["request"]["body"]["type"] == "estimate")
            })
            .await;
            let request = carrier
                .requests()
                .into_iter()
                .find(|message| message["request"]["body"]["type"] == "estimate")
                .unwrap();
            assert_eq!(request["request"]["body"]["intent"]["type"], "split");
            let asked = request["request"]["id"].as_u64().unwrap();
            let answer = AppReplyBody::Estimated { size: size(57, 19) };
            state.answer(&label, id, asked, Ok(answer)).unwrap();
        });
        let second = app
            .panes
            .apply(split(first))
            .await
            .unwrap()
            .created
            .pane
            .unwrap();
        answering.await.unwrap();

        let key = app.panes.session_of(second).unwrap();
        let sessions = app.state.list_all().await.unwrap();
        let started = sessions
            .iter()
            .find(|listed| listed.info.id == key.session)
            .unwrap();
        assert_eq!(started.info.size, size(57, 19));
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_workspace_comes_back_from_its_file_without_its_sessions() {
        let dir = TempDir::new("restore");
        plain_shell(&dir, "");
        let dirs = dir.dirs();
        let app = running_in(dir).await;
        let first = only_pane(&app);
        start(&app, first).await;
        let second = app
            .panes
            .apply(split(first))
            .await
            .unwrap()
            .created
            .pane
            .unwrap();
        app.panes
            .apply(Intent::TabNew {
                window: app.window,
                content: NewContent::Terminal(SpawnRequest::default()),
                index: None,
                activate: true,
                group: None,
            })
            .await
            .unwrap();
        app.panes
            .apply(Intent::Focus {
                target: FocusTarget::Pane { pane: second },
            })
            .await
            .unwrap();
        let before = app.panes.workspace();
        app.panes.close();
        app.state.shutdown().await;

        let again = running_in(TempDir::at(dirs)).await;
        let after = again.panes.workspace();
        assert_eq!(after.windows[0].centre.tabs.len(), 2);
        assert_eq!(after.windows[0].focus, Some(second));
        assert_eq!(after.panes().len(), before.panes().len());
        assert!(after.sessions().is_empty());
        // Each pane remembers where its session was started.
        let home = std::env::home_dir().unwrap().to_string_lossy().into_owned();
        for pane in after.panes() {
            assert_eq!(
                terminal_of(pane).unwrap().cwd.as_deref(),
                Some(home.as_str())
            );
        }
        again.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_workspace_file_that_cannot_be_read_is_set_aside_and_said_so() {
        let dir = TempDir::new("corrupt");
        plain_shell(&dir, "");
        let folder = dir.dirs().state_workspaces_dir();
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(format!("{WORKSPACE_NAME}.json")), "{ not json").unwrap();

        let app = running_in(dir).await;
        assert_eq!(app.panes.workspace().windows.len(), 1);
        let pending = app.panes.take_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0]["level"], "warning");
        assert!(app.panes.take_pending().is_empty());
        let aside = std::fs::read_dir(&folder)
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"));
        assert!(aside);
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_window_the_layout_gains_is_opened_and_one_it_loses_is_closed() {
        let app = running("windows", "").await;
        let opened = app
            .panes
            .apply(Intent::WindowNew {
                content: NewContent::Terminal(SpawnRequest::default()),
                bounds: None,
            })
            .await
            .unwrap();
        let second = opened.created.window.unwrap();
        let shell = app.shell.clone();
        until("the window to be opened", || {
            shell.open() == vec![label_of(second)]
        })
        .await;
        // Its pane waits for its page before it has a session.
        assert_eq!(app.panes.session_of(opened.created.pane.unwrap()), None);

        app.panes
            .apply(Intent::WindowClose { window: second })
            .await
            .unwrap();
        let shell = app.shell.clone();
        until("the window to be closed", || {
            shell.closed() == vec![label_of(second)]
        })
        .await;
        assert_eq!(app.shell.quits(), 0);
        assert_eq!(app.panes.workspace().windows.len(), 1);
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn another_workspace_is_put_on_show_and_the_first_ones_sessions_end() {
        let dir = TempDir::new("switch");
        plain_shell(&dir, "");
        let dirs = dir.dirs();
        let app = running_in(dir).await;
        let (carrier, _id) = page(&app).await;
        let first = only_pane(&app);
        start(&app, first).await;
        app.panes.apply(split(first)).await.unwrap();
        assert_eq!(listed(&app).await, 2);

        app.panes.switch("play", Find::New).await.unwrap();
        let play = app.panes.workspace();
        assert_eq!(play.name, "play");
        assert_eq!(play.panes().len(), 1);
        assert_eq!(app.panes.snapshot().epoch, 2);
        assert_eq!(app.panes.config.workspace().as_deref(), Some("play"));
        assert_eq!(
            crate::workspaces::last_workspace(&dirs),
            Some("play".to_owned())
        );
        // The page is given the new layout whole, and the old sessions are ended.
        until("the page to be given the layout", || {
            notices(&carrier, "layout_reset")
                .iter()
                .any(|notice| notice["epoch"] == 2 && notice["workspace"]["name"] == "play")
        })
        .await;
        let state = Arc::clone(&app.state);
        let waiting = async {
            while !state.list_all().await.unwrap().is_empty() {
                sleep(Duration::from_millis(20)).await;
            }
        };
        timeout(STEP, waiting)
            .await
            .expect("the sessions of the workspace that was put away never ended");

        // There is no such workspace to begin again, and none called `nowhere` to show.
        assert_eq!(
            app.panes.switch("PLAY", Find::New).await.unwrap_err().code,
            "exists"
        );
        assert_eq!(
            app.panes
                .switch("nowhere", Find::Existing)
                .await
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(
            app.panes
                .switch("../x", Find::Either)
                .await
                .unwrap_err()
                .code,
            "bad_request"
        );

        // The first workspace comes back as it was left, without its sessions.
        app.panes
            .switch(WORKSPACE_NAME, Find::Existing)
            .await
            .unwrap();
        let back = app.panes.workspace();
        assert_eq!(back.name, WORKSPACE_NAME);
        assert_eq!(back.panes().len(), 2);
        assert!(back.sessions().is_empty());
        assert_eq!(app.panes.snapshot().epoch, 3);

        let (active, workspaces) = app.panes.workspaces().unwrap();
        assert_eq!(active, WORKSPACE_NAME);
        let names: Vec<&str> = workspaces.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, vec![WORKSPACE_NAME, "play"]);
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_group_moves_to_another_workspace_and_is_brought_back() {
        let dir = TempDir::new("move-group");
        plain_shell(&dir, "");
        let app = running_in(dir).await;
        let (carrier, _id) = page(&app).await;
        // Another workspace to move to, written to its file.
        app.panes.switch("play", Find::New).await.unwrap();
        app.panes
            .switch(WORKSPACE_NAME, Find::Existing)
            .await
            .unwrap();
        let window = app.panes.focused_window().unwrap();
        let tab = app
            .panes
            .apply(Intent::TabNew {
                window,
                content: NewContent::Terminal(SpawnRequest::default()),
                index: None,
                activate: true,
                group: None,
            })
            .await
            .unwrap()
            .created
            .tab
            .unwrap();
        let group = app
            .panes
            .apply(Intent::GroupNew { tab })
            .await
            .unwrap()
            .created
            .group
            .unwrap();
        app.panes
            .apply(Intent::GroupSet {
                group,
                name: Some(Some("Billing".to_owned())),
                color: None,
                icon: None,
                collapsed: None,
                kept: None,
                muted: None,
            })
            .await
            .unwrap();

        assert_eq!(
            app.panes
                .move_group(group, "Default")
                .await
                .unwrap_err()
                .code,
            "bad_request"
        );
        assert_eq!(
            app.panes
                .move_group(group, "nowhere")
                .await
                .unwrap_err()
                .code,
            "not_found"
        );
        app.panes.move_group(group, "play").await.unwrap();
        let here = app.panes.workspace();
        assert!(here.tab(tab).is_none());
        assert!(here.windows[0].centre.groups.is_empty());
        let lume_layout::Loaded::Workspace(play) = app.panes.store.load("play", 0).unwrap() else {
            panic!("the other workspace was not written");
        };
        let moved = &play.windows[0].centre.groups;
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].name.as_deref(), Some("Billing"));
        assert_eq!(play.windows[0].centre.tabs.len(), 2);

        // The page is told, with a way back.
        let mut action = None;
        until("the page to be told where the group went", || {
            action = notices(&carrier, "notice")
                .into_iter()
                .find(|notice| notice["message"] == "Moved Billing to play")
                .map(|notice| notice["action"].clone());
            action.is_some()
        })
        .await;
        let action = action.unwrap();
        assert_eq!(action["command"], "lume.group.bring-back");
        assert_eq!(action["args"]["workspace"], "play");
        let there: lume_layout::GroupId =
            action["args"]["group"].as_str().unwrap().parse().unwrap();
        assert_eq!(there, moved[0].id);

        app.panes
            .bring_group_back("play", there, window)
            .await
            .unwrap();
        let here = app.panes.workspace();
        let back = &here.windows[0].centre.groups;
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].name.as_deref(), Some("Billing"));
        assert_eq!(here.windows[0].centre.tabs.len(), 2);
        let lume_layout::Loaded::Workspace(play) = app.panes.store.load("play", 0).unwrap() else {
            panic!("the other workspace was not written");
        };
        assert!(play.windows[0].centre.groups.is_empty());
        assert_eq!(play.windows[0].centre.tabs.len(), 1);
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_workspace_is_renamed_given_a_folder_and_deleted_with_both_its_files() {
        let dir = TempDir::new("rename");
        plain_shell(&dir, "");
        let dirs = dir.dirs();
        let app = running_in(dir).await;
        let (carrier, _id) = page(&app).await;
        let here = std::env::temp_dir().to_string_lossy().into_owned();

        app.panes.set_root(Some(&here)).await.unwrap();
        assert_eq!(
            app.panes.config.current().workspace.root.as_deref(),
            Some(here.as_str())
        );
        assert_eq!(
            app.panes
                .set_root(Some("/lume-no-such-directory-k7q"))
                .await
                .unwrap_err()
                .code,
            "not_found"
        );

        app.panes.rename("site").await.unwrap();
        assert_eq!(app.panes.workspace().name, "site");
        // The same layout under another name: no new epoch, and the pages are told.
        assert_eq!(app.panes.snapshot().epoch, 1);
        until("the page to be told the name", || {
            notices(&carrier, "layout_changed").iter().any(|notice| {
                notice["changes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|change| change["type"] == "workspace_name" && change["name"] == "site")
            })
        })
        .await;
        assert!(dirs.state_workspace_file("site").is_file());
        assert!(!dirs.state_workspace_file(WORKSPACE_NAME).exists());
        // What the user wrote about the workspace went with it.
        assert!(dirs.workspace_file("site").is_file());
        assert!(!dirs.workspace_file(WORKSPACE_NAME).exists());
        assert_eq!(
            app.panes.config.current().workspace.root.as_deref(),
            Some(here.as_str())
        );

        // The workspace on show is not deleted from under the windows.
        assert_eq!(
            app.panes.delete("site").await.unwrap_err().code,
            "bad_request"
        );
        app.panes.switch("play", Find::New).await.unwrap();
        // A name another workspace has is not taken.
        assert_eq!(app.panes.rename("Site").await.unwrap_err().code, "exists");
        assert_eq!(app.panes.workspace().name, "play");

        app.panes.delete("site").await.unwrap();
        assert!(!dirs.state_workspace_file("site").exists());
        assert!(!dirs.workspace_file("site").exists());
        assert_eq!(
            app.panes.delete("site").await.unwrap_err().code,
            "not_found"
        );
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_list_of_tabs_is_a_panel_at_the_side_the_configuration_names() {
        let app = running("tabpanel", "[layout]\ntab-placement = \"left\"\n").await;
        let tabs_at = |window: &lume_layout::Window| -> Vec<Area> {
            [Area::Left, Area::Right, Area::Bottom]
                .into_iter()
                .filter(|area| {
                    let panels = &window.dock.area(*area).panels;
                    panels
                        .iter()
                        .any(|panel| panel.id.as_str() == TAB_LIST_PANEL)
                })
                .collect()
        };
        assert_eq!(tabs_at(&app.panes.workspace().windows[0]), vec![Area::Left]);
        assert_eq!(app.panes.dock_to_toggle(app.window), Some(Area::Left));

        // A window that is opened later has the panel too.
        app.panes
            .apply(Intent::WindowNew {
                content: NewContent::Terminal(SpawnRequest::default()),
                bounds: None,
            })
            .await
            .unwrap();
        let panes = Arc::clone(&app.panes);
        until("the new window to have the list of tabs", || {
            tabs_at(&panes.workspace().windows[1]) == vec![Area::Left]
        })
        .await;

        // The setting moves it, and takes it away.
        app.panes
            .config
            .set("layout.tab-placement", &json!("right"))
            .unwrap();
        app.panes.sync_tab_panel().await;
        for window in &app.panes.workspace().windows {
            assert_eq!(tabs_at(window), vec![Area::Right]);
        }
        app.panes
            .config
            .set("layout.tab-placement", &json!("top"))
            .unwrap();
        app.panes.sync_tab_panel().await;
        for window in &app.panes.workspace().windows {
            assert_eq!(tabs_at(window), vec![]);
        }
        assert_eq!(app.panes.dock_to_toggle(app.window), None);
        app.state.shutdown().await;
    }

    /// The URL a shell on this machine reports when it is in `dir`.
    fn reported(dir: &Path) -> String {
        let machine = lume_session::cwd::machine_name().unwrap();
        let path = dir
            .to_string_lossy()
            .replace('\\', "/")
            .replace('%', "%25")
            .replace(' ', "%20");
        let slash = if path.starts_with('/') { "" } else { "/" };
        format!("file://{machine}{slash}{path}")
    }

    fn says_it_is_in(app: &Running, key: &SessionKey, url: &str) {
        let event = lume_session::SessionEvent::Cwd {
            url: url.to_owned(),
        };
        app.state.reports().on_event(key, &event);
    }

    fn context(app: &Running) -> crate::builtins::Context<'_> {
        crate::builtins::Context {
            panes: &app.panes,
            config: &app.panes.config,
            shell: app.shell.as_ref(),
            window: app.window,
            asker: crate::services::Asker::Outside,
        }
    }

    fn cwd_of(app: &Running, pane: PaneId) -> Option<String> {
        app.panes.terminal(pane).and_then(|terminal| terminal.cwd)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_terminal_opened_from_a_pane_starts_where_that_panes_shell_is() {
        let app = running("inherit", "").await;
        let here = app._dir.dirs().config;
        let pane = only_pane(&app);
        let key = start(&app, pane).await;

        // A shell that has not said where it is gives nothing to go by.
        assert_eq!(app.panes.inherited_dir(pane, None).await, None);
        says_it_is_in(&app, &key, &reported(&here));
        let found = app.panes.inherited_dir(pane, None).await.unwrap();
        assert_eq!(Path::new(&found), here);

        // A split is opened from the pane that is split.
        crate::builtins::run(&context(&app), "lume.pane.split-right", &json!({}))
            .await
            .unwrap();
        let opened: Vec<PaneId> = app
            .panes
            .workspace()
            .panes()
            .iter()
            .map(|pane| pane.id)
            .filter(|opened| *opened != pane)
            .collect();
        assert_eq!(opened.len(), 1);
        for opened in opened {
            assert_eq!(cwd_of(&app, opened).as_deref().map(Path::new), Some(&*here));
        }
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_directory_that_does_not_check_out_is_not_started_in() {
        let app = running("spoof", "").await;
        let here = app._dir.dirs().config;
        let pane = only_pane(&app);
        let key = start(&app, pane).await;
        let machine = lume_session::cwd::machine_name().unwrap();

        // Another machine's directory, though one of that name is here too.
        let elsewhere = reported(&here).replacen(&machine, "some-other-machine", 1);
        says_it_is_in(&app, &key, &elsewhere);
        assert_eq!(app.panes.inherited_dir(pane, None).await, None);
        // A directory that is not there.
        says_it_is_in(&app, &key, &reported(&here.join("gone")));
        assert_eq!(app.panes.inherited_dir(pane, None).await, None);
        // Something that is no directory at all.
        says_it_is_in(&app, &key, "javascript:alert(1)");
        assert_eq!(app.panes.inherited_dir(pane, None).await, None);

        // A split still opens, where panes start when nothing says otherwise.
        crate::builtins::run(&context(&app), "lume.pane.split-right", &json!({}))
            .await
            .unwrap();
        let home = std::env::home_dir().map(|home| home.to_string_lossy().into_owned());
        let opened = app.panes.workspace().panes()[1].id;
        assert_eq!(cwd_of(&app, opened), home);
        // Nothing a shell reports moves the workspace's own folder.
        assert_eq!(app.panes.config.current().workspace.root, None);
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_session_on_another_machine_never_gives_a_directory() {
        let app = running("remote", "").await;
        let here = app._dir.dirs().config;
        let pane = only_pane(&app);
        let key = start(&app, pane).await;
        // The report is as good as one gets, but the session is said to run elsewhere.
        app.state.reports().started(
            key.clone(),
            Launcher::Ssh {
                host: "box".to_owned(),
                user: None,
                port: None,
                extra_args: Vec::new(),
                client: None,
            },
            None,
        );
        says_it_is_in(&app, &key, &reported(&here));
        assert_eq!(app.panes.inherited_dir(pane, None).await, None);
        assert_eq!(app.panes.live_dir(pane).await, None);
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn new_panes_keep_to_the_workspace_when_the_settings_say_so() {
        let app = running("inherit-off", "[layout]\nnew-pane-cwd = \"workspace\"\n").await;
        let here = app._dir.dirs().config;
        let pane = only_pane(&app);
        let key = start(&app, pane).await;
        says_it_is_in(&app, &key, &reported(&here));
        assert_eq!(app.panes.inherited_dir(pane, None).await, None);
        // Where the shell is can still be asked for.
        let (_, dir) = app.panes.live_dir(pane).await.unwrap();
        assert_eq!(Path::new(&dir), here);
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_workspaces_file_remembers_where_each_shell_is_now() {
        let dir = TempDir::new("live");
        plain_shell(&dir, "");
        let dirs = dir.dirs();
        let app = running_in(dir).await;
        let here = dirs.config.clone();
        let pane = only_pane(&app);
        let key = start(&app, pane).await;
        says_it_is_in(&app, &key, &reported(&here));

        app.panes.save().await;
        // The layout itself is not changed by where a shell goes.
        let home = std::env::home_dir().map(|home| home.to_string_lossy().into_owned());
        assert_eq!(cwd_of(&app, pane), home);
        app.panes.close();
        app.state.shutdown().await;

        let again = running_in(TempDir::at(dirs)).await;
        let kept = cwd_of(&again, only_pane(&again)).unwrap();
        assert_eq!(Path::new(&kept), here);
        again.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_pane_whose_program_ended_is_started_again_and_one_that_runs_is_not() {
        let app = running("restart", "").await;
        let (carrier, id) = page(&app).await;
        let pane = only_pane(&app);
        let first = start(&app, pane).await;
        attach(&app, id, &first, 1).await;
        says_it_is_in(&app, &first, "file://somewhere/else");

        let refused = app.panes.restart(pane).await.unwrap_err();
        assert_eq!(refused.code, "bad_request");
        assert_eq!(app.panes.session_of(pane), Some(first.clone()));

        type_into(&app, id, 1, "exit 3\r").await;
        until("the program's end", || app.state.ended(&first)).await;
        let second = app.panes.restart(pane).await.unwrap();
        assert_ne!(second, first);
        assert_eq!(app.panes.session_of(pane), Some(second.clone()));
        // What the first program said about itself went with it.
        assert_eq!(app.state.reports().state(&first), None);
        // The page is told, so that it shows the new session.
        until("the pane's new session", || {
            notices(&carrier, "layout_changed")
                .iter()
                .any(|notice| notice.to_string().contains(&second.session.to_string()))
        })
        .await;
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_directory_of_a_pane_is_copied_and_refused_where_it_is_not_known() {
        let app = running("copy", "").await;
        let here = app._dir.dirs().config;
        let pane = only_pane(&app);
        let key = start(&app, pane).await;
        let refused = crate::builtins::run(&context(&app), "lume.pane.copy-cwd", &json!({}))
            .await
            .unwrap_err();
        assert_eq!(refused.code, "not_found");

        says_it_is_in(&app, &key, &reported(&here));
        crate::builtins::run(&context(&app), "lume.pane.copy-cwd", &json!({}))
            .await
            .unwrap();
        let copied = app.shell.copied();
        assert_eq!(copied.len(), 1);
        assert_eq!(Path::new(&copied[0]), here);
        app.state.shutdown().await;
    }

    #[test]
    fn two_sessions_are_in_one_place_on_this_machine_or_in_one_distribution() {
        let shell = |program: &str| Launcher::Shell {
            program: program.to_owned(),
            args: Vec::new(),
            login: false,
        };
        let wsl = |distro: &str, user: Option<&str>| Launcher::Wsl {
            distro: distro.to_owned(),
            user: user.map(str::to_owned),
        };
        let ssh = Launcher::Ssh {
            host: "box".to_owned(),
            user: None,
            port: None,
            extra_args: Vec::new(),
            client: None,
        };
        assert!(same_place(&shell("pwsh"), &shell("cmd.exe")));
        assert!(same_place(&wsl("Ubuntu", None), &wsl("ubuntu", None)));
        assert!(!same_place(&wsl("Ubuntu", None), &wsl("Debian", None)));
        assert!(!same_place(
            &wsl("Ubuntu", None),
            &wsl("Ubuntu", Some("root"))
        ));
        assert!(!same_place(&shell("bash"), &wsl("Ubuntu", None)));
        // Two connections to one machine are still not this machine.
        assert!(!same_place(&ssh, &ssh));
        assert!(!same_place(&ssh, &shell("bash")));
    }

    #[test]
    fn equal_shares_add_up_to_a_whole() {
        for count in 1..=20 {
            let ratios = equal_ratios(count);
            assert_eq!(ratios.len(), count);
            assert_eq!(ratios.iter().sum::<u32>(), UNITS);
            assert!(ratios.iter().max().unwrap() - ratios.iter().min().unwrap() < count as u32);
        }
    }

    #[test]
    fn a_session_starts_where_its_pane_was_or_in_the_workspace_or_at_home() {
        let here = std::env::temp_dir().to_string_lossy().into_owned();
        let home = std::env::home_dir().map(|home| home.to_string_lossy().into_owned());
        let gone = "/lume-no-such-directory-k7q";
        assert_eq!(
            start_dir(Some(&here), None, NewPaneCwd::Home),
            Some(here.clone())
        );
        // A pane whose directory has gone falls back to the workspace's folder.
        assert_eq!(
            start_dir(Some(gone), Some(&here), NewPaneCwd::Home),
            Some(here.clone())
        );
        assert_eq!(
            start_dir(None, Some(&here), NewPaneCwd::Workspace),
            Some(here.clone())
        );
        assert_eq!(
            start_dir(None, Some(&here), NewPaneCwd::Inherit),
            Some(here.clone())
        );
        assert_eq!(start_dir(None, Some(&here), NewPaneCwd::Home), home);
        assert_eq!(start_dir(None, Some(gone), NewPaneCwd::Workspace), home);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_session_shown_in_two_panes_ends_with_the_last_of_them() {
        let app = running("two-views", "").await;
        let (_carrier, id) = page(&app).await;
        let first = only_pane(&app);
        let key = start(&app, first).await;

        crate::host::reattach(&app.panes, app.window, key.clone(), crate::host::Place::Tab)
            .await
            .unwrap();
        let shown = app.panes.panes_of(&key);
        assert_eq!(shown.len(), 2, "{shown:?}");
        let second = shown[1];
        assert!(app.panes.owns(app.window, &key));
        // The window's page may attach to it once for each pane.
        let label = label_of(app.window);
        for stream in [1, 2] {
            app.state
                .attach(&label, id, &key.backend, key.session, stream, facts())
                .await
                .unwrap();
        }

        let local = app.state.router().get(&Router::local_id()).unwrap();
        let running = async || {
            local
                .list()
                .await
                .unwrap()
                .iter()
                .any(|info| info.id == key.session && info.exit.is_none())
        };
        app.panes
            .apply(Intent::Close { pane: first })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(
            running().await,
            "closing one pane ended what the other shows"
        );
        assert_eq!(app.panes.panes_of(&key), vec![second]);

        app.panes
            .apply(Intent::Close { pane: second })
            .await
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while running().await {
            assert!(
                std::time::Instant::now() < deadline,
                "the session outlived its last pane"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        app.state.shutdown().await;
    }
}
