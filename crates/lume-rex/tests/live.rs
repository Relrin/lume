//! The backend against a Rex server that is really running on this machine.
//!
//! Not run with the other tests: it needs the user's own server to be up, and it makes
//! and destroys a session there.
//!
//! cargo test -p lume-rex --test live -- --ignored

mod common;

use std::time::Duration;

use common::{client, until};
use lume_proto::tuning::CREDIT_WINDOW;
use lume_proto::{CellPx, ClientFacts, ColorScheme, ExitStatus, Palette, TermSize};
use lume_rex::tuning::{TERMINAL_DATA_REF, TERMINAL_SUBPROTOCOL};
use lume_rex::{
    Control, DataConn, DataEnd, DataOpen, Endpoint, Link, Note, Opening, Piece, Reject, RexBackend,
    RexError, Splitter, Ticket, dial,
};
use lume_session::{Attachment, Hydrate, OutputFrame, SessionBackend, SessionError};
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

/// The frames of an attachment as a view would take them: output is granted as it is
/// consumed, and everything is kept to be looked at.
struct Viewing {
    attachment: Attachment,
    output: Vec<u8>,
    sizes: Vec<(u16, u16)>,
    copies: Vec<Hydrate>,
    exit: Option<ExitStatus>,
    over: bool,
}

impl Viewing {
    async fn new(attachment: Attachment) -> Self {
        attachment.input.credit(CREDIT_WINDOW).await.unwrap();
        Self {
            attachment,
            output: Vec::new(),
            sizes: Vec::new(),
            copies: Vec::new(),
            exit: None,
            over: false,
        }
    }

    async fn until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        let viewing = async {
            while !done(self) {
                match self.attachment.output.next().await {
                    Some(OutputFrame::Data(bytes)) => {
                        let count = u32::try_from(bytes.len()).unwrap();
                        self.output.extend(bytes);
                        let _ = self.attachment.input.credit(count).await;
                    }
                    Some(OutputFrame::Resized(size)) => self.sizes.push((size.cols(), size.rows())),
                    Some(OutputFrame::Hydrate(copy)) => self.copies.push(copy),
                    Some(OutputFrame::Exit(status)) => self.exit = Some(status),
                    Some(OutputFrame::Event(_)) => {}
                    None => {
                        self.over = true;
                        assert!(done(self), "the attachment ended before {what}");
                    }
                }
            }
        };
        if tokio::time::timeout(Duration::from_secs(20), viewing)
            .await
            .is_err()
        {
            panic!("{what} never happened");
        }
    }

    fn shows(&self, text: &str) -> bool {
        String::from_utf8_lossy(&self.output).contains(text)
    }
}

/// The rows of a copy of a terminal, loaded in this application's own engine.
fn rows_of(copy: &Hydrate) -> Vec<String> {
    match copy {
        Hydrate::Snapshot { vt_version, bytes } => {
            assert_eq!(vt_version, lume_vt::library_version());
            let terminal = lume_vt::Terminal::from_snapshot(bytes).unwrap();
            terminal.screen_text().unwrap()
        }
        other => panic!("not a copy this engine loads: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs a Rex server running on this machine"]
async fn a_terminal_of_a_real_server_is_attached_to() {
    let io = dial(&Endpoint::Local)
        .await
        .expect("is a Rex server running?");
    let (other, _events) = Control::open(io, "local", &client(), None).await.unwrap();
    let created = other
        .call(
            "session.create",
            json!({
                "label": "lume-live-attach",
                "initial_windows": [{"layout": {"block": plain_shell()}}],
            }),
        )
        .await
        .unwrap();
    let session = created.payload["session_id"].as_str().unwrap().to_owned();
    let block = created.payload["initial_windows"][0]["block_ids"][0]
        .as_str()
        .unwrap()
        .to_owned();
    other
        .call("session.attach", json!({ "session_id": session }))
        .await
        .unwrap();
    let of_block = |args: Value| json!({ "session_id": session, "block_id": block, "args": args });
    let typed = |tag: &str| {
        other.call(
            "com.superlogical.terminal.write",
            of_block(json!({ "data": echo(tag) })),
        )
    };

    let backend = RexBackend::connect(Endpoint::Local, client());
    backend.ready().await.unwrap();
    backend.open_session(&session).await.unwrap();
    let id = backend.list().await.unwrap()[0].id;

    // Something on the screen before anyone looks.
    typed("1-before").await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // The view starts from the server's copy of the terminal, which this engine loads.
    let attachment = backend.attach(id, facts()).await.unwrap();
    assert_eq!((attachment.size.cols(), attachment.size.rows()), (80, 24));
    let rows = rows_of(&attachment.hydrate);
    assert!(rows.iter().any(|row| row == "lume-1-before"), "{rows:?}");
    let again = backend.attach(id, facts()).await;
    assert!(matches!(again, Err(SessionError::Refused(_))));

    // What is typed afterwards comes as output, and a new size in its place.
    let mut view = Viewing::new(attachment).await;
    typed("2-after").await.unwrap();
    view.until("the echo", |view| view.shows("lume-2-after\r\n"))
        .await;
    other
        .call(
            "com.superlogical.terminal.resize",
            of_block(json!({ "columns": 100, "rows": 30 })),
        )
        .await
        .unwrap();
    view.until("the new size", |view| view.sizes == [(100, 30)])
        .await;

    // The view lets go of its screen; what is printed meanwhile is in the copy it
    // starts again from, at the terminal's size.
    view.attachment.input.release().await.unwrap();
    typed("3-unseen").await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    view.attachment.input.rehydrate().await.unwrap();
    view.until("a new copy", |view| !view.copies.is_empty())
        .await;
    let rows = rows_of(&view.copies[0]);
    assert_eq!(rows.len(), 30);
    for line in ["lume-1-before", "lume-2-after", "lume-3-unseen"] {
        assert!(rows.iter().any(|row| row == line), "{line}: {rows:?}");
    }
    assert!(!view.shows("lume-3-unseen\r\n"));
    typed("4-again").await.unwrap();
    view.until("the echo after the copy", |view| {
        view.shows("lume-4-again\r\n")
    })
    .await;

    // Detached and attached again: the stream was closed, so the way is free.
    view.attachment.input.detach().await.unwrap();
    let attachment = backend.attach(id, facts()).await.unwrap();
    assert_eq!((attachment.size.cols(), attachment.size.rows()), (100, 30));
    let mut view = Viewing::new(attachment).await;

    // The shell ends: the view hears of it, and nothing after.
    other
        .call(
            "com.superlogical.terminal.write",
            of_block(json!({ "data": "exit 7\r" })),
        )
        .await
        .unwrap();
    view.until("the end", |view| view.over).await;
    assert!(
        matches!(
            view.exit,
            Some(ExitStatus::Code(7)) | Some(ExitStatus::Unknown)
        ),
        "{:?}",
        view.exit
    );
    println!("the shell's end was told as {:?}", view.exit);

    let _ = other
        .call("session.destroy", json!({ "session_id": session }))
        .await;
    backend.shutdown();
}
