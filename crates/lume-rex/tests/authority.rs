//! A backend's layout against a pretend server: the session on show is given as a
//! workspace, what the server changes is followed, and a view that cannot be shown
//! changes nothing.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{SESSION, Served, Tell, World, client, serve, streams, until};
use lume_layout::{
    ChangeStream, Dir, Intent, LayoutError, LayoutEvent, LayoutSnapshot, NewContent, Node,
    PaneContent, SpawnRequest, UNITS, Window, WindowId, Workspace, WorkspaceId, mirror,
};
use lume_rex::RexBackend;
use lume_session::SessionBackend;
use serde_json::{Value, json};

const OTHER: &str = "session:other";

/// A backend connected to a pretend server holding `world`.
async fn connected(world: World) -> (Arc<RexBackend>, Served, common::Listener) {
    let (dialer, mut listener) = streams();
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
    let served = serve(listener.accept().await, world);
    backend.ready().await.unwrap();
    (backend, served, listener)
}

/// The recorded session and a second one with the same layout under other names.
fn two_sessions() -> World {
    let mut world = World::recorded();
    let view = serde_json::to_string(&world.views[SESSION]).unwrap();
    let mut other: Value = serde_json::from_str(
        &view
            .replace("block:", "block:other-")
            .replace("window:", "window:other-")
            .replace("split:", "split:other-")
            .replace("layer:", "layer:other-"),
    )
    .unwrap();
    other["session_id"] = json!(OTHER);
    other["label"] = json!("the other");
    world.sessions.push((OTHER.to_owned(), "other".to_owned()));
    world.views.insert(OTHER.to_owned(), other);
    world
}

/// Puts a changed view of a session in the server's place of it and says so.
fn change(served: &Served, world: &mut World, session: &str, with: impl FnOnce(&mut Value)) {
    let view = world.views.get_mut(session).unwrap();
    let revision = view["revision"].as_u64().unwrap() + 1;
    view["revision"] = json!(revision);
    with(view);
    served.tell(Tell::World(world.clone()));
    served.tell(Tell::SessionEvent(
        session.to_owned(),
        "session_view_changed".to_owned(),
        json!({ "revision": revision }),
    ));
}

async fn next(changes: &mut ChangeStream) -> Option<LayoutEvent> {
    tokio::time::timeout(Duration::from_secs(20), changes.next())
        .await
        .expect("no event of the layout came")
}

/// The titles of the panes of the first tab, in the order they are laid out.
fn titles(snapshot: &LayoutSnapshot) -> Vec<Option<String>> {
    let window = &snapshot.workspace.windows[0];
    window.centre.tabs[0]
        .root
        .panes()
        .into_iter()
        .map(|pane| pane.title.clone())
        .collect()
}

fn tabs(snapshot: &LayoutSnapshot) -> usize {
    snapshot.workspace.windows[0].centre.tabs.len()
}

