//! The application with a Rex server it can reach: the client as it is, over a port of
//! this machine, and the pretend server the client itself is tested against.

#[path = "../../../lume-rex/tests/common/mod.rs"]
mod common;

use std::time::Duration;

use common::{Conn, Listener, Served, Tell, World, door, serve};
use lume_layout::{Intent, PaneContent, Store};
use lume_proto::app::{
    APP_PROTO_VERSION, ASKPASS_TOKEN_VAR, AppMessage, AppReplyBody, AppRequest, AppRequestBody,
    AppResponse, PageResult,
};
use lume_proto::{ClientKind, ControlDecoder};
use lume_rex::{PromptRun, Prompter as _};
use lume_session::{HostConfig, LocalBackend, Router};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::runtime::Handle;
use tokio::time::{sleep, timeout};

use super::super::start;
use super::*;
use crate::builtins;
use crate::control::{self, Control};
use crate::marks::Marks;
use crate::panes::PaneStart;
use crate::plugins::Plugins;
use crate::services::{Asker, Services};
use crate::testing::{FakeShell, RecordingCarrier, STEP, TempDir, size};
use crate::tuning::WORKSPACE_NAME;
use crate::window::label_of;

#[path = "hostile_tests.rs"]
mod hostile;

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
    /// The id of the page's carrier.
    page: u32,
    asking: Arc<Asking>,
    shell: Arc<FakeShell>,
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
        let page = state.open(&label_of(window), carrier.clone()).await;
        let asking = Asking::new("test", "lume".into());
        let services = Arc::new(Services {
            rex: Hosts::new(&config, &state, &panes, Some(Arc::clone(&asking))),
            state,
            panes,
            config,
            registry,
            shell: shell.clone(),
            plugins,
        });
        Self {
            services,
            carrier,
            page,
            asking,
            shell,
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

/// A connection to the control endpoint, as the program the SSH client asks through
/// makes one.
struct Askpass(DuplexStream);

impl Askpass {
    async fn ask(app: &Running, run: &PromptRun, words: &str) -> Self {
        let (near, far) = tokio::io::duplex(64 * 1024);
        let control = Control::new(Arc::clone(&app.services), STEP);
        tokio::spawn(async move { control::serve_client(far, &control).await });
        let mut this = Self(near);
        this.say(
            1,
            AppRequestBody::Hello {
                proto: APP_PROTO_VERSION,
                proto_min: APP_PROTO_VERSION,
                client_kind: ClientKind::Ctl,
            },
        )
        .await;
        this.heard().await.unwrap();
        let (_, token) = run
            .env
            .iter()
            .find(|(name, _)| name == ASKPASS_TOKEN_VAR)
            .unwrap();
        let body = AppRequestBody::Askpass {
            token: Secret::new(token.to_str().unwrap().to_owned()),
            prompt: words.to_owned(),
            hint: None,
        };
        this.say(2, body).await;
        this
    }

    async fn say(&mut self, id: u64, body: AppRequestBody) {
        let mut frames = Vec::new();
        let request = AppMessage::Request(AppRequest { id, body });
        request.encode(&mut frames).unwrap();
        self.0.write_all(&frames).await.unwrap();
    }

    async fn heard(&mut self) -> Result<AppReplyBody, AppError> {
        let mut decoder = ControlDecoder::new();
        let mut chunk = [0; 1024];
        loop {
            if let Some(json) = decoder.next().unwrap() {
                let AppMessage::Response(AppResponse { result, .. }) =
                    AppMessage::decode(&json).unwrap()
                else {
                    panic!("not an answer");
                };
                return result.map(|reply| reply.body);
            }
            let read = timeout(STEP, self.0.read(&mut chunk))
                .await
                .unwrap()
                .unwrap();
            assert_ne!(read, 0, "the endpoint closed the connection");
            decoder.push(&chunk[..read]);
        }
    }
}

impl Running {
    fn put(&self) -> Vec<Value> {
        self.carrier
            .requests()
            .into_iter()
            .filter_map(|sent| sent.get("request").cloned())
            .filter(|request| request["body"]["type"] == "sign_in")
            .collect()
    }

    /// The nth question put to the user.
    async fn nth(&self, nth: usize) -> Value {
        until("a question is put", || self.put().len() > nth).await;
        self.put()[nth].clone()
    }

    fn answer(&self, put: &Value, secret: &str) {
        let window = self.services.panes.focused_window().unwrap();
        let answer = PageResult::Ok(AppReplyBody::Secret {
            secret: Some(Secret::new(secret.to_owned())),
        });
        let id = put["id"].as_u64().unwrap();
        self.services
            .state
            .answer(&label_of(window), self.page, id, answer.into())
            .unwrap();
    }
}

#[tokio::test]
async fn a_question_comes_in_at_the_endpoint_and_either_side_can_take_it_back() {
    let app = Running::with("rex-askpass", "ssh://me@build").await;
    let window = app.services.panes.focused_window().unwrap();
    let state = Arc::downgrade(&app.services.state);
    let attempt = app.asking.attempt(&state, window, HOST, "ssh://me@build");
    let run = attempt.run().unwrap();
    let words = "me@build's password: ";

    let mut asker = Askpass::ask(&app, &run, words).await;
    let put = app.nth(0).await;
    assert_eq!(put["body"]["words"], words.trim());
    app.answer(&put, "hunter2");
    let secret = Some(Secret::new("hunter2".to_owned()));
    assert_eq!(asker.heard().await, Ok(AppReplyBody::Secret { secret }));
    // The connection goes on to be one.
    asker.say(3, AppRequestBody::Status {}).await;
    assert!(matches!(
        asker.heard().await,
        Ok(AppReplyBody::Status { .. })
    ));

    // The program that asks is ended.
    let asker = Askpass::ask(&app, &run, words).await;
    let put = app.nth(1).await;
    drop(asker);
    until("the question is withdrawn", || {
        app.told("sign_in_withdrawn")
            .iter()
            .any(|notice| notice["ask"] == put["body"]["ask"])
    })
    .await;

    // The run of the client is.
    let mut asker = Askpass::ask(&app, &run, words).await;
    app.nth(2).await;
    drop(run);
    assert_eq!(
        asker.heard().await,
        Ok(AppReplyBody::Secret { secret: None })
    );
    until("the question is withdrawn", || {
        app.told("sign_in_withdrawn").len() == 2
    })
    .await;
    app.stop().await;
}

impl Running {
    /// Starts on a server, as the command line or the settings ask.
    fn start_on(&self, host: &str, said: start::Said) {
        start::begin(&self.services, host.to_owned(), said, STEP);
    }

    /// What the page was asked to run.
    fn commands(&self) -> Vec<Value> {
        self.carrier
            .requests()
            .into_iter()
            .filter_map(|sent| sent.get("request").cloned())
            .map(|request| request["body"].clone())
            .filter(|body| body["type"] == "command")
            .collect()
    }

    /// The one notice the page was given.
    async fn noticed(&self) -> Value {
        until("the page is told", || !self.told("notice").is_empty()).await;
        let told = self.told("notice");
        assert_eq!(told.len(), 1, "{told:?}");
        told[0].clone()
    }
}

#[tokio::test]
async fn the_session_last_shown_is_there_for_the_next_start() {
    let server = Server::start(World::recorded().changing()).await;
    let app = Running::with("rex-start", &server.url()).await;
    app.outside("lume.rex.open", json!({ "host": HOST }))
        .await
        .unwrap();
    app.outside(
        "lume.rex.new-session",
        json!({ "host": HOST, "label": "work" }),
    )
    .await
    .unwrap();
    let last = app.host()["on_show"].clone();
    assert_ne!(last, common::SESSION);
    // What the next run of it finds.
    let held = super::super::remembered::Remembered::load(&app.dir.dirs());
    let id = BackendId(format!("rex:{}", server.url()));
    assert_eq!(json!(held.of(&id)), last);
    app.stop().await;
}

#[tokio::test]
async fn a_server_is_started_on_by_its_label_or_address() {
    let server = Server::start(World::recorded()).await;
    let app = Running::with("rex-start-on", &server.url()).await;
    app.start_on(&server.url(), start::Said::Settings);
    until("the session is on show", || {
        app.host()["on_show"] == common::SESSION
    })
    .await;
    assert_eq!(app.told("notice"), Vec::<Value>::new());
    app.stop().await;
}

#[tokio::test]
async fn a_start_that_comes_to_nothing_says_why_and_leaves_the_workspace() {
    let (address, listener) = door().await;
    drop(listener);
    let app = Running::with("rex-start-gone", &format!("http://{address}")).await;
    app.start_on(HOST, start::Said::Here);
    let notice = app.noticed().await;
    let said = notice["message"].as_str().unwrap();
    assert!(said.starts_with("Build box could not be shown: "), "{said}");
    assert_eq!(notice["action"]["command"], "lume.rex.open");
    assert_eq!(notice["action"]["args"]["host"], HOST);
    assert!(app.own_on_show());
    app.stop().await;
}

#[tokio::test]
async fn a_start_on_a_server_that_is_not_listed_offers_to_add_it_or_says_so() {
    let app = Running::with("rex-start-unknown", "https://build.example").await;
    app.start_on("nobody", start::Said::Here);
    let notice = app.noticed().await;
    assert_eq!(notice["message"], "There is no Rex server called nobody.");
    assert_eq!(notice.get("action"), None);
    app.start_on("ssh://me@nas", start::Said::Settings);
    until("the page is told", || app.told("notice").len() == 2).await;
    let said = app.told("notice")[1]["message"].clone();
    assert!(
        said.as_str().unwrap().contains("not listed any more"),
        "{said}"
    );
    assert_eq!(app.commands(), Vec::<Value>::new());

    app.start_on("ssh://me@nas", start::Said::Here);
    until("the dialog is asked for", || !app.commands().is_empty()).await;
    assert_eq!(
        app.commands(),
        [json!({
            "type": "command",
            "id": "lume.connection.add",
            "args": { "kind": "rex", "address": "ssh://me@nas" },
        })]
    );
    assert_eq!(app.told("notice").len(), 2);
    app.stop().await;
}

#[tokio::test]
async fn a_start_on_a_server_without_sessions_lists_it() {
    let mut world = World::recorded();
    world.sessions.clear();
    let server = Server::start(world).await;
    let app = Running::with("rex-start-empty", &server.url()).await;
    app.start_on(HOST, start::Said::Here);
    until("the palette is asked for", || !app.commands().is_empty()).await;
    assert_eq!(app.commands()[0]["id"], "lume.connect.open");
    assert_eq!(app.commands()[0]["args"], json!({ "host": HOST }));
    assert!(app.own_on_show());
    app.stop().await;
}

#[tokio::test]
async fn a_start_on_a_connection_without_encryption_asks_first() {
    let app = Running::with("rex-start-plain", "http://192.0.2.1:7297").await;
    app.start_on(HOST, start::Said::Settings);
    until("the user is asked", || {
        !app.told("confirm_requested").is_empty()
    })
    .await;
    let asked = app.told("confirm_requested")[0].clone();
    assert_eq!(asked["command"], "lume.rex.open");
    assert_eq!(asked["args"], json!({ "host": HOST, "confirmed": true }));
    app.stop().await;
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
async fn a_session_is_called_something_else_wherever_it_is_named() {
    let server = Server::start(World::recorded()).await;
    let app = Running::with("rex-renamed", &server.url()).await;
    let args = json!({ "host": HOST, "session": "lume-fixture", "label": "play" });
    // Nothing is connected to for it.
    let error = app
        .outside("lume.rex.rename-session", args.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code, "disconnected");
    assert_eq!(server.connections(), 0);

    app.outside("lume.rex.connect", json!({ "host": HOST }))
        .await
        .unwrap();
    app.outside("lume.rex.rename-session", args.clone())
        .await
        .unwrap();
    until("the new name is listed", || {
        app.host()["sessions"][0]["label"] == "play"
    })
    .await;
    let error = app
        .outside("lume.rex.rename-session", args)
        .await
        .unwrap_err();
    assert_eq!(error.code, "not_found");
    let nameless = json!({ "host": HOST, "session": "play", "label": "  " });
    let error = app
        .outside("lume.rex.rename-session", nameless)
        .await
        .unwrap_err();
    assert_eq!(error.code, "bad_request");
    app.stop().await;
}

#[tokio::test]
async fn a_session_is_ended_when_the_user_agrees_and_the_own_workspace_is_shown() {
    let server = Server::start(World::recorded()).await;
    let app = Running::with("rex-end", &server.url()).await;
    app.outside("lume.rex.open", json!({ "host": HOST }))
        .await
        .unwrap();
    let args = json!({ "host": app.host()["id"], "session": common::SESSION });
    let refused = app
        .outside("lume.rex.end-session", args.clone())
        .await
        .unwrap_err();
    assert_eq!(refused.code, "confirmation_required");
    // A window that asks by itself is told the same, and nobody is asked twice.
    let mut asked_itself = args.clone();
    asked_itself["confirmed"] = json!(false);
    let refused = app
        .command("lume.rex.end-session", asked_itself, Asker::Page)
        .await
        .unwrap_err();
    assert_eq!(refused.code, "confirmation_required");
    assert_eq!(app.told("confirm_requested"), Vec::<Value>::new());

    app.command("lume.rex.end-session", args, Asker::Page)
        .await
        .unwrap();
    until("the window asks", || {
        !app.told("confirm_requested").is_empty()
    })
    .await;
    let asked = app.told("confirm_requested")[0].clone();
    assert_eq!(asked["title"], "End “lume-fixture”?");
    assert!(asked["message"].as_str().unwrap().contains(HOST));
    assert_eq!(app.host()["on_show"], common::SESSION);

    app.command("lume.rex.end-session", asked["args"].clone(), Asker::Page)
        .await
        .unwrap();
    assert!(app.own_on_show());
    until("it is listed no more", || {
        app.host()["sessions"] == json!([])
    })
    .await;
    sleep(REX_LOOK_EVERY * 3).await;
    // The user is not told what they did.
    assert_eq!(app.told("notice"), Vec::<Value>::new());
    assert_eq!(app.kind(), "connected");
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
    let error = app.rex().connect(HOST, false, None).await.unwrap_err();
    assert_eq!(error.code, "unreachable");
    let error = app.rex().connect("nobody", false, None).await.unwrap_err();
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

    // A window that asks the user itself is told why.
    let refused = app
        .command(
            "lume.rex.connect",
            json!({ "host": HOST, "confirmed": false }),
            Asker::Page,
        )
        .await
        .unwrap_err();
    assert_eq!(refused.code, "confirmation_required");
    assert_eq!(app.told("confirm_requested"), Vec::<Value>::new());

    // Any other asks, and would run the same command again with the user's yes.
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
    rex.begin(&host, false, false, None);
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

#[tokio::test]
async fn an_address_is_tried_and_let_go_of() {
    let server = Server::start(World::recorded()).await;
    let app = Running::start("rex-tried", "[rex]\nlocal = false\n").await;
    let quiet = || app.rex().ssh_options("", "", None);
    let tried = attempt::attempt(endpoint_of(&server.url()).unwrap(), false, quiet())
        .await
        .unwrap();
    assert_eq!(tried["version"], "0.1.0-preview.22");
    assert!(tried["ms"].is_u64(), "{tried}");
    assert_eq!(server.connections(), 1);
    // It is in no list, and no backend of the application's.
    assert_eq!(app.rex().view()["hosts"], json!([]));

    let (address, listener) = door().await;
    drop(listener);
    let gone = endpoint_of(&format!("http://{address}")).unwrap();
    let refused = attempt::attempt(gone, false, quiet()).await.unwrap_err();
    assert_eq!(refused.code, "unreachable", "{refused}");
    // Nothing is sent unencrypted over a network before the user agrees.
    let far = endpoint_of("http://192.0.2.1:7297").unwrap();
    let refused = attempt::attempt(far, false, quiet()).await.unwrap_err();
    assert_eq!(refused.code, "confirmation_required", "{refused}");
    app.stop().await;
}

#[tokio::test]
async fn a_server_that_is_written_is_listed_and_one_that_is_taken_out_is_let_go_of() {
    let server = Server::start(World::recorded()).await;
    let app = Running::start("rex-written", "# Mine.\n[rex]\nlocal = false\n").await;
    let entry = |label: &str, address: &str| RexHostEntry {
        label: label.to_owned(),
        address: address.to_owned(),
    };
    let config = &app.services.config;
    let first = entry("one", &server.url());
    app.rex().check(None, &first).unwrap();
    config.put_rex_host(None, &first).unwrap();
    app.rex().config_changed();
    assert_eq!(app.host()["label"], "one");
    assert_eq!(app.host()["written"], true);

    // Another way to write the same address is the same server.
    let same = entry("two", &format!("{}/", server.url().replace("http", "HTTP")));
    let refused = app.rex().check(None, &same).unwrap_err();
    assert!(refused.message.contains("`one`"), "{refused}");
    app.rex().check(Some("one"), &same).unwrap();
    assert!(app.rex().check(None, &entry("x", "ftp://x")).is_err());
    assert!(app.rex().check(None, &entry("x", "  ")).is_err());
    assert_eq!(app.rex().read(&server.url(), None).unwrap()["taken"], "one");
    assert_eq!(
        app.rex().read(&server.url(), Some("one")).unwrap()["taken"],
        Value::Null
    );

    // A connection outlives what the server is called, and not the server.
    app.outside("lume.rex.connect", json!({ "host": "one" }))
        .await
        .unwrap();
    config
        .put_rex_host(Some("one"), &entry("other", &server.url()))
        .unwrap();
    app.rex().config_changed();
    assert_eq!(app.host()["label"], "other");
    assert_eq!(app.kind(), "connected");
    assert_eq!(server.connections(), 1);
    config.remove_rex_host("other").unwrap();
    app.rex().config_changed();
    assert_eq!(app.rex().view()["hosts"], json!([]));
    until("it is let go of", || {
        app.rex().inner().connections.is_empty()
    })
    .await;
    let text = std::fs::read_to_string(app.dir.dirs().config_file()).unwrap();
    assert_eq!(text, "# Mine.\n[rex]\nlocal = false\n");
    app.stop().await;
}
