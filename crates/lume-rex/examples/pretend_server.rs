//! The pretend server of this crate's tests as a program of its own, for what is tested
//! through the application's window: a server on a port of this machine that does as a
//! server does, until it is told to do otherwise.
//!
//! Its first line says where it is. Then it is told what to do by the words on its
//! standard input, one a line, and answers each with a line of JSON when it has done it.
//! It ends when its standard input does, so it never outlives the test that started it.
//!
//! - `calm`: from now on a sound server again, with its one session
//! - `names`: sessions, tabs, panes and titles are called every hostile name there is
//! - `reports`: every report a real server made, with each thing in it changed
//! - `layouts`: layouts with something in them changed
//! - `answers`: from now on something is changed in every answer, or it is an error
//! - `streams`: the open terminals are sent what does most in a terminal, and noise
//! - `copies`: the open streams end, and the next of them begin with damaged copies
//! - `flood`: more reports and output than anybody could take
//! - `clipboard`: the last block of the session writes to the clipboard
//! - `silence`: from now on nothing is answered, and connections are taken all the same
//! - `close`: the connections are closed, with a word of the application's own
//! - `count`: nothing but the answer
//!
//! With `--revision <n>` it speaks that revision of the API only, and turns a client away
//! that does not; with `--welcomes` it welcomes such a client all the same.

#[path = "../tests/common/mod.rs"]
mod common;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead as _, Write as _};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use common::hostile::{self, CODES, NAMES, Numbers};
use common::{Asked, Conn, DataSide, NONCE, Recorded, SESSION, World, note_bytes};
use data_encoding::BASE64URL_NOPAD;
use futures_util::StreamExt as _;
use lume_vt::{Format, Terminal};
use serde_json::{Value, json};
use tokio::io::DuplexStream;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::Message;

/// How much of a stream one message holds at most.
const PART: usize = 32 * 1024;

/// How many reports are sent before the client is given a moment to take them.
const AT_ONCE: usize = 200;

/// How often the streams are ended until the damaged copies have all been sent.
const COPIES_WAIT: usize = 20;

const MOMENT: Duration = Duration::from_millis(10);

/// The ticket of a data connection is sixteen bytes, of which the first ones say which
/// connection it is for.
const TICKET_BYTES: usize = 16;

/// What the control connection of a client is told to send.
enum Say {
    Event(String, Value),
    Close(u16, String),
}

/// What a terminal's stream is told to send.
enum Put {
    Bytes(Vec<u8>),
    Close(u16),
}

struct Stream {
    block: String,
    put: mpsc::UnboundedSender<Put>,
}

struct Server {
    revision: Option<u64>,
    welcomes: bool,
    world: tokio::sync::Mutex<World>,
    /// The control connections, in the order they were made.
    controls: RefCell<Vec<mpsc::UnboundedSender<Say>>>,
    streams: RefCell<Vec<Stream>>,
    terminals: RefCell<HashMap<String, Terminal>>,
    /// The block each ticket was given for.
    tickets: RefCell<Vec<String>>,
    /// What the next streams begin with instead of a copy of their terminals.
    copies: RefCell<Vec<Vec<u8>>>,
    silent: Cell<bool>,
    /// The revision of the session's layout: it only goes up.
    shown: Cell<u64>,
    /// How many control connections there were, and how many streams.
    connected: Cell<usize>,
    streamed: Cell<usize>,
    /// The methods that were called.
    called: RefCell<HashSet<String>>,
    /// What was typed into each block.
    typed: RefCell<HashMap<String, String>>,
}

/// The blocks of the recorded session, in the order they are laid out.
fn blocks() -> Vec<String> {
    let view = &World::recorded().views[SESSION];
    view["windows"][0]["layers"][0]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|block| block["block_id"].as_str().unwrap().to_owned())
        .collect()
}

impl Server {
    fn new(revision: Option<u64>, welcomes: bool) -> Self {
        Self {
            revision,
            welcomes,
            world: tokio::sync::Mutex::new(World::recorded()),
            controls: RefCell::default(),
            streams: RefCell::default(),
            terminals: RefCell::default(),
            tickets: RefCell::default(),
            copies: RefCell::default(),
            silent: Cell::new(false),
            shown: Cell::new(1_000),
            connected: Cell::new(0),
            streamed: Cell::new(0),
            called: RefCell::default(),
            typed: RefCell::default(),
        }
    }

