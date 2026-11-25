//! A server that means harm, or is broken: whatever it says, the client goes on, keeps
//! within its bounds, and hands on nothing but what can be shown.

mod common;

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

use common::hostile::{self, CODES, NAMES, Numbers, OWN_CODES, is_plain};
use common::programs::{self, Played, Programs, SCRIPT};
use common::views::{layer, leaf, tiled, view, window};
use common::{
    DataSide, Listener, NONCE, Recorded, SESSION, Served, Tell, World, client, note_bytes, serve,
    streams, until,
};
use lume_proto::{
    CellPx, ClientFacts, ColorScheme, ErrorCode, Palette, SessionEvent, SessionId, TermSize,
};
use lume_rex::tuning::{
    CELLS_MAX, DESCRIBE_WAIT, REPORTS_PER_WINDOW_MAX, RUN_HOLD_BYTES, RUN_PART_BYTES, SESSIONS_MAX,
};
use lume_rex::{Link, Ran, RexBackend, RexError, RunAsked, RunError, RunLimits};
use lume_session::{Attachment, Hydrate, OutputFrame, SessionBackend, SessionError};
use serde_json::{Value, json};
use tokio::time::timeout;

/// How long anything may take that is not told to wait.
const SOON: Duration = Duration::from_secs(20);

/// How many times the client panicked, on whatever thread or task: a task that does
/// takes nothing else with it, and would not be missed.
static PANICS: AtomicUsize = AtomicUsize::new(0);

fn count_panics() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let usual = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let tests = info
                .location()
                .is_some_and(|place| place.file().contains("tests"));
            PANICS.fetch_add(usize::from(!tests), Ordering::SeqCst);
            usual(info);
        }));
    });
}

fn nothing_panicked() {
    assert_eq!(PANICS.load(Ordering::SeqCst), 0, "something panicked");
}

fn facts() -> ClientFacts {
    ClientFacts {
        size: TermSize::new(80, 24).unwrap(),
        cell_px: CellPx { w: 8, h: 16 },
        palette: Palette::default(),
        scheme: ColorScheme::Dark,
        focused: true,
        visible: true,
    }
}

fn copy_of(text: &str) -> Vec<u8> {
    let mut terminal = lume_vt::Terminal::new(80, 24).unwrap();
    terminal.write(text.as_bytes());
    terminal.snapshot().unwrap()
}

struct Setup {
    backend: Arc<RexBackend>,
    served: Served,
    listener: Listener,
}

async fn setup(world: World) -> Setup {
    count_panics();
    let (dialer, mut listener) = streams();
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
    let served = serve(listener.accept().await, world);
    timeout(SOON, backend.ready()).await.unwrap().unwrap();
    Setup {
        backend,
        served,
        listener,
    }
}

impl Setup {
    async fn opened(world: World) -> Self {
        let setup = setup(world).await;
        setup.backend.open_session(SESSION).await.unwrap();
        setup
    }

    async fn first(&self) -> SessionId {
        self.backend.list().await.unwrap()[0].id
    }

    /// Attaches to a terminal whose stream begins as a server's does.
    async fn attach(&mut self, id: SessionId) -> (Attachment, DataSide) {
        let backend = Arc::clone(&self.backend);
        let attaching = tokio::spawn(async move { backend.attach(id, facts()).await });
        let mut side = DataSide::accept(self.listener.stream().await).await;
        side.bytes(&copy_of("as it stood")).await;
        side.size(80, 24).await;
        (attaching.await.unwrap().unwrap(), side)
    }
}

