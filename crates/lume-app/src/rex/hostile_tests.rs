//! The application with a server that means harm, or is broken: whatever it says, the
//! user is told and asked nothing in its words, the application keeps within its bounds,
//! and what is the user's own stands as it did.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Once;
use std::sync::atomic::AtomicUsize;

use lume_layout::PaneId;
use lume_proto::{Body, Message, SessionId};
use lume_session::cwd::{file_url_of, machine_name};

use super::common::hostile::{self, CODES, NAMES, Numbers, OWN_CODES, is_plain};
use super::common::programs::{self, Played, Programs, SCRIPT};
use super::common::views::{leaf, tiled, view};
use super::common::{DataSide, Recorded, Refusing, SESSION, asks_for_control};
use super::*;
use crate::carrier::SessionKey;
use crate::host::{Place, reattach};
use crate::testing::{CREDIT, PRINT_MARK, client_frames, facts, shell};
use crate::tuning::{SERVED_COPIES_PER_WINDOW, SERVER_LIST_WAIT};

/// How long anything may take that is not told to wait.
const SOON: Duration = Duration::from_secs(20);

/// What the notices a server can bring about may offer to run.
const OFFERED: [&str; 1] = ["lume.rex.open"];

/// How many times something panicked that is not a test. A task that does takes nothing
/// else with it, and would not be missed. The tests here run on their own threads.
static PANICS: AtomicUsize = AtomicUsize::new(0);

fn count_panics() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let usual = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current();
            let here = thread.name().is_some_and(|name| name.contains("hostile"));
            let tests = info
                .location()
                .is_some_and(|place| place.file().contains("tests"));
            PANICS.fetch_add(usize::from(here && !tests), Ordering::SeqCst);
            usual(info);
        }));
    });
}

/// A server whose terminals can be attached to, and which begins their streams as a
/// server does.
struct Hostile {
    address: std::net::SocketAddr,
    served: Arc<Mutex<Vec<Served>>>,
    accepting: AbortHandle,
}

impl Hostile {
    async fn start(world: World) -> Self {
        count_panics();
        let (address, mut listener) = door().await;
        let served = Arc::new(Mutex::new(Vec::new()));
        let keep = Arc::clone(&served);
        let accepting = tokio::spawn(async move {
            loop {
                let Some(stream) = listener.next().await else {
                    return;
                };
                let Some((control, stream)) = asks_for_control(stream).await else {
                    continue;
                };
                if control {
                    let conn = Conn::accept(stream, Recorded::load().welcome()).await;
                    keep.lock().unwrap().push(serve(conn, world.clone()));
                } else {
                    tokio::spawn(async move {
                        let mut side = DataSide::accept(stream).await;
                        side.bytes(b"not a copy at all").await;
                        side.size(80, 24).await;
                        side.closed_by_client().await;
                    });
                }
            }
        })
        .abort_handle();
        Self {
            address,
            served,
            accepting,
        }
    }

    fn tell(&self, what: Tell) {
        self.served.lock().unwrap().last().unwrap().tell(what);
    }

    fn connections(&self) -> usize {
        self.served.lock().unwrap().len()
    }

    /// How often a view was read.
    fn views_read(&self) -> usize {
        let served = self.served.lock().unwrap();
        let all = served
            .iter()
            .map(|served| served.asked("session.view").len());
        all.sum()
    }

    /// Replaces what the server holds of the recorded session, and says so.
    fn show(&self, shown: Value, revision: u64) {
        let mut world = World::recorded();
        let mut shown = shown;
        if shown.is_object() {
            shown["revision"] = json!(revision);
        }
        world.views.insert(SESSION.to_owned(), shown);
        self.tell(Tell::World(world));
        self.tell(Tell::SessionEvent(
            SESSION.to_owned(),
            "session_view_changed".to_owned(),
            json!({ "revision": revision }),
        ));
    }

    fn of_block(&self, block: &str, name: &str, payload: Value) {
        self.tell(Tell::SessionEvent(
            SESSION.to_owned(),
            "block_event".to_owned(),
            json!({
                "block_id": block, "creator_name": "com.superlogical.terminal",
                "name": name, "payload": payload,
            }),
        ));
    }

    fn called(&self, label: &str) {
        self.tell(Tell::SessionEvent(
            SESSION.to_owned(),
            "session_label_changed".to_owned(),
            json!({ "label": label }),
        ));
    }
}

impl Drop for Hostile {
    fn drop(&mut self) {
        self.accepting.abort();
    }
}