#[tokio::test]
async fn a_backend_gives_the_layout_of_the_session_on_show() {
    let (backend, served, _listener) = connected(World::recorded()).await;
    let layout = backend.layout().expect("a Rex server keeps a layout");

    // Nothing is on show: a window with nothing in it.
    let empty = layout.view().await.unwrap();
    assert_eq!(empty.workspace.windows.len(), 1);
    assert_eq!(empty.workspace.windows[0].id, WindowId::new(1));
    assert_eq!(tabs(&empty), 0);
    assert_eq!(backend.shown_session(), None);

    let mut changes = layout.changes();
    backend.show_session(SESSION).await.unwrap();
    assert_eq!(backend.shown_session().as_deref(), Some(SESSION));
    // Showing it opened it.
    assert!(backend.view(SESSION).is_some());

    // Another session's layout is another layout, given whole.
    let Some(LayoutEvent::Reset(given)) = next(&mut changes).await else {
        panic!("the session's layout was not given whole");
    };
    assert!(given.epoch > empty.epoch);
    assert_eq!(*given, layout.view().await.unwrap());
    assert_eq!(given.workspace.name, "lume-fixture");
    let window = &given.workspace.windows[0];
    assert_eq!(window.id, WindowId::new(1));
    assert_eq!(window.centre.tabs.len(), 1);
    let Node::Split(split) = &window.centre.tabs[0].root else {
        panic!("the recorded session has a split");
    };
    assert_eq!(split.ratios, vec![UNITS / 2, UNITS / 2]);

    // Its terminal panes are sessions of this backend, by the numbers the backend
    // lists them under.
    let listed: Vec<_> = backend
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|info| info.id)
        .collect();
    let mut shown: Vec<_> = given
        .workspace
        .sessions()
        .into_iter()
        .map(|(_, session)| session)
        .collect();
    shown.sort();
    assert_eq!(shown, listed);
    for pane in given.workspace.panes() {
        let PaneContent::Terminal(terminal) = &pane.content else {
            panic!("the recorded blocks are terminals");
        };
        assert_eq!(terminal.backend.as_ref(), Some(backend.id()));
    }

    // The application takes it as a copy in a workspace of its own.
    let mut own = Workspace::new(WorkspaceId::new(1).unwrap(), "mine");
    own.windows.push(Window::new(WindowId::new(1)));
    own.focused_window = Some(WindowId::new(1));
    own.next_id = 2;
    mirror(&own, given.workspace.clone(), backend.id()).unwrap();

    // Showing what is on show already changes nothing.
    backend.show_session(SESSION).await.unwrap();
    let asked = served.called();
    assert_eq!(
        asked
            .iter()
            .filter(|method| *method == "session.attach")
            .count(),
        1
    );
    assert_eq!(layout.view().await.unwrap(), *given);
}

#[tokio::test]
async fn what_the_server_changes_is_followed_as_what_differs() {
    let mut world = World::recorded();
    let (backend, served, _listener) = connected(world.clone()).await;
    let layout = backend.layout().unwrap();
    backend.show_session(SESSION).await.unwrap();
    let shown = layout.view().await.unwrap();
    let mut changes = layout.changes();

    // A revision the backend already has says nothing new, and neither does a change
    // to a session that is not on show.
    served.tell(Tell::SessionEvent(
        SESSION.to_owned(),
        "session_view_changed".to_owned(),
        json!({ "revision": world.views[SESSION]["revision"] }),
    ));

    change(&served, &mut world, SESSION, |view| {
        view["windows"][0]["layers"][0]["blocks"][1]["label"] = json!("logs");
        view["windows"][0]["layers"][0]["layout"]["split"]["ratio"] = json!(0.25);
    });
    let Some(LayoutEvent::Changed {
        epoch,
        revision,
        changes: differs,
    }) = next(&mut changes).await
    else {
        panic!("the change was not reported as a step");
    };
    assert_eq!((epoch, revision), (shown.epoch, shown.revision + 1));
    assert!(!differs.is_empty());
    let now = layout.view().await.unwrap();
    assert_eq!((now.epoch, now.revision), (epoch, revision));
    assert_eq!(titles(&now), [None, Some("logs".to_owned())]);
    let Node::Split(split) = &now.workspace.windows[0].centre.tabs[0].root else {
        panic!("still a split");
    };
    assert_eq!(split.ratios, vec![UNITS / 4, UNITS / 4 * 3]);
    // The panes are the ones they were.
    assert_eq!(
        now.workspace.windows[0].centre.tabs[0].pane_ids(),
        shown.workspace.windows[0].centre.tabs[0].pane_ids()
    );

    // A view that says the same in the application's terms is no step: the owner of a
    // block is nothing a layout shows.
    change(&served, &mut world, SESSION, |view| {
        view["windows"][0]["layers"][0]["blocks"][0]["owner"] = json!("client:someone");
    });
    change(&served, &mut world, SESSION, |view| {
        view["windows"][0]["label"] = json!("build");
    });
    let Some(LayoutEvent::Changed {
        revision: after, ..
    }) = next(&mut changes).await
    else {
        panic!("the rename was not reported");
    };
    assert_eq!(after, revision + 1);
    let now = layout.view().await.unwrap();
    assert_eq!(
        now.workspace.windows[0].centre.tabs[0].title.as_deref(),
        Some("build")
    );
}