    /// Does something with a block's terminal, which is made when it is first named.
    fn terminal<T>(&self, block: &str, act: impl FnOnce(&mut Terminal) -> T) -> T {
        let mut terminals = self.terminals.borrow_mut();
        let nth = terminals.len() + 1;
        let terminal = terminals.entry(block.to_owned()).or_insert_with(|| {
            let mut made = Terminal::new(80, 24).unwrap();
            made.write(format!("terminal {nth} of the pretend server\r\n").as_bytes());
            made
        });
        act(terminal)
    }

    /// Sends something to everybody who is shown a block.
    fn put(&self, block: &str, bytes: &[u8]) {
        for stream in self.streams.borrow().iter() {
            if stream.block == block {
                for part in bytes.chunks(PART) {
                    let _ = stream.put.send(Put::Bytes(part.to_vec()));
                }
            }
        }
    }

    /// A block's program prints something.
    fn print(&self, block: &str, bytes: &[u8]) {
        self.terminal(block, |terminal| terminal.write(bytes));
        self.put(block, bytes);
    }

    fn size_note(cols: u16, rows: u16) -> Vec<u8> {
        note_bytes(
            NONCE,
            &json!({
                "type": "resize", "cols": cols, "rows": rows,
                "cell_width_px": 8, "cell_height_px": 16, "owner": null,
            }),
        )
    }

    fn event(&self, name: &str, payload: Value) {
        if let Some(last) = self.controls.borrow().last() {
            let _ = last.send(Say::Event(name.to_owned(), payload));
        }
    }

    fn session_event(&self, name: &str, payload: Value) {
        self.event(
            "session_event",
            json!({ "session_id": SESSION, "event": { "type": name, "payload": payload } }),
        );
    }

    fn of_block(&self, block: &str, name: &str, payload: Value) {
        self.session_event(
            "block_event",
            json!({
                "block_id": block, "creator_name": "com.superlogical.terminal",
                "name": name, "payload": payload,
            }),
        );
    }

    /// Replaces the layout of the session, and gives its new revision.
    async fn put_view(&self, mut view: Value) -> u64 {
        let revision = self.shown.get() + 1;
        self.shown.set(revision);
        if view.is_object() {
            view["revision"] = json!(revision);
        }
        self.world
            .lock()
            .await
            .views
            .insert(SESSION.to_owned(), view);
        revision
    }

    /// Replaces the layout of the session, and says so.
    async fn show(&self, view: Value) {
        let revision = self.put_view(view).await;
        self.session_event("session_view_changed", json!({ "revision": revision }));
    }

    // What a client asks.

    /// Answers one request, with what is this program's own to answer.
    async fn answer(&self, conn: &mut Conn, asked: &Asked) {
        self.called.borrow_mut().insert(asked.method.clone());
        let block = asked.payload["block_id"].as_str().unwrap_or_default();
        let args = &asked.payload["args"];
        if self.world.lock().await.twist.is_none() {
            match asked.method.as_str() {
                "data.connect" => {
                    let mut ticket = [0; TICKET_BYTES];
                    {
                        let mut tickets = self.tickets.borrow_mut();
                        ticket[..8].copy_from_slice(&(tickets.len() as u64).to_le_bytes());
                        tickets.push(block.to_owned());
                    }
                    let ticket = BASE64URL_NOPAD.encode(&ticket);
                    conn.respond(
                        &asked.id,
                        json!({ "ticket": ticket, "expires_in_ms": 60_000 }),
                    )
                    .await;
                    return;
                }
                "com.superlogical.terminal.write" => {
                    let data = args["data"].as_str().unwrap_or_default();
                    self.typed
                        .borrow_mut()
                        .entry(block.to_owned())
                        .or_default()
                        .push_str(data);
                    self.print(block, data.replace('\r', "\r\n").as_bytes());
                    conn.respond(&asked.id, json!({ "bytes_written": data.len() }))
                        .await;
                    return;
                }
                "com.superlogical.terminal.resize" => {
                    let side = |name: &str| args[name].as_u64().unwrap_or(1) as u16;
                    let (cols, rows) = (side("columns"), side("rows"));
                    let _ = self.terminal(block, |terminal| terminal.resize(cols, rows));
                    self.put(block, &Self::size_note(cols, rows));
                    conn.respond(&asked.id, json!({})).await;
                    return;
                }
                "session.focus_block" => {
                    let mut view = self.world.lock().await.views[SESSION].clone();
                    if let Some(window) = view.pointer_mut("/windows/0") {
                        window["focused_block_id"] = json!(block);
                    }
                    let revision = self.put_view(view).await;
                    conn.session_event(
                        SESSION,
                        "session_view_changed",
                        json!({ "revision": revision }),
                    )
                    .await;
                    conn.respond(&asked.id, json!({ "revision": revision }))
                        .await;
                    return;
                }
                "com.superlogical.terminal.size" if !block.is_empty() => {
                    let size = self.terminal(block, |terminal| (terminal.cols(), terminal.rows()));
                    conn.respond(
                        &asked.id,
                        json!({
                            "columns": size.0, "rows": size.1,
                            "cell_width_px": 8, "cell_height_px": 16, "desired": [], "owner": null,
                        }),
                    )
                    .await;
                    return;
                }
                "com.superlogical.terminal.format" => {
                    let screen = self.terminal(block, |terminal| terminal.format(Format::Replay));
                    let screen = String::from_utf8_lossy(&screen.unwrap().bytes).into_owned();
                    conn.respond(&asked.id, json!({ "content": screen })).await;
                    return;
                }
                _ => {}
            }
        }
        self.world.lock().await.answer(conn, asked).await;
    }

