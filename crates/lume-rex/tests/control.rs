//! The control connection against a pretend server, well behaved and not.

mod common;

use std::time::Duration;

use common::{Conn, Recorded, client, pair};
use lume_rex::tuning::{API_REVISION, CONTROL_SUBPROTOCOL, EVENT_BACKLOG, MESSAGE_MAX_BYTES};
use lume_rex::{Control, Events, RexError};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn welcome() -> Value {
    Recorded::load().welcome()
}

/// A connection that is open, and the server's side of it.
async fn connected() -> (Control, Events, Conn) {
    let (near, far) = pair();
    let server = tokio::spawn(Conn::accept(far, welcome()));
    let (control, events) = Control::open(near, "test", &client(), None).await.unwrap();
    (control, events, server.await.unwrap())
}

/// Opens a connection to a server that does `server` with its side of the stream.
async fn open_against<F, Fut>(server: F) -> Result<(Control, Events), RexError>
where
    F: FnOnce(tokio::io::DuplexStream) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (near, far) = pair();
    let served = tokio::spawn(server(far));
    let opened = Control::open(near, "test", &client(), None).await;
    let _ = served.await;
    opened
}

#[tokio::test]
async fn a_connection_opens_with_a_hello_and_the_servers_welcome() {
    let (control, _events, conn) = connected().await;

    assert_eq!(conn.asked.as_deref(), Some(CONTROL_SUBPROTOCOL));
    assert_eq!(conn.hello["type"], "hello");
    assert_eq!(conn.hello["api_revision"], API_REVISION);
    assert_eq!(conn.hello["client"]["name"], "lume-test");
    assert_eq!(conn.hello["client"]["kind"], "app");
    assert!(conn.hello.get("reconnect_token").is_none());

    let welcome = control.welcome();
    assert_eq!(welcome.revisions.used, API_REVISION);
    assert_eq!(welcome.revisions.server, API_REVISION);
    assert_eq!(welcome.max_message_bytes, 1_048_576);
    assert!(welcome.client_id.as_deref().unwrap().starts_with("client:"));
    assert!(!welcome.resumed);
    assert!(control.ended().is_none());
}

#[tokio::test]
async fn a_token_from_an_earlier_welcome_is_sent_in_the_hello() {
    let (near, far) = pair();
    let server = tokio::spawn(async move {
        let mut welcome = welcome();
        welcome["client"]["resumed"] = json!(true);
        Conn::accept(far, welcome).await
    });
    let (control, _events) = Control::open(near, "test", &client(), Some("earlier"))
        .await
        .unwrap();
    let conn = server.await.unwrap();
    assert_eq!(conn.hello["reconnect_token"], "earlier");
    assert!(control.welcome().resumed);
}

#[tokio::test]
async fn a_request_gets_the_response_with_its_id_whatever_the_order() {
    let (control, _events, mut conn) = connected().await;
    let recorded = Recorded::load();
    let version = recorded.result("server.version", 0);

    let server = tokio::spawn(async move {
        let first = conn.asked().await.unwrap();
        let second = conn.asked().await.unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(first.method, "server.version");
        assert!(first.payload.is_null());
        assert_eq!(second.payload, json!({ "session_id": "session:1" }));
        // Answered the other way round.
        conn.respond(&second.id, json!({ "revision": 4 })).await;
        conn.respond(&first.id, version).await;
        conn
    });
    let (first, second) = tokio::join!(
        control.call("server.version", Value::Null),
        control.call("session.view", json!({ "session_id": "session:1" })),
    );
    assert_eq!(first.unwrap().payload["rex_version"], "0.1.0-preview.22");
    assert_eq!(second.unwrap().payload, json!({ "revision": 4 }));
    drop(server.await.unwrap());
}