#[tokio::test]
async fn another_session_on_show_is_another_layout() {
    let mut world = two_sessions();
    let (backend, served, _listener) = connected(world.clone()).await;
    let layout = backend.layout().unwrap();
    backend.show_session(SESSION).await.unwrap();
    let first = layout.view().await.unwrap();
    let mut changes = layout.changes();

    backend.show_session(OTHER).await.unwrap();
    let Some(LayoutEvent::Reset(second)) = next(&mut changes).await else {
        panic!("the other session's layout was not given whole");
    };
    assert_eq!(second.epoch, first.epoch + 1);
    assert_eq!(second.workspace.name, "the other");
    assert_ne!(second.workspace.id, first.workspace.id);
    // Nothing of the one is anything of the other.
    let panes = |snapshot: &LayoutSnapshot| snapshot.workspace.windows[0].centre.tabs[0].pane_ids();
    assert!(
        panes(&second)
            .iter()
            .all(|pane| !panes(&first).contains(pane))
    );

    // The first session is still opened and kept up to date, and changes to it are no
    // changes to the layout on show.
    change(&served, &mut world, SESSION, |view| {
        view["windows"][0]["label"] = json!("unseen");
    });
    until("the first session's view is read", || {
        backend
            .view(SESSION)
            .is_some_and(|view| view.windows[0].label == "unseen")
    })
    .await;
    change(&served, &mut world, OTHER, |view| {
        view["windows"][0]["label"] = json!("seen");
    });
    let Some(LayoutEvent::Changed { epoch, .. }) = next(&mut changes).await else {
        panic!("the change to the session on show was not reported");
    };
    assert_eq!(epoch, second.epoch);
    let now = layout.view().await.unwrap();
    assert_eq!(
        now.workspace.windows[0].centre.tabs[0].title.as_deref(),
        Some("seen")
    );

    // Nothing on show: the empty window, whole.
    backend.show_nothing();
    let Some(LayoutEvent::Reset(empty)) = next(&mut changes).await else {
        panic!("the empty layout was not given whole");
    };
    assert_eq!(tabs(&empty), 0);
    assert_eq!(backend.shown_session(), None);
    assert!(backend.view(OTHER).is_some());
}

#[tokio::test]
async fn a_session_that_is_gone_leaves_an_empty_window() {
    let world = two_sessions();
    let (backend, served, _listener) = connected(world).await;
    let layout = backend.layout().unwrap();
    backend.show_session(SESSION).await.unwrap();
    let mut changes = layout.changes();

    served.tell(Tell::Event(
        "session_destroyed".to_owned(),
        json!({ "session_id": SESSION }),
    ));
    let Some(LayoutEvent::Reset(empty)) = next(&mut changes).await else {
        panic!("the layout of a session that is gone was not replaced");
    };
    assert_eq!(tabs(&empty), 0);
    assert_eq!(backend.shown_session(), None);

    // Letting go of the session on show does the same.
    backend.show_session(OTHER).await.unwrap();
    assert!(matches!(
        next(&mut changes).await,
        Some(LayoutEvent::Reset(_))
    ));
    backend.close_session(OTHER).await.unwrap();
    let Some(LayoutEvent::Reset(empty)) = next(&mut changes).await else {
        panic!("the layout of a session that was let go of was not replaced");
    };
    assert_eq!(tabs(&empty), 0);
    assert_eq!(layout.view().await.unwrap(), *empty);
}

#[tokio::test]
async fn the_layout_is_read_again_after_the_connection_was_lost() {
    let mut world = World::recorded();
    let (backend, served, mut listener) = connected(world.clone()).await;
    let layout = backend.layout().unwrap();
    backend.show_session(SESSION).await.unwrap();
    let shown = layout.view().await.unwrap();
    let mut changes = layout.changes();

    // The connection drops; the layout stays as it was.
    served.tell(Tell::Drop);
    served.over().await;
    assert_eq!(layout.view().await.unwrap(), shown);

    // The server changed meanwhile. The same session is the same layout, a step on.
    let view = world.views.get_mut(SESSION).unwrap();
    view["revision"] = json!(view["revision"].as_u64().unwrap() + 40);
    view["windows"][0]["label"] = json!("after the gap");
    let _served = serve(listener.accept().await, world);
    let Some(LayoutEvent::Changed {
        epoch, revision, ..
    }) = next(&mut changes).await
    else {
        panic!("what changed while the connection was down was not reported");
    };
    assert_eq!((epoch, revision), (shown.epoch, shown.revision + 1));
    let now = layout.view().await.unwrap();
    assert_eq!(
        now.workspace.windows[0].centre.tabs[0].title.as_deref(),
        Some("after the gap")
    );
    assert_eq!(
        now.workspace.windows[0].centre.tabs[0].pane_ids(),
        shown.workspace.windows[0].centre.tabs[0].pane_ids()
    );
}