/// The first block of the recorded session.
fn block() -> String {
    let shown = World::recorded().views[SESSION].clone();
    shown["windows"][0]["layers"][0]["blocks"][0]["block_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Every text in a value.
fn texts(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::String(text) => found.push(text.clone()),
        Value::Array(items) => items.iter().for_each(|item| texts(item, found)),
        Value::Object(fields) => fields.values().for_each(|field| texts(field, found)),
        _ => {}
    }
}

fn all_plain(value: &Value) {
    let mut found = Vec::new();
    texts(value, &mut found);
    for text in found {
        assert!(is_plain(&text), "{text:?} in {value}");
    }
}

/// What is the user's own, and what was done on their desktop.
#[derive(Debug, PartialEq)]
struct Own {
    files: BTreeMap<String, String>,
    sessions: Vec<SessionId>,
    backends: Vec<BackendId>,
    workspace: String,
    windows: Vec<String>,
    closed: Vec<String>,
    copied: Vec<String>,
    notified: Vec<(String, String)>,
    settings: Vec<Option<String>>,
    quits: usize,
}

fn files(dir: &Path, found: &mut BTreeMap<String, String>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, found);
        } else if path.file_name().is_some_and(|name| name != "rex.json") {
            // Which session was last on show is kept there, and nothing else of a server.
            let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
            // When a workspace was written says nothing of what is in it.
            let text = text
                .lines()
                .filter(|line| !line.contains("saved_unix_ms"))
                .collect();
            found.insert(path.display().to_string(), text);
        }
    }
}

impl Running {
    /// The application with a shell of its own that starts fast and keeps no history.
    async fn plain(name: &str, server: &Hostile) -> Self {
        let shell = if cfg!(windows) {
            "shell = \"cmd.exe\"\nshell-args = [\"/d\"]\n"
        } else {
            "shell = \"/bin/sh\"\n"
        };
        let host = configured(&format!("http://{}", server.address));
        let keep = "[workspace]\non-switch = \"keep\"\n";
        Self::start(name, &format!("{host}\n{keep}[terminal]\n{shell}")).await
    }

    async fn opened(&self) {
        self.outside("lume.rex.open", json!({ "host": HOST }))
            .await
            .unwrap();
    }

    /// Puts the session on show again, whatever the server has said of it meanwhile.
    async fn opened_again(&self, server: &Hostile) {
        server.tell(Tell::World(World::recorded()));
        server.tell(Tell::Event(
            "session_created".to_owned(),
            json!({ "session": { "session_id": SESSION, "label": "again" } }),
        ));
        let backend = self.backend();
        until("it is listed", || {
            backend.sessions().iter().any(|one| one.id == SESSION)
        })
        .await;
        let args = json!({ "host": HOST, "session": SESSION });
        self.outside("lume.rex.open", args).await.unwrap();
    }

    fn backend(&self) -> Arc<RexBackend> {
        let inner = self.rex().inner();
        let connection = inner.connections.values().next().unwrap();
        Arc::clone(&connection.backend)
    }

    fn label(&self) -> String {
        label_of(self.services.panes.focused_window().unwrap())
    }

    /// Starts the shells of the workspace on show.
    async fn own_shells(&self) -> Vec<SessionKey> {
        let panes = &self.services.panes;
        let window = panes.focused_window().unwrap();
        let asked = panes
            .workspace()
            .panes()
            .iter()
            .map(|pane| PaneStart {
                pane: pane.id,
                size: size(80, 24),
            })
            .collect();
        let started = panes.start_sessions(window, asked).await.started;
        assert!(!started.is_empty());
        started
            .into_iter()
            .map(|started| SessionKey {
                backend: started.backend,
                session: started.session,
            })
            .collect()
    }

    /// Attaches the page to a session, on a stream.
    async fn attach(&self, key: &SessionKey, stream: u32) -> Result<(), CommandError> {
        let state = &self.services.state;
        let label = self.label();
        state
            .attach(
                &label,
                self.page,
                &key.backend,
                key.session,
                stream,
                facts(),
            )
            .await?;
        self.say(stream, Body::Credit(CREDIT)).await;
        Ok(())
    }

    async fn say(&self, stream: u32, body: Body) {
        let frames = client_frames(&[Message::new(stream, body)]);
        let state = &self.services.state;
        state.send(&self.label(), self.page, &frames).await.unwrap();
    }

    fn served_panes(&self) -> Vec<(PaneId, SessionKey)> {
        let panes = &self.services.panes;
        let shown = panes.workspace();
        shown
            .panes()
            .into_iter()
            .filter_map(|pane| Some((pane.id, panes.session_of(pane.id)?)))
            .collect()
    }