    /// Welcomes a client as a server that speaks this one's revisions does, or does not.
    async fn greet(&self, mut conn: Conn) -> Option<Conn> {
        let mut welcome = Recorded::load().welcome();
        if let Some(revision) = self.revision {
            let side = |name: &str| conn.hello[name].as_u64().unwrap_or_default();
            let spoken = side("api_min_revision")..=side("api_revision");
            if !self.welcomes && !spoken.contains(&revision) {
                let update = if *spoken.start() > revision {
                    "the server"
                } else {
                    "the client"
                };
                conn.close(
                    1008,
                    &format!(
                        "no common API revision: the server speaks {revision}; update {update}"
                    ),
                )
                .await;
                return None;
            }
            for name in ["api_revision", "api_server_revision", "api_min_revision"] {
                welcome[name] = json!(revision);
            }
        }
        conn.send(welcome).await;
        Some(conn)
    }

    async fn control(self: Rc<Self>, stream: DuplexStream) {
        let Some(mut conn) = self.greet(Conn::shake(stream).await).await else {
            return;
        };
        self.connected.set(self.connected.get() + 1);
        let (say, mut said) = mpsc::unbounded_channel();
        self.controls.borrow_mut().push(say);
        loop {
            if self.silent.get() {
                // The connection stands, and nobody is at the other end of it.
                sleep(MOMENT).await;
                if let Ok(Say::Close(..)) = said.try_recv() {
                    return;
                }
                continue;
            }
            tokio::select! {
                asked = conn.asked() => {
                    let Some(asked) = asked else { break };
                    if !self.silent.get() {
                        self.answer(&mut conn, &asked).await;
                    }
                }
                say = said.recv() => match say {
                    Some(Say::Event(name, payload)) => conn.event(&name, payload).await,
                    Some(Say::Close(code, reason)) => {
                        conn.close(code, &reason).await;
                        return;
                    }
                    None => return,
                },
                // Looked at again, for the silence that may have begun.
                () = sleep(Duration::from_millis(100)) => {}
            }
        }
    }

    /// The block a data connection is for, by the ticket in its first message.
    fn block_of(&self, first: &[u8]) -> String {
        let tickets = self.tickets.borrow();
        let nth: Option<[u8; 8]> = first.get(1..9).and_then(|nth| nth.try_into().ok());
        nth.and_then(|nth| tickets.get(u64::from_le_bytes(nth) as usize).cloned())
            .unwrap_or_default()
    }

    async fn data(self: Rc<Self>, stream: DuplexStream) {
        let mut side = DataSide::accept(stream).await;
        let block = self.block_of(&side.first);
        self.streamed.set(self.streamed.get() + 1);
        let (put, mut given) = mpsc::unbounded_channel();
        let other = self.copies.borrow_mut().pop();
        let (copy, size) = self.terminal(&block, |terminal| {
            (
                terminal.snapshot().unwrap(),
                Self::size_note(terminal.cols(), terminal.rows()),
            )
        });
        self.streams.borrow_mut().push(Stream {
            block,
            put: put.clone(),
        });
        for part in other.unwrap_or(copy).chunks(PART) {
            side.bytes(part).await;
        }
        side.bytes(&size).await;
        loop {
            tokio::select! {
                put = given.recv() => match put {
                    Some(Put::Bytes(bytes)) => side.bytes(&bytes).await,
                    Some(Put::Close(code)) => {
                        side.close(code, "the stream is over").await;
                        break;
                    }
                    None => break,
                },
                message = side.socket.next() => {
                    if matches!(message, None | Some(Err(_) | Ok(Message::Close(_)))) {
                        break;
                    }
                }
            }
        }
        self.streams
            .borrow_mut()
            .retain(|stream| !stream.put.same_channel(&put));
    }