#[tokio::test]
async fn an_error_from_the_server_keeps_its_code_and_the_connection() {
    let (control, _events, mut conn) = connected().await;
    let recorded = Recorded::load();
    let missing = recorded.response("session.view", 1)["error"].clone();
    let server = tokio::spawn(async move {
        let asked = conn.asked().await.unwrap();
        let seq = conn.seq;
        conn.send(json!({ "type": "response", "id": asked.id, "seq": seq, "error": missing }))
            .await;
        let asked = conn.asked().await.unwrap();
        conn.respond(&asked.id, json!({})).await;
        conn
    });

    let refused = control
        .call("session.view", json!({ "session_id": "session:gone" }))
        .await
        .unwrap_err();
    match &refused {
        RexError::Call {
            method,
            code,
            status,
            ..
        } => {
            assert_eq!(method, "session.view");
            assert_eq!(code, "session_not_found");
            assert_eq!(*status, 404);
        }
        other => panic!("{other:?}"),
    }
    // By this client's own name for it, whatever the server called it.
    assert_eq!(refused.code(), "not_found");
    assert!(control.call("session.list", Value::Null).await.is_ok());
    drop(server.await.unwrap());
}

#[tokio::test]
async fn events_arrive_in_order_with_their_numbers() {
    let (_control, mut events, mut conn) = connected().await;
    conn.event(
        "session_created",
        json!({ "session": { "session_id": "s", "label": "l" } }),
    )
    .await;
    conn.session_event("s", "session_view_changed", json!({ "revision": 2 }))
        .await;
    // A kind of message a newer server might send is passed over.
    conn.send(json!({ "type": "notice_from_the_future", "text": "hello" }))
        .await;
    conn.event("session_destroyed", json!({ "session_id": "s" }))
        .await;

    let first = events.next().await.unwrap();
    assert_eq!((first.seq, first.name.as_str()), (1, "session_created"));
    assert_eq!(first.payload["session"]["label"], "l");
    let second = events.next().await.unwrap();
    assert_eq!((second.seq, second.name.as_str()), (2, "session_event"));
    assert_eq!(second.payload["event"]["type"], "session_view_changed");
    let third = events.next().await.unwrap();
    assert_eq!((third.seq, third.name.as_str()), (3, "session_destroyed"));
}

#[tokio::test]
async fn an_event_out_of_turn_ends_the_connection() {
    let (control, mut events, mut conn) = connected().await;
    conn.event("session_destroyed", json!({ "session_id": "a" }))
        .await;
    // One was lost on the way.
    conn.seq += 1;
    conn.event("session_destroyed", json!({ "session_id": "c" }))
        .await;

    assert_eq!(events.next().await.unwrap().seq, 1);
    assert!(events.next().await.is_none());
    let why = control.until_ended().await;
    assert_eq!(why.code(), "protocol_error", "{why}");
    let refused = control.call("session.list", Value::Null).await.unwrap_err();
    assert_eq!(refused.code(), "protocol_error");
}

#[tokio::test]
async fn a_close_from_the_server_is_told_with_its_status_and_reason() {
    let (control, mut events, mut conn) = connected().await;
    let waiting = tokio::spawn(async move {
        let asked = conn.asked().await.unwrap();
        assert_eq!(asked.method, "session.list");
        conn.close(1008, "event backlog exceeded").await;
    });
    // A request that was waiting is told why it will never be answered.
    let unanswered = control.call("session.list", Value::Null).await.unwrap_err();
    waiting.await.unwrap();
    match unanswered {
        RexError::Closed { code, reason } => {
            assert_eq!(code, 1008);
            assert_eq!(reason, "event backlog exceeded");
        }
        other => panic!("{other:?}"),
    }
    assert!(events.next().await.is_none());
    assert!(matches!(
        control.ended(),
        Some(RexError::Closed { code: 1008, .. })
    ));
    assert!(control.ended().unwrap().is_transient());
}

#[tokio::test]
async fn a_connection_that_drops_is_lost_and_says_so() {
    let (control, _events, conn) = connected().await;
    drop(conn);
    let why = control.until_ended().await;
    assert!(matches!(why, RexError::Disconnected), "{why:?}");
    assert!(why.is_transient());
}

#[tokio::test]
async fn closing_says_goodbye_to_the_server() {
    let (control, _events, mut conn) = connected().await;
    control.close();
    // The server sees a close, not a stream that ends.
    assert!(conn.text().await.is_none());
    assert!(matches!(
        control.until_ended().await,
        RexError::Disconnected
    ));
}