    async fn own(&self) -> Own {
        let state = &self.services.state;
        self.services.panes.save().await;
        let mut found = BTreeMap::new();
        files(self.dir.path(), &mut found);
        let local = state.router().get(&Router::local_id()).unwrap();
        let shell = &self.shell;
        Own {
            files: found,
            sessions: local.list().await.unwrap().iter().map(|i| i.id).collect(),
            backends: state.router().ids(),
            workspace: serde_json::to_string(&self.services.panes.workspace().windows).unwrap(),
            windows: shell.open(),
            closed: shell.closed(),
            copied: shell.copied(),
            notified: shell.notified(),
            settings: shell.settings(),
            quits: shell.quits(),
        }
    }

    /// Nothing the user was told or asked is in a server's words, or does more than say
    /// something.
    fn told_nothing_amiss(&self) {
        for notice in self.told("notice") {
            all_plain(&notice);
            if let Some(action) = notice.get("action").filter(|action| !action.is_null()) {
                assert!(
                    OFFERED.contains(&action["command"].as_str().unwrap()),
                    "{notice}"
                );
                assert_eq!(action["args"].get("confirmed"), None, "{notice}");
            }
        }
        assert_eq!(self.told("confirm_requested"), Vec::<Value>::new());
        assert_eq!(self.commands(), Vec::<Value>::new());
        assert_eq!(self.put(), Vec::<Value>::new());
        assert_eq!(PANICS.load(Ordering::SeqCst), 0, "something panicked");
    }
}

#[tokio::test]
async fn what_a_server_calls_things_is_plain_wherever_the_user_meets_it() {
    for name in NAMES {
        let mut world = World::recorded();
        world.sessions[0].1 = name.to_owned();
        let shown = world.views.get_mut(SESSION).unwrap();
        shown["label"] = json!(name);
        shown["windows"][0]["label"] = json!(name);
        shown["windows"][0]["layers"][0]["blocks"][0]["label"] = json!(name);
        shown["windows"][0]["layers"][0]["blocks"][1]["label"] = json!(name);
        shown["windows"][0]["layers"][0]["blocks"][1]["creator_name"] = json!(name);
        let said = name.to_owned();
        world.twist = Some(Arc::new(move |method, mut answer| {
            match method {
                "server.version" => {
                    answer["rex_version"] = json!(said);
                    answer["platform"] = json!(said);
                }
                "com.superlogical.terminal.title" => answer["title"] = json!(said),
                "com.superlogical.terminal.process" => {
                    answer["foreground"] = json!({ "pid": 7, "name": said, "cwd": said });
                }
                _ => {}
            }
            Some(answer)
        }));
        let server = Hostile::start(world).await;
        let app = Running::plain("rex-hostile-names", &server).await;
        app.opened().await;
        all_plain(&app.rex().view());
        let shown = app.services.panes.workspace();
        all_plain(&serde_json::to_value(&shown.windows).unwrap());
        all_plain(&json!(shown.name));
        for listed in app.services.state.list_all().await.unwrap() {
            all_plain(&serde_json::to_value(&listed.info).unwrap());
        }

        // The one question a name of a server's is in.
        app.command(
            "lume.rex.end-session",
            json!({ "host": HOST, "session": SESSION }),
            Asker::Page,
        )
        .await
        .unwrap();
        until("the window asks", || {
            !app.told("confirm_requested").is_empty()
        })
        .await;
        let asked = app.told("confirm_requested")[0].clone();
        all_plain(&asked);
        assert!(asked["title"].as_str().unwrap().starts_with("End “"));
        assert_eq!(asked["command"], "lume.rex.end-session");
        app.stop().await;
    }
}

#[tokio::test]
async fn a_servers_word_for_a_failure_asks_the_user_nothing() {
    let refused: [(&str, &str, Value); 4] = [
        ("lume.rex.open", "session.attach", json!({ "host": HOST })),
        (
            "lume.rex.new-session",
            "session.create",
            json!({ "host": HOST }),
        ),
        (
            "lume.rex.rename-session",
            "session.set_label",
            json!({ "host": HOST, "session": SESSION, "label": "x" }),
        ),
        (
            "lume.rex.end-session",
            "session.destroy",
            json!({ "host": HOST, "session": SESSION, "confirmed": true }),
        ),
    ];
    for code in CODES {
        let refusing: Refusing = Arc::new(move |method| {
            let methods = ["session.attach", "session.create", "session.set_label"];
            (methods.contains(&method) || method == "session.destroy")
                .then(|| (code.to_owned(), 418, code.to_owned()))
        });
        let world = World {
            refusing: Some(refusing),
            ..World::recorded()
        };
        let server = Hostile::start(world).await;
        let app = Running::plain("rex-hostile-codes", &server).await;
        app.rex().connect(HOST, false, None).await.unwrap();
        for (command, method, args) in &refused {
            let error = app
                .command(command, args.clone(), Asker::Page)
                .await
                .unwrap_err();
            assert!(
                OWN_CODES.contains(&error.code.as_ref()),
                "{command} {method} as {code:?}: {error}"
            );
            assert!(is_plain(&error.message), "{:?}", error.message);
        }
        assert!(app.own_on_show());

        // What ends a connection, and what refuses one, has no word of a server's either.
        server.tell(Tell::Close(1008, code.to_owned()));
        until("the connection is over", || app.kind() != "connected").await;
        all_plain(&app.rex().view());
        assert_ne!(app.host()["state"]["code"], "confirmation_required");
        app.told_nothing_amiss();
        app.stop().await;
    }
}

