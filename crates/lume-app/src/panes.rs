//! The panes of the application's windows, and the sessions in them.
//!
//! The layout says which panes there are; this says what follows from that. A terminal
//! pane has a session for as long as the pane exists: the session is started when the
//! pane is made, or when a page first knows how large the pane is, and is ended when
//! the pane closes. A page that is reloaded changes nothing here. It reads the layout
//! and attaches to the sessions it finds in it.
//!
//! One task follows the layout's changes. It tells every page, ends the sessions of
//! panes that have closed, closes a window whose last tab has gone, and writes the
//! workspace to its file a moment after it last changed.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use lume_config::schema::{CloseOnExit, NewPaneCwd};
use lume_layout::{
    Applied, ChangeStream, Intent, LayoutAuthority, LayoutChange, LayoutEvent, LayoutSnapshot,
    Loaded, LocalLayout, NewContent, Pane, PaneContent, PaneId, SpawnFailure, SpawnRequest,
    Spawner, Started, Store, UNITS, WindowId, Workspace, WorkspaceId, needs,
};
use lume_proto::app::{AppReplyBody, PageRequestBody};
use lume_session::{
    BackendId, ExitStatus, Launcher, SessionId, SpawnSpec, TermSize, default_shell,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::carrier::SessionKey;
use crate::configsvc::ConfigService;
use crate::control::Shell;
use crate::error::CommandError;
use crate::state::AppState;
use crate::tuning::{
    DEFAULT_SPAWN_COLS, DEFAULT_SPAWN_ROWS, ESTIMATE_TIMEOUT, PENDING_NOTICES_MAX, SAVE_DEBOUNCE,
    SAVE_MAX_DELAY,
};
use crate::window::label_of;

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

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
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

/// The directory a session starts in. `asked` is where its pane was last started; a
/// directory that is no longer there is passed over.
fn start_dir(asked: Option<&str>, root: Option<&str>, new_panes: NewPaneCwd) -> Option<String> {
    let there = |dir: &&str| Path::new(dir).is_dir();
    if let Some(dir) = asked.filter(there) {
        return Some(dir.to_owned());
    }
    // A pane that had a directory belongs to the workspace's work, whatever new panes
    // are set to start in.
    let in_root = asked.is_some() || new_panes == NewPaneCwd::Workspace;
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

/// Starts and ends the sessions of panes on the application's backends.
struct PaneSpawner {
    state: Weak<AppState>,
    config: Arc<ConfigService>,
}

impl PaneSpawner {
    fn failure(error: impl Into<CommandError>) -> SpawnFailure {
        let error = error.into();
        SpawnFailure {
            code: error.code.into_owned(),
            message: error.message,
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
        let terminal = &resolved.config.terminal;
        let backend = request
            .backend
            .clone()
            .unwrap_or_else(|| state.default_backend());
        let cwd = start_dir(
            request.cwd.as_deref(),
            resolved.workspace.root.as_deref(),
            resolved.config.layout.new_pane_cwd,
        );
        let spec = SpawnSpec {
            launcher: request
                .launcher
                .clone()
                .unwrap_or_else(|| start_launcher(&terminal.shell, &terminal.shell_args)),
            cwd: cwd.clone(),
            env: request.env.clone(),
            size: request.size.unwrap_or_else(default_size),
        };
        let session = state
            .router()
            .get(&backend)
            .map_err(Self::failure)?
            .spawn(spec)
            .await
            .map_err(Self::failure)?;
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
        if let Err(error) = state.end(&key).await {
            tracing::debug!(session = %key.session, %error, "a session nobody needs could not be ended");
        }
    }
}

/// The terminal a pane shows, if it shows one.
fn terminal_of(pane: &Pane) -> Option<&lume_layout::TerminalPane> {
    match &pane.content {
        PaneContent::Terminal(terminal) => Some(terminal),
        PaneContent::PluginView { .. } => None,
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
    state: Weak<AppState>,
    layout: Arc<LocalLayout>,
    spawner: Arc<PaneSpawner>,
    config: Arc<ConfigService>,
    store: Arc<Store>,
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
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Panes {
    /// Reads the workspace called `name` from `store`, or begins one, and takes charge
    /// of its panes. The workspace has at least one window afterwards.
    pub async fn open(
        state: &Arc<AppState>,
        config: Arc<ConfigService>,
        store: Store,
        name: &str,
    ) -> Arc<Self> {
        let mut pending = Vec::new();
        let workspace = match store.load(name, now_unix_ms()) {
            Ok(Loaded::Workspace(workspace)) => *workspace,
            Ok(Loaded::Missing) => Workspace::new(fresh_workspace_id(), name),
            Ok(Loaded::SetAside { to, reason }) => {
                tracing::warn!(file = %to.display(), reason, "the workspace's file was set aside");
                pending.push(json!({
                    "type": "notice",
                    "level": "warning",
                    "message": format!(
                        "The saved workspace could not be read ({reason}) and a new one was \
                         begun. The file is kept as {}.",
                        to.display()
                    ),
                }));
                Workspace::new(fresh_workspace_id(), name)
            }
            Err(error) => {
                // Nothing was moved or lost; the workspace is simply not saved from here
                // if the same fault stops the writing too.
                tracing::error!(%error, "the workspace's file could not be read");
                Workspace::new(fresh_workspace_id(), name)
            }
        };
        let spawner = Arc::new(PaneSpawner {
            state: Arc::downgrade(state),
            config: Arc::clone(&config),
        });
        let layout = LocalLayout::new(workspace, 1, spawner.clone());
        // Taken before anything can change the layout, so the task that follows it
        // misses nothing.
        let changes = layout.changes();
        let panes = Arc::new(Self {
            state: Arc::downgrade(state),
            layout,
            spawner,
            config,
            store: Arc::new(store),
            changes: Mutex::new(Some(changes)),
            starting: tokio::sync::Mutex::new(()),
            pending: Mutex::new(pending),
            closing: AtomicBool::new(false),
        });
        if panes.layout.workspace().windows.is_empty() {
            let opened = panes
                .layout
                .apply_core(Intent::WindowNew {
                    content: NewContent::Terminal(SpawnRequest::default()),
                    bounds: None,
                })
                .await;
            if let Err(error) = opened {
                tracing::error!(%error, "the workspace's first window could not be made");
            }
        }
        panes
    }

    /// The layout as it is now.
    pub fn snapshot(&self) -> LayoutSnapshot {
        self.layout.snapshot()
    }

    pub fn workspace(&self) -> Workspace {
        self.layout.workspace()
    }

    /// The layout as the page of `window` is given it.
    pub fn view(&self, window: WindowId) -> Value {
        let snapshot = self.snapshot();
        json!({
            "window": window,
            "epoch": snapshot.epoch,
            "revision": snapshot.revision,
            "workspace": snapshot.workspace,
        })
    }

    /// What a page that has just connected is to be told, once.
    pub fn take_pending(&self) -> Vec<Value> {
        std::mem::take(&mut *lock(&self.pending))
    }

    /// Tells every page something, or keeps it for the first page if there is none yet.
    pub fn notice(&self, level: &str, message: impl Into<String>) {
        let notice = json!({ "type": "notice", "level": level, "message": message.into() });
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

    /// The session a pane shows, if it is a terminal that has one.
    pub fn session_of(&self, pane: PaneId) -> Option<SessionKey> {
        let workspace = self.workspace();
        let terminal = terminal_of(workspace.pane(pane)?)?;
        Some(SessionKey {
            backend: self.backend_of(terminal),
            session: terminal.session?,
        })
    }

    fn backend_of(&self, terminal: &lume_layout::TerminalPane) -> BackendId {
        terminal
            .backend
            .clone()
            .unwrap_or_else(lume_session::Router::local_id)
    }

    /// The pane that shows a session, if one does.
    pub fn pane_of(&self, key: &SessionKey) -> Option<PaneId> {
        let workspace = self.workspace();
        workspace.panes().into_iter().find_map(|pane| {
            let terminal = terminal_of(pane)?;
            (terminal.session == Some(key.session) && self.backend_of(terminal) == key.backend)
                .then_some(pane.id)
        })
    }

    /// Whether a session is in a pane of `window`: whether that window's page may
    /// attach to it.
    pub fn owns(&self, window: WindowId, key: &SessionKey) -> bool {
        self.pane_of(key)
            .is_some_and(|pane| self.workspace().window_of(pane) == Some(window))
    }

    /// Applies an intent that someone other than the application asked for.
    ///
    /// An intent that makes a terminal pane and does not say how large the pane will
    /// be has the window's page asked, since only the page knows the window's pixels.
    pub async fn apply(&self, intent: Intent) -> Result<Applied, CommandError> {
        if matches!(
            intent,
            Intent::WindowNew { .. } | Intent::WindowClose { .. }
        ) {
            return Err(CommandError::unsupported(
                "the application has one window; more cannot be opened yet",
            ));
        }
        let intent = self.sized(intent).await;
        Ok(self.layout.apply(intent).await?)
    }

    /// Applies an intent on the application's own behalf.
    pub async fn apply_core(&self, intent: Intent) -> Result<Applied, CommandError> {
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
        }
        let request = SpawnRequest {
            backend: terminal.backend.clone(),
            profile: terminal.profile.clone(),
            launcher: terminal.launcher.clone(),
            cwd: terminal.cwd.clone(),
            env: terminal.env.clone(),
            size: Some(size),
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

    /// Acts on the end of a session's program: its pane closes, or stays to say how
    /// the program ended, as the configuration has it.
    pub fn exited(self: &Arc<Self>, key: &SessionKey, status: ExitStatus) {
        if self.closing.load(Ordering::Relaxed) {
            return;
        }
        let Some(pane) = self.pane_of(key) else {
            return;
        };
        let close = match self.config.current().config.terminal.close_on_exit {
            CloseOnExit::Always => true,
            CloseOnExit::Clean => status == ExitStatus::Code(0),
            CloseOnExit::Never => false,
        };
        let Some(state) = self.state.upgrade() else {
            return;
        };
        if close {
            let panes = Arc::clone(self);
            state.runtime().spawn(async move {
                if let Err(error) = panes.layout.apply_core(Intent::Close { pane }).await {
                    tracing::debug!(%pane, %error, "the pane of a program that ended could not be closed");
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
                        self.act_on(&state, change, shell.as_ref()).await;
                    }
                }
                LayoutEvent::Reset(snapshot) => {
                    for (label, link) in state.links() {
                        let Some(window) = crate::window::window_of(&label) else {
                            continue;
                        };
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
            unsaved = Some((unsaved.map_or(now, |(first, _)| first), now));
            // Whoever waits for a pane to have a session looks again.
            state.wake();
        }
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
                if self.closing.load(Ordering::Relaxed)
                    || !self.config.current().config.window.close_with_last_tab
                {
                    return;
                }
                let closed = self
                    .layout
                    .apply_core(Intent::WindowClose { window: *window })
                    .await;
                if let Err(error) = closed {
                    tracing::debug!(%window, %error, "a window without tabs could not be closed");
                }
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

    /// Writes the workspace to its file.
    async fn save(&self) {
        let workspace = self.workspace();
        let store = Arc::clone(&self.store);
        let written = tokio::task::spawn_blocking(move || {
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
        let saved = self
            .store
            .save(&self.workspace(), env!("CARGO_PKG_VERSION"), now_unix_ms());
        if let Err(error) = saved {
            tracing::error!(%error, "the workspace could not be saved");
        }
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
    use lume_proto::app::codes;
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

    struct Running {
        state: Arc<AppState>,
        panes: Arc<Panes>,
        shell: Arc<FakeShell>,
        window: WindowId,
        _dir: TempDir,
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
        let panes = Panes::open(&state, config, store, WORKSPACE_NAME).await;
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
    async fn more_windows_cannot_be_opened_yet() {
        let app = running("windows", "").await;
        let refused = app
            .panes
            .apply(Intent::WindowNew {
                content: NewContent::Terminal(SpawnRequest::default()),
                bounds: None,
            })
            .await
            .unwrap_err();
        assert_eq!(refused.code, codes::UNSUPPORTED);
        app.state.shutdown().await;
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
        assert_eq!(start_dir(None, Some(&here), NewPaneCwd::Home), home);
        assert_eq!(start_dir(None, Some(gone), NewPaneCwd::Workspace), home);
    }
}
