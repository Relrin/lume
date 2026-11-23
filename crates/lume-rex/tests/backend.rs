//! The backend against a pretend server: what it knows of the server, how it keeps up,
//! and how it comes back after its connection is lost.

mod common;

use std::sync::atomic::Ordering;
use std::time::Duration;

use common::{Conn, Recorded, SESSION, Tell, World, client, serve, streams, until};
use lume_proto::Responder;
use lume_rex::tuning::SESSIONS_MAX;
use lume_rex::{Link, RexBackend, RexSession};
use lume_session::{SessionBackend, SessionError};
use serde_json::{Value, json};

fn names(backend: &RexBackend) -> Vec<String> {
    backend
        .sessions()
        .into_iter()
        .map(|session| session.label)
        .collect()
}

#[tokio::test]
async fn a_backend_connects_and_knows_the_servers_sessions() {
    let (dialer, mut listener) = streams();
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
    assert_eq!(*backend.link().borrow(), Link::Connecting);
    let served = serve(listener.accept().await, World::recorded());

    let version = backend.ready().await.unwrap();
    assert_eq!(version.rex_version, "0.1.0-preview.22");
    assert_eq!(version.platform, "windows_amd64");
    assert_eq!(version.api_revision, 19);
    assert!(matches!(&*backend.link().borrow(), Link::Connected(_)));
    assert_eq!(
        backend.sessions(),
        vec![RexSession {
            id: SESSION.to_owned(),
            label: "lume-fixture".to_owned(),
            opened: false,
        }]
    );
    // It says what it is, and asks for nothing it was not told to.
    assert_eq!(served.hello["client"]["name"], "lume-test");
    assert_eq!(served.called(), ["server.version", "session.attach_all"]);

    let caps = backend.caps();
    assert!(caps.persistent && caps.multi_client && caps.server_layout);
    assert_eq!(caps.responder, Responder::Backend);
    assert!(caps.hydrate.snapshot);
    // With no session opened it has no terminals to list.
    assert_eq!(backend.list().await.unwrap(), vec![]);
}

#[tokio::test]
async fn the_sessions_follow_the_servers_events() {
    let (dialer, mut listener) = streams();
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
    let served = serve(listener.accept().await, World::recorded());
    backend.ready().await.unwrap();
    let mut changed = backend.changed();
    let before = *changed.borrow_and_update();

    let created = |id: &str, label: &str| {
        Tell::Event(
            "session_created".to_owned(),
            json!({ "session": { "session_id": id, "label": label } }),
        )
    };
    served.tell(created("session:b", "second"));
    served.tell(created("session:c", "third"));
    until("the new sessions are known", || {
        backend.sessions().len() == 3
    })
    .await;
    assert_eq!(names(&backend), ["lume-fixture", "second", "third"]);
    assert!(*changed.borrow_and_update() > before);

    // The last goes before the first; the first goes last.
    served.tell(Tell::Event(
        "session_moved".to_owned(),
        json!({ "session_id": "session:c", "before_session_id": SESSION }),
    ));
    served.tell(Tell::Event(
        "session_moved".to_owned(),
        json!({ "session_id": SESSION }),
    ));
    until("the sessions are in their new order", || {
        names(&backend) == ["third", "second", "lume-fixture"]
    })
    .await;

    served.tell(Tell::SessionEvent(
        "session:b".to_owned(),
        "session_label_changed".to_owned(),
        json!({ "label": "renamed" }),
    ));
    served.tell(Tell::Event(
        "session_destroyed".to_owned(),
        json!({ "session_id": "session:c" }),
    ));
    // Events this client does nothing with, or that are not as they should be.
    served.tell(Tell::Event("keymap_changed".to_owned(), json!({})));
    served.tell(Tell::Event(
        "session_created".to_owned(),
        json!({ "session": 4 }),
    ));
    served.tell(Tell::Event(
        "session_moved".to_owned(),
        json!({ "session_id": "session:unknown" }),
    ));
    until("the label and the removal are known", || {
        names(&backend) == ["renamed", "lume-fixture"]
    })
    .await;
    assert!(matches!(&*backend.link().borrow(), Link::Connected(_)));
}