/// The blocks of the recorded session's first window.
fn block(nth: usize) -> String {
    let view = World::recorded().views[SESSION].clone();
    view["windows"][0]["layers"][0]["blocks"][nth]["block_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn block_event(name: &str, payload: Value) -> Tell {
    Tell::SessionEvent(
        SESSION.to_owned(),
        "block_event".to_owned(),
        json!({
            "block_id": block(0), "creator_name": "com.superlogical.terminal",
            "name": name, "payload": payload,
        }),
    )
}

#[tokio::test]
async fn what_a_server_calls_things_is_made_plain_wherever_it_is_handed_on() {
    for name in NAMES {
        let mut world = World::recorded();
        world.sessions[0].1 = name.to_owned();
        let shown = world.views.get_mut(SESSION).unwrap();
        shown["label"] = json!(name);
        shown["windows"][0]["label"] = json!(name);
        shown["windows"][0]["layers"][0]["blocks"][0]["label"] = json!(name);
        // Beside the terminals, something whose kind is a name as well.
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
        let mut setup = setup(world).await;
        let Link::Connected(version) = setup.backend.link().borrow().clone() else {
            panic!("not connected");
        };
        assert!(is_plain(&version.rex_version) && is_plain(&version.platform));
        assert!(is_plain(&setup.backend.sessions()[0].label), "{name:?}");

        setup.backend.show_session(SESSION).await.unwrap();
        let layout = setup.backend.layout().unwrap().view().await.unwrap();
        // Everything a person is shown of a layout is in what it is written as.
        fn texts(value: &Value, found: &mut Vec<String>) {
            match value {
                Value::String(text) => found.push(text.clone()),
                Value::Array(items) => items.iter().for_each(|item| texts(item, found)),
                Value::Object(fields) => fields.values().for_each(|field| texts(field, found)),
                _ => {}
            }
        }
        let mut found = Vec::new();
        texts(
            &serde_json::to_value(&layout.workspace).unwrap(),
            &mut found,
        );
        assert!(found.iter().all(|text| is_plain(text)), "{found:?}");

        let listed = setup.backend.list().await.unwrap();
        for info in &listed {
            assert!(is_plain(&info.program) && info.title.as_deref().is_none_or(is_plain));
            // Nothing of this machine is named by a server that is not on it.
            assert_eq!((info.pid, info.cwd.as_deref()), (None, None));
        }

        let (mut attachment, _side) = setup.attach(listed[0].id).await;
        setup
            .served
            .tell(block_event("title_changed", json!({ "title": name })));
        setup.served.tell(block_event(
            "program_status_changed",
            json!({ "id": "a", "record": { "state": "working", "app": name, "title": name, "msg": name } }),
        ));
        setup.served.tell(block_event("bell", json!({})));
        loop {
            match timeout(SOON, attachment.output.next()).await.unwrap() {
                Some(OutputFrame::Event(SessionEvent::Bell)) => break,
                Some(OutputFrame::Event(SessionEvent::Title { title })) => {
                    assert!(is_plain(&title));
                }
                Some(OutputFrame::Event(SessionEvent::ProgramStatus(status))) => {
                    for text in [status.app, status.title, status.message] {
                        assert!(text.as_deref().is_none_or(is_plain));
                    }
                }
                Some(_) => {}
                None => panic!("the attachment ended"),
            }
        }
    }
    nothing_panicked();
}

#[tokio::test]
async fn a_folder_of_this_machines_own_server_is_one_that_can_be_gone_to() {
    count_panics();
    for (named, given) in [
        ("/home/user", Some("file:///home/user")),
        (r"C:\Users\me", Some("file:///C:/Users/me")),
        (r"\\elsewhere\share", None),
        ("//elsewhere/share", None),
        ("relative", None),
        ("/home/\u{202e}user", None),
        ("/home/us\ner", None),
        ("", None),
    ] {
        let (mut dialer, mut listener) = streams();
        dialer.here = true;
        let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
        let mut world = World::recorded();
        world.twist = Some(Arc::new(move |method, mut answer| {
            if method == "com.superlogical.terminal.process" {
                answer["foreground"]["cwd"] = json!(named);
                answer["child"]["cwd"] = json!(named);
            }
            Some(answer)
        }));
        let _served = serve(listener.accept().await, world);
        backend.ready().await.unwrap();
        backend.open_session(SESSION).await.unwrap();
        assert_eq!(backend.list().await.unwrap()[0].cwd.as_deref(), given);
    }
    nothing_panicked();
}

#[tokio::test]
async fn a_failure_goes_by_this_clients_own_name_whatever_the_server_calls_it() {
    for code in CODES {
        let mut world = World::recorded().changing();
        world.refuse = Some((code.to_owned(), 418, code.to_owned()));
        let setup = Setup::opened(world).await;
        let refused = setup
            .backend
            .rename_session("session:none", "x")
            .await
            .unwrap_err();
        assert_eq!(refused.code(), "not_found");
        let refused = setup.backend.kill(setup.first().await).await.unwrap_err();
        let SessionError::Refused(refused) = refused else {
            panic!("{refused:?}");
        };
        assert_eq!(refused.code, ErrorCode::Internal);
        assert!(is_plain(&refused.message), "{:?}", refused.message);
    }
    for status in 0..=u16::MAX {
        let failed = RexError::Call {
            method: "m".to_owned(),
            code: "confirmation_required".to_owned(),
            message: String::new(),
            status,
        };
        assert!(OWN_CODES.contains(&failed.code()), "{status}");
    }
    nothing_panicked();
}

/// What a backend does with a server for as long as it is used: none of it may hang, and
/// what comes of it is not looked at.
async fn use_it(world: World, data: Vec<u8>) {
    let (dialer, mut listener) = streams();
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
    let serving = tokio::spawn(async move {
        let mut served = Vec::new();
        let mut streams = Vec::new();
        loop {
            let stream = listener.stream().await;
            let data = data.clone();
            let Some((control, stream)) = common::asks_for_control(stream).await else {
                continue;
            };
            if control {
                let conn = common::Conn::accept(stream, Recorded::load().welcome()).await;
                served.push(serve(conn, world.clone()));
            } else {
                streams.push(tokio::spawn(async move {
                    let mut side = DataSide::accept(stream).await;
                    side.bytes(&data).await;
                    side.closed_by_client().await;
                }));
            }
        }
    });
    let using = async {
        // A server whose version cannot be read is tried again: that much is enough.
        if timeout(Duration::from_millis(400), backend.ready())
            .await
            .is_err()
        {
            return;
        }
        let _ = backend.open_session(SESSION).await;
        let _ = backend.show_session(SESSION).await;
        let _ = backend.layout().unwrap().view().await;
        if let Some(info) = backend.list().await.unwrap_or_default().first() {
            let attached =
                timeout(Duration::from_millis(400), backend.attach(info.id, facts())).await;
            if let Ok(Ok(mut attachment)) = attached {
                let _ = attachment.input.credit(100_000).await;
                let _ = timeout(Duration::from_millis(50), attachment.output.next()).await;
                let _ = attachment.input.detach().await;
            }
            let _ = backend.screen(info.id).await;
            let _ = backend.send(info.id, "x").await;
        }
        let _ = backend.round_trip().await;
        let _ = backend.close_session(SESSION).await;
    };
    let within = timeout(SOON, using).await;
    serving.abort();
    backend.shutdown();
    assert!(within.is_ok(), "the client waits for ever");
}

/// Uses each of these servers, many at once. Says which one if something goes wrong.
async fn use_each(worlds: Vec<(String, World, Vec<u8>)>) {
    use futures_util::StreamExt;
    let before = PANICS.load(Ordering::SeqCst);
    let mut using = futures_util::stream::iter(worlds)
        .map(|(named, world, data)| async move {
            let used = tokio::spawn(use_it(world, data)).await;
            (named, used.is_ok())
        })
        .buffer_unordered(256);
    while let Some((named, whole)) = using.next().await {
        assert!(
            whole && PANICS.load(Ordering::SeqCst) == before,
            "at or beside {named}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whatever_is_changed_in_what_a_server_answers_the_client_goes_on() {
    count_panics();
    let recorded = Recorded::load();
    let begun: Vec<u8> = [
        copy_of("x"),
        note_bytes(NONCE, &json!({ "type": "resize", "cols": 80, "rows": 24 })),
    ]
    .concat();
    let shown = World::recorded().views[SESSION].clone();
    let answers = [
        ("server.version", recorded.result("server.version", 0)),
        (
            "session.attach_all",
            json!({ "sessions": [{ "session_id": SESSION, "label": "a" }] }),
        ),
        (
            "session.attach",
            json!({ "client_id": "client:test", "view": shown }),
        ),
        (
            "com.superlogical.terminal.size",
            recorded.result("com.superlogical.terminal.size", 0),
        ),
        (
            "com.superlogical.terminal.process",
            recorded.result("com.superlogical.terminal.process", 0),
        ),
        ("com.superlogical.terminal.title", json!({ "title": "a" })),
        (
            "com.superlogical.terminal.pwd",
            json!({ "pwd": "file:///home" }),
        ),
        (
            "com.superlogical.terminal.program_status",
            json!({ "records": [{ "id": "a/b", "state": "blocked", "kind": "auth", "progress": 3, "app": "a", "title": "t", "msg": "m" }] }),
        ),
        (
            "com.superlogical.terminal.modes",
            json!({ "screen": "primary", "modes": [] }),
        ),
        (
            "session.own_block",
            json!({ "blocks": [{ "block_id": block(0), "owner": "client:test" }] }),
        ),
        (
            "data.connect",
            json!({ "ticket": common::TICKET, "expires_in_ms": 60_000 }),
        ),
        (
            "com.superlogical.terminal.format",
            json!({ "content": "screen", "id": 7 }),
        ),
        (
            "com.superlogical.terminal.blob_read",
            json!({ "id": 7, "data": "more", "total": 10, "eof": true }),
        ),
    ];
    let mut worlds = Vec::new();
    for (asked, answer) in answers {
        for (place, other) in hostile::changed(&answer) {
            let mut world = World::recorded();
            world.twist = Some(Arc::new(move |method, usual| {
                Some(if method == asked {
                    other.clone()
                } else {
                    usual
                })
            }));
            worlds.push((format!("{asked} {place}"), world, begun.clone()));
        }
    }
    assert!(worlds.len() > 500, "{}", worlds.len());
    use_each(worlds).await;
    nothing_panicked();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whatever_is_changed_in_what_a_server_reports_the_client_goes_on() {
    let mut setup = Setup::opened(World::recorded()).await;
    setup.backend.show_session(SESSION).await.unwrap();
    let id = setup.first().await;
    let (attachment, _side) = setup.attach(id).await;
    let mut told = 0usize;
    for (name, payload) in hostile::reports() {
        for (_, other) in hostile::changed(&payload) {
            setup.served.tell(Tell::Event(name.clone(), other));
            told += 1;
            // Not more at once than the client has room for: that ends a connection, as
            // it is meant to.
            if told.is_multiple_of(500) {
                let _ = setup.backend.round_trip().await;
            }
        }
    }
    assert!(told > 500, "{told}");
    assert!(
        timeout(SOON, setup.backend.round_trip())
            .await
            .unwrap()
            .is_ok()
    );
    assert!(matches!(
        &*setup.backend.link().borrow(),
        Link::Connected(_)
    ));
    // What came of it is within bounds, and the terminal is still the terminal.
    assert!(setup.backend.sessions().len() <= SESSIONS_MAX);
    attachment.input.credit(1000).await.unwrap();
    nothing_panicked();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whatever_a_stream_holds_the_client_goes_on() {
    count_panics();
    let mut worlds = Vec::new();
    let size = |json: Value| note_bytes(NONCE, &json);
    let usual = size(json!({ "type": "resize", "cols": 80, "rows": 24 }));
    for (place, note) in hostile::changed(
        &json!({ "type": "resize", "cols": 80, "rows": 24, "cell_width_px": 8, "cell_height_px": 16, "owner": "c" }),
    )
    .into_iter()
    .chain(hostile::changed(
        &json!({ "type": "theme", "foreground": "#ffffff", "palette": ["1=#000000"], "scheme": "dark" }),
    ))
    .chain(hostile::changed(&json!({ "type": "child_exited", "exit_code": 3 })))
    {
        let data = [copy_of("x"), usual.clone(), size(note)].concat();
        worlds.push((place, World::recorded(), data));
    }
    for seed in 0..200 {
        let mut numbers = Numbers(seed);
        let mut data = copy_of("x");
        let at = numbers.below(data.len());
        let count = numbers.below(64);
        match numbers.below(4) {
            0 => data.truncate(at),
            1 => data.splice(at..at, numbers.bytes(count)).for_each(drop),
            2 => data = numbers.bytes(2000),
            _ => data[at] ^= 0x55,
        }
        data.extend(&usual);
        data.extend(numbers.bytes(4000));
        data.extend(&usual[..numbers.below(usual.len())]);
        worlds.push((format!("stream {seed}"), World::recorded(), data));
    }
    use_each(worlds).await;
    nothing_panicked();
}

#[tokio::test]
async fn blocks_that_no_session_has_any_more_are_forgotten() {
    let setup = Setup::opened(World::recorded()).await;
    let first = setup.first().await;
    let mut seen = HashSet::from([first]);
    let mut world = World::recorded();
    for round in 0..12u64 {
        let windows: Vec<Value> = (0..15)
            .map(|nth| {
                tiled(
                    &format!("window:{round}:{nth}"),
                    leaf(&format!("block:{round}:{nth}")),
                )
            })
            .collect();
        let mut shown = view(windows);
        shown["revision"] = json!(1_000 + round);
        world.views.insert(SESSION.to_owned(), shown);
        setup.served.tell(Tell::World(world.clone()));
        setup.served.tell(Tell::SessionEvent(
            SESSION.to_owned(),
            "session_view_changed".to_owned(),
            json!({ "revision": 1_000 + round }),
        ));
        until("the view is read", || {
            setup
                .backend
                .view(SESSION)
                .is_some_and(|view| view.revision == 1_000 + round)
        })
        .await;
        let listed = setup.backend.list().await.unwrap();
        assert_eq!(listed.len(), 15);
        for info in &listed {
            // A number is never another block's.
            assert!(seen.insert(info.id));
        }
    }
    let now: HashSet<SessionId> = setup
        .backend
        .list()
        .await
        .unwrap()
        .iter()
        .map(|info| info.id)
        .collect();
    for id in seen.difference(&now) {
        let Err(SessionError::Refused(refused)) = setup.backend.kill(*id).await else {
            panic!("{id} is still known");
        };
        assert_eq!(refused.code, ErrorCode::NotFound);
    }
    nothing_panicked();
}

#[tokio::test]
async fn a_block_that_is_shown_keeps_its_number_while_no_view_names_it() {
    let mut setup = Setup::opened(World::recorded()).await;
    let id = setup.first().await;
    let (attachment, _side) = setup.attach(id).await;
    let mut world = World::recorded();
    let mut gone = view(vec![]);
    gone["revision"] = json!(1_000);
    world.views.insert(SESSION.to_owned(), gone);
    setup.served.tell(Tell::World(world));
    setup.served.tell(Tell::SessionEvent(
        SESSION.to_owned(),
        "session_view_changed".to_owned(),
        json!({ "revision": 1_000 }),
    ));
    until("the view is read", || {
        setup
            .backend
            .view(SESSION)
            .is_some_and(|view| view.revision == 1_000)
    })
    .await;
    // It is still this block to whoever holds the number.
    assert!(setup.backend.send(id, "x").await.is_ok());
    attachment.input.detach().await.unwrap();
    // The feed lets go of it a moment after the last view has.
    let started = Instant::now();
    while setup.backend.send(id, "x").await.is_ok() {
        assert!(started.elapsed() < SOON, "it is never let go of");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    nothing_panicked();
}

#[tokio::test]
async fn terminals_that_do_not_say_what_they_are_hold_up_nobody_and_are_still_there() {
    let mut world = World::recorded();
    let windows: Vec<Value> = (0..60)
        .map(|nth| {
            window(
                &format!("window:{nth}"),
                vec![layer(
                    &format!("layer:{nth}"),
                    "tiled",
                    leaf(&format!("block:{nth}")),
                )],
            )
        })
        .collect();
    world.views.insert(SESSION.to_owned(), view(windows));
    world.twist = Some(Arc::new(|method, answer| {
        (method != "com.superlogical.terminal.process").then_some(answer)
    }));
    let setup = Setup::opened(world).await;
    let started = Instant::now();
    let listed = setup.backend.list().await.unwrap();
    assert!(started.elapsed() < DESCRIBE_WAIT + Duration::from_secs(3));
    // A terminal that is slow has not ended.
    assert_eq!(listed.len(), 60);
    assert!(listed.iter().all(|info| info.exit.is_none()));
    nothing_panicked();
}

#[tokio::test]
async fn reports_and_sizes_beyond_what_is_passed_on_are_told_as_what_they_add_up_to() {
    let mut setup = Setup::opened(World::recorded()).await;
    let id = setup.first().await;
    let (mut attachment, mut side) = setup.attach(id).await;
    let many = 3_000;
    for nth in 1..=many {
        setup.served.tell(block_event(
            "title_changed",
            json!({ "title": format!("title {nth}") }),
        ));
    }
    for nth in 1..=many {
        side.size(100 + (nth % 200) as u16, 30).await;
    }
    side.size(91, 33).await;
    let (mut frames, mut title, mut size) = (0u32, String::new(), None);
    let deadline = Instant::now() + SOON;
    while title != format!("title {many}") || size != TermSize::new(91, 33).ok() {
        assert!(Instant::now() < deadline, "{title} {size:?}");
        match timeout(Duration::from_secs(2), attachment.output.next()).await {
            Ok(Some(OutputFrame::Event(SessionEvent::Title { title: now }))) => title = now,
            Ok(Some(OutputFrame::Event(SessionEvent::State { state }))) => {
                title = state.title.unwrap_or_default();
            }
            Ok(Some(OutputFrame::Resized(now))) => size = Some(now),
            Ok(Some(_)) => {}
            Ok(None) => panic!("the attachment ended"),
            Err(_) => continue,
        }
        frames += 1;
    }
    // A handful of windows at most: nothing like what was sent.
    assert!(
        frames < 6 * REPORTS_PER_WINDOW_MAX,
        "{frames} of {}",
        2 * many
    );
    nothing_panicked();
}

#[tokio::test]
async fn a_server_that_keeps_ending_a_stream_is_gone_along_with_ever_more_slowly() {
    let mut setup = Setup::opened(World::recorded()).await;
    let id = setup.first().await;
    let (mut attachment, side) = setup.attach(id).await;
    let _ = attachment.input.credit(1_000_000).await;
    let began = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&began);
    let mut listener = setup.listener;
    let serving = tokio::spawn(async move {
        side.close(1013, "").await;
        loop {
            let mut side = DataSide::accept(listener.stream().await).await;
            counted.fetch_add(1, Ordering::SeqCst);
            side.bytes(&copy_of("again")).await;
            side.size(80, 24).await;
            side.close(1013, "").await;
        }
    });
    tokio::time::sleep(Duration::from_secs(4)).await;
    let began = began.load(Ordering::SeqCst);
    assert!((2..=8).contains(&began), "{began} in four seconds");
    // The view was started again as often, and goes on.
    let mut copies = 0;
    while let Ok(Some(frame)) = timeout(Duration::from_millis(50), attachment.output.next()).await {
        copies += usize::from(matches!(frame, OutputFrame::Hydrate(_)));
    }
    assert!((1..=began).contains(&copies));
    serving.abort();
    nothing_panicked();
}

#[tokio::test]
async fn a_size_is_never_more_than_is_shown() {
    let mut setup = Setup::opened(World::recorded()).await;
    let id = setup.first().await;
    let (mut attachment, mut side) = setup.attach(id).await;
    for (cols, rows) in [(u16::MAX, u16::MAX), (4096, 200), (u16::MAX, 1)] {
        side.size(cols, rows).await;
        loop {
            match timeout(SOON, attachment.output.next()).await.unwrap() {
                Some(OutputFrame::Resized(size)) => {
                    assert!(u32::from(size.cols()) * u32::from(size.rows()) <= CELLS_MAX);
                    break;
                }
                Some(_) => {}
                None => panic!("the attachment ended"),
            }
        }
    }
    nothing_panicked();
}

/// A copy of a terminal in which the first page asks for a gigabyte of room for text. It
/// is sound in every other way, its checksum too.
fn greedy() -> Vec<u8> {
    let mut copy = copy_of("greedy");
    let length =
        |copy: &[u8], at: usize| u32::from_le_bytes(copy[at + 2..at + 6].try_into().unwrap());
    let mut at = 10;
    while copy[at] != 3 {
        at += 10 + length(&copy, at) as usize;
    }
    let held = at + 10;
    copy[held + 16..held + 20].copy_from_slice(&(1u32 << 30).to_le_bytes());
    let mut sum = u32::MAX;
    let end = held + length(&copy, at) as usize;
    for byte in copy[at..at + 6].iter().chain(&copy[held..end]) {
        sum ^= u32::from(*byte);
        for _ in 0..8 {
            sum = (sum >> 1) ^ (0x82f6_3b78 & (sum & 1).wrapping_neg());
        }
    }
    copy[at + 6..held].copy_from_slice(&(!sum).to_le_bytes());
    // It can be loaded, and that is the harm of it.
    assert!(lume_vt::Terminal::from_snapshot(&copy).is_ok());
    copy
}

#[tokio::test]
async fn a_copy_that_asks_for_too_much_gives_way_to_the_screen_written_out() {
    let mut world = World::recorded();
    world.screen = vec!["written out".to_owned()];
    let mut setup = Setup::opened(world).await;
    let id = setup.first().await;
    let backend = Arc::clone(&setup.backend);
    let attaching = tokio::spawn(async move { backend.attach(id, facts()).await });
    let mut side = DataSide::accept(setup.listener.stream().await).await;
    side.bytes(&greedy()).await;
    side.size(80, 24).await;
    let attachment = attaching.await.unwrap().unwrap();
    assert_eq!(
        attachment.hydrate,
        Hydrate::VtReplay(b"written out".to_vec())
    );
    nothing_panicked();
}

/// A server on which programs are run, whose scripts ask only what the test has them ask.
async fn with_runs(programs: &Programs) -> Setup {
    Setup::opened(World::recorded().running(programs)).await
}

fn some_program() -> (RunAsked, RunLimits) {
    let asked = RunAsked {
        program: "git".to_owned(),
        ..RunAsked::default()
    };
    let limits = RunLimits {
        stdin: Vec::new(),
        timeout: SOON,
        max_output: 10,
    };
    (asked, limits)
}

impl Setup {
    fn perform(&self, payload: Value) {
        let event = Tell::Event("client_perform_requested".to_owned(), payload);
        self.served.tell(event);
    }

    /// Has some client ask this one for something, and gives what it is answered.
    async fn asks(&self, call: &str, client: &str, action: &str, args: Value) -> Value {
        let action = format!("lume.run.{action}");
        self.perform(programs::request(call, client, &action, args));
        let replied = || {
            let replies = self.served.asked("client.reply");
            replies.into_iter().find(|reply| reply["call_id"] == call)
        };
        until("it is answered", || replied().is_some()).await;
        replied().unwrap()
    }

    /// Asks for a program that writes at most ten bytes, and gives the run with the secret
    /// of its block.
    async fn begins(
        &self,
        programs: &Programs,
    ) -> (tokio::task::JoinHandle<Result<Ran, RunError>>, String) {
        let before = programs.seen(|seen| seen.blocks.len());
        let backend = Arc::clone(&self.backend);
        let run = tokio::spawn(async move {
            let (asked, limits) = some_program();
            backend.run(&asked, limits).await
        });
        until("a block is made", || {
            programs.seen(|seen| seen.blocks.len()) > before
        })
        .await;
        (run, programs.seen(|seen| seen.secret(before)))
    }
}

/// Programs whose scripts do nothing of their own.
fn unplayed() -> Programs {
    let mut programs = Programs::new(|_| Played::code(0));
    programs.comes = false;
    programs
}

#[tokio::test]
async fn nobody_but_a_runs_own_script_is_told_of_it_or_believed() {
    let programs = unplayed();
    let setup = with_runs(&programs).await;
    let (run, secret) = setup.begins(&programs).await;
    let (other, theirs) = setup.begins(&programs).await;
    let refused = |reply: &Value| reply["error"]["code"] == "refused";
    for (nth, args) in [
        json!({}),
        json!({ "secret": "" }),
        json!({ "secret": null }),
        json!({ "secret": secret.to_uppercase() + "0" }),
        json!({ "secret": &secret[..31] }),
        json!({ "secret": [secret] }),
    ]
    .into_iter()
    .enumerate()
    {
        for action in ["job", "in", "out", "alive", "done"] {
            let call = format!("wrong-{nth}-{action}");
            let reply = setup.asks(&call, "client:one", action, args.clone()).await;
            assert!(refused(&reply), "{action} {args}: {reply}");
        }
    }
    let shown = || json!({ "secret": secret });
    let reply = setup.asks("no-client", "", "job", shown()).await;
    assert!(refused(&reply), "{reply}");

    // The first that shows the secret is the script.
    let reply = setup.asks("job", "client:one", "job", shown()).await;
    assert_eq!(reply["result"]["argv"], json!(["git"]));
    for (nth, action) in ["job", "in", "out", "alive", "done"]
        .into_iter()
        .enumerate()
    {
        let args = json!({ "secret": secret, "code": 9, "to": "out", "data": "lies" });
        let call = format!("other-{nth}");
        let reply = setup.asks(&call, "client:two", action, args).await;
        assert!(refused(&reply), "{action}: {reply}");
    }
    // Nor is one run's script told of another by its own secret.
    let args = json!({ "secret": theirs });
    let reply = setup.asks("cross", "client:three", "job", args).await;
    assert!(reply.get("result").is_some());
    let args = json!({ "secret": secret, "to": "out", "data": "truth" });
    setup.asks("out", "client:one", "out", args).await;
    let args = || json!({ "secret": secret, "code": 1 });
    setup.asks("done", "client:one", "done", args()).await;
    let ran = timeout(SOON, run).await.unwrap().unwrap().unwrap();
    assert_eq!((ran.code, ran.stdout), (Some(1), b"truth".to_vec()));
    let reply = setup.asks("again", "client:one", "done", args()).await;
    assert!(refused(&reply), "{reply}");
    assert!(!other.is_finished());
    other.abort();
    nothing_panicked();
}

#[tokio::test]
async fn a_script_that_does_otherwise_than_it_was_asked_ends_its_run() {
    let programs = unplayed();
    let setup = with_runs(&programs).await;
    let cases = [
        ("job", json!({})),
        ("out", json!({})),
        ("out", json!({ "to": "out" })),
        ("out", json!({ "to": "out", "data": "" })),
        ("out", json!({ "to": "out", "data": 7 })),
        ("out", json!({ "to": "out", "data": ["x"] })),
        ("out", json!({ "to": "out", "data": "\u{100}" })),
        ("out", json!({ "to": "err", "data": "twelve bytes" })),
        (
            "out",
            json!({ "to": "out", "data": "x".repeat(200 * 1024) }),
        ),
        ("out", json!({ "to": "both", "data": "x" })),
        ("out", json!({ "data": "x" })),
        ("began", json!({})),
    ];
    for (nth, (action, args)) in cases.into_iter().enumerate() {
        let (run, secret) = setup.begins(&programs).await;
        let with = |mut args: Value| {
            args["secret"] = json!(secret);
            args
        };
        let call = |what: &str| format!("{nth}-{what}");
        let job = with(json!({}));
        setup.asks(&call("job"), SCRIPT, "job", job).await;
        let bad = with(args.clone());
        let reply = setup.asks(&call("bad"), SCRIPT, action, bad).await;
        assert_eq!(reply["error"]["code"], "refused", "{action}");
        let ended = timeout(SOON, run).await.unwrap().unwrap();
        assert!(matches!(ended, Err(RunError::Failed(_))), "{ended:?}");
        let block = format!("block:run-{nth}");
        until("its block is closed", || {
            programs.seen(|seen| seen.closed.contains(&block))
        })
        .await;
    }
    // Nor is one that says something before it asked what it is to run.
    let (run, secret) = setup.begins(&programs).await;
    let early = json!({ "secret": secret, "code": 0 });
    setup.asks("early", SCRIPT, "done", early).await;
    let ended = timeout(SOON, run).await.unwrap().unwrap();
    assert!(matches!(ended, Err(RunError::Failed(_))), "{ended:?}");
    nothing_panicked();
}

#[tokio::test]
async fn output_handed_on_without_waiting_ends_a_program_that_is_read() {
    let programs = unplayed();
    let setup = with_runs(&programs).await;
    let backend = Arc::clone(&setup.backend);
    let starting = tokio::spawn(async move { backend.start(&some_program().0).await });
    until("a block is made", || {
        programs.seen(|seen| seen.blocks.len()) == 1
    })
    .await;
    let secret = programs.seen(|seen| seen.secret(0));
    let with = |mut args: Value| {
        args["secret"] = json!(secret);
        args
    };
    setup.asks("job", SCRIPT, "job", with(json!({}))).await;
    setup.asks("began", SCRIPT, "began", with(json!({}))).await;
    let started = timeout(SOON, starting).await.unwrap().unwrap().unwrap();
    let mut taken = 0;
    for nth in 0..1_000 {
        let data = "x".repeat(100 * 1024);
        taken += data.len();
        let part = with(json!({ "to": "out", "data": data }));
        let call = format!("out-{nth}");
        let reply = setup.asks(&call, SCRIPT, "out", part).await;
        if reply.get("error").is_some() {
            break;
        }
    }
    assert!(taken <= RUN_HOLD_BYTES + 2 * RUN_PART_BYTES, "{taken}");
    until("it has ended", || started.run.try_wait() == Some(None)).await;
    until("its block is closed", || {
        programs.seen(|seen| seen.closed.len()) == 1
    })
    .await;
    nothing_panicked();
}

#[tokio::test]
async fn whatever_is_changed_in_what_a_script_asks_its_run_ends_and_others_are_run() {
    let programs = unplayed();
    let setup = with_runs(&programs).await;
    let like = |action: &str, secret: &str| {
        let args = match action {
            "out" => json!({ "secret": secret, "to": "out", "data": "hello" }),
            "done" => json!({ "secret": secret, "code": 0, "signal": null, "timed_out": false }),
            _ => json!({ "secret": secret }),
        };
        let mut asked = programs::request("like", SCRIPT, &format!("lume.run.{action}"), args);
        // Nothing is made of what else is said of who asks.
        asked["requester"] = json!({ "client_id": SCRIPT });
        asked
    };
    for action in ["job", "out", "done"] {
        let changes = hostile::changed(&like(action, "secret")).len();
        for nth in 0..changes {
            let (run, secret) = setup.begins(&programs).await;
            if action != "job" {
                setup.perform(like("job", &secret));
            }
            let (what, payload) = hostile::changed(&like(action, &secret)).swap_remove(nth);
            setup.perform(payload);
            // However that was taken, it is over with these.
            setup.perform(like("job", &secret));
            setup.perform(like("done", &secret));
            if what.contains("client_id =") && action == "job" {
                // Another showed the secret first, and is the script.
                run.abort();
                continue;
            }
            let ended = timeout(SOON, run).await;
            let ended = ended.unwrap_or_else(|_| panic!("{action} {what}"));
            if let Ok(ran) = ended.unwrap() {
                assert!(ran.stdout.len() <= 10, "{action} {what}");
            }
        }
    }
    let (run, secret) = setup.begins(&programs).await;
    for action in ["job", "out", "done"] {
        setup.perform(like(action, &secret));
    }
    let ran = timeout(SOON, run).await.unwrap().unwrap().unwrap();
    assert_eq!(ran.stdout, b"hello");
    assert!(matches!(
        &*setup.backend.link().borrow(),
        Link::Connected(_)
    ));
    nothing_panicked();
}

#[tokio::test]
async fn a_flood_of_requests_holds_up_nothing() {
    let programs = Programs::new(|_| Played::says("sound"));
    let setup = with_runs(&programs).await;
    let big = "x".repeat(2_000);
    for nth in 0..3_000 {
        let action = ["lume.run.job", "lume.run.done", "pane.zoom"][nth % 3];
        let args = json!({ "secret": big, "more": big });
        let call = nth.to_string();
        setup.perform(programs::request(&call, "client:x", action, args));
    }
    // Events go on behind it.
    setup.served.tell(Tell::Event(
        "session_created".to_owned(),
        json!({ "session": { "session_id": "session:b", "label": "behind" } }),
    ));
    until("what came behind it is known", || {
        setup.backend.sessions().len() == 2
    })
    .await;
    // Whoever sent it is answered no more than it waited for.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(setup.served.asked("client.reply").len() <= 3_000);
    let (asked, limits) = some_program();
    let ran = timeout(SOON, setup.backend.run(&asked, limits)).await;
    assert_eq!(ran.unwrap().unwrap().stdout, b"sound");
    nothing_panicked();
}

#[tokio::test]
async fn a_server_that_makes_no_block_or_says_nonsense_of_one_runs_nothing() {
    let sound = |method: &str| match method {
        "server.status" => json!({ "listeners": [{ "endpoint": "x", "primary": true }] }),
        _ => json!({ "block_id": "block:1" }),
    };
    for method in ["server.status", "session.new_block"] {
        for code in CODES {
            let mut world = World::recorded().running(&unplayed());
            world.refusing = Some(Arc::new(move |called| {
                (called == method).then(|| (code.to_owned(), 409, "\u{202e}no\n".to_owned()))
            }));
            let setup = Setup::opened(world).await;
            let (asked, limits) = some_program();
            let ended = timeout(Duration::from_secs(5), setup.backend.run(&asked, limits)).await;
            let error = ended.expect("it is waited for").unwrap_err();
            assert!(is_plain(&error.to_string()), "{error:?}");
        }
        for (what, answer) in hostile::changed(&sound(method)) {
            let mut programs = unplayed();
            programs.answers = Some((method.to_owned(), answer));
            let setup = with_runs(&programs).await;
            let (asked, limits) = some_program();
            let running = setup.backend.run(&asked, limits);
            // One that says it made a block goes on to be a run that never comes.
            if let Ok(ended) = timeout(Duration::from_millis(50), running).await {
                let error = ended.unwrap_err();
                assert!(is_plain(&error.to_string()), "{what}: {error:?}");
            }
        }
    }
    nothing_panicked();
}
