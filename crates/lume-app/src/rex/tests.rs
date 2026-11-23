//! The application with a Rex server it can reach: the client as it is, over a port of
//! this machine, and the pretend server the client itself is tested against.

#[path = "../../../lume-rex/tests/common/mod.rs"]
mod common;

use std::time::Duration;

use common::{Conn, Listener, Served, Tell, World, door, serve};
use lume_layout::{Intent, PaneContent, Store};
use lume_session::{HostConfig, LocalBackend, Router};
use tokio::runtime::Handle;
use tokio::time::{sleep, timeout};

use super::*;
use crate::builtins;
use crate::marks::Marks;
use crate::panes::PaneStart;
use crate::plugins::Plugins;
use crate::services::{Asker, Services};
use crate::testing::{FakeShell, RecordingCarrier, STEP, TempDir, size};
use crate::tuning::WORKSPACE_NAME;
use crate::window::label_of;

const HOST: &str = "Build box";

/// The server as it is run for a test: each control connection it takes, in order.
struct Server {
    address: std::net::SocketAddr,
    taken: Arc<Mutex<Vec<Served>>>,
    accepting: AbortHandle,
}

impl Server {
    async fn start(world: World) -> Self {
        let (address, listener) = door().await;
        Self::on(address, listener, world)
    }

    fn on(address: std::net::SocketAddr, mut listener: Listener, world: World) -> Self {
        let taken = Arc::new(Mutex::new(Vec::new()));
        let keep = Arc::clone(&taken);
        let accepting = tokio::spawn(async move {
            loop {
                let conn = listener.accept().await;
                let served = serve(conn, world.clone());
                keep.lock().unwrap().push(served);
            }
        })
        .abort_handle();
        Self {
            address,
            taken,
            accepting,
        }
    }

    fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    fn connections(&self) -> usize {
        self.taken.lock().unwrap().len()
    }

    /// Does something with the connection made last.
    fn last<T>(&self, act: impl FnOnce(&Served) -> T) -> T {
        act(self.taken.lock().unwrap().last().unwrap())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.accepting.abort();
    }
}

struct Running {
    services: Arc<Services>,
    carrier: Arc<RecordingCarrier>,
    dir: TempDir,
}

impl Running {
    /// The application with `text` in its configuration, and a page in its window.
    async fn start(name: &str, text: &str) -> Self {
        let dir = TempDir::new(name);
        std::fs::create_dir_all(dir.dirs().config).unwrap();
        std::fs::write(dir.dirs().config_file(), text).unwrap();
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
        let (panes, _) =
            Panes::open(&state, Arc::clone(&config), store, WORKSPACE_NAME, None).await;
        state.set_panes(Arc::clone(&panes));
        let shell = Arc::new(FakeShell::default());
        panes.run(shell.clone());
        let registry = Arc::new(builtins::registry());
        let plugins = Plugins::new(
            config.dirs(),
            Arc::clone(&registry),
            Arc::clone(&state),
            Arc::clone(&config),
        );
        let carrier = RecordingCarrier::new();
        let window = panes.focused_window().unwrap();
        state.open(&label_of(window), carrier.clone()).await;
        let services = Arc::new(Services {
            rex: Hosts::new(&config, &state, &panes),
            state,
            panes,
            config,
            registry,
            shell,
            plugins,
        });
        Self {
            services,
            carrier,
            dir,
        }
    }

    /// The application with one server in its configuration.
    async fn with(name: &str, address: &str) -> Self {
        Self::start(name, &configured(address)).await
    }

    fn rex(&self) -> &Arc<Hosts> {
        &self.services.rex
    }

    async fn command(&self, id: &str, args: Value, asker: Asker) -> Result<(), CommandError> {
        let window = self.services.panes.focused_window().unwrap();
        self.services.run_command(window, id, &args, asker).await
    }

    async fn outside(&self, id: &str, args: Value) -> Result<(), CommandError> {
        self.command(id, args, Asker::Outside).await
    }

    /// How the one server stands.
    fn host(&self) -> Value {
        self.rex().view()["hosts"][0].clone()
    }

    fn kind(&self) -> String {
        self.host()["state"]["kind"].as_str().unwrap().to_owned()
    }