#[tokio::test]
async fn events_the_list_already_holds_are_not_applied_again() {
    let (dialer, mut listener) = streams();
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
    let mut conn = listener.accept().await;
    let recorded = Recorded::load();

    let version = conn.asked().await.unwrap();
    conn.respond(&version.id, recorded.result("server.version", 0))
        .await;
    let attach_all = conn.asked().await.unwrap();
    assert_eq!(attach_all.payload, json!({ "event_scope": "session" }));
    // A session moved before the list was read: the list has it where it now is.
    conn.event(
        "session_moved",
        json!({ "session_id": "session:a", "before_session_id": "session:b" }),
    )
    .await;
    conn.respond(
        &attach_all.id,
        json!({ "sessions": [
            { "session_id": "session:b", "label": "b" },
            { "session_id": "session:a", "label": "a" },
        ] }),
    )
    .await;
    // And one that happened after it.
    conn.event("session_destroyed", json!({ "session_id": "session:b" }))
        .await;

    backend.ready().await.unwrap();
    until("the later event is applied", || {
        backend.sessions().len() == 1
    })
    .await;
    assert_eq!(names(&backend), ["a"]);
    drop(conn);
}

#[tokio::test]
async fn an_opened_sessions_terminals_are_listed_as_a_server_describes_them() {
    let (dialer, mut listener) = streams();
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
    let mut world = World::recorded();
    // Something that is not a terminal, beside the two that are.
    let blocks = world.views.get_mut(SESSION).unwrap()["windows"][0]["layers"][0]["blocks"]
        .as_array_mut()
        .unwrap();
    blocks.push(json!({ "block_id": "block:tunnel", "creator_name": "com.superlogical.tunnel" }));
    let served = serve(listener.accept().await, world);
    backend.ready().await.unwrap();

    let view = backend.open_session(SESSION).await.unwrap();
    assert_eq!(view.windows.len(), 1);
    assert_eq!(view.blocks().count(), 3);
    assert!(backend.sessions()[0].opened);
    assert_eq!(backend.view(SESSION), Some(view));

    let listed = backend.list().await.unwrap();
    assert_eq!(listed.len(), 2);
    for info in &listed {
        assert_eq!((info.size.cols(), info.size.rows()), (80, 24));
        assert_eq!(info.program, "cmd.exe");
        assert_eq!(info.pid, Some(1000));
        assert_eq!(info.cwd.as_deref(), Some("/home/user"));
        assert_eq!(info.exit, None);
        assert_eq!(info.title, None);
    }
    assert_ne!(listed[0].id, listed[1].id);
    // A terminal keeps its number from one listing to the next.
    let again = backend.list().await.unwrap();
    assert_eq!(again, listed);

    backend.close_session(SESSION).await.unwrap();
    assert!(!backend.sessions()[0].opened);
    assert_eq!(backend.list().await.unwrap(), vec![]);
    assert!(served.called().contains(&"session.detach".to_owned()));

    // A session the server does not have cannot be opened.
    let missing = backend.open_session("session:none").await.unwrap_err();
    assert_eq!(missing.code(), "session_not_found");
}