#[tokio::test]
async fn a_server_that_refuses_to_say_what_it_is_stands_as_one_that_cannot_be_reached() {
    for code in CODES {
        let world = World {
            refusing: Some(Arc::new(move |method| {
                (method == "server.version").then(|| (code.to_owned(), 403, code.to_owned()))
            })),
            ..World::recorded()
        };
        let server = Hostile::start(world).await;
        let app = Running::plain("rex-hostile-version", &server).await;
        let error = app
            .command("lume.rex.connect", json!({ "host": HOST }), Asker::Page)
            .await
            .unwrap_err();
        assert_eq!(error.code, "unreachable", "{code:?}: {error}");
        assert_eq!(app.host()["state"]["code"], "unreachable");
        let endpoint = endpoint_of(&format!("http://{}", server.address)).unwrap();
        let asks = app.rex().ssh_options("", "", None);
        let error = attempt::attempt(endpoint, false, asks).await.unwrap_err();
        assert_eq!(error.code, "unreachable", "{code:?}: {error}");
        assert!(is_plain(&error.message));
        app.told_nothing_amiss();
        app.stop().await;
    }
}

#[test]
fn there_is_nothing_to_agree_to_where_a_connection_is_always_encrypted() {
    let named =
        |address: &str, code: &str| refusal(&Endpoint::parse(address).unwrap(), code.to_owned());
    for address in ["http://build", "build", "build:7297"] {
        assert_eq!(named(address, UNCONFIRMED), UNCONFIRMED, "{address}");
    }
    for address in ["https://build", "ssh://me@build", "local"] {
        assert_eq!(named(address, UNCONFIRMED), "refused", "{address}");
        assert_eq!(named(address, "untrusted"), "untrusted");
    }
}

#[tokio::test]
async fn pages_are_told_how_servers_stand_no_more_than_so_often() {
    let server = Hostile::start(World::recorded()).await;
    let app = Running::plain("rex-hostile-labels", &server).await;
    app.rex().connect(HOST, false, None).await.unwrap();
    until("it is listed", || app.host()["sessions"] != json!([])).await;
    let before = app.told("rex_hosts_changed").len();
    let began = Instant::now();
    for nth in 0..3_000 {
        server.called(&format!("label {nth}"));
        if nth % 100 == 0 {
            sleep(Duration::from_millis(10)).await;
        }
    }
    let last = json!([{ "id": SESSION, "label": "label 2999" }]);
    until("the last is told", || {
        app.told("rex_hosts_changed")
            .last()
            .is_some_and(|notice| notice["hosts"][0]["sessions"] == last)
    })
    .await;
    let told = app.told("rex_hosts_changed");
    let most = began.elapsed().as_millis() / REX_TELL_EVERY.as_millis() + 2;
    assert!(((told.len() - before) as u128) <= most, "{}", told.len());
    // Each is newer than the one before it, and a look is never behind.
    let revisions: Vec<u64> = told
        .iter()
        .map(|notice| notice["revision"].as_u64().unwrap())
        .collect();
    assert!(revisions.is_sorted_by(|one, next| one < next));
    assert_eq!(app.host()["sessions"], last);
    app.stop().await;
}