/// Makes a view one that is no layout: its two blocks are one.
fn spoil(view: &mut Value) {
    let split = &mut view["windows"][0]["layers"][0]["layout"]["split"];
    split["after"]["block_id"] = split["before"]["block_id"].clone();
}

#[tokio::test]
async fn a_view_that_cannot_be_shown_changes_nothing() {
    let mut world = two_sessions();
    let (backend, served, _listener) = connected(world.clone()).await;
    let layout = backend.layout().unwrap();
    backend.show_session(SESSION).await.unwrap();
    let shown = layout.view().await.unwrap();
    let mut changes = layout.changes();

    let good = world.views[SESSION].clone();
    change(&served, &mut world, SESSION, spoil);
    let spoiled = world.views[SESSION]["revision"].as_u64().unwrap();
    until("the spoiled view is read", || {
        backend
            .view(SESSION)
            .is_some_and(|view| view.revision == spoiled)
    })
    .await;
    // Asking says what is wrong, and goes on saying so.
    for _ in 0..2 {
        let refused = loop {
            match layout.view().await {
                Err(refused) => break refused,
                Ok(_) => tokio::time::sleep(Duration::from_millis(5)).await,
            }
        };
        assert_eq!(refused.code(), "unsupported");
        assert!(refused.to_string().contains("used twice"), "{refused}");
    }

    // A view that can be shown again is a step on from the last one that was.
    change(&served, &mut world, SESSION, |view| {
        let revision = view["revision"].clone();
        *view = good;
        view["revision"] = revision;
        view["windows"][0]["label"] = json!("mended");
    });
    let Some(LayoutEvent::Changed {
        epoch, revision, ..
    }) = next(&mut changes).await
    else {
        panic!("nothing was reported between the two views that could be shown");
    };
    assert_eq!((epoch, revision), (shown.epoch, shown.revision + 1));
    let mended = layout.view().await.unwrap();
    assert_eq!(
        mended.workspace.windows[0].centre.tabs[0].pane_ids(),
        shown.workspace.windows[0].centre.tabs[0].pane_ids()
    );

    // A session that cannot be shown is not put on show, and what was stays.
    spoil(world.views.get_mut(OTHER).unwrap());
    world
        .sessions
        .push(("session:marker".to_owned(), "marker".to_owned()));
    served.tell(Tell::World(world.clone()));
    served.tell(Tell::Event(
        "session_created".to_owned(),
        json!({ "session": { "session_id": "session:marker", "label": "marker" } }),
    ));
    until("the server holds the spoiled session", || {
        backend.sessions().len() == 3
    })
    .await;
    let refused = backend.show_session(OTHER).await.unwrap_err();
    assert!(refused.to_string().contains("used twice"), "{refused}");
    assert_eq!(backend.shown_session().as_deref(), Some(SESSION));
    assert_eq!(layout.view().await.unwrap(), mended);
}

#[tokio::test]
async fn nothing_is_changed_from_here_yet_and_the_layout_ends_with_its_backend() {
    let (backend, _served, _listener) = connected(World::recorded()).await;
    let layout = backend.layout().unwrap();
    backend.show_session(SESSION).await.unwrap();
    let shown = layout.view().await.unwrap();
    let pane = shown.workspace.windows[0].centre.tabs[0].first_pane();

    let refused = layout
        .apply(Intent::Split {
            pane,
            dir: Dir::Right,
            ratio: UNITS / 2,
            content: NewContent::Terminal(SpawnRequest::default()),
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code(), "unsupported");
    assert_eq!(layout.view().await.unwrap(), shown);

    let mut changes = layout.changes();
    backend.shutdown();
    assert_eq!(next(&mut changes).await, None);
    assert_eq!(layout.view().await.unwrap_err(), LayoutError::Closed);
    // Whoever asks to follow it now is told at once that there is nothing to follow.
    assert_eq!(next(&mut layout.changes()).await, None);
}