    // What it is told to do.

    /// A sound server again: what was said before is taken back.
    async fn calm(&self) {
        self.silent.set(false);
        self.copies.borrow_mut().clear();
        *self.world.lock().await = World::recorded();
        self.event(
            "session_created",
            json!({ "session": { "session_id": SESSION, "label": "lume-fixture" } }),
        );
        self.session_event("session_label_changed", json!({ "label": "lume-fixture" }));
        self.show(World::recorded().views[SESSION].clone()).await;
    }

    async fn names(&self) {
        let blocks = blocks();
        for (nth, name) in NAMES.iter().enumerate() {
            let id = format!("session:named-{nth}");
            self.world
                .lock()
                .await
                .sessions
                .push((id.clone(), (*name).to_owned()));
            self.event(
                "session_created",
                json!({ "session": { "session_id": id, "label": name } }),
            );
            let mut view = World::recorded().views[SESSION].clone();
            view["label"] = json!(name);
            view["windows"][0]["label"] = json!(name);
            for block in view["windows"][0]["layers"][0]["blocks"]
                .as_array_mut()
                .unwrap()
            {
                block["label"] = json!(name);
            }
            self.show(view).await;
            self.session_event("session_label_changed", json!({ "label": name }));
            for block in &blocks {
                self.of_block(block, "title_changed", json!({ "title": name }));
                let record =
                    json!({ "state": "blocked", "kind": "auth", "title": name, "msg": name });
                self.of_block(
                    block,
                    "program_status_changed",
                    json!({ "id": name, "record": record }),
                );
                self.of_block(
                    block,
                    "process_changed",
                    json!({ "foreground": { "name": name, "cwd": name, "path": name, "pid": 4 } }),
                );
            }
            sleep(MOMENT * 5).await;
        }
    }

    async fn reports(&self) {
        let mut said = 0usize;
        for (name, payload) in hostile::reports() {
            for (_, other) in hostile::changed(&payload) {
                self.event(&name, other);
                said += 1;
                if said.is_multiple_of(AT_ONCE) {
                    sleep(MOMENT).await;
                }
            }
        }
    }

    async fn layouts(&self) {
        let all = hostile::changed(&World::recorded().views[SESSION]);
        let mut numbers = Numbers(11);
        for nth in 0..200 {
            let (_, view) = all[numbers.below(all.len())].clone();
            self.show(view).await;
            // Some of them one upon the other, and some shown before the next is there.
            if nth % 4 == 0 {
                sleep(MOMENT * 5).await;
            }
        }
    }

    async fn answers(&self) {
        let mut world = self.world.lock().await;
        let numbers = Arc::new(std::sync::Mutex::new(Numbers(13)));
        let drawn = Arc::clone(&numbers);
        world.twist = Some(Arc::new(move |_, answer| {
            let all = hostile::changed(&answer);
            let nth = drawn.lock().unwrap().below(all.len());
            Some(all[nth].1.clone())
        }));
        world.refusing = Some(Arc::new(move |_| {
            let mut numbers = numbers.lock().unwrap();
            let refused = numbers.below(3) == 0;
            let code = CODES[numbers.below(CODES.len())];
            refused.then(|| (code.to_owned(), 409, code.to_owned()))
        }));
    }