#[tokio::test]
async fn a_layout_that_is_changed_without_end_is_shown_as_it_comes_to_stand() {
    let server = Hostile::start(World::recorded()).await;
    let app = Running::plain("rex-hostile-layouts", &server).await;
    app.opened().await;
    let began = Instant::now();
    let shown = |nth: u64| {
        view(
            (0..4)
                .map(|window| {
                    tiled(
                        &format!("window:{nth}:{window}"),
                        leaf(&format!("block:{nth}:{window}")),
                    )
                })
                .collect(),
        )
    };
    for nth in 0..400 {
        server.show(shown(nth), 10_000 + nth);
        // Each is read and shown on its own, as far as that goes.
        let _ = app.backend().round_trip().await;
    }
    let backend = app.backend();
    until("the last is shown", || {
        let Some(view) = backend.view(SESSION) else {
            return false;
        };
        view.revision == 10_399
            && app.services.panes.workspace().panes().len() == 4
            && app
                .served_panes()
                .iter()
                .all(|(_, key)| key.backend.0.starts_with("rex:"))
    })
    .await;
    let changes = app.told("layout_changed").len() as u64;
    let windows = began.elapsed().as_secs() + 1;
    assert!(
        changes <= windows * u64::from(SERVED_COPIES_PER_WINDOW) + 5,
        "{changes} in {:?}",
        began.elapsed()
    );
    app.told_nothing_amiss();
    app.stop().await;
}

#[tokio::test]
async fn what_blocks_reported_is_forgotten_when_they_are_gone() {
    let server = Hostile::start(World::recorded()).await;
    let app = Running::plain("rex-hostile-churn", &server).await;
    let own = app.own_shells().await;
    let reports = Arc::clone(app.services.state.reports());
    app.opened().await;
    let mut stream = 0;
    let mut before: Option<(SessionKey, String)> = None;
    for round in 0..8u64 {
        let names: Vec<String> = (0..10).map(|nth| format!("block:{round}:{nth}")).collect();
        let windows = names
            .iter()
            .map(|name| tiled(&format!("window:{name}"), leaf(name)))
            .collect();
        server.show(view(windows), 1_000 + round);
        let backend = app.backend();
        until("the layout is shown", || {
            backend
                .view(SESSION)
                .is_some_and(|view| view.revision == 1_000 + round)
                && app.served_panes().len() == 10
        })
        .await;
        // The panes of the round before may be on show for a moment still.
        sleep(Duration::from_millis(100)).await;
        let shown = app.served_panes();
        for ((_, key), name) in shown.iter().zip(&names).take(3) {
            stream += 1;
            app.attach(key, stream).await.unwrap();
            server.of_block(name, "title_changed", json!({ "title": "x" }));
        }
        until("they reported", || {
            shown
                .iter()
                .take(3)
                .all(|(_, key)| reports.state(key).is_some())
        })
        .await;
        assert!(reports.len() <= own.len() + 10 + 3, "{}", reports.len());
        // What is said of a block that is gone is kept no more, though the page has not
        // let go of it yet.
        if let Some((gone, name)) = &before {
            server.of_block(name, "title_changed", json!({ "title": "late" }));
            let _ = app.backend().round_trip().await;
            sleep(Duration::from_millis(50)).await;
            assert_eq!(reports.state(gone), None);
            app.say(stream - 3, Body::Detach).await;
        }
        before = Some((shown[2].1.clone(), names[2].clone()));
        // The page lets go of what it does not show any more.
        for each in stream - 2..stream {
            app.say(each, Body::Detach).await;
        }
    }
    // What is the user's own is known still.
    assert!(own.iter().all(|key| reports.launched(key).is_some()));
    app.outside("lume.rex.disconnect", json!({ "host": HOST }))
        .await
        .unwrap();
    until("nothing of the server is known", || {
        reports.len() == own.len()
    })
    .await;
    assert_eq!(app.services.state.router().ids(), [Router::local_id()]);
    assert!(app.rex().inner().connections.is_empty());
    app.told_nothing_amiss();
    app.stop().await;
}

#[tokio::test]
async fn a_server_that_does_not_answer_holds_up_nothing_of_the_users_own() {
    let mut world = World::recorded();
    world.twist = Some(Arc::new(|method, answer| {
        (method != "com.superlogical.terminal.process").then_some(answer)
    }));
    let server = Hostile::start(world).await;
    let app = Running::plain("rex-hostile-slow", &server).await;
    app.rex().connect(HOST, false, None).await.unwrap();
    app.backend().open_session(SESSION).await.unwrap();
    let state = &app.services.state;
    let label = app.label();
    let local = Router::local_id();
    let session = state
        .spawn(&label, app.page, &local, shell())
        .await
        .unwrap();

    // Others ask for every session meanwhile.
    let listing = {
        let state = Arc::clone(state);
        tokio::spawn(async move {
            let began = Instant::now();
            (state.list_all().await, began.elapsed())
        })
    };
    let began = Instant::now();
    assert_eq!(state.find(session).await.unwrap().backend, local);
    let key = SessionKey {
        backend: local,
        session,
    };
    app.attach(&key, 1).await.unwrap();
    app.say(1, Body::Input(PRINT_MARK.to_vec())).await;
    until("the shell prints", || {
        app.carrier.messages().iter().any(|message| {
            matches!(&message.body, Body::Data(bytes)
                if String::from_utf8_lossy(bytes).contains("lume-k7q-done"))
        })
    })
    .await;
    assert!(began.elapsed() < Duration::from_secs(4));
    let (listed, took) = listing.await.unwrap();
    assert!(
        took <= SERVER_LIST_WAIT + Duration::from_secs(1),
        "{took:?}"
    );
    assert!(listed.unwrap().iter().any(|one| one.info.id == session));
    app.told_nothing_amiss();
    app.stop().await;
}

