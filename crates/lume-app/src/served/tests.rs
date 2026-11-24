use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use lume_layout::{
    Area, Dir, LayoutSnapshot, LocalLayout, NewContent, PaneId, SpawnFailure, SpawnRequest,
    Spawner, Started, Store, TabId, Window, WindowId, Workspace, WorkspaceId,
};
use lume_proto::{
    BackendCaps, ClientFacts, HydrateKinds, Responder, SessionId, SessionInfo, SpawnSpec,
};
use lume_session::{Attachment, HostConfig, LocalBackend, Router, SessionBackend, SessionError};
use tokio::runtime::Handle;
use tokio::time::{sleep, timeout};

use super::*;
use crate::configsvc::ConfigService;
use crate::marks::Marks;
use crate::state::AppState;
use crate::testing::{FakeShell, RecordingCarrier, STEP, TempDir};
use crate::tuning::WORKSPACE_NAME;
use crate::window::label_of;
use crate::workspaces::Find;

fn server_id() -> BackendId {
    BackendId("server".to_owned())
}

/// Gives each pane of the server's layout a session that is a number.
#[derive(Default)]
struct ServerSessions {
    next: AtomicU64,
}

#[async_trait]
impl Spawner for ServerSessions {
    async fn spawn(&self, request: &SpawnRequest) -> Result<Started, SpawnFailure> {
        let number = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        Ok(Started {
            backend: server_id(),
            session: SessionId::new(number).unwrap(),
            cwd: request.cwd.clone(),
        })
    }

    async fn discard(&self, _started: &Started) {}
}

/// A server that owns a layout: a layout of its own, and a note of what it was asked.
struct Server {
    layout: Arc<LocalLayout>,
    asked: Mutex<Vec<Intent>>,
    killed: AtomicUsize,
}

impl Server {
    /// A server whose layout has one window, numbered `window`, with one tab.
    async fn with_window(window: u32) -> Arc<Self> {
        let mut workspace = Workspace::new(WorkspaceId::new(7).unwrap(), "on the server");
        let id = WindowId::new(window);
        workspace.windows.push(Window::new(id));
        workspace.next_id = window + 1;
        let layout = LocalLayout::new(workspace, 1, Arc::new(ServerSessions::default()));
        layout
            .apply_core(Intent::TabNew {
                window: id,
                content: NewContent::Terminal(SpawnRequest::default()),
                index: None,
                activate: true,
                group: None,
            })
            .await
            .unwrap();
        Arc::new(Self {
            layout,
            asked: Mutex::new(Vec::new()),
            killed: AtomicUsize::new(0),
        })
    }

    fn panes(&self) -> Vec<PaneId> {
        self.layout
            .workspace()
            .panes()
            .into_iter()
            .map(|pane| pane.id)
            .collect()
    }

    fn asked(&self) -> Vec<Intent> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait]
impl LayoutAuthority for Server {
    async fn view(&self) -> Result<LayoutSnapshot, LayoutError> {
        Ok(self.layout.snapshot())
    }

    async fn apply(&self, intent: Intent) -> Result<Applied, LayoutError> {
        self.asked.lock().unwrap().push(intent.clone());
        self.layout.apply(intent).await
    }

    fn changes(&self) -> ChangeStream {
        self.layout.changes()
    }
}

struct ServerBackend(Arc<Server>);

fn refused() -> SessionError {
    SessionError::Unreachable("not in this test".to_owned())
}

#[async_trait]
impl SessionBackend for ServerBackend {
    fn caps(&self) -> BackendCaps {
        BackendCaps {
            persistent: true,
            multi_client: true,
            server_layout: true,
            responder: Responder::Backend,
            hydrate: HydrateKinds {
                snapshot: true,
                vt_replay: false,
            },
        }
    }

    async fn list(&self) -> lume_session::Result<Vec<SessionInfo>> {
        Ok(Vec::new())
    }

    async fn spawn(&self, _spec: SpawnSpec) -> lume_session::Result<SessionId> {
        Err(refused())
    }

    async fn attach(
        &self,
        _id: SessionId,
        _facts: ClientFacts,
    ) -> lume_session::Result<Attachment> {
        Err(refused())
    }

