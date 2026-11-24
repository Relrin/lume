//! A server that means harm, or is broken: whatever it says, the client goes on, keeps
//! within its bounds, and hands on nothing but what can be shown.

mod common;

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

use common::hostile::{self, CODES, NAMES, Numbers, OWN_CODES, is_plain};
use common::views::{layer, leaf, tiled, view, window};
use common::{
    DataSide, Listener, NONCE, Recorded, SESSION, Served, Tell, World, client, note_bytes, serve,
    streams, until,
};
use lume_proto::{
    CellPx, ClientFacts, ColorScheme, ErrorCode, Palette, SessionEvent, SessionId, TermSize,
};
use lume_rex::tuning::{CELLS_MAX, DESCRIBE_WAIT, REPORTS_PER_WINDOW_MAX, SESSIONS_MAX};
use lume_rex::{Link, RexBackend, RexError};
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
        ("/home/user", Some("/home/user")),
        (r"C:\Users\me", Some(r"C:\Users\me")),
        ("/home/\u{202e}user", None),
        ("/home/us\ner", None),
        ("", Some("")),
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
    let recorded = Recorded::load();
    let mut setup = Setup::opened(World::recorded()).await;
    setup.backend.show_session(SESSION).await.unwrap();
    let id = setup.first().await;
    let (attachment, _side) = setup.attach(id).await;
    let mut events: Vec<(String, Value)> = recorded
        .events()
        .into_iter()
        .map(|(name, payload)| (name.to_owned(), payload.clone()))
        .collect();
    events.push((
        "event_dropped".to_owned(),
        json!({ "event_type": "block_event", "session_id": SESSION, "size": 9 }),
    ));
    // Each kind of event once: the same changes to another of the kind say no more.
    let mut kinds = HashSet::new();
    events.retain(|(name, payload)| {
        kinds.insert(format!(
            "{name} {} {}",
            payload["event"]["type"], payload["event"]["payload"]["name"]
        ))
    });
    let mut told = 0usize;
    for (name, payload) in events {
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