#[tokio::test]
async fn what_a_server_says_of_this_machine_is_believed_of_no_session_of_its() {
    let server = Hostile::start(World::recorded()).await;
    let app = Running::plain("rex-hostile-here", &server).await;
    let panes = &app.services.panes;
    app.rex().connect(HOST, false, None).await.unwrap();
    app.backend().open_session(SESSION).await.unwrap();
    let listed = app.services.state.list_all().await.unwrap();
    let key = listed
        .iter()
        .find(|listed| listed.backend.0.starts_with("rex:"))
        .map(|listed| SessionKey {
            backend: listed.backend.clone(),
            session: listed.info.id,
        })
        .unwrap();
    let window = panes.focused_window().unwrap();
    reattach(panes, window, key.clone(), Place::Tab)
        .await
        .unwrap();
    app.attach(&key, 1).await.unwrap();

    let folder = app.dir.path().join("k7q-folder");
    std::fs::create_dir_all(&folder).unwrap();
    let url = file_url_of(&folder.to_string_lossy()).unwrap();
    let url = url.replacen("file://", &format!("file://{}", machine_name().unwrap()), 1);
    let reports = app.services.state.reports();
    let name = block();
    server.of_block(&name, "pwd_changed", json!({ "pwd": url }));
    server.of_block(
        &name,
        "process_changed",
        json!({ "foreground": { "pid": std::process::id(), "name": "pwsh", "cwd": url } }),
    );
    until("it has reported", || {
        reports
            .state(&key)
            .is_some_and(|state| state.cwd.as_ref() == Some(&url) && state.foreground.is_some())
    })
    .await;

    let pane = panes.pane_of(&key).unwrap();
    assert!(!app.services.state.is_here(&key));
    assert_eq!(panes.inherited_dir(pane, None).await, None);
    assert_eq!(panes.live_dir(pane).await, None);
    assert!(reports.busy(&key, false));
    let mut kept = BTreeMap::new();
    panes.save().await;
    files(&app.dir.dirs().state, &mut kept);
    assert!(kept.values().any(|text| text.contains("rex:http")));
    assert!(kept.values().all(|text| !text.contains("k7q-folder")));
    app.services.plugins.start(&app.services);
    let told = app.services.plugins.session_list().await.unwrap();
    assert!(!told.to_string().contains("k7q-folder"), "{told}");

    // The pane stays when the server is let go of, and nothing that was said in it.
    app.outside("lume.rex.disconnect", json!({ "host": HOST }))
        .await
        .unwrap();
    assert!(panes.pane_of(&key).is_some());
    assert_eq!(reports.state(&key), None);
    app.told_nothing_amiss();
    app.stop().await;
}