    /// What the application's page was told, of one type.
    fn told(&self, kind: &str) -> Vec<Value> {
        self.carrier
            .requests()
            .into_iter()
            .filter_map(|sent| sent.get("notify").cloned())
            .filter(|notice| notice["type"] == kind)
            .collect()
    }

    fn own_on_show(&self) -> bool {
        let shown = self.services.panes.workspace();
        shown.backend.is_none() && shown.name == WORKSPACE_NAME
    }

    async fn stop(self) {
        self.rex().shutdown();
        self.services.state.shutdown().await;
    }
}

fn configured(address: &str) -> String {
    format!("[rex]\nlocal = false\n\n[[rex.host]]\nlabel = \"{HOST}\"\naddress = \"{address}\"\n")
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

#[tokio::test]
async fn nothing_is_connected_to_until_somebody_asks() {
    let server = Server::start(World::recorded()).await;
    let app = Running::with("rex-asked", &server.url()).await;
    assert_eq!(app.kind(), "idle");
    assert_eq!(app.host()["label"], HOST);
    assert_eq!(app.host()["sessions"], json!([]));
    app.rex().look().await;
    assert_eq!(server.connections(), 0);

    app.outside("lume.rex.connect", json!({ "host": "build BOX" }))
        .await
        .unwrap();
    let host = app.host();
    assert_eq!(host["state"]["kind"], "connected");
    assert_eq!(host["state"]["version"], "0.1.0-preview.22");
    assert_eq!(host["state"]["api_revision"], 19);
    assert_eq!(
        host["sessions"],
        json!([{ "id": common::SESSION, "label": "lume-fixture" }])
    );
    // What it is said to be is what it is.
    assert_eq!(
        server.last(|served| served.hello["client"]["name"].clone()),
        "Lume"
    );
    until("the answer is timed", || app.host()["latency_ms"].is_u64()).await;
    // Connecting again is the same connection.
    app.outside("lume.rex.connect", json!({ "host": server.url() }))
        .await
        .unwrap();
    assert_eq!(server.connections(), 1);
    app.stop().await;
}

#[tokio::test]
async fn a_session_is_shown_as_a_workspace_and_left_again() {
    let server = Server::start(World::recorded()).await;
    let app = Running::with("rex-shown", &server.url()).await;
    app.outside(
        "lume.rex.open",
        json!({ "host": HOST, "session": "lume-fixture" }),
    )
    .await
    .unwrap();
    let panes = &app.services.panes;
    let shown = panes.workspace();
    let id = BackendId(app.host()["id"].as_str().unwrap().to_owned());
    assert_eq!(shown.backend, Some(id.clone()));
    assert_eq!(app.host()["on_show"], common::SESSION);
    assert!(!shown.panes().is_empty());

    // Each pane's session is the server's, and is given without anything being started
    // or asked of the server.
    let window = panes.focused_window().unwrap();
    let before = server.last(Served::called).len();
    let terminals: Vec<PaneStart> = shown
        .panes()
        .into_iter()
        .filter(|pane| matches!(pane.content, PaneContent::Terminal(_)))
        .map(|pane| PaneStart {
            pane: pane.id,
            size: size(80, 24),
        })
        .collect();
    let count = terminals.len();
    let started = panes.start_sessions(window, terminals).await;
    assert_eq!(started.started.len(), count, "{:?}", started.failed);
    assert!(started.started.iter().all(|pane| pane.backend == id));
    assert_eq!(server.last(Served::called).len(), before);

    // Opening it again changes nothing.
    let epoch = panes.snapshot().epoch;
    app.outside("lume.rex.open", json!({ "host": HOST }))
        .await
        .unwrap();
    assert_eq!(panes.snapshot().epoch, epoch);

    // A workspace of the application's own takes its place, and the server is told that
    // nobody looks here any more.
    app.outside("lume.workspace.switch", json!({ "name": WORKSPACE_NAME }))
        .await
        .unwrap();
    assert!(app.own_on_show());
    until("the session is let go of", || {
        server.last(|served| !served.called_of(&["session.detach"]).is_empty())
    })
    .await;
    // It stays connected, and knows what was last on show.
    assert_eq!(app.kind(), "connected");
    assert_eq!(app.host()["on_show"], Value::Null);
    app.outside("lume.rex.open", json!({ "host": HOST }))
        .await
        .unwrap();
    assert_eq!(app.host()["on_show"], common::SESSION);
    app.stop().await;
}

#[tokio::test]
async fn a_session_is_made_and_shown_and_the_one_before_it_is_let_go_of() {
    let server = Server::start(World::recorded().changing()).await;
    let app = Running::with("rex-made", &server.url()).await;
    app.outside("lume.rex.open", json!({ "host": HOST }))
        .await
        .unwrap();
    app.outside(
        "lume.rex.new-session",
        json!({ "host": HOST, "label": "work" }),
    )
    .await
    .unwrap();
    assert_eq!(app.host()["on_show"], "session:made-1");
    assert_eq!(app.host()["sessions"][1]["label"], "work");
    assert_eq!(
        server.last(|served| served.asked("session.detach")[0]["session_id"].clone()),
        common::SESSION
    );
    until("the new session is on show", || {
        app.services.panes.workspace().name == "work"
    })
    .await;

    // What is asked of the layout is asked of the server.
    let pane = app.services.panes.workspace().panes()[0].id;
    app.services
        .panes
        .apply(Intent::PaneRename {
            pane,
            title: Some("logs".to_owned()),
        })
        .await
        .unwrap();
    assert_eq!(
        server.last(|served| served.asked("session.set_block_label")[0]["label"].clone()),
        "logs"
    );
    app.stop().await;
}

#[tokio::test]
async fn a_session_that_ends_on_its_server_gives_way_to_the_workspace_before_it() {
    let server = Server::start(World::recorded()).await;
    let app = Running::with("rex-ended", &server.url()).await;
    app.outside("lume.rex.open", json!({ "host": HOST }))
        .await
        .unwrap();
    server.last(|served| {
        served.tell(Tell::Event(
            "session_destroyed".to_owned(),
            json!({ "session_id": common::SESSION }),
        ));
    });
    until("the own workspace is shown", || app.own_on_show()).await;
    until("it is said why", || {
        app.told("notice").iter().any(|notice| {
            notice["message"]
                .as_str()
                .unwrap()
                .contains("ended on its server")
        })
    })
    .await;
    assert_eq!(app.kind(), "connected");
    let none = app.outside("lume.rex.open", json!({ "host": HOST })).await;
    assert_eq!(none.unwrap_err().code, "no_session");
    app.stop().await;
}

#[tokio::test]
async fn a_session_left_with_nothing_in_it_gives_way_and_can_be_opened_empty() {
    let server = Server::start(World::recorded()).await;
    let app = Running::with("rex-emptied", &server.url()).await;
    app.outside("lume.rex.open", json!({ "host": HOST }))
        .await
        .unwrap();
    let mut emptied = World::recorded();
    emptied.views.insert(
        common::SESSION.to_owned(),
        json!({ "revision": 9000, "session_id": common::SESSION, "label": "lume-fixture" }),
    );
    server.last(|served| {
        served.tell(Tell::World(emptied));
        served.tell(Tell::SessionEvent(
            common::SESSION.to_owned(),
            "session_view_changed".to_owned(),
            json!({ "revision": 9000 }),
        ));
    });
    until("the own workspace is shown", || app.own_on_show()).await;

    // The session is there still, and is shown when it is asked for.
    app.outside("lume.rex.open", json!({ "host": HOST }))
        .await
        .unwrap();
    sleep(REX_LOOK_EVERY * 3).await;
    let shown = app.services.panes.workspace();
    assert!(shown.backend.is_some());
    assert!(shown.panes().is_empty());
    app.stop().await;
}

#[tokio::test]
async fn disconnecting_shows_the_own_workspace_and_nothing_of_the_server_stays() {
    let server = Server::start(World::recorded()).await;
    let app = Running::with("rex-disconnected", &server.url()).await;
    app.outside("lume.rex.open", json!({ "host": HOST }))
        .await
        .unwrap();
    let id = BackendId(app.host()["id"].as_str().unwrap().to_owned());
    app.outside("lume.rex.disconnect", json!({ "host": HOST }))
        .await
        .unwrap();
    assert!(app.own_on_show());
    assert_eq!(app.kind(), "idle");
    assert!(app.services.state.router().get(&id).is_err());
    assert_eq!(app.services.state.router().ids(), [Router::local_id()]);
    app.stop().await;
}

#[tokio::test]
async fn a_server_that_is_not_there_fails_once_and_stands_in_nobodys_way() {
    // A port nothing listens on.
    let nobody = {
        let door = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        door.local_addr().unwrap()
    };
    let app = Running::with("rex-absent", &format!("http://{nobody}")).await;
    let error = app
        .outside("lume.rex.open", json!({ "host": HOST }))
        .await
        .unwrap_err();
    assert_eq!(error.code, "unreachable");
    let host = app.host();
    assert_eq!(host["state"]["kind"], "failed");
    assert_eq!(host["state"]["code"], "unreachable");
    assert!(app.own_on_show());
    app.services.state.list_all().await.unwrap();

    // Trying again tries again.
    let error = app.rex().connect(HOST, false).await.unwrap_err();
    assert_eq!(error.code, "unreachable");
    let error = app.rex().connect("nobody", false).await.unwrap_err();
    assert_eq!(error.code, "not_found");
    app.stop().await;
}

#[tokio::test]
async fn a_server_out_of_reach_does_not_keep_other_sessions_from_being_listed() {
    let server = Server::start(World::recorded()).await;
    let app = Running::with("rex-listed", &server.url()).await;
    app.outside("lume.rex.open", json!({ "host": HOST }))
        .await
        .unwrap();
    let all = app.services.state.list_all().await.unwrap();
    assert!(
        all.iter()
            .any(|listed| listed.backend.0.starts_with("rex:"))
    );

    // The server goes without a word, and nothing answers where it was.
    server.accepting.abort();
    server.last(|served| served.tell(Tell::Drop));
    until("the connection is seen to be lost", || {
        app.kind() == "waiting"
    })
    .await;
    let left = app.services.state.list_all().await.unwrap();
    assert!(
        left.iter()
            .all(|listed| listed.backend == Router::local_id())
    );
    // What was on show stays on show meanwhile, and its panes keep their sessions.
    let shown = app.services.panes.workspace();
    assert!(shown.backend.is_some());
    let window = app.services.panes.focused_window().unwrap();
    let pane = shown.panes()[0].id;
    let kept = app
        .services
        .panes
        .start_sessions(
            window,
            vec![PaneStart {
                pane,
                size: size(80, 24),
            }],
        )
        .await;
    assert_eq!(kept.started.len(), 1);
    app.stop().await;
}

#[tokio::test]
async fn a_connection_that_is_lost_is_made_again_with_the_session_on_show() {
    let server = Server::start(World::recorded()).await;
    let app = Running::with("rex-again", &server.url()).await;
    app.outside("lume.rex.open", json!({ "host": HOST }))
        .await
        .unwrap();
    let panes = &app.services.panes;
    let before = panes.workspace().panes().len();
    let epoch = panes.snapshot().epoch;
    server.last(|served| served.tell(Tell::Drop));
    until("it is connected again", || {
        server.connections() == 2 && app.kind() == "connected"
    })
    .await;
    assert_eq!(app.host()["on_show"], common::SESSION);
    assert_eq!(panes.workspace().panes().len(), before);
    assert_eq!(panes.snapshot().epoch, epoch);
    assert_eq!(app.told("notice"), Vec::<Value>::new());
    app.stop().await;
}

#[tokio::test]
async fn a_connection_that_is_refused_for_good_leaves_the_copy_and_offers_to_connect() {
    let (address, mut listener) = door().await;
    let app = Running::with("rex-refused", &format!("http://{address}")).await;
    let taking = tokio::spawn(async move {
        let first = serve(listener.accept().await, World::recorded());
        (first, listener)
    });
    app.outside("lume.rex.open", json!({ "host": HOST }))
        .await
        .unwrap();
    let (first, mut listener) = taking.await.unwrap();

    // The server is replaced by one that shares no revision with this client.
    first.tell(Tell::Drop);
    let stream = listener.stream().await;
    let refusing = tokio::spawn(async move {
        let mut welcome = common::Recorded::load().welcome();
        welcome["api_revision"] = json!(99);
        let conn = Conn::accept(stream, welcome).await;
        conn.close(1008, "over").await;
    });
    until("it is refused", || app.kind() == "failed").await;
    refusing.abort();
    assert_eq!(app.host()["state"]["code"], "version_mismatch");
    assert!(app.services.panes.workspace().backend.is_some());
    until("it is said and offered", || {
        app.told("notice").iter().any(|notice| {
            notice["action"]["command"] == "lume.rex.open"
                && notice["action"]["args"]["session"] == common::SESSION
        })
    })
    .await;

    // The server is as it was, and the user takes the offer up.
    let again = Server::on(address, listener, World::recorded());
    let action = app.told("notice")[0]["action"].clone();
    app.command("lume.rex.open", action["args"].clone(), Asker::Page)
        .await
        .unwrap();
    assert_eq!(again.connections(), 1);
    assert_eq!(app.kind(), "connected");
    // And the layout is followed again.
    again.last(|served| {
        served.tell(Tell::Event(
            "session_destroyed".to_owned(),
            json!({ "session_id": common::SESSION }),
        ));
    });
    until("the own workspace is shown", || app.own_on_show()).await;
    app.stop().await;
}

#[tokio::test]
async fn a_connection_that_is_not_encrypted_is_made_only_when_the_user_agrees() {
    // An address kept for documentation: nothing is there, and nothing is dialled.
    let app = Running::with("rex-plain", "http://192.0.2.1:7297").await;
    let args = json!({ "host": HOST });
    let refused = app
        .outside("lume.rex.connect", args.clone())
        .await
        .unwrap_err();
    assert_eq!(refused.code, "confirmation_required");
    assert_eq!(app.host()["state"]["code"], "confirmation_required");
    assert_eq!(app.told("confirm_requested"), Vec::<Value>::new());

    // A window asks, and would run the same command again with the user's yes.
    app.command("lume.rex.connect", args, Asker::Page)
        .await
        .unwrap();
    until("the window asks", || {
        !app.told("confirm_requested").is_empty()
    })
    .await;
    let asked = &app.told("confirm_requested")[0];
    assert_eq!(asked["command"], "lume.rex.connect");
    assert_eq!(asked["args"], json!({ "host": HOST, "confirmed": true }));
    app.stop().await;
}

#[tokio::test]
async fn a_server_taken_out_of_the_configuration_is_let_go_of() {
    let server = Server::start(World::recorded()).await;
    let app = Running::with("rex-taken-out", &server.url()).await;
    app.outside("lume.rex.open", json!({ "host": HOST }))
        .await
        .unwrap();

    // Calling it something else is not taking it out.
    let renamed = configured(&server.url()).replace(HOST, "other");
    std::fs::write(app.dir.dirs().config_file(), renamed).unwrap();
    app.services.config.reload().unwrap();
    app.rex().config_changed();
    assert_eq!(app.host()["label"], "other");
    assert_eq!(app.kind(), "connected");

    std::fs::write(app.dir.dirs().config_file(), "[rex]\nlocal = false\n").unwrap();
    app.services.config.reload().unwrap();
    app.rex().config_changed();
    assert_eq!(app.rex().view()["hosts"], json!([]));
    until("the own workspace is shown", || app.own_on_show()).await;
    until("nothing of it stays", || {
        app.services.state.router().ids() == [Router::local_id()]
    })
    .await;
    app.stop().await;
}

#[tokio::test]
async fn the_server_of_this_machine_is_let_go_of_when_nothing_lists_servers() {
    let server = Server::start(World::recorded()).await;
    let app = Running::with("rex-lease", &server.url()).await;
    let rex = app.rex();
    let host = rex.inner().hosts[0].clone();
    // As the server of this machine is connected when something lists servers.
    rex.begin(&host, false, false);
    rex.inner().connections.get_mut(&host.id).unwrap().local = true;
    rex.inner().lease = Some(Instant::now() + Duration::from_secs(600));
    until("it is connected", || app.kind() == "connected").await;
    sleep(REX_LOOK_EVERY * 3).await;
    assert_eq!(app.kind(), "connected");

    // A session of it on show keeps it, whatever is listed.
    app.outside("lume.rex.open", json!({ "host": HOST }))
        .await
        .unwrap();
    {
        let mut inner = rex.inner();
        inner.lease = None;
        // The user opened a session; they did not ask for a connection to be kept.
        inner.connections.get_mut(&host.id).unwrap().kept = false;
    }
    sleep(REX_LOOK_EVERY * 3).await;
    assert_eq!(app.kind(), "connected");

    app.outside("lume.workspace.switch", json!({ "name": WORKSPACE_NAME }))
        .await
        .unwrap();
    until("it is let go of", || app.kind() == "idle").await;
    app.stop().await;
}
