//! The backend against a Rex server that is really running on this machine.
//!
//! Not run with the other tests: it needs the user's own server to be up, and it makes
//! and destroys a session there.
//!
//! cargo test -p lume-rex --test live -- --ignored

mod common;

use std::time::Duration;

use common::{client, until};
use lume_rex::tuning::{TERMINAL_DATA_REF, TERMINAL_SUBPROTOCOL};
use lume_rex::{
    Control, DataConn, DataEnd, DataOpen, Endpoint, Link, Note, Opening, Piece, Reject, RexBackend,
    RexError, Splitter, Ticket, dial,
};
use lume_session::SessionBackend;
use serde_json::{Value, json};

/// Reads a stream until `done` says so, and returns what it held: the copy it opened
/// with, the sizes it named, its output, and how many of its bytes were taken.
struct Reading {
    opening: Option<Opening>,
    splitter: Splitter,
    snapshot: Option<Vec<u8>>,
    sizes: Vec<(u16, u16)>,
    output: Vec<u8>,
    received: u64,
}

impl Reading {
    fn new(nonce: u64) -> Self {
        Self {
            opening: Some(Opening::new(nonce)),
            splitter: Splitter::new(nonce),
            snapshot: None,
            sizes: Vec::new(),
            output: Vec::new(),
            received: 0,
        }
    }

    fn take(&mut self, bytes: Vec<u8>) {
        self.received += bytes.len() as u64;
        let rest = match &mut self.opening {
            Some(opening) => match opening.feed(&bytes) {
                Some(opened) => {
                    self.opening = None;
                    self.snapshot = opened.snapshot;
                    opened.rest
                }
                None => return,
            },
            None => bytes,
        };
        for piece in self.splitter.feed(&rest) {
            match piece {
                Piece::Output(bytes) => self.output.extend(bytes),
                Piece::Note(Note::Resize { size, .. }) => {
                    self.sizes.push((size.cols(), size.rows()));
                }
                Piece::Note(_) => {}
            }
        }
    }

    async fn until(&mut self, conn: &mut DataConn, what: &str, done: impl Fn(&Self) -> bool) {
        let reading = async {
            while !done(self) {
                match conn.next().await {
                    Ok(bytes) => self.take(bytes),
                    Err(end) => panic!("the stream ended before {what}: {end:?}"),
                }
            }
        };
        if tokio::time::timeout(Duration::from_secs(20), reading)
            .await
            .is_err()
        {
            panic!("{what} never happened");
        }
    }
}

/// A terminal whose shell has nothing of the user's in it: it reads no profile and
/// keeps no history of what is typed.
fn plain_shell() -> Value {
    let command = if cfg!(windows) {
        json!(["cmd.exe", "/d"])
    } else {
        json!(["/bin/sh"])
    };
    json!({
        "flavor": "com.superlogical.terminal.shell",
        "options": { "shell": "none", "command": command },
    })
}

/// A line that prints `lume-<tag>` when typed into that shell, and does not read so
/// itself: what is looked for is then the shell's output, not the echo of the typing.
fn echo(tag: &str) -> String {
    if cfg!(windows) {
        format!("echo lume-^{tag}\r")
    } else {
        format!("echo lume-''{tag}\r")
    }
}

async fn ticket(control: &Control, session: &str, block: &str) -> Ticket {
    let connect = control
        .call(
            "data.connect",
            json!({
                "session_id": session, "block_id": block,
                "data_ref_id": TERMINAL_DATA_REF, "subprotocol": TERMINAL_SUBPROTOCOL,
            }),
        )
        .await
        .unwrap();
    Ticket::read(connect.payload["ticket"].as_str().unwrap()).unwrap()
}