#[tokio::test]
async fn a_view_is_read_again_when_the_server_says_it_changed_and_only_then() {
    let (dialer, mut listener) = streams();
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
    let world = World::recorded();
    let served = serve(listener.accept().await, world.clone());
    backend.ready().await.unwrap();
    let opened = backend.open_session(SESSION).await.unwrap();
    let views = |served: &common::Served| {
        served
            .called()
            .iter()
            .filter(|method| *method == "session.view")
            .count()
    };

    // A revision the view already has, or an older one that arrives late, says nothing
    // new. Nor does a change to a session that is not opened here.
    for (session, revision) in [
        (SESSION, opened.revision),
        (SESSION, 1),
        ("session:other", 99),
    ] {
        served.tell(Tell::SessionEvent(
            session.to_owned(),
            "session_view_changed".to_owned(),
            json!({ "revision": revision }),
        ));
    }
    // A later event is seen to have been taken, so the ones before it were too.
    served.tell(Tell::Event(
        "session_created".to_owned(),
        json!({ "session": { "session_id": "session:marker", "label": "marker" } }),
    ));
    until("the events were taken", || backend.sessions().len() == 2).await;
    assert_eq!(views(&served), 0);

    let mut next = world;
    let view = next.views.get_mut(SESSION).unwrap();
    view["revision"] = json!(opened.revision + 1);
    view["label"] = json!("changed on the server");
    served.tell(Tell::World(next));
    served.tell(Tell::SessionEvent(
        SESSION.to_owned(),
        "session_view_changed".to_owned(),
        json!({ "revision": opened.revision + 1 }),
    ));
    until("the view is read again", || {
        backend
            .view(SESSION)
            .is_some_and(|view| view.label == "changed on the server")
    })
    .await;
    assert_eq!(backend.view(SESSION).unwrap().revision, opened.revision + 1);
    assert_eq!(views(&served), 1);
}

#[tokio::test]
async fn what_an_event_too_large_to_send_would_have_changed_is_read_again() {
    let (dialer, mut listener) = streams();
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
    let world = World::recorded();
    let served = serve(listener.accept().await, world.clone());
    backend.ready().await.unwrap();
    let opened = backend.open_session(SESSION).await.unwrap();

    // The server changes while the events that would have said so are too large to
    // send: a session is renamed and gains a block, another appears.
    let mut next = world;
    next.sessions[0].1 = "renamed unheard".to_owned();
    next.sessions
        .push(("session:new".to_owned(), "new".to_owned()));
    let view = next.views.get_mut(SESSION).unwrap();
    view["revision"] = json!(opened.revision + 3);
    view["label"] = json!("renamed unheard");
    served.tell(Tell::World(next));
    served.tell(Tell::Event(
        "event_dropped".to_owned(),
        json!({ "event_type": "block_event", "session_id": SESSION, "size": 2_000_000 }),
    ));

    until("the sessions and the view are read again", || {
        names(&backend) == ["renamed unheard", "new"]
            && backend
                .view(SESSION)
                .is_some_and(|view| view.revision == opened.revision + 3)
    })
    .await;
    let called = served.called();
    assert!(called.contains(&"session.list".to_owned()), "{called:?}");
    assert!(called.contains(&"session.view".to_owned()), "{called:?}");

    // One that was about the whole server reads the sessions again and no view.
    let views = called
        .iter()
        .filter(|method| *method == "session.view")
        .count();
    let mut last = World::recorded();
    last.sessions.clear();
    served.tell(Tell::World(last));
    served.tell(Tell::Event(
        "event_dropped".to_owned(),
        json!({ "event_type": "hosts_changed", "size": 2_000_000 }),
    ));
    until("the sessions are read again", || {
        backend.sessions().is_empty()
    })
    .await;
    // A session that is no longer listed is no longer opened.
    assert!(backend.view(SESSION).is_none());
    let called = served.called();
    assert_eq!(
        called
            .iter()
            .filter(|method| *method == "session.view")
            .count(),
        views
    );
    assert!(matches!(&*backend.link().borrow(), Link::Connected(_)));
}