    async fn kill(&self, _id: SessionId) -> lume_session::Result<()> {
        self.0.killed.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn layout(&self) -> Option<Arc<dyn LayoutAuthority>> {
        Some(self.0.clone())
    }
}

struct Running {
    state: Arc<AppState>,
    panes: Arc<Panes>,
    shell: Arc<FakeShell>,
    server: Arc<Server>,
    dir: TempDir,
}

/// The application with a workspace of its own on show and a server it can reach.
async fn running(name: &str, server_window: u32) -> Running {
    let dir = TempDir::new(name);
    let local = Arc::new(
        LocalBackend::in_process(HostConfig::default())
            .await
            .unwrap(),
    );
    let server = Server::with_window(server_window).await;
    let router = Router::new();
    router.insert(Router::local_id(), local.clone());
    router.insert(server_id(), Arc::new(ServerBackend(server.clone())));
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
    Running {
        state,
        panes,
        shell,
        server,
        dir,
    }
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

fn split(pane: PaneId, dir: Dir) -> Intent {
    Intent::Split {
        pane,
        dir,
        ratio: 500_000,
        content: NewContent::Terminal(SpawnRequest::default()),
    }
}

#[tokio::test]
async fn a_servers_layout_is_shown_in_the_window_that_was_open() {
    let app = running("served-shown", 1).await;
    let window = app.panes.focused_window().unwrap();
    app.panes
        .apply_core(Intent::PanelToggle {
            window,
            area: Area::Left,
            on: Some(false),
        })
        .await
        .unwrap();
    let dock = app.panes.workspace().window(window).unwrap().dock.clone();

    app.panes.show_served(&server_id()).await.unwrap();

    let shown = app.panes.workspace();
    assert_eq!(shown.name, "on the server");
    assert_eq!(shown.backend, Some(server_id()));
    assert_eq!(app.panes.served_by(), Some(server_id()));
    // The server numbers its window as the one on show was, so that window stays, with
    // the panels around its tabs as they were.
    assert_eq!(shown.windows.len(), 1);
    assert_eq!(shown.windows[0].id, window);
    assert_eq!(shown.windows[0].dock, dock);
    assert_eq!(app.shell.closed(), Vec::<String>::new());
    let panes: Vec<PaneId> = shown.panes().into_iter().map(|pane| pane.id).collect();
    assert_eq!(panes, app.server.panes());
    // The workspace that was put away is in its file; the server's layout is in none.
    let store = Store::new(app.dir.dirs().state_workspaces_dir());
    assert_eq!(store.list().unwrap(), vec![WORKSPACE_NAME.to_owned()]);
    app.panes.save().await;
    assert_eq!(store.list().unwrap(), vec![WORKSPACE_NAME.to_owned()]);
    app.state.shutdown().await;
}

#[tokio::test]
async fn a_window_the_server_numbers_otherwise_is_opened_in_place_of_the_one_on_show() {
    let app = running("served-other-window", 9).await;
    let window = app.panes.focused_window().unwrap();
    // The window on show is open, as the application's first window is.
    crate::control::Shell::open_window(app.shell.as_ref(), window, None);

    app.panes.show_served(&server_id()).await.unwrap();

    let served = WindowId::new(9);
    until("the server's window is opened", || {
        app.shell.open().contains(&label_of(served))
            && app.shell.closed().contains(&label_of(window))
    })
    .await;
    app.state.shutdown().await;
}

#[tokio::test]
async fn what_is_asked_of_tabs_and_panes_is_asked_of_the_server() {
    let app = running("served-asked", 1).await;
    app.panes.show_served(&server_id()).await.unwrap();
    let pane = app.server.panes()[0];
    let before = app.panes.snapshot();

    let applied = app.panes.apply(split(pane, Dir::Right)).await.unwrap();

    // The server was asked, and made the pane.
    assert!(matches!(
        app.server.asked().as_slice(),
        [Intent::Split { .. }]
    ));
    assert_eq!(app.server.panes().len(), 2);
    let made = applied.created.pane.unwrap();
    assert!(app.server.panes().contains(&made));
    // Whoever asked is told where the copy stands once it has the change.
    assert_eq!(applied.epoch, before.epoch);
    assert_eq!(applied.revision, before.revision + 1);
    // The copy follows the server, as a step in the same layout.
    until("the copy has the new pane", || {
        app.panes.workspace().pane(made).is_some()
    })
    .await;
    assert_eq!(app.panes.snapshot().epoch, before.epoch);
    app.state.shutdown().await;
}

#[tokio::test]
async fn what_is_the_applications_own_is_changed_here_and_outlasts_the_servers_changes() {
    let app = running("served-own", 1).await;
    app.panes.show_served(&server_id()).await.unwrap();
    let window = app.panes.focused_window().unwrap();
    let collapsed = |app: &Running| {
        app.panes
            .workspace()
            .window(window)
            .unwrap()
            .dock
            .area(Area::Left)
            .collapsed
    };
    let was = collapsed(&app);

    app.panes
        .apply(Intent::PanelToggle {
            window,
            area: Area::Left,
            on: Some(was),
        })
        .await
        .unwrap();

    assert!(app.server.asked().is_empty());
    assert_eq!(collapsed(&app), !was);

    // The server changes its layout on its own, as when another client does.
    let pane = app.server.panes()[0];
    app.server
        .layout
        .apply(split(pane, Dir::Down))
        .await
        .unwrap();
    until("the copy has the server's change", || {
        app.panes.workspace().panes().len() == 2
    })
    .await;
    assert_eq!(collapsed(&app), !was);
    app.state.shutdown().await;
}

#[tokio::test]
async fn a_pane_that_leaves_the_servers_layout_does_not_end_its_session() {
    let app = running("served-pane-gone", 1).await;
    app.panes.show_served(&server_id()).await.unwrap();
    let first = app.server.panes()[0];
    let made = app
        .panes
        .apply(split(first, Dir::Right))
        .await
        .unwrap()
        .created
        .pane
        .unwrap();
    until("the copy has the new pane", || {
        app.panes.workspace().pane(made).is_some()
    })
    .await;

    app.server
        .layout
        .apply(Intent::Close { pane: made })
        .await
        .unwrap();
    until("the copy has lost the pane", || {
        app.panes.workspace().pane(made).is_none()
    })
    .await;
    // Time for a session to be ended, were it going to be.
    sleep(Duration::from_millis(100)).await;
    assert_eq!(app.server.killed.load(Ordering::Relaxed), 0);
    app.state.shutdown().await;
}

#[tokio::test]
async fn what_only_the_application_knows_has_no_place_in_a_servers_layout() {
    let app = running("served-core-only", 1).await;
    app.panes.show_served(&server_id()).await.unwrap();
    let pane = app.server.panes()[0];

    let refused = app
        .panes
        .apply_core(Intent::SessionSet {
            pane,
            session: None,
            backend: None,
            cwd: None,
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code, "unsupported");
    assert!(app.server.asked().is_empty());
    assert_eq!(
        app.panes.rename("mine now").await.unwrap_err().code,
        "unsupported"
    );
    app.state.shutdown().await;
}

#[tokio::test]
async fn a_workspace_of_the_applications_own_takes_the_servers_place_again() {
    let app = running("served-left", 1).await;
    app.panes.show_served(&server_id()).await.unwrap();

    app.panes
        .switch(WORKSPACE_NAME, Find::Existing)
        .await
        .unwrap();

    let shown = app.panes.workspace();
    assert_eq!(shown.name, WORKSPACE_NAME);
    assert_eq!(shown.backend, None);
    assert_eq!(app.panes.served_by(), None);
    // Leaving a server's layout ends nothing on the server.
    sleep(Duration::from_millis(100)).await;
    assert_eq!(app.server.killed.load(Ordering::Relaxed), 0);
    // What the server does next changes nothing here.
    let panes = shown.panes().len();
    let pane = app.server.panes()[0];
    app.server
        .layout
        .apply(split(pane, Dir::Right))
        .await
        .unwrap();
    sleep(Duration::from_millis(100)).await;
    assert_eq!(app.panes.workspace().panes().len(), panes);
    // And intents are the application's to apply again.
    let asked = app.server.asked().len();
    let pane = app.panes.workspace().panes()[0].id;
    app.panes
        .apply(Intent::PaneRename {
            pane,
            title: Some("mine".to_owned()),
        })
        .await
        .unwrap();
    assert_eq!(app.server.asked().len(), asked);
    app.state.shutdown().await;
}

/// What a page that shows the server's layout was told, of one type.
fn told(carrier: &RecordingCarrier, kind: &str) -> usize {
    carrier
        .requests()
        .iter()
        .filter(|sent| sent["notify"]["type"] == kind)
        .count()
}

/// The server changes what its first pane is called.
fn call_it(app: &Running, title: &str) {
    app.server.layout.update(|shown| {
        let mut next = shown.clone();
        let pane = next.panes()[0].id;
        next.pane_mut(pane).unwrap().title = Some(title.to_owned());
        Some(next)
    });
}

#[tokio::test]
async fn a_server_that_keeps_sending_layouts_that_cannot_be_shown_is_complained_of_once() {
    let app = running("served-alternating", 1).await;
    let carrier = RecordingCarrier::new();
    app.state
        .open(&label_of(WindowId::new(1)), carrier.clone())
        .await;
    app.panes.show_served(&server_id()).await.unwrap();
    let sound = app.server.layout.workspace();
    for round in 0..6 {
        // Two tabs with one pane.
        app.server.layout.update(|shown| {
            let mut broken = shown.clone();
            let mut tab = broken.windows[0].centre.tabs[0].clone();
            tab.id = TabId::new(99);
            broken.windows[0].centre.tabs.push(tab);
            Some(broken)
        });
        until("it is told", || told(&carrier, "notice") == 1).await;
        sleep(Duration::from_millis(20)).await;
        app.server.layout.update(|_| Some(sound.clone()));
        let title = format!("sound {round}");
        call_it(&app, &title);
        until("the sound one is shown", || {
            app.panes.workspace().panes()[0].title.as_ref() == Some(&title)
        })
        .await;
    }
    assert_eq!(told(&carrier, "notice"), 1);
    app.state.shutdown().await;
}

#[tokio::test]
async fn a_layout_changed_more_often_than_it_is_shown_comes_to_stand_as_the_server_has_it() {
    let app = running("served-often", 1).await;
    let carrier = RecordingCarrier::new();
    app.state
        .open(&label_of(WindowId::new(1)), carrier.clone())
        .await;
    app.panes.show_served(&server_id()).await.unwrap();
    let began = Instant::now();
    for nth in 0..1_000 {
        call_it(&app, &format!("title {nth}"));
        tokio::task::yield_now().await;
        if nth % 10 == 0 {
            sleep(Duration::from_millis(1)).await;
        }
    }
    until("the last is shown", || {
        app.panes.workspace().panes()[0].title.as_deref() == Some("title 999")
    })
    .await;
    let windows = u32::try_from(began.elapsed().as_secs()).unwrap() + 1;
    let most = windows * SERVED_COPIES_PER_WINDOW;
    let copies = told(&carrier, "layout_changed") + told(&carrier, "layout_reset");
    // One is of the server's layout being put on show.
    assert!(copies <= most as usize + 1, "{copies} of {most}");

    // Afterwards a change is shown at once again.
    sleep(SERVED_WINDOW).await;
    let began = Instant::now();
    call_it(&app, "at once");
    until("it is shown", || {
        app.panes.workspace().panes()[0].title.as_deref() == Some("at once")
    })
    .await;
    assert!(began.elapsed() < Duration::from_millis(500));
    app.state.shutdown().await;
}

#[tokio::test]
async fn what_a_servers_session_reported_goes_when_nothing_shows_it() {
    let app = running("served-reports", 1).await;
    app.panes.show_served(&server_id()).await.unwrap();
    let first = app.server.panes()[0];
    let made = app
        .panes
        .apply(split(first, Dir::Right))
        .await
        .unwrap()
        .created
        .pane
        .unwrap();
    until("the copy has the new pane", || {
        app.panes.workspace().pane(made).is_some()
    })
    .await;
    let reports = app.state.reports();
    let keys = [first, made].map(|pane| app.panes.session_of(pane).unwrap());
    for key in &keys {
        reports.shown(key);
    }
    let known = |nth: usize| reports.state(&keys[nth]).is_some();
    app.server
        .layout
        .apply(Intent::Close { pane: made })
        .await
        .unwrap();
    until("the one that is gone is forgotten", || !known(1)).await;
    assert!(known(0));
    app.panes
        .switch(WORKSPACE_NAME, Find::Existing)
        .await
        .unwrap();
    assert!(!known(0));
    app.state.shutdown().await;
}

#[tokio::test]
async fn a_backend_that_keeps_no_layout_and_a_layout_that_is_not_sound_are_refused() {
    let app = running("served-refused", 1).await;
    let none = app
        .panes
        .show_served(&Router::local_id())
        .await
        .unwrap_err();
    assert_eq!(none.code, "unsupported");

    // Two windows with one number: not a layout that can be shown.
    app.server.layout.update(|shown| {
        let mut broken = shown.clone();
        broken.windows.push(Window::new(WindowId::new(1)));
        Some(broken)
    });
    let unsound = app.panes.show_served(&server_id()).await.unwrap_err();
    assert_eq!(unsound.code, "protocol_error");
    assert_eq!(app.panes.workspace().backend, None);
    assert_eq!(app.panes.served_by(), None);
    app.state.shutdown().await;
}