    fn streams(&self) {
        let mut numbers = Numbers(17);
        for block in blocks() {
            for hard in hardest() {
                self.put(&block, &hard);
            }
            for name in NAMES {
                self.put(&block, &without_clipboard(name.as_bytes().to_vec()));
                for number in [0, 1, 2, 7, 9, 99, 777, 1337] {
                    let named = format!("\x1b]{number};{name}\x07").into_bytes();
                    self.put(&block, &without_clipboard(named));
                }
            }
            for round in 0..40 {
                let noise = numbers.bytes(4_096);
                self.put(&block, &without_clipboard(noise));
                let side = |numbers: &mut Numbers| 1 + numbers.below(4_096) as u16;
                let (cols, rows) = (side(&mut numbers), side(&mut numbers));
                self.put(&block, &Self::size_note(cols, rows));
                if round % 8 == 0 {
                    self.put(&block, &Self::size_note(1, 1));
                    self.put(&block, &Self::size_note(u16::MAX, u16::MAX));
                }
            }
            // As its window last asked, so that what comes next is seen.
            let size = self.terminal(&block, |terminal| (terminal.cols(), terminal.rows()));
            self.put(&block, b"\x18\x1b\\\x1bc\x1b[?1049l");
            self.put(&block, &Self::size_note(size.0, size.1));
        }
    }

    async fn copies(&self) {
        let mut numbers = Numbers(19);
        let mut sound = Terminal::new(80, 24).unwrap();
        sound.write(b"a copy\r\n");
        let sound = sound.snapshot().unwrap();
        let damaged: Vec<Vec<u8>> = (0..6)
            .map(|nth| {
                let mut copy = sound.clone();
                let at = numbers.below(copy.len());
                match nth % 3 {
                    0 => copy.truncate(at),
                    1 => copy[at] ^= 0x55,
                    _ => copy = numbers.bytes(2_000),
                }
                copy
            })
            .collect();
        self.copies.borrow_mut().extend(damaged);
        // A client makes do without a copy, so each stream takes one.
        for _ in 0..COPIES_WAIT {
            if self.copies.borrow().is_empty() {
                break;
            }
            for stream in self.streams.borrow().iter() {
                let _ = stream.put.send(Put::Close(1013));
            }
            sleep(MOMENT * 50).await;
        }
    }

    async fn flood(&self) {
        let blocks = blocks();
        let line = "what is printed and printed and never read\r\n".repeat(700);
        for nth in 0..2_000 {
            let name = NAMES[nth % NAMES.len()];
            let block = &blocks[nth % blocks.len()];
            self.of_block(block, "title_changed", json!({ "title": name }));
            self.of_block(block, "bell", json!({}));
            self.session_event("session_label_changed", json!({ "label": name }));
            if nth % 8 == 0 {
                self.put(block, line.as_bytes());
            }
            if nth % 100 == 0 {
                sleep(MOMENT).await;
            }
        }
    }

    fn close(&self) {
        for control in self.controls.borrow().iter() {
            let _ = control.send(Say::Close(1001, "confirmation_required".to_owned()));
        }
        self.controls.borrow_mut().clear();
    }

    /// Does what it is told, and says how it stands then.
    async fn told(&self, word: &str) -> Value {
        match word {
            "calm" => self.calm().await,
            "names" => self.names().await,
            "reports" => self.reports().await,
            "layouts" => self.layouts().await,
            "answers" => self.answers().await,
            "streams" => self.streams(),
            "copies" => self.copies().await,
            "flood" => self.flood().await,
            "clipboard" => self.put(
                blocks().last().unwrap(),
                b"\x1b]52;c;aG9zdGlsZQ==\x07\x1b]52;p;aG9zdGlsZQ==\x07\x1b]52;c;?\x07",
            ),
            "silence" => self.silent.set(true),
            "close" => self.close(),
            "count" => {}
            other => return json!({ "unknown": other }),
        }
        let mut called: Vec<String> = self.called.borrow().iter().cloned().collect();
        called.sort();
        json!({
            "done": word,
            "connected": self.connected.get(),
            "streamed": self.streamed.get(),
            "streams": self.streams.borrow().len(),
            "called": called,
            "typed": *self.typed.borrow(),
        })
    }
}