#[tokio::test]
async fn a_lost_connection_is_made_again_and_what_was_opened_is_opened_again() {
    let (dialer, mut listener) = streams();
    let dialled = dialer.dialled.clone();
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
    let mut world = World::recorded();
    world
        .sessions
        .push(("session:gone".to_owned(), "gone".to_owned()));
    world.views.insert(
        "session:gone".to_owned(),
        json!({ "revision": 1, "session_id": "session:gone", "windows": [] }),
    );
    let first = serve(listener.accept().await, world);
    backend.ready().await.unwrap();
    backend.open_session(SESSION).await.unwrap();
    backend.open_session("session:gone").await.unwrap();
    assert!(first.hello.get("reconnect_token").is_none());

    let mut link = backend.link();
    link.borrow_and_update();
    first.tell(Tell::Drop);
    first.over().await;
    link.changed().await.unwrap();
    match &*link.borrow_and_update() {
        Link::Waiting { attempts, error } => {
            assert_eq!(*attempts, 1);
            assert!(!error.is_empty());
        }
        other => panic!("{other:?}"),
    }
    // While it is away nothing can be asked.
    assert!(matches!(
        backend.list().await.unwrap_err(),
        SessionError::Disconnected
    ));

    // The server has lost one of the sessions meanwhile, and renamed the other.
    let mut later = World::recorded();
    later.sessions[0].1 = "renamed while away".to_owned();
    let second = serve(listener.accept().await, later);
    backend.ready().await.unwrap();
    assert_eq!(dialled.load(Ordering::Relaxed), 2);
    // The client is continued with what the first welcome gave it.
    assert_eq!(second.hello["reconnect_token"], "TOKEN");
    assert_eq!(names(&backend), ["renamed while away"]);
    until("the sessions are attached again", || {
        second
            .called()
            .iter()
            .filter(|method| *method == "session.attach")
            .count()
            == 2
    })
    .await;
    assert!(backend.view(SESSION).is_some());
    assert!(backend.view("session:gone").is_none());
    assert_eq!(backend.list().await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_server_that_closes_the_connection_is_reached_again() {
    let (dialer, mut listener) = streams();
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
    let first = serve(listener.accept().await, World::recorded());
    backend.ready().await.unwrap();
    let mut link = backend.link();
    link.borrow_and_update();

    first.tell(Tell::Close(1008, "event backlog exceeded".to_owned()));
    link.changed().await.unwrap();
    match &*link.borrow_and_update() {
        Link::Waiting { error, .. } => assert!(error.contains("event backlog exceeded"), "{error}"),
        other => panic!("{other:?}"),
    }
    let _second = serve(listener.accept().await, World::recorded());
    backend.ready().await.unwrap();
}

#[tokio::test]
async fn a_server_that_cannot_be_reached_is_tried_again_until_it_can() {
    let (dialer, mut listener) = streams();
    let down = dialer.down.clone();
    let dialled = dialer.dialled.clone();
    *down.lock().unwrap() = true;
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());

    let mut link = backend.link();
    until("it has been tried more than once", || {
        dialled.load(Ordering::Relaxed) >= 2
    })
    .await;
    match &*link.borrow_and_update() {
        Link::Waiting { attempts, error } => {
            assert!(*attempts >= 1);
            assert!(error.contains("no Rex server could be reached"), "{error}");
        }
        other => panic!("{other:?}"),
    }

    *down.lock().unwrap() = false;
    let _served = serve(listener.accept().await, World::recorded());
    backend.ready().await.unwrap();
    assert_eq!(names(&backend), ["lume-fixture"]);
}

#[tokio::test]
async fn a_server_of_another_revision_is_refused_and_not_tried_again() {
    let (dialer, mut listener) = streams();
    let dialled = dialer.dialled.clone();
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
    let stream = listener.stream().await;
    let mut welcome = Recorded::load().welcome();
    welcome["api_revision"] = json!(20);
    welcome["api_server_revision"] = json!(27);
    welcome["api_min_revision"] = json!(20);
    let _conn = Conn::accept(stream, welcome).await;

    let refused = backend.ready().await.unwrap_err();
    assert!(
        refused.to_string().contains("update this application"),
        "{refused}"
    );
    match &*backend.link().borrow() {
        Link::Refused { code, error } => {
            assert_eq!(code, "version_mismatch");
            assert!(error.contains("20 to 27"), "{error}");
        }
        other => panic!("{other:?}"),
    }
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(dialled.load(Ordering::Relaxed), 1);
    assert!(backend.sessions().is_empty());
}

#[tokio::test]
async fn what_a_server_sends_beyond_all_bounds_is_refused_and_does_no_harm() {
    let (dialer, mut listener) = streams();
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());

    // More sessions than any server holds: the connection is given up and made again.
    let mut conn = listener.accept().await;
    let version = conn.asked().await.unwrap();
    conn.respond(&version.id, Recorded::load().result("server.version", 0))
        .await;
    let attach_all = conn.asked().await.unwrap();
    let many: Vec<Value> = (0..=SESSIONS_MAX)
        .map(|n| json!({ "session_id": format!("session:{n}"), "label": "x" }))
        .collect();
    conn.respond(&attach_all.id, json!({ "sessions": many }))
        .await;
    let mut link = backend.link();
    until("the list is refused", || {
        matches!(&*link.borrow_and_update(), Link::Waiting { .. })
    })
    .await;
    assert!(backend.sessions().is_empty());
    drop(conn);

    // A view that is not a view: the session is not opened, and the backend carries on.
    let mut world = World::recorded();
    world.views.insert(
        SESSION.to_owned(),
        json!({ "revision": "seven", "windows": "none" }),
    );
    let served = serve(listener.accept().await, world.clone());
    backend.ready().await.unwrap();
    let refused = backend.open_session(SESSION).await.unwrap_err();
    assert_eq!(refused.code(), "protocol_error", "{refused}");
    assert!(!backend.sessions()[0].opened);

    // A view with numbers that are not numbers, and labels far too long.
    let mut view = World::recorded().views[SESSION].clone();
    view["label"] = json!("l".repeat(1_000_000));
    world.views.insert(SESSION.to_owned(), view.clone());
    served.tell(Tell::World(world.clone()));
    // What was told is in effect once a later request has been answered.
    backend.open_session("session:none").await.unwrap_err();
    let opened = backend.open_session(SESSION).await.unwrap();
    assert_eq!(opened.label.len(), 1024);
    assert!(matches!(&*backend.link().borrow(), Link::Connected(_)));
}

