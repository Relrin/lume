use std::time::Duration;

use lume_layout::{Dir, FocusTarget};
use lume_proto::{Body, Message};
use lume_session::{HostConfig, LocalBackend, Router};
use tokio::runtime::Handle;
use tokio::time::{sleep, timeout};

use super::*;
use crate::marks::Marks;
use crate::testing::{
    CREDIT, FakeServer, FakeShell, RecordingCarrier, STEP, TempDir, client_frames, facts,
    server_session, size,
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
    let there: lume_layout::GroupId = action["args"]["group"].as_str().unwrap().parse().unwrap();
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
    app.state.reports().shown(key);
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
async fn a_servers_session_in_an_own_workspace_never_gives_a_directory_or_a_process() {
    let dir = TempDir::new("served-elsewhere");
    plain_shell(&dir, "");
    let dirs = dir.dirs();
    let app = running_in(dir).await;
    let here = dirs.config.join("k7q-folder");
    std::fs::create_dir_all(&here).unwrap();
    let mut info = server_session(7, "pwsh.exe");
    info.pid = Some(std::process::id());
    let key = SessionKey {
        backend: BackendId("server".to_owned()),
        session: info.id,
    };
    app.state.router().insert(
        key.backend.clone(),
        Arc::new(FakeServer {
            sessions: vec![info],
            ..FakeServer::default()
        }),
    );
    crate::host::reattach(&app.panes, app.window, key.clone(), crate::host::Place::Tab)
        .await
        .unwrap();
    let pane = app.panes.pane_of(&key).unwrap();

    // The report is as good as one gets.
    says_it_is_in(&app, &key, &reported(&here));
    assert_eq!(app.state.reports().launched(&key), None);
    assert_eq!(app.panes.inherited_dir(pane, None).await, None);
    assert_eq!(app.panes.live_dir(pane).await, None);
    // What a program of this machine's number does there says nothing of it.
    let running = lume_session::Foreground {
        pid: std::process::id(),
        name: "pwsh".to_owned(),
    };
    app.state
        .reports()
        .on_event(&key, &lume_session::SessionEvent::Foreground(running));
    assert!(app.state.reports().busy(&key, false));

    app.panes.save().await;
    let kept = std::fs::read_dir(dirs.state_workspaces_dir())
        .unwrap()
        .map(|file| std::fs::read_to_string(file.unwrap().path()).unwrap())
        .collect::<String>();
    assert!(kept.contains("server"), "{kept}");
    assert!(!kept.contains("k7q-folder"), "{kept}");
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