/// What asks most of a terminal, or of what it is in: nothing in it is for a user to see.
fn hardest() -> Vec<Vec<u8>> {
    let mut all: Vec<Vec<u8>> = [
        // Asked about itself, over and over: every answer would be typed.
        "\x1b[c\x1b[>c\x1b[=c\x1b[5n\x1b[6n\x1b[?996n\x1b[18t\x1b[21t\x1b]10;?\x07\x1b]11;?\x07",
        "\x1bP+q544e\x1b\\\x1bP$qm\x1b\\\x05\x1b[?u\x1b[>q\x1b]4;1;?\x07",
        // What would name a folder, open a place, or tell the desktop something.
        "\x1b]7;file://localhost/C:/Windows\x07\x1b]7;file:////server/share\x07",
        "\x1b]8;;file:///C:/Windows/System32/calc.exe\x07here\x1b]8;;\x07",
        "\x1b]8;;javascript:alert(1)\x07there\x1b]8;;\x07",
        "\x1b]9;a message\x07\x1b]99;i=1:d=0;a message\x1b\\\x1b]777;notify;a;message\x07",
        "\x1b]9;4;1;50\x07\x1b]9;4;3\x07\x1b]9;9;C:\\Windows\x07",
        "\x1b]133;A\x07\x1b]133;B\x07\x1b]133;C\x07\x1b]133;D;1\x07\x1b]633;E;format c:\x07",
        "\x1b]1337;File=inline=1:AAAA\x07\x1b]1337;RequestAttention=yes\x07",
        "\x1b]1337;SetUserVar=a=YQ==\x07\x1b]1337;CurrentDir=/\x07\x1b]50;?\x07",
        // Modes: the mouse, the keyboard, another screen, what is pasted, what is synced.
        "\x1b[?1000;1002;1003;1004;1005;1006;1015;1016h\x1b[>31u\x1b[=31;1u\x1b[?9001h",
        "\x1b[?1049h\x1b[?2004h\x1b[?2026h\x1b[?1h\x1b=\x1b[?66h\x1b[?2031h\x1b[4;20h",
        // Pictures that ask for more room than there is, and ones that never come.
        "\x1b_Ga=T,f=32,s=65535,v=65535;AAAA\x1b\\\x1b_Ga=t,t=f,f=100;L2V0Yy9wYXNzd2Q=\x1b\\",
        "\x1b_Ga=t,t=t,f=100;QzpcV2luZG93c1x3aW4uaW5p\x1b\\\x1bPq#0;2;0;0;0!99999~\x1b\\",
        // Numbers without end, and strings nobody finishes.
        "\x1b[999999999;999999999H\x1b[999999999X\x1b[999999999@\x1b[999999999L\x1b[999999999b",
        "\x1b[1;999999999r\x1b[999999999S\x1b[999999999T\x1b#8\x1b[3J\x1b[8;9999;9999t\x1b[9;1t",
    ]
    .iter()
    .map(|text| text.as_bytes().to_vec())
    .collect();
    all.push("\u{7}".repeat(20_000).into_bytes());
    all.push(format!("\x1b]2;{}\x07", "t".repeat(300_000)).into_bytes());
    all.push(format!("e{}", "\u{301}".repeat(50_000)).into_bytes());
    all.push(format!("\x1b[{}m", "38;5;1;".repeat(20_000)).into_bytes());
    all.push(format!("\x1b]8;id={0};http://{0}\x07x", "l".repeat(60_000)).into_bytes());
    all.push(format!("\x1bP{}", "q".repeat(300_000)).into_bytes());
    all
}

/// Takes out of noise what would write to the clipboard, which is the user's own where
/// the noise is tried out.
fn without_clipboard(mut noise: Vec<u8>) -> Vec<u8> {
    for at in 1..noise.len() {
        if noise[at - 1..=at] == *b"52" {
            noise[at] = b' ';
        }
    }
    noise
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let revision = args
        .iter()
        .position(|arg| arg == "--revision")
        .map(|at| args[at + 1].parse().expect("a revision is a number"));
    let welcomes = args.iter().any(|arg| arg == "--welcomes");
    let server = Rc::new(Server::new(revision, welcomes));

    let (heard, mut words) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for word in std::io::stdin().lock().lines().map_while(Result::ok) {
            let _ = heard.send(word);
        }
    });
    let say = |line: Value| {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}").and_then(|()| out.flush());
    };
    let (address, mut listener) = common::door().await;
    say(json!({ "address": address.to_string() }));

    let serving = Rc::clone(&server);
    tokio::task::LocalSet::new()
        .run_until(async move {
            tokio::task::spawn_local(async move {
                while let Some(stream) = listener.next().await {
                    let server = Rc::clone(&serving);
                    tokio::task::spawn_local(async move {
                        match common::asks_for_control(stream).await {
                            Some((true, stream)) => server.control(stream).await,
                            Some((false, stream)) => server.data(stream).await,
                            None => {}
                        }
                    });
                }
            });
            while let Some(word) = words.recv().await {
                say(server.told(word.trim()).await);
            }
        })
        .await;
}