#[tokio::test]
async fn a_backend_that_is_shut_down_says_goodbye_and_stays_away() {
    let (dialer, mut listener) = streams();
    let dialled = dialer.dialled.clone();
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
    let served = serve(listener.accept().await, World::recorded());
    backend.ready().await.unwrap();

    backend.shutdown();
    assert_eq!(*backend.link().borrow(), Link::Closed);
    served.over().await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(dialled.load(Ordering::Relaxed), 1);
    assert!(backend.ready().await.is_err());
}

#[tokio::test]
async fn what_is_asked_of_a_terminal_that_is_not_known_is_refused() {
    let (dialer, mut listener) = streams();
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
    let _served = serve(listener.accept().await, World::recorded());
    backend.ready().await.unwrap();

    let id = lume_proto::SessionId::new(1).unwrap();
    let refusals = [
        backend.kill(id).await.unwrap_err(),
        backend.send(id, "text").await.unwrap_err(),
        backend.screen(id).await.unwrap_err(),
        backend
            .wait_text(id, "text", Duration::from_millis(10))
            .await
            .unwrap_err(),
    ];
    for refusal in refusals {
        match refusal {
            SessionError::Refused(error) => {
                assert_eq!(error.code, lume_proto::ErrorCode::NotFound);
            }
            other => panic!("{other:?}"),
        }
    }
    // A layout is kept, and with nothing on show there is nothing in it.
    assert!(backend.layout().is_some());
}
