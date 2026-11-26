use lume_layout::Store;
use lume_proto::app::OpenSpec;
use lume_proto::{Body, Message};
use lume_session::{ExitStatus, HostConfig, LocalBackend, Router};
use tokio::io::DuplexStream;
use tokio::runtime::Handle;
use tokio::time::timeout;

use super::*;
use crate::builtins;
use crate::configsvc::ConfigService;
use crate::marks::Marks;
use crate::panes::{PaneStart, Panes};
use crate::state::AppState;
use crate::testing::{
    CREDIT, FakeShell, RecordingCarrier, STEP, TempDir, client_frames, facts, size,
};
use crate::tuning::WORKSPACE_NAME;

struct Running {
    services: Arc<Services>,
    shell: Arc<FakeShell>,
    control: Arc<Control>,
    client: Client,
    window: WindowId,
    _dir: TempDir,
}

impl Running {
    /// A second connection to the endpoint.
    fn connect(&self) -> Client {
        connect(&self.control)
    }
}

/// A client of the endpoint, speaking over an in-memory connection.
struct Client {
    io: DuplexStream,
    decoder: ControlDecoder,
    next_id: u64,
}

impl Client {
    async fn ask(&mut self, body: AppRequestBody) -> Result<AppReply, AppError> {
        let id = self.next_id;
        self.next_id += 1;
        let mut frames = Vec::new();
        AppMessage::Request(AppRequest { id, body })
            .encode(&mut frames)
            .unwrap();
        self.io.write_all(&frames).await.unwrap();
        let mut chunk = [0; 4096];
        let json = loop {
            if let Some(json) = self.decoder.next().unwrap() {
                break json;
            }
            let read = timeout(STEP, self.io.read(&mut chunk))
                .await
                .expect("the endpoint answers")
                .unwrap();
            assert_ne!(read, 0, "the endpoint closed the connection");
            self.decoder.push(&chunk[..read]);
        };
        match AppMessage::decode(&json).unwrap() {
            AppMessage::Response(response) => {
                assert_eq!(response.id, id);
                response.result
            }
            other => panic!("the endpoint sent {other:?} for an answer"),
        }
    }

    /// The next report the endpoint sends to a client that follows sessions.
    async fn event(&mut self) -> AppEvent {
        let mut chunk = vec![0; READ_CHUNK];
        let json = loop {
            if let Some(json) = self.decoder.next().unwrap() {
                break json;
            }
            let read = tokio::time::timeout(Duration::from_secs(20), self.io.read(&mut chunk))
                .await
                .expect("the endpoint sends a report")
                .unwrap();
            assert_ne!(read, 0, "the endpoint closed the connection");
            self.decoder.push(&chunk[..read]);
        };
        match AppMessage::decode(&json).unwrap() {
            AppMessage::Event(event) => event,
            other => panic!("the endpoint sent {other:?} for a report"),
        }
    }

    async fn hello(&mut self) {
        self.ask(AppRequestBody::Hello {
            proto: APP_PROTO_VERSION,
            proto_min: APP_PROTO_VERSION,
            client_kind: ClientKind::Test,
        })
        .await
        .unwrap();
    }
}

async fn running(name: &str) -> Running {
    let dir = TempDir::new(name);
    // A shell that starts fast and keeps no history, with a mark a command can print
    // that does not appear in the command's own echo.
    let (shell, args) = if cfg!(windows) {
        ("cmd.exe", "[\"/d\"]")
    } else {
        ("/bin/sh", "[]")
    };
    std::fs::create_dir_all(&dir.dirs().config).unwrap();
    std::fs::write(
        dir.dirs().config_file(),
        format!("[terminal]\nshell = \"{shell}\"\nshell-args = {args}\n"),
    )
    .unwrap();

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
    let (panes, _) = Panes::open(&state, Arc::clone(&config), store, WORKSPACE_NAME, None).await;
    state.set_panes(Arc::clone(&panes));
    let shell = Arc::new(FakeShell::default());
    panes.run(shell.clone());
    let window = panes.focused_window().unwrap();
    let registry = Arc::new(builtins::registry());
    let plugins = crate::plugins::Plugins::new(
        config.dirs(),
        Arc::clone(&registry),
        Arc::clone(&state),
        Arc::clone(&config),
    );
    let services = Arc::new(Services {
        rex: crate::rex::Hosts::new(&config, &state, &panes, None),
        state,
        panes,
        config,
        registry,
        shell: shell.clone(),
        plugins,
    });
    let control = Control::new(Arc::clone(&services), Duration::from_millis(200));
    let client = connect(&control);
    Running {
        services,
        shell,
        control,
        client,
        window,
        _dir: dir,
    }
}

