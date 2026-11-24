//! The backend against a Rex server that is really running on this machine.
//!
//! Not run with the other tests: it needs the user's own server to be up, and it makes
//! and destroys a session there.
//!
//! cargo test -p lume-rex --test live -- --ignored

mod common;

use std::time::Duration;

use common::{client, until};
use lume_layout::{
    Axis, Dir, FocusTarget, Intent, LayoutAuthority, LayoutError, LayoutEvent, Minimised,
    NewContent, Node, Place, SpawnRequest, TabTarget, UNITS, UnitRect, WindowId, Workspace,
    is_normalised, validate,
};
use lume_proto::tuning::CREDIT_WINDOW;
use lume_proto::{
    BackendId, CellPx, ClientFacts, ColorScheme, ExitStatus, Launcher, Palette, TermSize,
};
use lume_rex::tuning::{TERMINAL_DATA_REF, TERMINAL_SUBPROTOCOL};
use lume_rex::{
    Control, DataConn, DataEnd, DataOpen, Endpoint, Link, Note, Opening, Piece, Reject, RexBackend,
    RexError, Splitter, Ticket, dial,
};
use lume_session::{Attachment, Hydrate, OutputFrame, SessionBackend};
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

    /// Reads until the stream has been quiet for a moment.
    async fn settle(&mut self, conn: &mut DataConn) {
        while let Ok(next) = tokio::time::timeout(Duration::from_millis(400), conn.next()).await {
            match next {
                Ok(bytes) => self.take(bytes),
                Err(end) => panic!("the stream ended: {end:?}"),
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

    // A client has one stream of a block at a time: a second one, with a ticket of its
    // own, is refused while the first is open.
    let second = ticket(&control, &session, &block).await;
    let io = dial(&Endpoint::Local).await.unwrap();
    match DataConn::open(io, "local", DataOpen::Start(&second)).await {
        Err(RexError::Rejected(Reject::AlreadyConnected)) => {}
        Err(other) => panic!("{other:?}"),
        Ok(_) => panic!("a client was given two streams of one block"),
    }
    // Another client of the same server is given its own.
    let io = dial(&Endpoint::Local).await.unwrap();
    let (other, _other_events) = Control::open(io, "local", &client(), None).await.unwrap();
    other
        .call("session.attach", json!({ "session_id": session }))
        .await
        .unwrap();
    let theirs = ticket(&other, &session, &block).await;
    let io = dial(&Endpoint::Local).await.unwrap();
    let their_conn = DataConn::open(io, "local", DataOpen::Start(&theirs))
        .await
        .unwrap();
    assert_ne!(their_conn.welcome().nonce, conn.welcome().nonce);
    their_conn.close().await;
    drop(other);

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
    // byte it stopped at: nothing twice, nothing missing. Everything the server sent
    // is read first: a server does not continue a stream from before what it sent.
    reading.settle(&mut conn).await;
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

    let backend = RexBackend::connect(BackendId("rex".to_owned()), Endpoint::Local, client());
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
    assert!(matches!(&refused, RexError::Call { code, .. } if code == "method_not_found"));
    assert_eq!(refused.code(), "not_found");
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

    let backend = RexBackend::connect(BackendId("rex".to_owned()), Endpoint::Local, client());
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

    // What is typed afterwards comes as output, and a new size in its place.
    let mut view = Viewing::new(attachment).await;
    typed("2-after").await.unwrap();
    view.until("the echo", |view| view.shows("lume-2-after\r\n"))
        .await;
    // The view has the focus, so the block is its own and the size is its to give.
    view.attachment
        .input
        .resize(TermSize::new(100, 30).unwrap(), CellPx { w: 8, h: 16 })
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

    // Detached and attached again: the stream was closed, so the way is free. The new
    // view has the focus and its own size, which the terminal has before it is shown.
    view.attachment.input.detach().await.unwrap();
    let attachment = backend.attach(id, facts()).await.unwrap();
    assert_eq!((attachment.size.cols(), attachment.size.rows()), (80, 24));
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

#[tokio::test]
#[ignore = "needs a Rex server running on this machine"]
async fn a_terminal_of_a_real_server_is_typed_into_sized_and_read() {
    let io = dial(&Endpoint::Local)
        .await
        .expect("is a Rex server running?");
    let (other, _events) = Control::open(io, "local", &client(), None).await.unwrap();
    let created = other
        .call(
            "session.create",
            json!({
                "label": "lume-live-input",
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

    let backend = RexBackend::connect(BackendId("rex".to_owned()), Endpoint::Local, client());
    backend.ready().await.unwrap();
    backend.open_session(&session).await.unwrap();
    let id = backend.list().await.unwrap()[0].id;

    // A view with the focus gives the terminal its size before its stream begins.
    let mut wanted = facts();
    wanted.size = TermSize::new(100, 30).unwrap();
    let attachment = backend.attach(id, wanted.clone()).await.unwrap();
    assert_eq!((attachment.size.cols(), attachment.size.rows()), (100, 30));
    let mut view = Viewing::new(attachment).await;

    // What the view types is typed, in order, however it is cut up.
    let line = echo("1-typed");
    for piece in line.as_bytes().chunks(3) {
        view.attachment.input.write(piece).await.unwrap();
    }
    view.until("the echo", |view| view.shows("lume-1-typed\r\n"))
        .await;

    // The screen is read without a view, and says what the view was sent.
    backend.send(id, &echo("2-sent")).await.unwrap();
    let found = backend
        .wait_text(id, "lume-2-sent", Duration::from_secs(10))
        .await
        .unwrap();
    let screen = backend.screen(id).await.unwrap();
    assert_eq!((screen.size.cols(), screen.size.rows()), (100, 30));
    assert!(!screen.alternate);
    assert_eq!(screen.lines.len(), 30);
    for line in ["lume-1-typed", "lume-2-sent"] {
        assert!(
            screen.lines.iter().any(|row| row == line),
            "{line}: {screen:?}"
        );
    }
    assert!(found.line.contains("lume-2-sent"));
    // The cursor stands after the prompt, on the last row that holds anything.
    let last = screen
        .lines
        .iter()
        .rposition(|row| !row.is_empty())
        .unwrap();
    assert_eq!(usize::from(screen.cursor.row), last);
    assert_eq!(
        usize::from(screen.cursor.col),
        screen.lines[last].chars().count()
    );
    println!("the screen as read: {:?}", &screen.lines[..=last]);

    // The view changes size, and the terminal with it.
    view.attachment
        .input
        .resize(TermSize::new(90, 25).unwrap(), CellPx { w: 8, h: 16 })
        .await
        .unwrap();
    view.until("the view's new size", |view| view.sizes == [(90, 25)])
        .await;

    // Another client takes the block and wants another size: the terminal follows it.
    other
        .call(
            "session.own_block",
            json!({ "session_id": session, "block_ids": [block], "mode": "take" }),
        )
        .await
        .unwrap();
    other
        .call(
            "com.superlogical.terminal.resize",
            of_block(json!({ "columns": 70, "rows": 20 })),
        )
        .await
        .unwrap();
    view.until("the other client's size", |view| {
        view.sizes.last() == Some(&(70, 20))
    })
    .await;
    // While it is not the owner, what the view asks for does not change the terminal.
    view.attachment
        .input
        .resize(TermSize::new(95, 28).unwrap(), CellPx { w: 8, h: 16 })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let size = other
        .call("com.superlogical.terminal.size", of_block(json!({})))
        .await
        .unwrap();
    assert_eq!(
        (&size.payload["columns"], &size.payload["rows"]),
        (&json!(70), &json!(20))
    );
    // The view gains the focus: it takes the block, and the terminal has the size the
    // view last asked for.
    wanted.focused = false;
    view.attachment.input.facts(&wanted).await.unwrap();
    wanted.focused = true;
    view.attachment.input.facts(&wanted).await.unwrap();
    view.until("the view's size again", |view| {
        view.sizes.last() == Some(&(95, 28))
    })
    .await;
    println!("the sizes the view was told: {:?}", view.sizes);

    // Detached, the view gives the block up; the other client is next in line.
    view.attachment.input.detach().await.unwrap();
    let size = other
        .call("com.superlogical.terminal.size", of_block(json!({})))
        .await
        .unwrap();
    assert_eq!(size.payload["owner"], created_owner(&other));

    // Ended without a view.
    backend.kill(id).await.unwrap();
    until("the block is gone", || {
        backend
            .view(&session)
            .is_none_or(|view| view.blocks().count() == 0)
    })
    .await;

    let _ = other
        .call("session.destroy", json!({ "session_id": session }))
        .await;
    backend.shutdown();
}

/// The id a server knows a client by.
fn created_owner(control: &Control) -> Value {
    json!(control.welcome().client_id)
}

#[tokio::test]
#[ignore = "needs a Rex server running on this machine"]
async fn two_views_of_a_terminal_of_a_real_server_share_its_stream() {
    let io = dial(&Endpoint::Local)
        .await
        .expect("is a Rex server running?");
    let (other, _events) = Control::open(io, "local", &client(), None).await.unwrap();
    let created = other
        .call(
            "session.create",
            json!({
                "label": "lume-live-shared",
                "initial_windows": [{"layout": {"block": plain_shell()}}],
            }),
        )
        .await
        .unwrap();
    let session = created.payload["session_id"].as_str().unwrap().to_owned();
    // Attached, so that it can destroy the session when all is done.
    other
        .call("session.attach", json!({ "session_id": session }))
        .await
        .unwrap();

    let backend = RexBackend::connect(BackendId("rex".to_owned()), Endpoint::Local, client());
    backend.ready().await.unwrap();
    backend.open_session(&session).await.unwrap();
    let id = backend.list().await.unwrap()[0].id;

    // One view, and something typed in it.
    let mut first = Viewing::new(backend.attach(id, facts()).await.unwrap()).await;
    first
        .attachment
        .input
        .write(echo("1-first").as_bytes())
        .await
        .unwrap();
    first
        .until("the echo", |view| view.shows("lume-1-first\r\n"))
        .await;

    // A second view of the same terminal, without the focus: the server gives this
    // client no second stream, so the one there is begins again, and both views
    // start from the same new copy, which holds what was typed.
    let mut unfocused = facts();
    unfocused.focused = false;
    let second = backend.attach(id, unfocused.clone()).await.unwrap();
    let rows = rows_of(&second.hydrate);
    assert!(rows.iter().any(|row| row == "lume-1-first"), "{rows:?}");
    let mut second = Viewing::new(second).await;
    first
        .until("the copy the first view starts again from", |view| {
            !view.copies.is_empty()
        })
        .await;
    assert_eq!(rows_of(&first.copies[0]), rows);

    // What either types, both are sent.
    second
        .attachment
        .input
        .write(echo("2-second").as_bytes())
        .await
        .unwrap();
    for view in [&mut first, &mut second] {
        view.until("the echo in both", |view| view.shows("lume-2-second\r\n"))
            .await;
    }

    // The view with the focus gives the size. The other's wish counts once it has the
    // focus, without being said again.
    second
        .attachment
        .input
        .resize(TermSize::new(70, 20).unwrap(), CellPx { w: 8, h: 16 })
        .await
        .unwrap();
    first
        .attachment
        .input
        .resize(TermSize::new(110, 35).unwrap(), CellPx { w: 8, h: 16 })
        .await
        .unwrap();
    for view in [&mut first, &mut second] {
        view.until("the first view's size", |view| {
            view.sizes.last() == Some(&(110, 35))
        })
        .await;
    }
    assert!(!second.sizes.contains(&(70, 20)));
    unfocused.focused = true;
    second.attachment.input.facts(&unfocused).await.unwrap();
    for view in [&mut first, &mut second] {
        view.until("the second view's size", |view| {
            view.sizes.last() == Some(&(70, 20))
        })
        .await;
    }

    // One view goes; the other is served on, and the block stays this client's.
    second.attachment.input.detach().await.unwrap();
    first
        .attachment
        .input
        .write(echo("3-alone").as_bytes())
        .await
        .unwrap();
    first
        .until("the echo, alone", |view| view.shows("lume-3-alone\r\n"))
        .await;
    assert_eq!(first.copies.len(), 1);

    // The shell ends, and the view with it.
    first.attachment.input.write(b"exit\r").await.unwrap();
    first.until("the end", |view| view.over).await;

    let _ = other
        .call("session.destroy", json!({ "session_id": session }))
        .await;
    backend.shutdown();
}

/// A view's own terminal: what a view of this application holds, kept the way the
/// page keeps it, in the same engine.
struct Mirror {
    attachment: Attachment,
    terminal: lume_vt::Terminal,
    /// Whether the terminal takes lines back out of scrollback when it grows, as the
    /// attachment says a view's terminal should.
    pulls: bool,
    exit: Option<ExitStatus>,
}

impl Mirror {
    async fn new(attachment: Attachment) -> Self {
        attachment.input.credit(CREDIT_WINDOW).await.unwrap();
        let pulls = attachment.resize_pulls_scrollback;
        let terminal = Self::loaded(&attachment.hydrate, pulls);
        Self {
            attachment,
            terminal,
            pulls,
            exit: None,
        }
    }

    fn loaded(copy: &Hydrate, pulls: bool) -> lume_vt::Terminal {
        match copy {
            Hydrate::Snapshot { bytes, .. } => {
                let mut terminal = lume_vt::Terminal::from_snapshot(bytes).unwrap();
                terminal.set_resize_pulls_scrollback(pulls).unwrap();
                terminal
            }
            other => panic!("not a copy this engine loads: {other:?}"),
        }
    }

    /// Takes frames until none has come for a while.
    async fn settle(&mut self) {
        let quiet = Duration::from_millis(1500);
        while let Ok(frame) = tokio::time::timeout(quiet, self.attachment.output.next()).await {
            match frame {
                Some(OutputFrame::Data(bytes)) => {
                    let count = u32::try_from(bytes.len()).unwrap();
                    self.terminal.write(&bytes);
                    let _ = self.attachment.input.credit(count).await;
                }
                Some(OutputFrame::Resized(size)) => {
                    self.terminal.resize(size.cols(), size.rows()).unwrap();
                }
                Some(OutputFrame::Hydrate(copy)) => {
                    self.terminal = Self::loaded(&copy, self.pulls);
                }
                Some(OutputFrame::Exit(status)) => self.exit = Some(status),
                Some(OutputFrame::Event(_)) => {}
                None => return,
            }
        }
    }

    fn rows(&self) -> Vec<String> {
        self.terminal.screen_text().unwrap()
    }

    /// Everything the terminal holds as text, scrollback first, as a server writes a
    /// terminal out: blank lines at the end left off.
    fn text(&self) -> String {
        let text = self.terminal.format(lume_vt::Format::Plain).unwrap();
        String::from_utf8_lossy(&text.bytes).trim_end().to_owned()
    }
}

/// A terminal's screen and scrollback as the server writes them out as text: what
/// `rex capture --trim` prints.
async fn captured(control: &Control, session: &str, block: &str) -> String {
    let formatted = control
        .call(
            "com.superlogical.terminal.format",
            json!({
                "session_id": session, "block_id": block,
                "args": { "format": "text", "trim": true },
            }),
        )
        .await
        .unwrap();
    formatted.payload["content"]
        .as_str()
        .unwrap()
        .replace("\r\n", "\n")
        .trim_end()
        .to_owned()
}

/// The rows on a terminal's screen as the server itself holds them: read from the
/// copy at the head of a stream that another client of the server opens.
async fn server_rows(
    reach: &lume_rex::Reach,
    control: &Control,
    session: &str,
    block: &str,
) -> Vec<String> {
    let ticket = ticket(control, session, block).await;
    let dialed = reach.dial().await.unwrap();
    let how = DataOpen::Start(&ticket);
    let mut conn = DataConn::open_at(dialed.io, "the server", &dialed.authority, how)
        .await
        .unwrap();
    let mut reading = Reading::new(conn.welcome().nonce);
    reading
        .until(&mut conn, "the server's copy", |reading| {
            !reading.sizes.is_empty()
        })
        .await;
    conn.close().await;
    let copy = reading.snapshot.expect("a copy");
    let terminal = lume_vt::Terminal::from_snapshot(&copy).unwrap();
    terminal.screen_text().unwrap()
}

/// Lines as they are compared: blanks at their ends do not count.
fn lines_of(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.trim_end().to_owned())
        .collect()
}

async fn session_with(control: &Control, label: &str, block: Value) -> (String, String) {
    let created = control
        .call(
            "session.create",
            json!({ "label": label, "initial_windows": [{"layout": {"block": block}}] }),
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
    (session, block)
}

#[tokio::test]
#[ignore = "needs a Rex server running on this machine, and vim"]
async fn a_full_screen_program_is_shown_as_the_server_shows_it() {
    let vim = if cfg!(windows) {
        r"C:\Program Files\Git\usr\bin\vim.exe"
    } else {
        "vim"
    };
    let io = dial(&Endpoint::Local)
        .await
        .expect("is a Rex server running?");
    let (other, _events) = Control::open(io, "local", &client(), None).await.unwrap();
    // An editor that reads and writes nothing of the user's.
    let editor = json!({
        "flavor": "com.superlogical.terminal.shell",
        "options": { "shell": "none", "command": [vim, "-i", "NONE", "-u", "NONE"] },
    });
    let (session, block) = session_with(&other, "lume-live-screen", editor).await;

    let backend = RexBackend::connect(BackendId("rex".to_owned()), Endpoint::Local, client());
    backend.ready().await.unwrap();
    backend.open_session(&session).await.unwrap();
    let id = backend.list().await.unwrap()[0].id;

    let mut wanted = facts();
    wanted.size = TermSize::new(100, 30).unwrap();
    let mut view = Mirror::new(backend.attach(id, wanted).await.unwrap()).await;
    view.attachment
        .input
        .write(b"iten columns of text, and a long line: ")
        .await
        .unwrap();
    view.attachment
        .input
        .write("wörter ".repeat(30).as_bytes())
        .await
        .unwrap();
    view.attachment.input.write(b"\x1b").await.unwrap();
    view.settle().await;

    for size in [(100, 30), (120, 40), (60, 15)] {
        if (view.terminal.cols(), view.terminal.rows()) != size {
            view.attachment
                .input
                .resize(
                    TermSize::new(size.0, size.1).unwrap(),
                    CellPx { w: 8, h: 16 },
                )
                .await
                .unwrap();
            view.settle().await;
        }
        assert_eq!((view.terminal.cols(), view.terminal.rows()), size);
        // The view's own terminal, the screen read without a view, and what the
        // server writes out all show the same.
        let screen = backend.screen(id).await.unwrap();
        let server = captured(&other, &session, &block).await;
        let shown = lines_of(&view.rows().join("\n"));
        let shown: Vec<String> = shown
            .into_iter()
            .rev()
            .skip_while(String::is_empty)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        assert!(view.terminal.alternate_screen().unwrap());
        assert!(screen.alternate, "{size:?}");
        assert_eq!((screen.size.cols(), screen.size.rows()), size);
        assert_eq!(shown, lines_of(&server), "the view at {size:?}");
        assert_eq!(view.rows(), screen.lines, "the screen read at {size:?}");
        assert!(shown[0].starts_with("ten columns of text"), "{shown:?}");
        let cursor = view.terminal.cursor().unwrap();
        assert_eq!((cursor.x, cursor.y), (screen.cursor.col, screen.cursor.row));
    }

    view.attachment.input.write(b":q!\r").await.unwrap();
    view.settle().await;
    println!("the editor's end was told as {:?}", view.exit);
    let _ = other
        .call("session.destroy", json!({ "session_id": session }))
        .await;
    backend.shutdown();
}

#[tokio::test]
#[ignore = "needs a Rex server running on this machine"]
async fn a_shell_with_scrollback_is_shown_as_the_server_shows_it_at_every_size() {
    scrollback_compared(Endpoint::Local, lume_rex::Options::new()).await;
}

#[tokio::test]
#[ignore = "needs a Rex server on a machine the SSH client logs in to, named in LUME_REX_LIVE_SSH"]
async fn a_shell_with_scrollback_on_another_machine_is_shown_as_its_server_shows_it() {
    let Ok(address) = std::env::var(LIVE_SSH) else {
        eprintln!("skipped: {LIVE_SSH} is not set");
        return;
    };
    let mut ssh = lume_rex::SshOptions::default();
    if let Ok(config) = std::env::var(LIVE_SSH_CONFIG) {
        ssh.args = vec!["-F".to_owned(), config];
    }
    let options = lume_rex::Options::new().ssh(ssh);
    scrollback_compared(Endpoint::parse(&address).unwrap(), options).await;
}

/// Fills a terminal of the server at `endpoint` past its screen, and holds a view's own
/// terminal against the server's at one size after another.
async fn scrollback_compared(endpoint: Endpoint, options: lume_rex::Options) {
    let backend = RexBackend::connect_with(
        BackendId("rex".to_owned()),
        endpoint.clone(),
        client(),
        options.clone(),
    );
    let version = tokio::time::timeout(Duration::from_secs(90), backend.ready())
        .await
        .expect("is a Rex server running?")
        .unwrap();
    let windows = version.platform.starts_with("windows");

    let reach = lume_rex::Reach::new(endpoint.clone(), options);
    let dialed = reach.dial().await.unwrap();
    let name = endpoint.to_string();
    let (other, _events) = Control::open_at(dialed.io, &name, &dialed.authority, &client(), None)
        .await
        .unwrap();
    // A shell of the server's machine, with nothing of the user's in it.
    let command = if windows {
        json!(["cmd.exe", "/d"])
    } else {
        json!(["/bin/sh"])
    };
    let shell = json!({
        "flavor": "com.superlogical.terminal.shell",
        "options": { "shell": "none", "command": command },
    });
    let (session, block) = session_with(&other, "lume-live-scroll", shell).await;

    until("the session is listed", || {
        backend.sessions().iter().any(|listed| listed.id == session)
    })
    .await;
    backend.open_session(&session).await.unwrap();
    let id = backend.list().await.unwrap()[0].id;

    let mut view = Mirror::new(backend.attach(id, facts()).await.unwrap()).await;
    println!(
        "a server on {}; a view's terminal takes lines back out of scrollback when it grows: {}",
        version.platform, view.pulls
    );
    // More lines than the screen has rows, some of them longer than it is wide.
    let lines = if windows {
        "for /L %i in (1,1,60) do @echo line %i of sixty, with enough after it to be longer than a narrow terminal is wide, so that it wraps\r"
    } else {
        "for i in $(seq 1 60); do echo line $i of sixty, with enough after it to be longer than a narrow terminal is wide, so that it wraps; done\r"
    };
    view.attachment.input.write(lines.as_bytes()).await.unwrap();
    view.settle().await;

    // Taller, shorter, wider, narrower: each time the view's own terminal holds what
    // the server's does, scrollback and all.
    let mut differing = Vec::new();
    for size in [(80, 24), (80, 40), (80, 12), (140, 30), (50, 30), (80, 24)] {
        if (view.terminal.cols(), view.terminal.rows()) != size {
            view.attachment
                .input
                .resize(
                    TermSize::new(size.0, size.1).unwrap(),
                    CellPx { w: 8, h: 16 },
                )
                .await
                .unwrap();
            view.settle().await;
        }
        assert_eq!((view.terminal.cols(), view.terminal.rows()), size);
        let screen = backend.screen(id).await.unwrap();
        let server = lines_of(&captured(&other, &session, &block).await);
        let shown = lines_of(&view.text());
        let same_text = shown == server;
        // The server's own terminal is what counts; the screen read without a view
        // has to agree with it as well.
        let held = server_rows(&reach, &other, &session, &block).await;
        assert_eq!(screen.lines, held, "the screen read at {size:?}");
        let same_screen = view.rows() == held;
        println!(
            "{size:?}: all the text {}, the screen {} ({} lines here, {} there)",
            if same_text { "the same" } else { "DIFFERS" },
            if same_screen { "the same" } else { "DIFFERS" },
            shown.len(),
            server.len(),
        );
        if !same_text || !same_screen {
            differing.push((size, view.rows(), held));
        }
    }
    for (size, here, there) in &differing {
        println!("at {size:?}\n  the view:   {here:?}\n  the server: {there:?}");
    }
    assert!(differing.is_empty(), "{} sizes differ", differing.len());

    let _ = other
        .call("session.destroy", json!({ "session_id": session }))
        .await;
    backend.shutdown();
}

/// Waits until the layout is as `wanted` says, and gives it.
async fn laid_out(
    layout: &dyn LayoutAuthority,
    what: &str,
    wanted: impl Fn(&Workspace) -> bool,
) -> Workspace {
    let waiting = async {
        loop {
            if let Ok(snapshot) = layout.view().await
                && wanted(&snapshot.workspace)
            {
                return snapshot.workspace;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    match tokio::time::timeout(Duration::from_secs(20), waiting).await {
        Ok(workspace) => workspace,
        Err(_) => panic!(
            "{what} never showed in the layout: {:#?}",
            layout.view().await
        ),
    }
}

#[tokio::test]
#[ignore = "needs a Rex server running on this machine"]
async fn the_layout_of_a_real_session_follows_what_another_client_does() {
    let io = dial(&Endpoint::Local)
        .await
        .expect("is a Rex server running?");
    let (other, _events) = Control::open(io, "local", &client(), None).await.unwrap();
    let (session, first) = session_with(&other, "lume-live-layout", plain_shell()).await;
    let of_session = |mut args: Value| {
        args["session_id"] = json!(session);
        args
    };

    let backend = RexBackend::connect(BackendId("rex".to_owned()), Endpoint::Local, client());
    backend.ready().await.unwrap();
    until("the session is known", || {
        backend.sessions().iter().any(|known| known.id == session)
    })
    .await;
    let layout = backend.layout().unwrap();
    let mut changes = layout.changes();
    backend.show_session(&session).await.unwrap();

    // One window with one terminal: one tab with one pane, which is a session of the
    // backend.
    let shown = layout.view().await.unwrap().workspace;
    validate(&shown).unwrap();
    assert_eq!(shown.name, "lume-live-layout");
    let tab = &shown.windows[0].centre.tabs[0];
    let Node::Pane(pane) = &tab.root else {
        panic!("one block is one pane: {tab:#?}");
    };
    let first_pane = pane.id;
    assert_eq!(shown.windows[0].focus, Some(first_pane));
    assert_eq!(shown.sessions().len(), 1);
    assert_eq!(backend.list().await.unwrap()[0].id, shown.sessions()[0].1);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(20), changes.next()).await,
        Ok(Some(LayoutEvent::Reset(_)))
    ));

    // Two splits of the same direction, the second inside the first: one row of three.
    let shell = || json!({ "block": plain_shell() });
    let split = other
        .call(
            "session.new_split",
            of_session(json!({
                "anchor_id": first, "direction": "horizontal", "side": "after",
                "layout": shell(), "ratio": 0.25,
            })),
        )
        .await
        .unwrap();
    let second = split.payload["block_ids"][0].as_str().unwrap().to_owned();
    let shown = laid_out(&*layout, "the split", |workspace| {
        workspace.windows[0].centre.tabs[0].pane_ids().len() == 2
    })
    .await;
    let Node::Split(row) = &shown.windows[0].centre.tabs[0].root else {
        panic!("two blocks are a split");
    };
    println!(
        "a split asked for with ratio 0.25 has the shares {:?}",
        row.ratios
    );
    assert_eq!(row.axis, Axis::Row);
    assert_eq!(row.children[0].pane_ids(), [first_pane]);
    let row_id = row.id;

    let split = other
        .call(
            "session.new_split",
            of_session(json!({
                "anchor_id": second, "direction": "horizontal", "side": "after",
                "layout": shell(),
            })),
        )
        .await
        .unwrap();
    let third = split.payload["block_ids"][0].as_str().unwrap().to_owned();
    let shown = laid_out(&*layout, "the second split", |workspace| {
        workspace.windows[0].centre.tabs[0].pane_ids().len() == 3
    })
    .await;
    validate(&shown).unwrap();
    assert!(is_normalised(&shown));
    let Node::Split(row) = &shown.windows[0].centre.tabs[0].root else {
        panic!("three blocks in a row are a split");
    };
    assert_eq!(row.children.len(), 3, "{row:#?}");
    assert_eq!(row.ratios.iter().sum::<u32>(), UNITS);
    // The split is the one it was, and so is the first pane.
    assert_eq!(row.id, row_id);
    let panes = shown.windows[0].centre.tabs[0].pane_ids();
    assert_eq!(panes[0], first_pane);
    let (second_pane, third_pane) = (panes[1], panes[2]);
    assert_eq!(shown.sessions().len(), 3);

    // A split the other way stays a split of its own.
    other
        .call(
            "session.new_split",
            of_session(json!({
                "anchor_id": third, "direction": "vertical", "side": "before",
                "layout": shell(),
            })),
        )
        .await
        .unwrap();
    let shown = laid_out(&*layout, "the split the other way", |workspace| {
        workspace.windows[0].centre.tabs[0].pane_ids().len() == 4
    })
    .await;
    assert!(is_normalised(&shown));
    let Node::Split(row) = &shown.windows[0].centre.tabs[0].root else {
        panic!("a split");
    };
    let Node::Split(column) = &row.children[2] else {
        panic!("the third child is a split of its own: {row:#?}");
    };
    assert_eq!(column.axis, Axis::Column);
    assert_eq!(column.children[1].pane_ids(), [third_pane]);

    // Names: the window's, and a block's.
    let window = other
        .call("session.view", of_session(json!({})))
        .await
        .unwrap()
        .payload["windows"][0]["window_id"]
        .as_str()
        .unwrap()
        .to_owned();
    other
        .call(
            "session.set_window_label",
            of_session(json!({ "window_id": window, "label": "lume-live-tab" })),
        )
        .await
        .unwrap();
    other
        .call(
            "session.set_block_label",
            of_session(json!({ "block_id": second, "label": "logs" })),
        )
        .await
        .unwrap();
    let shown = laid_out(&*layout, "the names", |workspace| {
        workspace.windows[0].centre.tabs[0].title.as_deref() == Some("lume-live-tab")
            && workspace.pane(second_pane).unwrap().title.as_deref() == Some("logs")
    })
    .await;
    assert_eq!(shown.pane(first_pane).unwrap().title, None);

    // Focus and zoom.
    other
        .call(
            "session.focus_block",
            of_session(json!({ "block_id": second })),
        )
        .await
        .unwrap();
    laid_out(&*layout, "the focus", |workspace| {
        workspace.windows[0].focus == Some(second_pane)
    })
    .await;
    other
        .call(
            "session.zoom_block",
            of_session(json!({ "block_id": second, "zoom": true })),
        )
        .await
        .unwrap();
    laid_out(&*layout, "the zoom", |workspace| {
        workspace.windows[0].centre.tabs[0].zoomed == Some(second_pane)
    })
    .await;
    other
        .call(
            "session.zoom_block",
            of_session(json!({ "block_id": second, "zoom": false })),
        )
        .await
        .unwrap();
    laid_out(&*layout, "the end of the zoom", |workspace| {
        workspace.windows[0].centre.tabs[0].zoomed.is_none()
    })
    .await;

    // A block floats, is put away and brought back.
    let floated = other
        .call(
            "session.float_block",
            of_session(json!({ "block_id": second, "bounds": [0.25, 0.125, 0.5, 0.5] })),
        )
        .await
        .unwrap();
    let layer = floated.payload["layer_id"].as_str().unwrap().to_owned();
    let shown = laid_out(&*layout, "the floating block", |workspace| {
        workspace.windows[0].floating.len() == 1
    })
    .await;
    validate(&shown).unwrap();
    let floating = &shown.windows[0].floating[0];
    assert_eq!(floating.pane.id, second_pane);
    assert_eq!(
        (
            floating.rect.x,
            floating.rect.y,
            floating.rect.w,
            floating.rect.h
        ),
        (UNITS / 4, UNITS / 8, UNITS / 2, UNITS / 2)
    );
    assert_eq!(floating.minimised, None);
    assert_eq!(shown.windows[0].centre.tabs[0].pane_ids().len(), 3);
    other
        .call(
            "session.set_layer_state",
            of_session(json!({ "layer_id": layer, "state": "minimized" })),
        )
        .await
        .unwrap();
    let shown = laid_out(&*layout, "the floating block put away", |workspace| {
        workspace.windows[0].floating[0].minimised == Some(Minimised::Floating)
    })
    .await;
    validate(&shown).unwrap();
    assert_ne!(shown.windows[0].focus, Some(second_pane));

    // A second window is a second tab, on show; what floats over the first is not
    // shown with it.
    let opened = other
        .call(
            "session.new_window",
            of_session(json!({ "layout": shell(), "window_label": "lume-live-second" })),
        )
        .await
        .unwrap();
    let new_window = opened.payload["window_id"].as_str().unwrap().to_owned();
    let shown = laid_out(&*layout, "the second window", |workspace| {
        workspace.windows[0].centre.tabs.len() == 2
    })
    .await;
    validate(&shown).unwrap();
    let centre = &shown.windows[0].centre;
    assert_eq!(centre.active, Some(centre.tabs[1].id));
    assert_eq!(centre.tabs[1].title.as_deref(), Some("lume-live-second"));
    assert!(shown.windows[0].floating.is_empty());
    assert_eq!(shown.windows.len(), 1);
    let first_tab = centre.tabs[0].id;

    // The tabs change places, and the first is shown again with what floats over it.
    other
        .call(
            "session.move_window",
            of_session(json!({ "window_id": new_window, "before_window_id": window })),
        )
        .await
        .unwrap();
    other
        .call(
            "session.focus_window",
            of_session(json!({ "window_id": window })),
        )
        .await
        .unwrap();
    let shown = laid_out(&*layout, "the first window on show again", |workspace| {
        let centre = &workspace.windows[0].centre;
        centre.tabs[1].id == first_tab && centre.active == Some(first_tab)
    })
    .await;
    assert_eq!(shown.windows[0].floating.len(), 1);

    other
        .call(
            "session.close_window",
            of_session(json!({ "window_id": new_window })),
        )
        .await
        .unwrap();
    let shown = laid_out(&*layout, "the second window closed", |workspace| {
        workspace.windows[0].centre.tabs.len() == 1
    })
    .await;
    validate(&shown).unwrap();
    // After all of it the first pane is the one it was, showing the session it showed.
    assert_eq!(shown.windows[0].centre.tabs[0].id, first_tab);
    assert_eq!(shown.windows[0].centre.tabs[0].pane_ids()[0], first_pane);

    // Every step came as a step of the one layout, none as a new one.
    let mut steps = 0;
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(200), changes.next()).await
    {
        assert!(matches!(event, LayoutEvent::Changed { .. }), "{event:?}");
        steps += 1;
    }
    assert!(steps >= 10, "only {steps} steps were reported");

    // The session is destroyed: the empty window, whole.
    other
        .call("session.destroy", of_session(json!({})))
        .await
        .unwrap();
    let shown = laid_out(&*layout, "the session gone", |workspace| {
        workspace.windows[0].centre.tabs.is_empty()
    })
    .await;
    assert!(shown.windows[0].floating.is_empty());
    assert_eq!(backend.shown_session(), None);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(20), changes.next()).await,
        Ok(Some(LayoutEvent::Reset(_)))
    ));
    backend.shutdown();
}

/// A session's view as another client reads it.
async fn seen(other: &Control, session: &str) -> Value {
    other
        .call("session.view", json!({ "session_id": session }))
        .await
        .unwrap()
        .payload
}

/// The blocks of the first window's layout as another client sees them, in the order
/// they are laid out.
async fn seen_blocks(other: &Control, session: &str) -> Vec<String> {
    let view = seen(other, session).await;
    let mut blocks = Vec::new();
    common::views::blocks_in(&view["windows"][0]["layers"][0]["layout"], &mut blocks);
    blocks
}

/// A new terminal pane that runs a plain shell of the server's machine.
fn plain_pane() -> NewContent {
    let (program, args) = if cfg!(windows) {
        ("cmd.exe", vec!["/d".to_owned()])
    } else {
        ("/bin/sh", Vec::new())
    };
    NewContent::Terminal(SpawnRequest {
        launcher: Some(Launcher::Shell {
            program: program.to_owned(),
            args,
            login: false,
        }),
        ..SpawnRequest::default()
    })
}

fn close_to(value: &Value, wanted: f64) -> bool {
    (value.as_f64().unwrap() - wanted).abs() < 1e-4
}

#[tokio::test]
#[ignore = "needs a Rex server running on this machine"]
async fn what_is_asked_of_a_real_sessions_layout_is_seen_by_another_client() {
    let io = dial(&Endpoint::Local)
        .await
        .expect("is a Rex server running?");
    let (other, _events) = Control::open(io, "local", &client(), None).await.unwrap();
    let (session, first_block) = session_with(&other, "lume-live-apply", plain_shell()).await;

    let backend = RexBackend::connect(BackendId("rex".to_owned()), Endpoint::Local, client());
    backend.ready().await.unwrap();
    until("the session is known", || {
        backend.sessions().iter().any(|known| known.id == session)
    })
    .await;
    backend.show_session(&session).await.unwrap();
    let layout = backend.layout().unwrap();
    let window = WindowId::new(1);
    // The layout as it is the moment an answer came: nothing is waited for.
    let now = || async { layout.view().await.unwrap().workspace };
    let panes = || async { now().await.windows[0].centre.tabs[0].pane_ids() };
    let first = panes().await[0];

    // A split: the new pane is in the layout when the answer comes, and has the
    // quarter it was to have.
    let applied = layout
        .apply(Intent::Split {
            pane: first,
            dir: Dir::Right,
            ratio: UNITS / 4,
            content: plain_pane(),
        })
        .await
        .unwrap();
    let second = applied.created.pane.expect("the new pane");
    assert_eq!(panes().await, [first, second]);
    let shown = layout.view().await.unwrap();
    assert_eq!(applied.revision, shown.revision);
    let view = seen(&other, &session).await;
    let split = &view["windows"][0]["layers"][0]["layout"]["split"];
    assert_eq!(split["direction"], "horizontal");
    assert!(close_to(&split["ratio"], 0.75), "{split}");
    assert_eq!(split["before"]["block_id"], first_block.as_str());
    let blocks = seen_blocks(&other, &session).await;
    let second_block = blocks[1].clone();
    // A split takes the keyboard to the new pane.
    assert_eq!(
        view["windows"][0]["focused_block_id"],
        second_block.as_str()
    );
    assert_eq!(shown.workspace.windows[0].focus, Some(second));
    // The new pane is a terminal that can be attached to.
    let listed = backend.list().await.unwrap();
    assert_eq!(listed.len(), 2);

    // A second split to the right of the second pane: one row of three here.
    let applied = layout
        .apply(Intent::Split {
            pane: second,
            dir: Dir::Right,
            ratio: UNITS / 2,
            content: plain_pane(),
        })
        .await
        .unwrap();
    let third = applied.created.pane.unwrap();
    assert_eq!(panes().await, [first, second, third]);
    let Node::Split(row) = now().await.windows[0].centre.tabs[0].root.clone() else {
        panic!("a row");
    };
    assert_eq!(row.children.len(), 3);
    let third_block = seen_blocks(&other, &session).await[2].clone();

    // The row is resized: both of the server's splits move.
    let tenth = UNITS / 10;
    layout
        .apply(Intent::Resize {
            split: row.id,
            ratios: vec![2 * tenth, 3 * tenth, 5 * tenth],
        })
        .await
        .unwrap();
    let view = seen(&other, &session).await;
    let outer = &view["windows"][0]["layers"][0]["layout"]["split"];
    assert!(close_to(&outer["ratio"], 0.2), "{outer}");
    assert!(
        close_to(&outer["after"]["split"]["ratio"], 0.375),
        "{outer}"
    );
    let Node::Split(row) = now().await.windows[0].centre.tabs[0].root.clone() else {
        panic!("a row");
    };
    for (share, wanted) in row.ratios.iter().zip([2 * tenth, 3 * tenth, 5 * tenth]) {
        assert!(share.abs_diff(wanted) <= 2, "{:?}", row.ratios);
    }
    // Only the inner border: the outer split is left as it is.
    layout
        .apply(Intent::Resize {
            split: row.id,
            ratios: vec![2 * tenth, 4 * tenth, 4 * tenth],
        })
        .await
        .unwrap();
    let view = seen(&other, &session).await;
    let outer = &view["windows"][0]["layers"][0]["layout"]["split"];
    assert!(close_to(&outer["ratio"], 0.2), "{outer}");
    assert!(close_to(&outer["after"]["split"]["ratio"], 0.5), "{outer}");

    // Focus: a pane, a direction, the next and the one before.
    let focused = || async {
        seen(&other, &session).await["windows"][0]["focused_block_id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    layout
        .apply(Intent::Focus {
            target: FocusTarget::Pane { pane: first },
        })
        .await
        .unwrap();
    assert_eq!(focused().await, first_block);
    assert_eq!(now().await.windows[0].focus, Some(first));
    layout
        .apply(Intent::FocusDir {
            window,
            dir: Dir::Right,
        })
        .await
        .unwrap();
    assert_eq!(focused().await, second_block);
    layout
        .apply(Intent::Focus {
            target: FocusTarget::Next { window },
        })
        .await
        .unwrap();
    assert_eq!(focused().await, third_block);
    layout
        .apply(Intent::Focus {
            target: FocusTarget::Next { window },
        })
        .await
        .unwrap();
    assert_eq!(
        focused().await,
        first_block,
        "the next after the last is the first"
    );
    layout
        .apply(Intent::Focus {
            target: FocusTarget::Previous { window },
        })
        .await
        .unwrap();
    assert_eq!(focused().await, third_block);
    assert_eq!(now().await.windows[0].focus, Some(third));

    // Zoom, on and off.
    layout
        .apply(Intent::Zoom {
            pane: second,
            on: true,
        })
        .await
        .unwrap();
    let view = seen(&other, &session).await;
    assert_eq!(view["windows"][0]["zoomed_block_id"], second_block.as_str());
    assert_eq!(now().await.windows[0].centre.tabs[0].zoomed, Some(second));
    layout
        .apply(Intent::Zoom {
            pane: second,
            on: false,
        })
        .await
        .unwrap();
    let view = seen(&other, &session).await;
    assert!(view["windows"][0]["zoomed_block_id"].is_null(), "{view}");
    assert_eq!(now().await.windows[0].centre.tabs[0].zoomed, None);

    // A name, and no name.
    let label_of = |view: &Value, block: &str| {
        view["windows"][0]["layers"][0]["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|listed| listed["block_id"] == block)
            .unwrap()["label"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    layout
        .apply(Intent::PaneRename {
            pane: second,
            title: Some("logs".to_owned()),
        })
        .await
        .unwrap();
    assert_eq!(
        label_of(&seen(&other, &session).await, &second_block),
        "logs"
    );
    assert_eq!(
        now().await.pane(second).unwrap().title.as_deref(),
        Some("logs")
    );
    layout
        .apply(Intent::PaneRename {
            pane: second,
            title: None,
        })
        .await
        .unwrap();
    let label = label_of(&seen(&other, &session).await, &second_block);
    println!("a block whose name is taken away is called {label:?} by the server");
    assert_eq!(now().await.pane(second).unwrap().title, None, "{label}");

    // Two panes change places; everything else stays.
    layout
        .apply(Intent::Swap { a: first, b: third })
        .await
        .unwrap();
    assert_eq!(
        seen_blocks(&other, &session).await,
        [
            third_block.clone(),
            second_block.clone(),
            first_block.clone()
        ]
    );
    assert_eq!(panes().await, [third, second, first]);

    // A pane is moved below another.
    layout
        .apply(Intent::Move {
            pane: third,
            to: Place::Beside {
                pane: first,
                edge: Dir::Down,
            },
        })
        .await
        .unwrap();
    assert_eq!(
        seen_blocks(&other, &session).await,
        [
            second_block.clone(),
            first_block.clone(),
            third_block.clone()
        ]
    );
    let shown = now().await;
    validate(&shown).unwrap();
    assert!(is_normalised(&shown));
    let Node::Split(row) = &shown.windows[0].centre.tabs[0].root else {
        panic!("a row");
    };
    let Node::Split(column) = &row.children[1] else {
        panic!("the moved pane is below the other: {row:#?}");
    };
    assert_eq!(column.axis, Axis::Column);
    assert_eq!(column.children[1].pane_ids(), [third]);

    // What the server will not do is said as it says it, and nothing is changed: a
    // program that is not there is not run.
    let before = layout.view().await.unwrap();
    let refused = layout
        .apply(Intent::Split {
            pane: first,
            dir: Dir::Right,
            ratio: UNITS / 2,
            content: NewContent::Terminal(SpawnRequest {
                launcher: Some(Launcher::Shell {
                    program: "lume-live-no-such-program".to_owned(),
                    args: Vec::new(),
                    login: false,
                }),
                ..SpawnRequest::default()
            }),
        })
        .await
        .unwrap_err();
    println!("a split that runs a program that is not there: {refused}");
    assert!(
        matches!(refused, LayoutError::Refused { .. }),
        "{refused:?}"
    );
    assert!(refused.to_string().contains("lume-live-no-such-program"));
    assert_eq!(layout.view().await.unwrap(), before);

    // A pane is closed: its block is gone for everyone.
    let before = seen_blocks(&other, &session).await;
    layout.apply(Intent::Close { pane: third }).await.unwrap();
    let after = seen_blocks(&other, &session).await;
    assert_eq!(after.len(), before.len() - 1);
    assert!(!after.contains(&third_block));
    assert!(!panes().await.contains(&third));

    other
        .call("session.destroy", json!({ "session_id": session }))
        .await
        .unwrap();
    backend.shutdown();
}

/// The ids of a session's windows as another client sees them, in tab order.
fn windows_of(view: &Value) -> Vec<String> {
    view["windows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|window| window["window_id"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
#[ignore = "needs a Rex server running on this machine"]
async fn tabs_and_floating_panes_of_a_real_session_are_changed_from_here() {
    let io = dial(&Endpoint::Local)
        .await
        .expect("is a Rex server running?");
    let (other, _events) = Control::open(io, "local", &client(), None).await.unwrap();
    let (session, first_block) = session_with(&other, "lume-live-tabs", plain_shell()).await;

    let backend = RexBackend::connect(BackendId("rex".to_owned()), Endpoint::Local, client());
    backend.ready().await.unwrap();
    until("the session is known", || {
        backend.sessions().iter().any(|known| known.id == session)
    })
    .await;
    backend.show_session(&session).await.unwrap();
    let layout = backend.layout().unwrap();
    let window = WindowId::new(1);
    let now = || async { layout.view().await.unwrap().workspace };
    let first_tab = now().await.windows[0].centre.tabs[0].id;
    let first = now().await.windows[0].centre.tabs[0].first_pane();
    let first_window = windows_of(&seen(&other, &session).await)[0].clone();
    let new_tab = |index: Option<u32>, activate: bool| Intent::TabNew {
        window,
        content: plain_pane(),
        index,
        activate,
        group: None,
    };

    // A new tab, on show: the answer has its number and its pane's.
    let applied = layout.apply(new_tab(None, true)).await.unwrap();
    let second_tab = applied.created.tab.expect("the new tab");
    let shown = now().await;
    validate(&shown).unwrap();
    let centre = &shown.windows[0].centre;
    assert_eq!(centre.tabs.len(), 2);
    assert_eq!(centre.tabs[1].id, second_tab);
    assert_eq!(centre.active, Some(second_tab));
    assert_eq!(applied.created.pane, Some(centre.tabs[1].first_pane()));
    let view = seen(&other, &session).await;
    let second_window = windows_of(&view)[1].clone();
    assert_eq!(view["active_window_id"], second_window.as_str());

    // Its name, and no name.
    layout
        .apply(Intent::TabRename {
            tab: second_tab,
            title: Some("build".to_owned()),
        })
        .await
        .unwrap();
    assert_eq!(seen(&other, &session).await["windows"][1]["label"], "build");
    assert_eq!(
        now().await.windows[0].centre.tabs[1].title.as_deref(),
        Some("build")
    );
    layout
        .apply(Intent::TabRename {
            tab: second_tab,
            title: None,
        })
        .await
        .unwrap();
    assert_eq!(now().await.windows[0].centre.tabs[1].title, None);

    // A new tab at the front that is not put on show.
    let applied = layout.apply(new_tab(Some(0), false)).await.unwrap();
    let third_tab = applied.created.tab.unwrap();
    let shown = now().await;
    let order: Vec<_> = shown.windows[0]
        .centre
        .tabs
        .iter()
        .map(|tab| tab.id)
        .collect();
    assert_eq!(order, [third_tab, first_tab, second_tab]);
    assert_eq!(shown.windows[0].centre.active, Some(second_tab));
    let view = seen(&other, &session).await;
    let third_window = windows_of(&view)[0].clone();
    assert_eq!(
        windows_of(&view),
        [
            third_window.clone(),
            first_window.clone(),
            second_window.clone()
        ]
    );
    assert_eq!(view["active_window_id"], second_window.as_str());

    // It is moved to the end, and then between the two others.
    layout
        .apply(Intent::TabMove {
            tab: third_tab,
            index: 2,
        })
        .await
        .unwrap();
    assert_eq!(
        windows_of(&seen(&other, &session).await),
        [
            first_window.clone(),
            second_window.clone(),
            third_window.clone()
        ]
    );
    layout
        .apply(Intent::TabMove {
            tab: third_tab,
            index: 1,
        })
        .await
        .unwrap();
    assert_eq!(
        windows_of(&seen(&other, &session).await),
        [
            first_window.clone(),
            third_window.clone(),
            second_window.clone()
        ]
    );
    let order: Vec<_> = now().await.windows[0]
        .centre
        .tabs
        .iter()
        .map(|tab| tab.id)
        .collect();
    assert_eq!(order, [first_tab, third_tab, second_tab]);

    // The tab on show: by number, by place, the next and the one before.
    let active = || async {
        seen(&other, &session).await["active_window_id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let activate = |target: TabTarget| Intent::TabActivate { target };
    layout
        .apply(activate(TabTarget::Id { tab: first_tab }))
        .await
        .unwrap();
    assert_eq!(active().await, first_window);
    assert_eq!(now().await.windows[0].centre.active, Some(first_tab));
    layout
        .apply(activate(TabTarget::Previous { window }))
        .await
        .unwrap();
    assert_eq!(
        active().await,
        second_window,
        "before the first is the last"
    );
    layout
        .apply(activate(TabTarget::Next { window }))
        .await
        .unwrap();
    assert_eq!(active().await, first_window);
    layout
        .apply(activate(TabTarget::Index { window, index: 1 }))
        .await
        .unwrap();
    assert_eq!(active().await, third_window);

    // Tabs are closed: one, and then the other with its tab on show.
    layout
        .apply(Intent::TabClose { tab: third_tab })
        .await
        .unwrap();
    assert_eq!(
        windows_of(&seen(&other, &session).await),
        [first_window.clone(), second_window.clone()]
    );
    layout
        .apply(Intent::TabsClose {
            tabs: vec![second_tab],
        })
        .await
        .unwrap();
    let shown = now().await;
    validate(&shown).unwrap();
    assert_eq!(shown.windows[0].centre.tabs.len(), 1);
    assert_eq!(shown.windows[0].centre.active, Some(first_tab));
    assert_eq!(
        windows_of(&seen(&other, &session).await),
        std::slice::from_ref(&first_window)
    );

    // A second pane, which floats in the middle when it is not told where.
    let applied = layout
        .apply(Intent::Split {
            pane: first,
            dir: Dir::Right,
            ratio: UNITS / 2,
            content: plain_pane(),
        })
        .await
        .unwrap();
    let second = applied.created.pane.unwrap();
    layout
        .apply(Intent::Float {
            pane: second,
            rect: None,
        })
        .await
        .unwrap();
    let shown = now().await;
    validate(&shown).unwrap();
    assert_eq!(shown.windows[0].centre.tabs[0].pane_ids(), [first]);
    let floating = &shown.windows[0].floating[0];
    assert_eq!(floating.pane.id, second);
    let fifth = UNITS / 5;
    assert_eq!(
        (
            floating.rect.x,
            floating.rect.y,
            floating.rect.w,
            floating.rect.h
        ),
        (fifth, fifth, 3 * fifth, 3 * fifth)
    );
    let layers = |view: &Value| view["windows"][0]["layers"].as_array().unwrap().clone();
    let floated = layers(&seen(&other, &session).await);
    assert_eq!(floated.len(), 2);
    assert_eq!(floated[1]["kind"], "floating");
    for (bound, wanted) in floated[1]["bounds"]
        .as_array()
        .unwrap()
        .iter()
        .zip([0.2, 0.2, 0.6, 0.6])
    {
        assert!(close_to(bound, wanted), "{}", floated[1]["bounds"]);
    }

    // It is given another place.
    let eighth = UNITS / 8;
    layout
        .apply(Intent::FloatMove {
            pane: second,
            rect: UnitRect {
                x: 2 * eighth,
                y: eighth,
                w: 4 * eighth,
                h: 6 * eighth,
            },
        })
        .await
        .unwrap();
    let moved = layers(&seen(&other, &session).await);
    for (bound, wanted) in moved[1]["bounds"]
        .as_array()
        .unwrap()
        .iter()
        .zip([0.25, 0.125, 0.5, 0.75])
    {
        assert!(close_to(bound, wanted), "{}", moved[1]["bounds"]);
    }
    assert_eq!(now().await.windows[0].floating[0].rect.h, 6 * eighth);

    // Put away and brought back, with the keyboard.
    layout
        .apply(Intent::Minimise {
            pane: second,
            on: true,
        })
        .await
        .unwrap();
    assert_eq!(
        layers(&seen(&other, &session).await)[1]["state"],
        "minimized"
    );
    let shown = now().await;
    assert_eq!(
        shown.windows[0].floating[0].minimised,
        Some(Minimised::Floating)
    );
    assert_eq!(shown.windows[0].focus, Some(first));
    layout
        .apply(Intent::Minimise {
            pane: second,
            on: false,
        })
        .await
        .unwrap();
    assert!(layers(&seen(&other, &session).await)[1]["state"].is_null());
    let shown = now().await;
    assert_eq!(shown.windows[0].floating[0].minimised, None);
    assert_eq!(shown.windows[0].focus, Some(second));

    // Back among the tiles, where it is not told: beside the tab's pane.
    layout
        .apply(Intent::Dock {
            pane: second,
            to: None,
        })
        .await
        .unwrap();
    let shown = now().await;
    validate(&shown).unwrap();
    assert!(shown.windows[0].floating.is_empty());
    assert_eq!(shown.windows[0].centre.tabs[0].pane_ids(), [first, second]);
    assert_eq!(layers(&seen(&other, &session).await).len(), 1);

    // A pane among the tiles is put away: it floats, out of sight.
    layout
        .apply(Intent::Minimise {
            pane: second,
            on: true,
        })
        .await
        .unwrap();
    let shown = now().await;
    validate(&shown).unwrap();
    assert_eq!(shown.windows[0].centre.tabs[0].pane_ids(), [first]);
    assert_eq!(shown.windows[0].floating[0].pane.id, second);
    assert_eq!(
        shown.windows[0].floating[0].minimised,
        Some(Minimised::Floating)
    );
    let put_away = layers(&seen(&other, &session).await);
    assert_eq!(put_away[1]["state"], "minimized");

    // The last pane on show is not put away: the server refuses, in its words.
    let refused = layout
        .apply(Intent::Minimise {
            pane: first,
            on: true,
        })
        .await
        .unwrap_err();
    println!("putting away the last pane on show: {refused:?}");
    assert!(
        matches!(refused, LayoutError::Refused { .. }),
        "{refused:?}"
    );
    // It floated before it was refused, and that is what the layout says.
    let shown = now().await;
    validate(&shown).unwrap();
    assert_eq!(shown.windows[0].floating.len(), 2);
    assert_eq!(first_block, seen_floating(&seen(&other, &session).await)[1]);

    other
        .call("session.destroy", json!({ "session_id": session }))
        .await
        .unwrap();
    backend.shutdown();
}

/// The blocks of the first window's floating layers, from the back to the front.
fn seen_floating(view: &Value) -> Vec<String> {
    view["windows"][0]["layers"]
        .as_array()
        .unwrap()
        .iter()
        .skip(1)
        .map(|layer| layer["blocks"][0]["block_id"].as_str().unwrap().to_owned())
        .collect()
}

/// The address of a real server's port on this machine, for the test that reaches one
/// over a network: `LUME_REX_LIVE_HTTP=http://127.0.0.1:PORT`, for a server started
/// with `--extra-listen 127.0.0.1:PORT --allow-anonymous`.
const LIVE_HTTP: &str = "LUME_REX_LIVE_HTTP";

#[tokio::test]
#[ignore = "needs a Rex server listening on a port of this machine, named in LUME_REX_LIVE_HTTP"]
async fn a_real_server_is_reached_over_a_port_of_this_machine() {
    let Ok(address) = std::env::var(LIVE_HTTP) else {
        eprintln!("skipped: {LIVE_HTTP} is not set");
        return;
    };
    let endpoint = Endpoint::parse(&address).unwrap();
    assert!(matches!(endpoint, Endpoint::Http { .. }), "{endpoint}");

    // The session is made over the pipe, by this user; it is shown over the port, where
    // the server cannot tell who is calling.
    let io = dial(&Endpoint::Local)
        .await
        .expect("is a Rex server running?");
    let (other, _events) = Control::open(io, "local", &client(), None).await.unwrap();
    let (session, block) = session_with(&other, "lume-live-http", plain_shell()).await;
    let of_block = |args: Value| json!({ "session_id": session, "block_id": block, "args": args });
    let typed = |tag: &str| {
        other.call(
            "com.superlogical.terminal.write",
            of_block(json!({ "data": echo(tag) })),
        )
    };

    let backend = RexBackend::connect(BackendId("rex".to_owned()), endpoint, client());
    let version = backend.ready().await.unwrap();
    assert_eq!(version.api_revision, 19);
    assert!(backend.sessions().iter().any(|listed| listed.id == session));
    backend.open_session(&session).await.unwrap();
    let id = backend.list().await.unwrap()[0].id;

    typed("1-before").await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let attachment = backend.attach(id, facts()).await.unwrap();
    let rows = rows_of(&attachment.hydrate);
    assert!(rows.iter().any(|row| row == "lume-1-before"), "{rows:?}");

    // What is typed through the port is typed, and its output comes back through it.
    let mut view = Viewing::new(attachment).await;
    view.attachment
        .input
        .write(echo("2-through").as_bytes())
        .await
        .unwrap();
    view.until("the echo", |view| view.shows("lume-2-through\r\n"))
        .await;

    view.attachment.input.detach().await.unwrap();
    backend.shutdown();
    other
        .call("session.destroy", json!({ "session_id": session }))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs a Rex server running on this machine, and takes over a minute"]
async fn a_real_server_answers_this_clients_pings() {
    use lume_rex::tuning::PING_EVERY;

    let io = dial(&Endpoint::Local)
        .await
        .expect("is a Rex server running?");
    let (control, _events) = Control::open(io, "local", &client(), None).await.unwrap();
    let (session, block) = session_with(&control, "lume-live-pings", plain_shell()).await;
    let ticket = ticket(&control, &session, &block).await;
    let io = dial(&Endpoint::Local).await.unwrap();
    let mut conn = DataConn::open(io, "local", DataOpen::Start(&ticket))
        .await
        .unwrap();
    let mut reading = Reading::new(conn.welcome().nonce);
    reading.settle(&mut conn).await;

    // Long enough for each connection to ask twice, with nothing said meanwhile. A
    // server that did not answer would have been given up on by then.
    let quiet = PING_EVERY * 2 + Duration::from_secs(5);
    let silence = tokio::time::timeout(quiet, conn.next()).await;
    assert!(silence.is_err(), "the stream was not quiet: {silence:?}");
    assert!(control.ended().is_none());

    // Both connections still carry what is asked of them.
    control
        .call(
            "com.superlogical.terminal.write",
            json!({ "session_id": session, "block_id": block, "args": { "data": echo("pinged") } }),
        )
        .await
        .unwrap();
    reading
        .until(&mut conn, "the echo", |reading| {
            String::from_utf8_lossy(&reading.output).contains("lume-pinged")
        })
        .await;

    conn.close().await;
    control
        .call("session.destroy", json!({ "session_id": session }))
        .await
        .unwrap();
}

/// The address of a real server reached through the SSH client, for the test that does
/// so: `LUME_REX_LIVE_SSH=ssh://USER@HOST`. The client must log in to it without
/// asking anything.
const LIVE_SSH: &str = "LUME_REX_LIVE_SSH";
/// A configuration file for the SSH client in that test, when the user's own is not
/// the one to read.
const LIVE_SSH_CONFIG: &str = "LUME_REX_LIVE_SSH_CONFIG";
/// A command that cuts the SSH connections to that machine from outside, to see the
/// client started again. Run through this machine's shell.
const LIVE_SSH_CUT: &str = "LUME_REX_LIVE_SSH_CUT";

#[tokio::test]
#[ignore = "needs a Rex server on a machine the SSH client logs in to, named in LUME_REX_LIVE_SSH"]
async fn a_real_server_is_reached_through_the_ssh_client() {
    use lume_rex::{Options, Reach, SshOptions};

    let Ok(address) = std::env::var(LIVE_SSH) else {
        eprintln!("skipped: {LIVE_SSH} is not set");
        return;
    };
    let endpoint = Endpoint::parse(&address).unwrap();
    assert!(matches!(endpoint, Endpoint::Ssh { .. }), "{endpoint}");
    let mut ssh = SshOptions::default();
    if let Ok(config) = std::env::var(LIVE_SSH_CONFIG) {
        ssh.args = vec!["-F".to_owned(), config];
    }
    let options = Options::new().ssh(ssh);

    // Everything goes through the SSH client: this machine has no other way there. One
    // client carries the backend's connections, another this test's own.
    let backend = RexBackend::connect_with(
        BackendId("rex".to_owned()),
        endpoint.clone(),
        client(),
        options.clone(),
    );
    let version = match tokio::time::timeout(Duration::from_secs(90), backend.ready()).await {
        Ok(Ok(version)) => version,
        Ok(Err(error)) => panic!("the server was not reached: {error}"),
        // It is being tried again and again: how it stands says why.
        Err(_) => panic!("the server was not reached: {:?}", *backend.link().borrow()),
    };
    println!(
        "reached Rex {} on {} at API revision {}",
        version.rex_version, version.platform, version.api_revision
    );
    let windows = version.platform.starts_with("windows");

    let reach = Reach::new(endpoint, options);
    let dialed = reach.dial().await.unwrap();
    let (other, _events) =
        Control::open_at(dialed.io, &address, &dialed.authority, &client(), None)
            .await
            .unwrap();
    // A shell of the server's machine, with nothing of the user's in it.
    let command = if windows {
        json!(["cmd.exe", "/d"])
    } else {
        json!(["/bin/sh"])
    };
    let shell = json!({
        "flavor": "com.superlogical.terminal.shell",
        "options": { "shell": "none", "command": command },
    });
    let said = |tag: &str| {
        if windows {
            format!("echo lume-^{tag}\r")
        } else {
            format!("echo lume-''{tag}\r")
        }
    };
    let (session, block) = session_with(&other, "lume-live-ssh", shell).await;
    other
        .call(
            "com.superlogical.terminal.write",
            json!({ "session_id": session, "block_id": block, "args": { "data": said("1-before") } }),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;

    until("the session is listed", || {
        backend.sessions().iter().any(|listed| listed.id == session)
    })
    .await;
    backend.open_session(&session).await.unwrap();
    let id = backend.list().await.unwrap()[0].id;

    // The view starts from the server's copy of the terminal, which this engine loads.
    let attachment = backend.attach(id, facts()).await.unwrap();
    let rows = rows_of(&attachment.hydrate);
    assert!(rows.iter().any(|row| row == "lume-1-before"), "{rows:?}");

    // What is typed goes through the client and its output comes back through it.
    let mut view = Viewing::new(attachment).await;
    view.attachment
        .input
        .write(said("2-through").as_bytes())
        .await
        .unwrap();
    view.until("the echo", |view| view.shows("lume-2-through"))
        .await;
    view.attachment
        .input
        .resize(TermSize::new(100, 30).unwrap(), CellPx { w: 8, h: 16 })
        .await
        .unwrap();
    view.until("the new size", |view| view.sizes.contains(&(100, 30)))
        .await;

    // With a command that cuts the SSH connections from outside, the client that carried
    // them ends: a new one is started, and the view goes on.
    if let Ok(cut) = std::env::var(LIVE_SSH_CUT) {
        let mut link = backend.link();
        let (shell, flag) = if cfg!(windows) {
            ("cmd.exe", "/c")
        } else {
            ("/bin/sh", "-c")
        };
        let ran = std::process::Command::new(shell)
            .args([flag, &cut])
            .status()
            .unwrap();
        println!("`{cut}` ended with {ran}");
        tokio::time::timeout(
            Duration::from_secs(90),
            link.wait_for(|link| !matches!(link, Link::Connected(_))),
        )
        .await
        .expect("the cut was never noticed")
        .unwrap();
        tokio::time::timeout(
            Duration::from_secs(90),
            link.wait_for(|link| matches!(link, Link::Connected(_))),
        )
        .await
        .expect("the server was not reached again")
        .unwrap();
        view.attachment
            .input
            .write(said("3-again").as_bytes())
            .await
            .unwrap();
        view.until("the echo after the cut", |view| {
            view.shows("lume-3-again")
                || view
                    .copies
                    .iter()
                    .any(|copy| rows_of(copy).iter().any(|row| row == "lume-3-again"))
        })
        .await;
        println!("the view went on after the cut");
    }

    view.attachment.input.detach().await.unwrap();
    // This test's own client was cut too; a new one is started by asking again.
    let dialed = reach.dial().await.unwrap();
    let (other, _events) =
        Control::open_at(dialed.io, &address, &dialed.authority, &client(), None)
            .await
            .unwrap();
    other
        .call("session.attach", json!({ "session_id": session }))
        .await
        .unwrap();
    other
        .call("session.destroy", json!({ "session_id": session }))
        .await
        .unwrap();
    backend.shutdown();
}