#[tokio::test]
async fn a_server_with_no_revision_in_common_is_refused_either_way_it_says_so() {
    // A server that closes instead of welcoming.
    let closed = open_against(|far| async move {
        Conn::shake(far)
            .await
            .close(1008, "no common API revision: update the client")
            .await;
    })
    .await
    .err()
    .unwrap();
    assert_eq!(closed.code(), "version_mismatch", "{closed}");
    assert!(!closed.is_transient());

    // A server that welcomes with a revision this client was not written for.
    for (used, newest, oldest, update) in [
        (14, 14, 0, "the Rex server"),
        (20, 25, 20, "this application"),
    ] {
        let refused = open_against(move |far| async move {
            let mut welcome = welcome();
            welcome["api_revision"] = json!(used);
            welcome["api_server_revision"] = json!(newest);
            welcome["api_min_revision"] = json!(oldest);
            let _conn = Conn::accept(far, welcome).await;
        })
        .await
        .err()
        .unwrap();
        assert_eq!(refused.code(), "version_mismatch");
        assert!(refused.to_string().contains(update), "{refused}");
    }

    // One that leaves the revision out speaks revision 0.
    let silent = open_against(|far| async move {
        let mut welcome = welcome();
        welcome.as_object_mut().unwrap().remove("api_revision");
        let _conn = Conn::accept(far, welcome).await;
    })
    .await
    .err()
    .unwrap();
    assert_eq!(silent.code(), "version_mismatch");
}