/// A new connection to the endpoint, as another client would make.
fn connect(control: &Arc<Control>) -> Client {
    let control = Arc::clone(control);
    let (ours, theirs) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let _ = serve_client(theirs, &control).await;
    });
    Client {
        io: ours,
        decoder: ControlDecoder::new(),
        next_id: 1,
    }
}

/// What stands in for the window's page: a carrier, and the streams it has used.
struct Page {
    carrier: Arc<RecordingCarrier>,
    id: u32,
    label: String,
    next_stream: u32,
}

impl Page {
    async fn open(app: &Running) -> Self {
        let carrier = RecordingCarrier::new();
        let label = label_of(app.window);
        let id = app.services.state.open(&label, carrier.clone()).await;
        Self {
            carrier,
            id,
            label,
            next_stream: 1,
        }
    }

    /// Attaches to a session and lets its output flow.
    async fn attach(&mut self, state: &Arc<AppState>, key: &SessionKey) -> u32 {
        let stream = self.next_stream;
        self.next_stream += 1;
        state
            .attach(
                &self.label,
                self.id,
                &key.backend,
                key.session,
                stream,
                facts(),
            )
            .await
            .unwrap();
        let credit = client_frames(&[Message::new(stream, Body::Credit(CREDIT))]);
        state.send(&self.label, self.id, &credit).await.unwrap();
        stream
    }

    fn text(&self) -> String {
        let mut text = String::new();
        for message in self.carrier.messages() {
            if let Body::Data(bytes) = &message.body {
                text.push_str(&String::from_utf8_lossy(bytes));
            }
        }
        text
    }
}

/// Opens the window's page, starts the session of its one pane and attaches to it.
async fn shown(app: &Running) -> (Page, PaneId, SessionKey) {
    let mut page = Page::open(app).await;
    let pane = app.services.panes.workspace().panes()[0].id;
    let outcome = app
        .services
        .panes
        .start_sessions(
            app.window,
            vec![PaneStart {
                pane,
                size: size(80, 24),
            }],
        )
        .await;
    let started = &outcome.started[0];
    let key = SessionKey {
        backend: started.backend.clone(),
        session: started.session,
    };
    page.attach(&app.services.state, &key).await;
    (page, pane, key)
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    let waiting = async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    if timeout(STEP, waiting).await.is_err() {
        panic!("{what} never happened");
    }
}