#[tokio::test]
#[ignore = "needs a Rex server running on this machine"]
async fn a_terminals_stream_is_read_from_a_real_server() {
    let io = dial(&Endpoint::Local)
        .await
        .expect("is a Rex server running?");
    let (control, _events) = Control::open(io, "local", &client(), None).await.unwrap();
    let created = control
        .call(
            "session.create",
            json!({
                "label": "lume-live-stream",
                "initial_windows": [
                    {"layout": {"block": plain_shell()}}
                ],
            }),
        )
        .await
        .unwrap();
    let session = created.payload["session_id"].as_str().unwrap().to_owned();
    let block = created.payload["initial_windows"][0]["block_ids"][0]
        .as_str()
        .unwrap()
        .to_owned();
    control
        .call("session.attach", json!({ "session_id": session }))
        .await
        .unwrap();
    let of_block = |args: Value| json!({ "session_id": session, "block_id": block, "args": args });

    // The stream opens with a copy of the terminal and its size.
    let first = ticket(&control, &session, &block).await;
    let io = dial(&Endpoint::Local).await.unwrap();
    let mut conn = DataConn::open(io, "local", DataOpen::Start(&first))
        .await
        .unwrap();
    let mut reading = Reading::new(conn.welcome().nonce);
    reading
        .until(&mut conn, "the first size", |reading| {
            !reading.sizes.is_empty()
        })
        .await;
    assert!(reading.snapshot.as_ref().unwrap().starts_with(b"GHOSTSNP"));
    assert_eq!(reading.sizes, [(80, 24)]);

    // A ticket works once.
    let io = dial(&Endpoint::Local).await.unwrap();
    match DataConn::open(io, "local", DataOpen::Start(&first)).await {
        Err(RexError::Rejected(Reject::BadTicket)) => {}
        Err(other) => panic!("{other:?}"),
        Ok(_) => panic!("a ticket worked twice"),
    }

    // What is typed comes back as output, and a new size is said in the stream.
    control
        .call(
            "com.superlogical.terminal.write",
            of_block(json!({ "data": echo("42-live") })),
        )
        .await
        .unwrap();
    reading
        .until(&mut conn, "the echo", |reading| {
            String::from_utf8_lossy(&reading.output).contains("lume-42-live")
        })
        .await;
    control
        .call(
            "com.superlogical.terminal.resize",
            of_block(json!({ "columns": 100, "rows": 30 })),
        )
        .await
        .unwrap();
    reading
        .until(&mut conn, "the new size", |reading| {
            reading.sizes.len() == 2
        })
        .await;
    assert_eq!(reading.sizes[1], (100, 30));

    // The connection drops, more is printed, and the stream is continued from the very
    // byte it stopped at: nothing twice, nothing missing.
    let token = conn.welcome().token.clone();
    let nonce = conn.welcome().nonce;
    drop(conn);
    control
        .call(
            "com.superlogical.terminal.write",
            of_block(json!({ "data": echo("55-resumed") })),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let io = dial(&Endpoint::Local).await.unwrap();
    let mut conn = DataConn::open(
        io,
        "local",
        DataOpen::Resume {
            token: &token,
            received: reading.received,
        },
    )
    .await
    .unwrap();
    assert_eq!(conn.welcome().nonce, nonce);
    assert_ne!(conn.welcome().token, token);
    let before = reading.output.len();
    reading
        .until(&mut conn, "the output printed meanwhile", |reading| {
            String::from_utf8_lossy(&reading.output).contains("lume-55-resumed")
        })
        .await;
    let since = String::from_utf8_lossy(&reading.output[before..]).into_owned();
    // Printed once, on a line of its own. (A shell may also name it in its title.)
    assert_eq!(since.matches("lume-55-resumed\r\n").count(), 1, "{since:?}");

    // A count that is not exact ends the stream.
    let token = conn.welcome().token.clone();
    drop(conn);
    let io = dial(&Endpoint::Local).await.unwrap();
    let wrong = DataOpen::Resume {
        token: &token,
        received: reading.received + 1_000_000,
    };
    match DataConn::open(io, "local", wrong).await {
        Err(RexError::Rejected(Reject::Malformed)) => {}
        Err(other) => panic!("{other:?}"),
        Ok(_) => panic!("a wrong count was taken"),
    }

    // A new stream, and the block closes under it.
    let again = ticket(&control, &session, &block).await;
    let io = dial(&Endpoint::Local).await.unwrap();
    let mut conn = DataConn::open(io, "local", DataOpen::Start(&again))
        .await
        .unwrap();
    control
        .call(
            "block.close",
            json!({ "session_id": session, "block_id": block }),
        )
        .await
        .unwrap();
    let end = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Err(end) = conn.next().await {
                break end;
            }
        }
    })
    .await
    .expect("the stream of a closed block never ended");
    assert!(matches!(end, DataEnd::BlockClosed), "{end:?}");

    let _ = control
        .call("session.destroy", json!({ "session_id": session }))
        .await;
}

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
