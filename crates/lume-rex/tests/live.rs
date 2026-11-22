//! The backend against a Rex server that is really running on this machine.
//!
//! Not run with the other tests: it needs the user's own server to be up, and it makes
//! and destroys a session there.
//!
//! cargo test -p lume-rex --test live -- --ignored

mod common;

use common::{client, until};
use lume_rex::{Control, Endpoint, Link, RexBackend, dial};
use lume_session::SessionBackend;
use serde_json::{Value, json};

#[tokio::test]
#[ignore = "needs a Rex server running on this machine"]
async fn the_backend_follows_a_real_server() {
    // A second client, to do what some other client of the server might.
    let io = dial(&Endpoint::Local)
        .await
        .expect("is a Rex server running?");
    let (other, _events) = Control::open(io, "local", &client(), None).await.unwrap();

    let backend = RexBackend::connect(Endpoint::Local, client());
    let version = backend.ready().await.unwrap();
    assert!(!version.rex_version.is_empty());
    assert!(matches!(&*backend.link().borrow(), Link::Connected(_)));
    let before = backend.sessions().len();

    let created = other
        .call(
            "session.create",
            json!({
                "label": "lume-live-test",
                "initial_windows": [
                    {"layout": {"block": {"flavor": "com.superlogical.terminal.shell"}}}
                ],
            }),
        )
        .await
        .unwrap();
    let session = created.payload["session_id"].as_str().unwrap().to_owned();
    until("the new session is known", || {
        backend.sessions().len() == before + 1
    })
    .await;
    assert!(
        backend
            .sessions()
            .iter()
            .any(|known| known.id == session && known.label == "lume-live-test")
    );

    let view = backend.open_session(&session).await.unwrap();
    assert_eq!(view.blocks().count(), 1);
    let listed = backend.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!((listed[0].size.cols(), listed[0].size.rows()), (80, 24));

    // Another client changes the layout and the label; the backend hears of both.
    let block = view.blocks().next().unwrap().block_id.clone().unwrap();
    let _ = other
        .call("session.attach", json!({ "session_id": session }))
        .await
        .unwrap();
    other
        .call(
            "session.new_split",
            json!({
                "session_id": session, "anchor_id": block,
                "direction": "vertical", "side": "after",
                "layout": {"block": {"flavor": "com.superlogical.terminal.shell"}},
            }),
        )
        .await
        .unwrap();
    until("the split is seen", || {
        backend
            .view(&session)
            .is_some_and(|view| view.blocks().count() == 2)
    })
    .await;
    assert_eq!(backend.list().await.unwrap().len(), 2);
    other
        .call(
            "session.set_label",
            json!({ "session_id": session, "label": "lume-live-renamed" }),
        )
        .await
        .unwrap();
    until("the new label is seen", || {
        backend
            .sessions()
            .iter()
            .any(|known| known.label == "lume-live-renamed")
    })
    .await;

    other
        .call("session.destroy", json!({ "session_id": session }))
        .await
        .unwrap();
    until("the session is gone", || backend.sessions().len() == before).await;
    assert!(backend.view(&session).is_none());
    assert_eq!(backend.list().await.unwrap(), vec![]);

    // What the server refuses comes back as it said it.
    let refused = other.call("no.such_method", Value::Null).await.unwrap_err();
    assert_eq!(refused.code(), "method_not_found");
    backend.shutdown();
}