#[tokio::test]
async fn a_server_that_does_not_let_the_caller_in_says_why() {
    let refused = open_against(|mut far| async move {
        // Read the request, then answer as a server that could not tell who called.
        let mut request = vec![0u8; 4096];
        let _ = far.read(&mut request).await;
        let body = r#"{"error":"connection was not classified by its listener"}"#;
        let answer = format!(
            "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        far.write_all(answer.as_bytes()).await.unwrap();
        far.shutdown().await.unwrap();
    })
    .await
    .err()
    .unwrap();
    match &refused {
        RexError::Refused { endpoint, reason } => {
            assert_eq!(endpoint, "test");
            assert_eq!(reason, "connection was not classified by its listener");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(refused.code(), "forbidden");
    assert!(!refused.is_transient());
}

#[tokio::test]
async fn what_is_not_a_rex_server_is_not_mistaken_for_one() {
    // Something that answers, but not as a WebSocket.
    let not_http = open_against(|mut far| async move {
        let mut request = vec![0u8; 4096];
        let _ = far.read(&mut request).await;
        far.write_all(b"SSH-2.0-OpenSSH_9.5\r\n").await.unwrap();
        far.shutdown().await.unwrap();
    })
    .await
    .err()
    .unwrap();
    assert_eq!(not_http.code(), "protocol_error", "{not_http}");

    // A WebSocket that agrees to no subprotocol.
    let other = open_against(|far| async move {
        let _ = tokio_tungstenite::accept_async(far).await;
    })
    .await
    .err()
    .unwrap();
    assert_eq!(other.code(), "protocol_error", "{other}");

    // A first message that is not a welcome, of each kind.
    for first in [
        json!({ "type": "response", "id": "1", "seq": 0 }).to_string(),
        json!({ "type": "welcome", "protocol": "rex.control.v9", "limits": { "max_message_bytes": 1048576 } }).to_string(),
        json!({ "type": "welcome", "protocol": "rex.control.v1" }).to_string(),
        json!({ "type": "welcome", "protocol": "rex.control.v1", "api_revision": 19, "limits": { "max_message_bytes": 12 } }).to_string(),
        "not json at all".to_owned(),
        "[1, 2, 3]".to_owned(),
    ] {
        let shown = first.clone();
        let refused = open_against(move |far| async move {
            let mut conn = Conn::shake(far).await;
            conn.raw(first).await;
            let _ = conn.text().await;
        })
        .await
        .err()
        .unwrap();
        assert_eq!(refused.code(), "protocol_error", "{shown}: {refused}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_server_that_never_welcomes_is_not_waited_for_for_ever() {
    let (near, far) = pair();
    let server = tokio::spawn(async move {
        let mut conn = Conn::shake(far).await;
        // Says nothing, and stays.
        let _ = conn.text().await;
    });
    let refused = Control::open(near, "test", &client(), None)
        .await
        .err()
        .unwrap();
    assert_eq!(refused.code(), "timeout");
    assert!(refused.is_transient());
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn a_request_that_is_never_answered_gives_up() {
    let (control, _events, mut conn) = connected().await;
    let server = tokio::spawn(async move {
        let _ = conn.asked().await;
        tokio::time::sleep(Duration::from_secs(3600)).await;
        drop(conn);
    });
    let refused = control.call("session.list", Value::Null).await.unwrap_err();
    assert!(matches!(&refused, RexError::Timeout(method) if method == "session.list"));
    // The connection itself is still there.
    assert!(control.ended().is_none());
    server.abort();
}

#[tokio::test]
async fn messages_that_are_not_what_a_server_sends_do_no_harm() {
    let (control, mut events, mut conn) = connected().await;
    let server = tokio::spawn(async move {
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::Message;
        let asked = conn.asked().await.unwrap();
        // A response to nothing, a response with a number for an id, a binary message,
        // an event with nothing in it: none of them is the answer, none ends anything.
        conn.respond("no-such-request", json!({})).await;
        conn.send(json!({ "type": "response", "id": 7, "seq": 0 }))
            .await;
        conn.socket
            .send(Message::binary(vec![0x81, 0x00, 0xff]))
            .await
            .unwrap();
        conn.send(json!({ "type": "event", "seq": 1, "payload": 5 }))
            .await;
        conn.send(json!({ "type": "event", "seq": 2 })).await;
        // An error with nothing in it is still an error.
        conn.send(json!({ "type": "response", "id": asked.id, "seq": 2, "error": {} }))
            .await;
        let asked = conn.asked().await.unwrap();
        // Text far longer than anything is shown at.
        conn.fail(&asked.id, &"c".repeat(100_000), 400, &"m".repeat(100_000))
            .await;
        conn
    });

    let empty = control.call("session.list", Value::Null).await.unwrap_err();
    assert!(matches!(&empty, RexError::Call { code, status: 0, .. } if code.is_empty()));
    let long = control.call("session.list", Value::Null).await.unwrap_err();
    match long {
        RexError::Call { code, message, .. } => {
            assert_eq!(code.len(), 1024);
            assert_eq!(message.len(), 1024);
        }
        other => panic!("{other:?}"),
    }
    let first = events.next().await.unwrap();
    assert_eq!(
        (first.seq, first.name.as_str(), &first.payload),
        (1, "", &Value::Null)
    );
    assert_eq!(events.next().await.unwrap().seq, 2);
    assert!(control.ended().is_none());
    drop(server.await.unwrap());
}

#[tokio::test]
async fn a_message_too_large_to_take_ends_the_connection() {
    let (control, _events, mut conn) = connected().await;
    let huge = "x".repeat(MESSAGE_MAX_BYTES + 1024);
    tokio::spawn(async move {
        conn.send(json!({ "type": "event", "seq": 1, "payload": { "type": huge } }))
            .await;
        let _ = conn.text().await;
    });
    let why = control.until_ended().await;
    assert_eq!(why.code(), "protocol_error", "{why}");
}

#[tokio::test]
async fn a_request_larger_than_the_server_takes_is_not_sent() {
    let (control, _events, mut conn) = connected().await;
    let server = tokio::spawn(async move {
        // Only the request that fits arrives.
        let asked = conn.asked().await.unwrap();
        assert_eq!(asked.method, "session.list");
        conn.respond(&asked.id, json!({ "sessions": [] })).await;
        conn
    });
    let refused = control
        .call(
            "session.set_label",
            json!({ "label": "l".repeat(2 * 1024 * 1024) }),
        )
        .await
        .unwrap_err();
    assert_eq!(refused.code(), "too_large");
    assert!(control.call("session.list", Value::Null).await.is_ok());
    drop(server.await.unwrap());
}

#[tokio::test]
async fn events_nobody_takes_end_the_connection_before_they_pile_up_without_end() {
    let (control, events, mut conn) = connected().await;
    tokio::spawn(async move {
        for _ in 0..EVENT_BACKLOG + 8 {
            conn.event("session_destroyed", json!({ "session_id": "s" }))
                .await;
        }
        let _ = conn.text().await;
    });
    let why = control.until_ended().await;
    assert_eq!(why.code(), "protocol_error", "{why}");
    drop(events);
}