async fn stop(app: Running) {
    app.services.state.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_must_say_hello_first() {
    let mut app = running("hello").await;
    let refused = app.client.ask(AppRequestBody::List {}).await.unwrap_err();
    assert_eq!(refused.code, codes::HELLO_REQUIRED);
    stop(app).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_of_another_version_is_refused() {
    let mut app = running("version").await;
    let refused = app
        .client
        .ask(AppRequestBody::Hello {
            proto: APP_PROTO_VERSION + 2,
            proto_min: APP_PROTO_VERSION + 1,
            client_kind: ClientKind::Test,
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code, codes::VERSION_MISMATCH);
    stop(app).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn activate_and_quit_reach_the_window() {
    let mut app = running("quit").await;
    app.client.hello().await;

    let launched = app
        .client
        .ask(AppRequestBody::Launch {
            workspace: None,
            exec: Vec::new(),
            cwd: None,
            rex: None,
        })
        .await
        .unwrap();
    assert_eq!(
        launched.body,
        AppReplyBody::Launched {
            workspace: WORKSPACE_NAME.to_owned(),
            pane: None,
        }
    );
    assert_eq!(app.shell.activations(), 1);
    assert_eq!(app.shell.quits(), 0);

    let quitting = app.client.ask(AppRequestBody::Quit {}).await.unwrap();
    assert_eq!(quitting.body, AppReplyBody::Quitting {});
    // The window is told once the answer is on its way.
    let shell = app.shell.clone();
    until("quit", || shell.quits() == 1).await;
    stop(app).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_focused_pane_is_listed_typed_into_and_ended() {
    let mut app = running("focused").await;
    app.client.hello().await;
    let (page, pane, key) = shown(&app).await;

    let listed = app.client.ask(AppRequestBody::List {}).await.unwrap();
    let AppReplyBody::Sessions { sessions } = listed.body else {
        panic!("not a session list");
    };
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].info.id, key.session);
    assert_eq!(sessions[0].pane, Some(pane.to_string()));

    // Nothing named: the pane that has the keyboard.
    let command = if cfg!(windows) {
        "echo lume-%OS:~0,0%k7q-done\r"
    } else {
        "echo \"lume-\"\"k7q-done\"\r"
    };
    let sent = app
        .client
        .ask(AppRequestBody::Send {
            pane: None,
            session: None,
            text: command.to_owned(),
        })
        .await
        .unwrap();
    assert_eq!(
        sent.body,
        AppReplyBody::Sent {
            bytes: command.len() as u64
        }
    );
    until("the command's output", || {
        page.text().contains("lume-k7q-done")
    })
    .await;

    let status = app.client.ask(AppRequestBody::Status {}).await.unwrap();
    let AppReplyBody::Status {
        window,
        focused,
        marks,
        ..
    } = status.body
    else {
        panic!("not a status");
    };
    assert!(window);
    assert_eq!(
        focused,
        Some(Focused {
            pane: pane.to_string(),
            session: Some(key.session)
        })
    );
    assert!(marks.carrier_ms.is_some() && marks.attached_ms.is_some());

    let waiting = app
        .client
        .ask(AppRequestBody::WaitExit {
            pane: Some(pane.to_string()),
            session: None,
            timeout_ms: 50,
        })
        .await
        .unwrap_err();
    assert_eq!(waiting.code, codes::TIMEOUT);

    app.client
        .ask(AppRequestBody::Kill {
            session: key.session,
        })
        .await
        .unwrap();
    let exited = app
        .client
        .ask(AppRequestBody::WaitExit {
            pane: None,
            session: Some(key.session),
            timeout_ms: 20_000,
        })
        .await
        .unwrap();
    assert!(matches!(exited.body, AppReplyBody::Exited { .. }));
    stop(app).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_exit_code_is_reported_to_whoever_waits() {
    let mut app = running("code").await;
    app.client.hello().await;
    let (_page, pane, key) = shown(&app).await;
    app.client
        .ask(AppRequestBody::Send {
            pane: Some(pane.to_string()),
            session: None,
            text: "exit 7\r".to_owned(),
        })
        .await
        .unwrap();
    let exited = app
        .client
        .ask(AppRequestBody::WaitExit {
            pane: None,
            session: Some(key.session),
            timeout_ms: 20_000,
        })
        .await
        .unwrap();
    assert_eq!(
        exited.body,
        AppReplyBody::Exited {
            status: ExitStatus::Code(7)
        }
    );
    stop(app).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_that_follows_sessions_is_told_how_they_stand_and_what_they_report() {
    let mut app = running("follow").await;
    app.client.hello().await;
    let (_page, pane, key) = shown(&app).await;

    // What is not there cannot be followed, and the connection stays an ordinary one.
    let missing = app
        .client
        .ask(AppRequestBody::Events {
            pane: Some("p999".to_owned()),
            session: None,
        })
        .await
        .unwrap_err();
    assert_eq!(missing.code, codes::NOT_FOUND);
    let missing = app
        .client
        .ask(AppRequestBody::Events {
            pane: None,
            session: SessionId::new(0x0bad_0bad),
        })
        .await
        .unwrap_err();
    assert_eq!(missing.code, codes::NOT_FOUND);

    let mut follower = app.connect();
    follower.hello().await;
    let following = follower
        .ask(AppRequestBody::Events {
            pane: Some(pane.to_string()),
            session: None,
        })
        .await
        .unwrap();
    assert_eq!(following.body, AppReplyBody::Following {});

    // The program that has the terminal is looked up when a page shows the
    // session. The follower learns of it as part of how the session stands, or as
    // a report of its own, whichever way round the two happened.
    let name = loop {
        let event = follower.event().await;
        assert_eq!(event.pane, Some(pane.to_string()));
        assert_eq!(event.session, key.session);
        let foreground = match event.what {
            SessionNews::Event(SessionEvent::Foreground(foreground)) => Some(foreground),
            SessionNews::Event(SessionEvent::State { state }) => state.foreground,
            _ => None,
        };
        if let Some(foreground) = foreground {
            break foreground.name;
        }
    };
    if cfg!(windows) {
        assert_eq!(name, "cmd");
    } else {
        assert!(!name.is_empty());
    }

    app.client
        .ask(AppRequestBody::Send {
            pane: Some(pane.to_string()),
            session: None,
            text: "exit 3\r".to_owned(),
        })
        .await
        .unwrap();
    loop {
        if let SessionNews::Exit(status) = follower.event().await.what {
            assert_eq!(status, ExitStatus::Code(3));
            break;
        }
    }
    stop(app).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_follower_of_one_pane_hears_nothing_of_the_others() {
    let mut app = running("follow-one").await;
    app.client.hello().await;
    let (_page, pane, key) = shown(&app).await;
    let other = SessionKey {
        backend: key.backend.clone(),
        session: SessionId::new(0x0d0d).unwrap(),
    };
    let title = |text: &str| SessionEvent::Title {
        title: text.to_owned(),
    };

    let mut follower = app.connect();
    follower.hello().await;
    follower
        .ask(AppRequestBody::Events {
            pane: Some(pane.to_string()),
            session: None,
        })
        .await
        .unwrap();
    let reports = app.services.state.reports();
    reports.on_event(&other, &title("not this one"));
    reports.on_event(&key, &title("lume-k7q"));
    loop {
        let event = follower.event().await;
        assert_eq!(event.session, key.session);
        if event.what == SessionNews::Event(title("lume-k7q")) {
            break;
        }
    }

    // With nothing named, every session is followed, in a pane or not.
    let mut all = app.connect();
    all.hello().await;
    all.ask(AppRequestBody::Events {
        pane: None,
        session: None,
    })
    .await
    .unwrap();
    reports.on_event(&other, &title("this one too"));
    loop {
        let event = all.event().await;
        if event.session == other.session && event.what == SessionNews::Event(title("this one too"))
        {
            assert_eq!(event.pane, None);
            break;
        }
    }
    stop(app).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn what_needs_a_page_is_refused_without_one_and_passed_on_with_one() {
    let mut app = running("page").await;
    app.client.hello().await;

    let refused = app
        .client
        .ask(AppRequestBody::Resize {
            pane: None,
            session: None,
            size: size(4, 1),
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code, codes::NO_WINDOW);
    let refused = app
        .client
        .ask(AppRequestBody::Spawn {
            spec: OpenSpec::default(),
            place: SpawnPlace::Tab {},
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code, codes::NO_WINDOW);

    let (page, pane, key) = shown(&app).await;
    // The page answers as soon as it is asked.
    let state = Arc::clone(&app.services.state);
    let (carrier, id, label) = (page.carrier.clone(), page.id, page.label.clone());
    let answering = tokio::spawn(async move {
        // Notices arrive on the same stream; the request is among them.
        let asked = || {
            carrier
                .requests()
                .into_iter()
                .find(|message| message.get("request").is_some())
        };
        until("the request to the page", || asked().is_some()).await;
        let request = asked().unwrap();
        assert_eq!(request["request"]["body"]["type"], "resize");
        assert_eq!(request["request"]["body"]["pane"], pane.to_string());
        let asked = request["request"]["id"].as_u64().unwrap();
        let answer = AppReplyBody::Resized { size: size(4, 1) };
        state.answer(&label, id, asked, Ok(answer)).unwrap();
    });
    let resized = app
        .client
        .ask(AppRequestBody::Resize {
            pane: None,
            session: None,
            size: size(4, 1),
        })
        .await
        .unwrap();
    answering.await.unwrap();
    assert_eq!(
        resized.effect,
        format!("the terminal in pane {pane} is now 4 by 1")
    );

    // The screen is the host's to read; the page is not asked.
    let before = page.carrier.requests().len();
    let screen = app
        .client
        .ask(AppRequestBody::Screen {
            pane: None,
            session: None,
        })
        .await
        .unwrap();
    assert_eq!(
        screen.effect,
        format!("read 24 lines of the screen of session {}", key.session)
    );
    assert!(
        matches!(&screen.body, AppReplyBody::Screen { pane: shown, session, .. }
            if *shown == pane.to_string() && *session == key.session),
        "{:?}",
        screen.body
    );
    assert_eq!(page.carrier.requests().len(), before);
    stop(app).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_is_started_in_a_new_tab_or_beside_the_focused_pane() {
    let mut app = running("spawn").await;
    app.client.hello().await;
    let (mut page, first, _) = shown(&app).await;

    for place in [
        SpawnPlace::Tab {},
        SpawnPlace::Split {
            dir: Direction::Down,
        },
    ] {
        let before = app.services.panes.workspace().panes().len();
        let asking = app.client.ask(AppRequestBody::Spawn {
            spec: OpenSpec::default(),
            place,
        });
        // The page attaches to the new pane's session when the layout tells it of
        // the pane, as the real page does.
        let panes = Arc::clone(&app.services.panes);
        let state = Arc::clone(&app.services.state);
        let attaching = async {
            until("the new pane", || panes.workspace().panes().len() > before).await;
            let workspace = panes.workspace();
            let made = workspace.panes().into_iter().last().unwrap().id;
            let key = panes.session_of(made).unwrap();
            page.attach(&state, &key).await;
            (made, key)
        };
        let (answer, (made, key)) = tokio::join!(asking, attaching);
        let tab = match place {
            SpawnPlace::Tab {} => app.services.panes.workspace().windows[0]
                .centre
                .active
                .map(|tab| tab.to_string()),
            _ => None,
        };
        assert_eq!(
            answer.unwrap().body,
            AppReplyBody::Spawned {
                session: key.session,
                pane: made.to_string(),
                tab,
            }
        );
        assert_ne!(made, first);
    }
    let workspace = app.services.panes.workspace();
    assert_eq!(workspace.windows[0].centre.tabs.len(), 2);
    assert_eq!(workspace.panes().len(), 3);

    stop(app).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_start_shows_a_workspace_and_runs_a_program_in_a_new_tab() {
    let mut app = running("launch").await;
    app.client.hello().await;
    let (program, args) = if cfg!(windows) {
        ("cmd.exe", "/d")
    } else {
        ("/bin/sh", "-i")
    };

    let launched = app
        .client
        .ask(AppRequestBody::Launch {
            workspace: Some("play".to_owned()),
            exec: vec![program.to_owned(), args.to_owned()],
            cwd: None,
            rex: None,
        })
        .await
        .unwrap();
    let AppReplyBody::Launched { workspace, pane } = launched.body else {
        panic!("not an answer to a launch");
    };
    assert_eq!(workspace, "play");
    assert_eq!(app.shell.activations(), 1);

    // The workspace was begun with a window of its own, and the program has a tab.
    let on_show = app.services.panes.workspace();
    assert_eq!(on_show.name, "play");
    assert_eq!(on_show.windows[0].centre.tabs.len(), 2);
    let pane: PaneId = pane.unwrap().parse().unwrap();
    let lume_layout::PaneContent::Terminal(terminal) = &on_show.pane(pane).unwrap().content else {
        panic!("the program's pane is not a terminal");
    };
    assert!(terminal.session.is_some());
    assert_eq!(
        terminal.launcher,
        Some(lume_session::Launcher::Shell {
            program: program.to_owned(),
            args: vec![args.to_owned()],
            login: false,
        })
    );

    let listed = app.client.ask(AppRequestBody::Workspaces {}).await.unwrap();
    let AppReplyBody::Workspaces { active, workspaces } = listed.body else {
        panic!("not a list of workspaces");
    };
    assert_eq!(active, "play");
    let names: Vec<&str> = workspaces.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, vec![WORKSPACE_NAME, "play"]);
    stop(app).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_command_opens_a_window_and_another_closes_it() {
    let mut app = running("windows").await;
    app.client.hello().await;
    let run = |id: &str, window: Option<String>| AppRequestBody::Command {
        id: id.to_owned(),
        args: serde_json::Value::Null,
        window,
    };

    app.client.ask(run("lume.window.new", None)).await.unwrap();
    let workspace = app.services.panes.workspace();
    assert_eq!(workspace.windows.len(), 2);
    let second = workspace.windows[1].id;
    let shell = app.shell.clone();
    until("the new window to be opened", || {
        shell.open() == vec![label_of(second)]
    })
    .await;

    app.client
        .ask(run("lume.window.close", Some(second.to_string())))
        .await
        .unwrap();
    let shell = app.shell.clone();
    until("the window to be closed", || {
        shell.closed() == vec![label_of(second)]
    })
    .await;
    assert_eq!(app.services.panes.workspace().windows.len(), 1);
    assert_eq!(app.shell.quits(), 0);

    // Closing the only window that is left closes the application, and the layout
    // keeps the window for the next start.
    app.client
        .ask(run("lume.window.close", None))
        .await
        .unwrap();
    assert_eq!(app.shell.quits(), 1);
    assert_eq!(app.services.panes.workspace().windows.len(), 1);
    stop(app).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_settings_are_opened_by_a_command_at_the_section_named() {
    let mut app = running("settings").await;
    app.client.hello().await;
    let run = |args: serde_json::Value| AppRequestBody::Command {
        id: "lume.settings.open".to_owned(),
        args,
        window: None,
    };

    app.client.ask(run(serde_json::Value::Null)).await.unwrap();
    app.client
        .ask(run(serde_json::json!({ "section": "appearance" })))
        .await
        .unwrap();
    assert_eq!(
        app.shell.settings(),
        vec![None, Some("appearance".to_owned())]
    );
    // Showing the settings changes nothing in the layout.
    assert_eq!(app.services.panes.workspace().windows.len(), 1);
    stop(app).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_window_closed_from_its_own_page_is_asked_to_close_and_not_closed_outright() {
    let app = running("ask-close").await;
    let window = app.services.panes.workspace().windows[0].id;

    app.services
        .run_command(
            window,
            "lume.window.close",
            &serde_json::Value::Null,
            crate::services::Asker::Page,
        )
        .await
        .unwrap();
    // The window is asked, as its close control asks: whether the user is asked
    // first is decided where a window's closing is.
    assert_eq!(app.shell.asked_to_close(), vec![window]);
    assert_eq!(app.shell.quits(), 0);
    assert_eq!(app.shell.closed(), Vec::<String>::new());
    stop(app).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_layout_is_read_and_changed_and_commands_are_listed_and_run() {
    let mut app = running("layout").await;
    app.client.hello().await;
    let pane = app.services.panes.workspace().panes()[0].id;

    let read = app.client.ask(AppRequestBody::Layout {}).await.unwrap();
    let AppReplyBody::Layout {
        window,
        epoch,
        revision,
        workspace,
    } = read.body
    else {
        panic!("not a layout");
    };
    assert_eq!(window, Some(app.window.to_string()));
    assert_eq!(epoch, 1);
    assert_eq!(workspace["name"], WORKSPACE_NAME);

    let renamed = app
        .client
        .ask(AppRequestBody::LayoutApply {
            intent: serde_json::json!({
                "type": "pane_rename", "pane": pane.to_string(), "title": "logs"
            }),
        })
        .await
        .unwrap();
    assert_eq!(
        renamed.body,
        AppReplyBody::Applied {
            epoch: 1,
            revision: revision + 1,
            created: serde_json::json!({"pane": null, "tab": null, "window": null}),
        }
    );
    let again = app
        .client
        .ask(AppRequestBody::LayoutApply {
            intent: serde_json::json!({
                "type": "pane_rename", "pane": pane.to_string(), "title": "logs"
            }),
        })
        .await
        .unwrap();
    assert!(again.effect.contains("changed nothing"), "{}", again.effect);

    for (intent, code) in [
        (
            serde_json::json!({"type": "no_such_intent"}),
            codes::BAD_REQUEST,
        ),
        (
            serde_json::json!({"type": "close", "pane": "p999"}),
            codes::NOT_FOUND,
        ),
        (
            serde_json::json!({"type": "window_focus", "window": app.window.to_string()}),
            "forbidden",
        ),
    ] {
        let refused = app
            .client
            .ask(AppRequestBody::LayoutApply { intent })
            .await
            .unwrap_err();
        assert_eq!(refused.code, code);
    }

    let listed = app.client.ask(AppRequestBody::Commands {}).await.unwrap();
    let AppReplyBody::Commands { commands } = listed.body else {
        panic!("not a list of commands");
    };
    assert!(
        commands
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command["id"] == "lume.pane.zoom" && command["runs"] == "core")
    );

    let ran = app
        .client
        .ask(AppRequestBody::Command {
            id: "lume.pane.zoom".to_owned(),
            args: serde_json::Value::Null,
            window: None,
        })
        .await
        .unwrap();
    assert_eq!(
        ran.body,
        AppReplyBody::Ran {
            command: "lume.pane.zoom".to_owned()
        }
    );
    assert!(app.services.panes.is_zoomed(pane));

    let unknown = app
        .client
        .ask(AppRequestBody::Command {
            id: "lume.pane.fly".to_owned(),
            args: serde_json::Value::Null,
            window: None,
        })
        .await
        .unwrap_err();
    assert_eq!(unknown.code, codes::NOT_FOUND);
    // A command that runs in the page needs the window's page, and there is none.
    let pageless = app
        .client
        .ask(AppRequestBody::Command {
            id: "lume.terminal.copy".to_owned(),
            args: serde_json::Value::Null,
            window: None,
        })
        .await
        .unwrap_err();
    assert_eq!(pageless.code, codes::NO_WINDOW);
    stop(app).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pane_or_session_that_is_not_there_is_said_to_be_missing() {
    let mut app = running("missing").await;
    app.client.hello().await;
    let (_page, _pane, _key) = shown(&app).await;

    let unknown = app
        .client
        .ask(AppRequestBody::Send {
            pane: Some("p999".to_owned()),
            session: None,
            text: "x".to_owned(),
        })
        .await
        .unwrap_err();
    assert_eq!(unknown.code, codes::NOT_FOUND);
    let malformed = app
        .client
        .ask(AppRequestBody::Screen {
            pane: Some("nine".to_owned()),
            session: None,
        })
        .await
        .unwrap_err();
    assert_eq!(malformed.code, codes::BAD_REQUEST);

    // A session in no pane is typed into all the same, through its host.
    let stray = app
        .services
        .state
        .router()
        .get(&Router::local_id())
        .unwrap()
        .spawn(crate::testing::shell())
        .await
        .unwrap();
    let sent = app
        .client
        .ask(AppRequestBody::Send {
            pane: None,
            session: Some(stray),
            text: "echo typed-k7q\r".to_owned(),
        })
        .await
        .unwrap();
    assert!(
        matches!(sent.body, AppReplyBody::Sent { bytes: 15 }),
        "{sent:?}"
    );
    app.client
        .ask(AppRequestBody::WaitText {
            pane: None,
            session: Some(stray),
            text: "typed-k7q".to_owned(),
            timeout_ms: 10_000,
        })
        .await
        .unwrap();

    let unknown = app
        .client
        .ask(AppRequestBody::Kill {
            session: SessionId::new(1).unwrap(),
        })
        .await
        .unwrap_err();
    assert_eq!(unknown.code, codes::NOT_FOUND);
    stop(app).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_is_shown_to_a_client_as_a_terminal_of_its_own() {
    use lume_proto::{Body, FrameDecoder, HydratePayload, Message, Role};

    let mut app = running("attach").await;
    app.client.hello().await;
    let (page, _pane, key) = shown(&app).await;

    let attached = app
        .client
        .ask(AppRequestBody::Attach {
            pane: None,
            session: Some(key.session),
            size: lume_proto::TermSize::new(100, 30).unwrap(),
        })
        .await
        .unwrap();
    assert!(
        matches!(attached.body, AppReplyBody::Attached { session, .. } if session == key.session),
        "{attached:?}"
    );

    // From here the connection carries the attachment on stream 1.
    let mut frames = FrameDecoder::new(Role::Client);
    frames.push(&std::mem::take(&mut app.client.decoder).into_rest());
    let mut chunk = vec![0; READ_CHUNK];
    let mut next = async |frames: &mut FrameDecoder, io: &mut DuplexStream| loop {
        if let Some(message) = frames.next().unwrap() {
            return Some(message);
        }
        let read = timeout(STEP, io.read(&mut chunk)).await.ok()?.unwrap();
        if read == 0 {
            return None;
        }
        frames.push(&chunk[..read]);
    };
    let first = next(&mut frames, &mut app.client.io).await.unwrap();
    assert!(
        matches!(first.body, Body::Hydrate(HydratePayload::VtReplay(_))),
        "{first:?}"
    );

    let send = async |io: &mut DuplexStream, body: Body| {
        let mut out = Vec::new();
        lume_proto::encode(&Message::new(1, body), &mut out).unwrap();
        io.write_all(&out).await.unwrap();
    };
    send(&mut app.client.io, Body::Credit(1024 * 1024)).await;
    let command = if cfg!(windows) {
        "echo lume-%OS:~0,0%k7q-bridged\r"
    } else {
        "echo \"lume-\"\"k7q-bridged\"\r"
    };
    send(&mut app.client.io, Body::Input(command.as_bytes().to_vec())).await;
    let mut seen = String::new();
    while !seen.contains("lume-k7q-bridged") {
        match next(&mut frames, &mut app.client.io).await {
            Some(Message {
                body: Body::Data(bytes),
                ..
            }) => seen.push_str(&String::from_utf8_lossy(&bytes)),
            Some(_) => {}
            None => panic!("the output never came; got {seen:?}"),
        }
    }
    // The window showing the session saw it too.
    until("the window's copy of the output", || {
        page.text().contains("lume-k7q-bridged")
    })
    .await;

    // A detach ends what the connection was for.
    send(&mut app.client.io, Body::Detach).await;
    while next(&mut frames, &mut app.client.io).await.is_some() {}
    stop(app).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_verb_no_plugin_adds_is_refused_and_the_connection_carries_on() {
    let mut app = running("verbs").await;
    app.client.hello().await;
    let listed = app.client.ask(AppRequestBody::Verbs {}).await.unwrap();
    assert_eq!(
        listed.body,
        AppReplyBody::Verbs {
            verbs: serde_json::json!([])
        }
    );
    let verb = |stdin: Option<String>| AppRequestBody::Verb {
        words: vec!["agent".to_owned(), "run".to_owned()],
        cwd: "/work".to_owned(),
        stdin,
    };
    let refused = app.client.ask(verb(None)).await.unwrap_err();
    assert_eq!(refused.code, codes::NOT_FOUND);
    assert!(refused.message.contains("agent"), "{refused:?}");
    let refused = app
        .client
        .ask(verb(Some("x".repeat(STDIN_MAX + 1))))
        .await
        .unwrap_err();
    assert_eq!(refused.code, codes::BAD_REQUEST);
    // Neither ended the connection.
    app.client.ask(AppRequestBody::Verbs {}).await.unwrap();
    stop(app).await;
}