#[tokio::test]
async fn whatever_a_server_does_what_is_the_users_own_stands_as_it_did() {
    let server = Hostile::start(World::recorded()).await;
    let app = Running::plain("rex-hostile-all", &server).await;
    app.own_shells().await;
    let window = app.services.panes.focused_window().unwrap();
    crate::control::Shell::open_window(app.shell.as_ref(), window, None);
    // As it stands once a session was shown and put away again, with nothing amiss.
    app.opened().await;
    let disconnect = ("lume.rex.disconnect", json!({ "host": HOST }));
    app.outside(disconnect.0, disconnect.1.clone())
        .await
        .unwrap();
    let before = app.own().await;
    let known = app.services.state.reports().len();
    let mut opened = 1;
    app.opened().await;
    let mut stream = 0;
    for (_, key) in app.served_panes() {
        stream += 1;
        app.attach(&key, stream).await.unwrap();
    }

    // Every report a real server made, with each thing in it changed.
    let mut said = 0usize;
    for (name, payload) in hostile::reports() {
        for (_, other) in hostile::changed(&payload) {
            server.tell(Tell::Event(name.clone(), other));
            said += 1;
            if said.is_multiple_of(400) {
                let _ = app.backend().round_trip().await;
            }
        }
    }
    assert!(said > 500, "{said}");
    let _ = app.backend().round_trip().await;

    opened += 1;
    app.opened_again(&server).await;

    // Layouts with each thing in them changed, a few of each, the same every time.
    let shown = World::recorded().views[SESSION].clone();
    let mut numbers = Numbers(7);
    let others = hostile::changed(&shown);
    for nth in 0..150u64 {
        let (_, other) = others[numbers.below(others.len())].clone();
        // One that leaves nothing in the session gives way to the own workspace.
        if app.own_on_show() {
            opened += 1;
            app.opened_again(&server).await;
        }
        // Each is read before the next is there.
        let read = server.views_read();
        server.show(other, 50_000 + nth);
        for _ in 0..20 {
            if server.views_read() > read {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
    }
    let _ = app.backend().round_trip().await;
    assert!(server.views_read() > 100, "{}", server.views_read());

    opened += 1;
    app.opened_again(&server).await;

    // More than anybody could take of everything a terminal reports.
    server.show(World::recorded().views[SESSION].clone(), 90_000);
    until("its two terminals are shown", || {
        app.served_panes().len() == 2
    })
    .await;
    for (_, key) in app.served_panes() {
        stream += 1;
        app.attach(&key, stream).await.unwrap();
    }
    let name = block();
    for nth in 0..2_000 {
        let label = NAMES[nth % NAMES.len()];
        server.of_block(&name, "title_changed", json!({ "title": label }));
        server.of_block(&name, "bell", json!({}));
        let status = json!({ "state": "blocked", "kind": "auth", "title": label, "msg": label });
        server.of_block(
            &name,
            "program_status_changed",
            json!({ "id": format!("s{nth}"), "record": status }),
        );
        server.called(label);
        if nth % 100 == 0 {
            let _ = app.backend().round_trip().await;
        }
    }
    let _ = app.backend().round_trip().await;

    assert!(app.services.state.reports().len() > known);

    // And it goes, with a word, and is there again.
    let met = server.connections();
    server.tell(Tell::Close(
        1001,
        "confirmation_required\u{202e}".to_owned(),
    ));
    until("it is connected again", || {
        server.connections() > met && app.kind() == "connected"
    })
    .await;

    // Nobody is told more than that something happened, once for each time it did.
    app.told_nothing_amiss();
    assert!(
        app.told("notice").len() <= 2 * opened,
        "{:?}",
        app.told("notice")
    );

    app.outside(disconnect.0, disconnect.1).await.unwrap();
    assert!(app.own_on_show());
    until("nothing of the server is known", || {
        app.services.state.reports().len() == known
    })
    .await;
    sleep(Duration::from_millis(200)).await;
    assert_eq!(app.own().await, before);
    assert_eq!(app.kind(), "idle");
    assert!(timeout(SOON, app.services.state.list_all()).await.is_ok());
    app.stop().await;
}

/// What is the user's own, but for what plugins keep, which their runs add to.
async fn own_but_plugins(app: &Running) -> Own {
    let mut own = app.own().await;
    own.files.retain(|path, _| !path.contains("plugins"));
    own
}

/// A folder of the server's machine, or none: never something of this machine's.
fn sound_place(place: Option<lume_plugin_api::types::Place>) {
    let Some(folder) = place.and_then(|place| place.folder) else {
        return;
    };
    let drive = folder.starts_with(|c: char| c.is_ascii_alphabetic())
        && matches!(folder.get(1..3), Some(":\\" | ":/"));
    assert!(
        drive && is_plain(&folder) && folder.len() <= 1024,
        "{folder:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whatever_a_server_says_of_folders_and_runs_nothing_is_started_or_changed_here() {
    use crate::plugins::elsewhere_tests::{allow_on, entry, git, install, run, told};

    let played = Programs::new(|job| Played::says(format!("ran in {}", job.told["folder"])));
    // Where its terminals are said to be, one after another.
    let here = std::env::temp_dir().join("k7q-here");
    let mut folders: Vec<String> = vec![
        here.to_string_lossy().into_owned(),
        file_url_of(&here.to_string_lossy()).unwrap(),
        "/home/someone".to_owned(),
        r"\\elsewhere\share".to_owned(),
        r"\\?\C:\x".to_owned(),
        "C:".to_owned(),
        "${home}".to_owned(),
        "x".repeat(5_000),
    ];
    folders.extend(NAMES.iter().map(|name| format!(r"C:\{name}")));
    let said = Arc::new(Mutex::new(folders[0].clone()));
    let mut world = World::recorded().running(&played);
    let saying = Arc::clone(&said);
    world.twist = Some(Arc::new(move |method, mut answer| {
        if method == "com.superlogical.terminal.process" {
            let folder = saying.lock().unwrap().clone();
            answer["foreground"]["cwd"] = json!(folder);
            answer["child"]["cwd"] = json!(folder);
        }
        Some(answer)
    }));
    let server = Hostile::start(world).await;
    let app = Running::plain("rex-hostile-runs", &server).await;
    app.own_shells().await;
    app.services.plugins.start(&app.services);
    install(&app.services, app.dir.path()).await;
    app.opened().await;
    let machine = format!("rex:http://{}", server.address);
    allow_on(&app.services, &machine, &["git"]);
    let key = app.served_panes()[0].1.clone();
    let before = own_but_plugins(&app).await;

    for folder in &folders {
        *said.lock().unwrap() = folder.clone();
        let entry = entry(&app.services, &key).await;
        assert_eq!(entry.cwd, None, "{folder:?}");
        let place = entry.place.clone();
        sound_place(place.clone());
        // What the place says is asked of the server, and of nothing here.
        let mut call = git(&["status"], Some(&machine));
        if let lume_plugin_api::Call::ProcessRun(asked) = &mut call {
            asked.options.cwd = place.and_then(|place| place.folder);
        }
        let answer = run(&app.services, call).await.unwrap();
        assert!(
            answer["stdout"].as_str().unwrap().starts_with("ran in"),
            "{answer}"
        );
    }

    // Its terminal moves more often than anybody could follow.
    let name = block();
    let mut placed = 0;
    for nth in 0..2_000 {
        let folder = &folders[nth % folders.len()];
        let payload = json!({ "foreground": { "pid": 1000, "name": "cmd.exe", "cwd": folder } });
        server.of_block(&name, "process_changed", payload);
        let printed = json!({ "pwd": folder, "owner": { "pid": 1000, "name": "cmd.exe" } });
        server.of_block(&name, "pwd_changed", printed);
        if nth % 100 == 0 {
            let _ = app.backend().round_trip().await;
            let foreground = lume_proto::SessionEvent::Foreground(lume_proto::Foreground {
                pid: 1000,
                name: "cmd.exe".to_owned(),
            });
            let reports = told(&app.services, &key, foreground);
            let places: Vec<_> = reports.iter().filter(|(kind, _)| kind == "place").collect();
            assert!(places.len() <= 1, "{reports:?}");
            placed += places.len();
            for (_, place) in places {
                sound_place(serde_json::from_value(place.clone()).unwrap());
            }
        }
    }
    assert!(placed > 0);

    // It asks this client for runs that nobody made, as other clients and as their script.
    let secret = played.seen(|seen| seen.secret(0));
    let mut asked = 0;
    for client in [SCRIPT, "client:other", ""] {
        for action in [
            "lume.run.job",
            "lume.run.out",
            "lume.run.done",
            "lume.run.in",
            "other",
        ] {
            for secret in [secret.as_str(), "0".repeat(32).as_str(), ""] {
                let args = json!({ "secret": secret, "to": "out", "data": "x", "code": 0 });
                let call_id = format!("k7q-{asked}");
                let perform = programs::request(&call_id, client, action, args);
                server.tell(Tell::Event("client_perform_requested".to_owned(), perform));
                asked += 1;
            }
        }
    }
    for nth in 0..3_000 {
        let perform = programs::request(
            &format!("k7q-flood-{nth}"),
            SCRIPT,
            "lume.run.job",
            json!({}),
        );
        server.tell(Tell::Event("client_perform_requested".to_owned(), perform));
        if nth % 300 == 0 {
            let _ = app.backend().round_trip().await;
        }
    }
    assert!(
        timeout(SOON, app.backend().round_trip())
            .await
            .unwrap()
            .is_ok()
    );
    assert!(asked > 30);
    // And what is run still runs, there.
    *said.lock().unwrap() = r"C:\Users\me".to_owned();
    let mut call = git(&["status"], Some(&machine));
    if let lume_plugin_api::Call::ProcessRun(asked) = &mut call {
        asked.options.cwd = Some(r"C:\Users\me".to_owned());
    }
    let answer = run(&app.services, call).await.unwrap();
    assert_eq!(answer["code"], 0, "{answer}");

    let after = own_but_plugins(&app).await;
    assert_eq!(after, before);
    assert!(!after.files.values().any(|text| text.contains("k7q")));
    app.told_nothing_amiss();
    assert_eq!(app.told("notice"), Vec::<Value>::new());
    app.stop().await;
}
