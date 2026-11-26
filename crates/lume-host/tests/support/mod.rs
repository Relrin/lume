//! A test client written directly on the wire protocol, and the two ways of running the
//! host under it.
//!
//! The client deliberately shares no code with the real client library: the host is
//! tested against the wire, not against the code that normally talks to it.

// Each test file compiles this module separately and uses a different part of it.
#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use lume_host::{Host, HostConfig};
use lume_proto::{
    Body, CONTROL_STREAM, CellPx, ClientFacts, ClientKind, ColorScheme, Control, FrameDecoder,
    HydrateMode, HydratePayload, Launcher, Message, Notification, PROTO_VERSION, Palette,
    ProtoError, Request, RequestBody, ResponseBody, Role, SessionEvent, SessionId, SessionInfo,
    SpawnSpec, TermSize, encode,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// How long any single step of a test may take before the test fails.
pub const STEP: Duration = Duration::from_secs(20);

/// The value of `LUME_MARK` in every session these tests start.
pub const MARK: &str = "k7q";

/// What the shell prints for [`PRINT_MARK`].
pub const MARK_OUTPUT: &str = "lume-k7q-done";

/// A command line that prints [`MARK_OUTPUT`]. The marker comes from the environment so
/// that it appears in the output only when the command ran, not when it was echoed.
#[cfg(windows)]
pub const PRINT_MARK: &str = "echo lume-%LUME_MARK%-done";
#[cfg(unix)]
pub const PRINT_MARK: &str = "echo \"lume-${LUME_MARK}-done\"";

/// A command line that starts a program of its own, which prints a line now and then
/// until it is ended.
#[cfg(windows)]
pub const PRINTS_NOW_AND_THEN: &str =
    "cmd /d /c \"for /l %i in () do @(echo tick & ping -n 2 127.0.0.1 >nul)\"";
#[cfg(unix)]
pub const PRINTS_NOW_AND_THEN: &str = "sh -c 'while :; do echo tick; sleep 1; done'";

/// A command line that prints the terminal size, columns after rows.
#[cfg(windows)]
pub const PRINT_SIZE: &str = "mode con";
#[cfg(unix)]
pub const PRINT_SIZE: &str = "stty size";

pub fn size(cols: u16, rows: u16) -> TermSize {
    TermSize::new(cols, rows).unwrap()
}

pub fn facts() -> ClientFacts {
    ClientFacts {
        size: size(80, 24),
        cell_px: CellPx { w: 9, h: 18 },
        palette: Palette::default(),
        scheme: ColorScheme::Dark,
        focused: true,
        visible: true,
    }
}

fn spec(program: &str, args: &[&str]) -> SpawnSpec {
    SpawnSpec {
        launcher: Launcher::Shell {
            program: program.to_owned(),
            args: args.iter().map(|&arg| arg.to_owned()).collect(),
            login: false,
        },
        cwd: None,
        env: vec![("LUME_MARK".to_owned(), MARK.to_owned())],
        size: size(80, 24),
        shell_integration: false,
        scrollback_lines: None,
    }
}

/// An interactive shell.
pub fn shell() -> SpawnSpec {
    if cfg!(windows) {
        spec("cmd.exe", &["/d"])
    } else {
        spec("/bin/sh", &[])
    }
}

/// A program that prints lines for ever.
pub fn flood() -> SpawnSpec {
    if cfg!(windows) {
        spec(
            "cmd.exe",
            &[
                "/d",
                "/c",
                "for /l %i in () do @echo 0123456789abcdefghijklmnopqrstuvwxyz",
            ],
        )
    } else {
        spec(
            "/bin/sh",
            &["-c", "yes 0123456789abcdefghijklmnopqrstuvwxyz"],
        )
    }
}

/// A program that prints `text` and then waits for input.
pub fn prints_then_waits(text: &str) -> SpawnSpec {
    if cfg!(windows) {
        spec(
            "cmd.exe",
            &["/d", "/c", &format!("echo {text} & pause >nul")],
        )
    } else {
        spec("/bin/sh", &["-c", &format!("echo {text}; read reply")])
    }
}

/// A program that prints `text` and exits with `code`.
pub fn prints_then_exits(text: &str, code: u32) -> SpawnSpec {
    if cfg!(windows) {
        spec(
            "cmd.exe",
            &["/d", "/c", &format!("echo {text} & exit {code}")],
        )
    } else {
        spec("/bin/sh", &["-c", &format!("echo {text}; exit {code}")])
    }
}

/// A program that waits for a line, then reports what a shell with integration reports
/// (a title, a directory, a command that starts and fails, a status), prints `done` and
/// waits for input. Until the line comes nothing is printed, so a client that attaches
/// first is sent all of it as it happens, however soon the program starts.
pub fn reports_then_waits(done: &str) -> SpawnSpec {
    // What each control string says, between the introducer and the terminator.
    const REPORTS: [&str; 5] = [
        "2;lume-title-k7q",
        "7;file://box/srv/www%20x",
        "133;C;cmdline_url=make%20test",
        "133;D;3",
        "7501;state=done:app=make",
    ];
    if cfg!(windows) {
        let writes: String = REPORTS
            .iter()
            .map(|report| format!("[Console]::Out.Write($e+']{report}'+$e+'\\');"))
            .collect();
        spec(
            "powershell.exe",
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!(
                    "[void][Console]::In.ReadLine();$e=[string][char]27;{writes}\
                     [Console]::Out.WriteLine('{done}');[void][Console]::In.ReadLine()"
                ),
            ],
        )
    } else {
        let writes: String = REPORTS
            .iter()
            .map(|report| format!("printf '\\033]%s\\033\\\\' '{report}';"))
            .collect();
        spec(
            "/bin/sh",
            &["-c", &format!("read go; {writes} echo {done}; read reply")],
        )
    }
}

/// How [`prints_lines_then_waits`] writes line `n`.
pub fn line_number(n: u32) -> String {
    format!("line-{n:06}")
}

/// What [`prints_lines_then_waits`] prints after each number, to make each line about a
/// hundred bytes long.
const LINE_PADDING: &str =
    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";

/// A program that prints `count` numbered lines of about a hundred bytes and then
/// waits for input.
pub fn prints_lines_then_waits(count: u32) -> SpawnSpec {
    if cfg!(windows) {
        // Numbers padded to six digits with arithmetic, since `for /l` gives none.
        spec(
            "cmd.exe",
            &[
                "/d",
                "/v:on",
                "/c",
                &format!(
                    // Without the outer brackets the pause would be part of the loop.
                    "(for /l %i in (1,1,{count}) do @(set /a n=1000000+%i >nul & echo line-!n:~1! {LINE_PADDING})) \
                     & pause >nul"
                ),
            ],
        )
    } else {
        spec(
            "/bin/sh",
            &[
                "-c",
                &format!(
                    "i=1; while [ $i -le {count} ]; do printf 'line-%06d {LINE_PADDING}\\n' $i; i=$((i+1)); done; \
                     read reply"
                ),
            ],
        )
    }
}

/// What [`asks_for_the_cursor`] prints once it has read an answer.
pub const CURSOR_ANSWERED: &str = "cursor-answered";

/// A program that asks the terminal where the cursor is, reads the answer, and says so.
///
/// On Windows the console host between the program and the terminal answers this
/// question itself, so there the case shows only that the program is answered.
pub fn asks_for_the_cursor() -> SpawnSpec {
    if cfg!(windows) {
        spec(
            "powershell.exe",
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!(
                    "$p=$Host.UI.RawUI.CursorPosition; [Console]::Out.WriteLine('{CURSOR_ANSWERED}'); \
                     [void][Console]::In.ReadLine()"
                ),
            ],
        )
    } else {
        spec(
            "/bin/sh",
            &[
                "-c",
                &format!(
                    "stty raw -echo; printf '\\033[6n'; dd bs=1 count=6 >/dev/null 2>&1; \
                     stty sane; echo {CURSOR_ANSWERED}; read reply"
                ),
            ],
        )
    }
}

/// What [`asks_questions_when_told`] prints before the answers it read, and after them.
pub const ANSWERS_START: &str = "ANSWERS:";
pub const ANSWERS_END: &str = ":END";

/// A program that asks the terminal several questions once a line is typed, so that
/// clients can attach first: where the cursor is (`CSI 6 n`), what the terminal is
/// (`CSI c`), whether it is well (`CSI 5 n`) and its background colour (`OSC 11`). After
/// a second it prints every byte it was answered with, in hexadecimal, between
/// [`ANSWERS_START`] and [`ANSWERS_END`].
///
/// On Windows the console host between the program and the terminal answers such
/// questions itself, so there the program asks for the cursor through the console and
/// prints nothing between the marks: the case shows only that it is answered.
pub fn asks_questions_when_told() -> SpawnSpec {
    if cfg!(windows) {
        spec(
            "powershell.exe",
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!(
                    "[void][Console]::In.ReadLine(); $p=$Host.UI.RawUI.CursorPosition; \
                     [Console]::Out.WriteLine('{ANSWERS_START}{ANSWERS_END}'); \
                     [void][Console]::In.ReadLine()"
                ),
            ],
        )
    } else {
        spec(
            "/bin/sh",
            &[
                "-c",
                &format!(
                    "read go; stty raw -echo; \
                     printf '\\033[6n\\033[c\\033[5n\\033]11;?\\007'; sleep 1; \
                     stty -icanon min 0 time 10; \
                     got=$(dd bs=4096 count=1 2>/dev/null | od -An -tx1 | tr -d ' \\n'); \
                     stty sane; echo \"{ANSWERS_START}$got{ANSWERS_END}\"; read reply"
                ),
            ],
        )
    }
}

/// The answers [`asks_questions_when_told`] printed, from the lines of its screen:
/// the final byte of each control sequence (`R`, `c`, `n`), sorted, and how many
/// colour reports (`OSC 11`) there were.
pub fn answers_on(lines: &[String]) -> (Vec<char>, usize) {
    // A long line of answers wraps; the screen's rows join into it again.
    let joined: String = lines.concat();
    let start = joined
        .find(ANSWERS_START)
        .unwrap_or_else(|| panic!("no answers on the screen: {joined:?}"))
        + ANSWERS_START.len();
    let end = start
        + joined[start..]
            .find(ANSWERS_END)
            .unwrap_or_else(|| panic!("the answers did not end: {joined:?}"));
    let hex = &joined[start..end];
    let bytes: Vec<u8> = (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("hexadecimal"))
        .collect();
    let mut finals = Vec::new();
    let mut colours = 0;
    let mut i = 0;
    while i + 1 < bytes.len() {
        match (bytes[i], bytes[i + 1]) {
            (0x1b, b'[') => {
                let mut j = i + 2;
                while j < bytes.len() && !(0x40..=0x7e).contains(&bytes[j]) {
                    j += 1;
                }
                if let Some(&last) = bytes.get(j) {
                    finals.push(char::from(last));
                }
                i = j + 1;
            }
            (0x1b, b']') => {
                if bytes[i + 2..].starts_with(b"11;") {
                    colours += 1;
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    finals.sort_unstable();
    (finals, colours)
}

pub fn missing_program() -> SpawnSpec {
    spec("no-such-program-anywhere", &[])
}

/// A connection to a host, speaking the protocol by hand.
pub struct Client {
    writer: Box<dyn AsyncWrite + Send + Unpin>,
    /// Decoded messages. An `Err` is a decode failure; the end is the connection closing.
    incoming: mpsc::UnboundedReceiver<Result<Message, String>>,
    /// Messages received while waiting for a different one, oldest first.
    stash: VecDeque<Message>,
    /// The state each attachment started from, by stream.
    hydrated: HashMap<u32, HydratePayload>,
    /// The engine version this client says it runs.
    vt_version: Option<String>,
    next_request: u64,
    reading: JoinHandle<()>,
}

impl Drop for Client {
    fn drop(&mut self) {
        // Stop reading too, so the connection really closes when the client goes.
        self.reading.abort();
    }
}

impl Client {
    pub fn new<S>(io: S) -> Self
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let (mut reader, writer) = tokio::io::split(io);
        let (decoded, incoming) = mpsc::unbounded_channel();
        let reading = tokio::spawn(async move {
            let mut decoder = FrameDecoder::new(Role::Client);
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match reader.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(read) => decoder.push(&buf[..read]),
                }
                loop {
                    match decoder.next() {
                        Ok(Some(message)) => {
                            if decoded.send(Ok(message)).is_err() {
                                return;
                            }
                        }
                        Ok(None) => break,
                        Err(error) => {
                            let _ = decoded.send(Err(error.to_string()));
                            return;
                        }
                    }
                }
            }
        });
        Self {
            writer: Box::new(writer),
            incoming,
            stash: VecDeque::new(),
            hydrated: HashMap::new(),
            vt_version: Some(lume_host::vt_version().to_owned()),
            next_request: 1,
            reading,
        }
    }

    /// Stops reading the connection, as a client that hangs does; what the host sends
    /// piles up in the connection.
    #[allow(dead_code, reason = "not every test binary uses it")]
    pub fn stop_reading(&mut self) {
        self.reading.abort();
    }

    pub async fn send(&mut self, message: Message) {
        let mut frames = Vec::new();
        encode(&message, &mut frames).unwrap();
        self.send_raw(&frames).await;
    }

    /// Writes bytes as they are, valid frames or not.
    pub async fn send_raw(&mut self, bytes: &[u8]) {
        self.writer
            .write_all(bytes)
            .await
            .expect("the connection takes bytes");
        self.writer.flush().await.expect("the connection flushes");
    }

    /// The next message off the wire, or `None` if the connection closed or `wait` passed.
    async fn receive(&mut self, wait: Duration) -> Option<Message> {
        match tokio::time::timeout(wait, self.incoming.recv()).await {
            Ok(Some(Ok(message))) => Some(message),
            Ok(Some(Err(error))) => panic!("the host sent something undecodable: {error}"),
            Ok(None) | Err(_) => None,
        }
    }

    /// The oldest message for which `wanted` holds, waiting up to `wait` for one.
    /// Messages that do not match are kept for later calls, in order.
    pub async fn find(
        &mut self,
        wait: Duration,
        wanted: impl Fn(&Message) -> bool,
    ) -> Option<Message> {
        if let Some(at) = self.stash.iter().position(&wanted) {
            return self.stash.remove(at);
        }
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.checked_duration_since(Instant::now())?;
            let message = self.receive(left).await?;
            if wanted(&message) {
                return Some(message);
            }
            self.stash.push_back(message);
        }
    }

    /// Like [`find`](Self::find), but the message must arrive.
    pub async fn expect(&mut self, what: &str, wanted: impl Fn(&Message) -> bool) -> Message {
        match self.find(STEP, wanted).await {
            Some(message) => message,
            None => panic!("{what} never arrived; kept: {:?}", self.stash),
        }
    }

    pub async fn request(&mut self, body: RequestBody) -> Result<ResponseBody, ProtoError> {
        let id = self.next_request;
        self.next_request += 1;
        self.send(Message::control(Control::Request(Request { id, body })))
            .await;
        let is_answer = |message: &Message| matches!(&message.body, Body::Control(Control::Response(response)) if response.id == id);
        match self.expect("the response", is_answer).await.body {
            Body::Control(Control::Response(response)) => response.result,
            _ => unreachable!("matched as a response"),
        }
    }

    /// Says hello with the current protocol version and the host's own engine version,
    /// so that attachments start from a snapshot, and returns the host's process id.
    pub async fn hello(&mut self) -> u32 {
        let vt_version = self.vt_version.clone();
        self.hello_as(vt_version).await
    }

    /// Says hello as a client running engine `vt_version`, or none.
    pub async fn hello_as(&mut self, vt_version: Option<String>) -> u32 {
        self.vt_version = vt_version.clone();
        let hello = RequestBody::Hello {
            proto: PROTO_VERSION,
            proto_min: PROTO_VERSION,
            vt_version,
            client_kind: ClientKind::Test,
        };
        match self.request(hello).await {
            Ok(ResponseBody::HelloAck { pid, .. }) => pid,
            other => panic!("hello was answered with {other:?}"),
        }
    }

    pub async fn spawn(&mut self, spec: SpawnSpec) -> SessionId {
        match self.request(RequestBody::Spawn { spec }).await {
            Ok(ResponseBody::Spawned { session }) => session,
            other => panic!("spawn was answered with {other:?}"),
        }
    }

    /// Attaches, and takes the state the attachment starts from off the stream: it is
    /// the first thing sent on it. [`hydrated`](Self::hydrated) returns it.
    pub async fn attach(
        &mut self,
        session: SessionId,
        stream: u32,
    ) -> Result<ResponseBody, ProtoError> {
        self.attach_with(session, stream, facts()).await
    }

    /// Like [`attach`](Self::attach), with the client's view as given.
    pub async fn attach_with(
        &mut self,
        session: SessionId,
        stream: u32,
        facts: ClientFacts,
    ) -> Result<ResponseBody, ProtoError> {
        let response = self
            .request(RequestBody::Attach {
                session,
                stream,
                facts,
                replay: false,
            })
            .await;
        if let Ok(ResponseBody::Attached { hydrate, .. }) = &response
            && *hydrate != HydrateMode::None
        {
            let is_hydrate = |message: &Message| {
                message.stream == stream && matches!(message.body, Body::Hydrate(_))
            };
            let Body::Hydrate(payload) = self
                .expect("the screen to start from", is_hydrate)
                .await
                .body
            else {
                unreachable!("matched as a hydrate");
            };
            self.hydrated.insert(stream, payload);
        }
        response
    }

    /// What the attachment on `stream` started from.
    pub fn hydrated(&self, stream: u32) -> Option<&HydratePayload> {
        self.hydrated.get(&stream)
    }

    /// The text of the screen the attachment on `stream` started from, as a client's
    /// terminal of the host's own engine shows it once it has loaded it.
    pub fn hydrated_text(&self, stream: u32) -> String {
        let terminal = match self
            .hydrated(stream)
            .expect("the attachment started from a screen")
        {
            HydratePayload::Snapshot { bytes, .. } => {
                lume_vt::Terminal::from_snapshot(bytes).expect("the snapshot loads")
            }
            HydratePayload::VtReplay(bytes) => {
                let mut terminal = lume_vt::Terminal::new(80, 24).unwrap();
                terminal.write(bytes);
                terminal
            }
        };
        terminal.screen_text().unwrap().join("\n")
    }

    /// Attaches and grants a window of credit, as a real client does at once.
    pub async fn attach_with_credit(&mut self, session: SessionId, stream: u32) {
        let attached = self.attach(session, stream).await;
        assert!(
            matches!(attached, Ok(ResponseBody::Attached { .. })),
            "{attached:?}"
        );
        self.credit(stream, 1024 * 1024).await;
    }

    /// Takes the next copy of the screen sent on `stream` in the middle of its output,
    /// and the output before it.
    pub async fn rehydrated(&mut self, stream: u32) -> HydratePayload {
        let is_hydrate = |message: &Message| {
            message.stream == stream && matches!(message.body, Body::Hydrate(_))
        };
        match self.expect("a copy of the screen", is_hydrate).await.body {
            Body::Hydrate(payload) => payload,
            _ => unreachable!("matched as a hydrate"),
        }
    }

    pub async fn credit(&mut self, stream: u32, bytes: u32) {
        self.send(Message::new(stream, Body::Credit(bytes))).await;
    }

    /// Types `line` and presses Enter.
    pub async fn type_line(&mut self, stream: u32, line: &str) {
        self.send(Message::new(
            stream,
            Body::Input(format!("{line}\r").into_bytes()),
        ))
        .await;
    }

    pub async fn list(&mut self) -> Vec<SessionInfo> {
        match self.request(RequestBody::List {}).await {
            Ok(ResponseBody::Sessions { sessions }) => sessions,
            other => panic!("list was answered with {other:?}"),
        }
    }

    /// The one session the host is expected to have.
    pub async fn only_session(&mut self) -> SessionInfo {
        let mut sessions = self.list().await;
        assert_eq!(sessions.len(), 1, "{sessions:?}");
        sessions.remove(0)
    }

    /// The next output bytes on `stream`, if any arrive within `wait`.
    pub async fn data(&mut self, stream: u32, wait: Duration) -> Option<Vec<u8>> {
        let is_data =
            |message: &Message| message.stream == stream && matches!(message.body, Body::Data(_));
        match self.find(wait, is_data).await?.body {
            Body::Data(bytes) => Some(bytes),
            _ => unreachable!("matched as data"),
        }
    }

    /// Reads output on `stream` until it contains `needle`, and returns all of it.
    pub async fn read_until(&mut self, stream: u32, needle: &str) -> String {
        let deadline = Instant::now() + STEP;
        let mut text = String::new();
        while !text.contains(needle) {
            let left = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_default();
            match self.data(stream, left).await {
                Some(bytes) => text.push_str(&String::from_utf8_lossy(&bytes)),
                None => panic!("`{needle}` never appeared on stream {stream}; got {text:?}"),
            }
        }
        text
    }

    /// Discards the output on `stream` until none has arrived for a moment.
    pub async fn discard_output(&mut self, stream: u32) {
        while self
            .data(stream, Duration::from_millis(300))
            .await
            .is_some()
        {}
    }

    /// The next message on `stream` that is neither output nor something the session
    /// reported: a mark of where the terminal was resized or the program ended.
    pub async fn marker(&mut self, stream: u32) -> Body {
        let is_marker = |message: &Message| {
            message.stream == stream
                && !matches!(
                    message.body,
                    Body::Data(_) | Body::Event(_) | Body::Hydrate(_)
                )
        };
        self.expect("a marker", is_marker).await.body
    }

    /// The oldest report on `stream` for which `wanted` holds, if one arrives within
    /// `wait`.
    pub async fn event_within(
        &mut self,
        stream: u32,
        wait: Duration,
        wanted: impl Fn(&SessionEvent) -> bool,
    ) -> Option<SessionEvent> {
        let is_wanted = |message: &Message| {
            message.stream == stream && matches!(&message.body, Body::Event(event) if wanted(event))
        };
        match self.find(wait, is_wanted).await?.body {
            Body::Event(event) => Some(event),
            _ => unreachable!("matched as an event"),
        }
    }

    /// Like [`event_within`](Self::event_within), but the report must arrive.
    pub async fn event(
        &mut self,
        stream: u32,
        what: &str,
        wanted: impl Fn(&SessionEvent) -> bool,
    ) -> SessionEvent {
        match self.event_within(stream, STEP, wanted).await {
            Some(event) => event,
            None => panic!("{what} was never reported; kept: {:?}", self.stash),
        }
    }

    /// The next notification on the control stream.
    pub async fn notification(&mut self) -> Notification {
        let is_notification = |message: &Message| {
            message.stream == CONTROL_STREAM
                && matches!(message.body, Body::Control(Control::Notify(_)))
        };
        match self.expect("a notification", is_notification).await.body {
            Body::Control(Control::Notify(notification)) => notification,
            _ => unreachable!("matched as a notification"),
        }
    }

    /// Whether the host closes the connection within the step timeout.
    pub async fn closes(&mut self) -> bool {
        let deadline = Instant::now() + STEP;
        loop {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            match tokio::time::timeout(left, self.incoming.recv()).await {
                Ok(Some(Ok(message))) => self.stash.push_back(message),
                Ok(Some(Err(_))) | Ok(None) => return true,
                Err(_) => return false,
            }
        }
    }

    /// Polls the session list until `settled` holds for it.
    pub async fn list_until(
        &mut self,
        what: &str,
        settled: impl Fn(&[SessionInfo]) -> bool,
    ) -> Vec<SessionInfo> {
        let deadline = Instant::now() + STEP;
        loop {
            let sessions = self.list().await;
            if settled(&sessions) {
                return sessions;
            }
            assert!(Instant::now() < deadline, "{what}: {sessions:?}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// A host and a client connected to it.
///
/// The client is declared first so that it is dropped first: the host learns the
/// connection has closed before anything waits for the host to finish.
pub struct Running {
    pub client: Client,
    host: HostRun,
}

enum HostRun {
    /// The host serving the connection as a task in this process.
    Task(JoinHandle<()>),
    Process(HostProcess),
}

impl Running {
    /// Closes the connection and waits for the host to finish with it.
    ///
    /// Every case ends with this. The host ends the programs a connection started
    /// before it stops serving that connection, so once this returns none is running.
    /// Ending a test without it would leave the last sessions to whatever happens when
    /// the process exits, and a program in a pseudoconsole outlives a process that
    /// exits without closing it.
    pub async fn shutdown(self) {
        let Running { client, host } = self;
        drop(client);
        match host {
            HostRun::Task(serving) => {
                let finished = tokio::time::timeout(STEP, serving).await;
                finished
                    .expect("the host finishes with a closed connection")
                    .unwrap();
            }
            HostRun::Process(mut process) => {
                let deadline = Instant::now() + STEP;
                while !process.exited() {
                    assert!(Instant::now() < deadline, "the host process did not exit");
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        }
    }
}

/// A way of running the host for a test.
pub trait Harness {
    fn start() -> impl Future<Output = Running> + Send;
}

/// The host as a task in the test's own process, reached over an in-memory pipe.
pub struct InProcess;

impl Harness for InProcess {
    async fn start() -> Running {
        let (client_end, host_end) = tokio::io::duplex(256 * 1024);
        let serving = tokio::spawn(async move {
            let _ = Host::new(HostConfig::default()).serve(host_end).await;
        });
        Running {
            client: Client::new(client_end),
            host: HostRun::Task(serving),
        }
    }
}

/// The host as a separate process, reached over the per-user endpoint.
pub struct OutOfProcess;

struct HostProcess(std::process::Child);

impl HostProcess {
    fn exited(&mut self) -> bool {
        !matches!(self.0.try_wait(), Ok(None))
    }
}

impl Drop for HostProcess {
    fn drop(&mut self) {
        // After a shutdown the process has already exited. After a failed test it has
        // not: its connection has just been dropped, so give it the time it needs to
        // end its programs and exit by itself before it is ended by force, which would
        // leave those programs running.
        let deadline = Instant::now() + Duration::from_secs(8);
        while !self.exited() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Harness for OutOfProcess {
    async fn start() -> Running {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let name = format!(
            "host-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let endpoint = lume_ipc::Endpoint::named(&name).unwrap();

        let process = std::process::Command::new(env!("CARGO_BIN_EXE_lume-host"))
            .args(["--endpoint", &name])
            .stdin(std::process::Stdio::null())
            .spawn()
            .expect("the host binary starts");
        let process = HostProcess(process);

        // The host needs a moment to create its endpoint.
        let deadline = Instant::now() + STEP;
        let stream = loop {
            match lume_ipc::connect(&endpoint).await {
                Ok(stream) => break stream,
                Err(error) => {
                    assert!(
                        Instant::now() < deadline,
                        "the host never served {endpoint}: {error}"
                    );
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        };
        Running {
            client: Client::new(stream),
            host: HostRun::Process(process),
        }
    }
}

/// The memory a process holds in RAM, in bytes.
#[cfg(windows)]
pub fn resident_bytes(pid: u32) -> u64 {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    // SAFETY: takes no pointers. A null result is checked below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    assert!(!process.is_null(), "process {pid} cannot be opened");
    // SAFETY: plain data for which all-zero is a valid value.
    let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    let size = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: `process` is open and `counters` is valid for `size` bytes.
    let ok = unsafe { K32GetProcessMemoryInfo(process, &mut counters, size) };
    // SAFETY: `process` is open and owned here.
    unsafe { CloseHandle(process) };
    assert_ne!(ok, 0, "the memory of process {pid} cannot be read");
    counters.WorkingSetSize as u64
}

#[cfg(target_os = "linux")]
pub fn resident_bytes(pid: u32) -> u64 {
    let statm = std::fs::read_to_string(format!("/proc/{pid}/statm")).unwrap();
    let pages: u64 = statm.split_whitespace().nth(1).unwrap().parse().unwrap();
    // SAFETY: takes no pointers.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    pages * page as u64
}

#[cfg(all(unix, not(target_os = "linux")))]
pub fn resident_bytes(pid: u32) -> u64 {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let kib: u64 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap();
    kib * 1024
}

/// Whether a process with this id is running.
#[cfg(windows)]
pub fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    // SAFETY: takes no pointers. A null result is checked below.
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if process.is_null() {
        return false;
    }
    // SAFETY: `process` is open; a zero timeout only asks whether it has ended.
    let alive = unsafe { WaitForSingleObject(process, 0) } == WAIT_TIMEOUT;
    // SAFETY: `process` is open and owned here.
    unsafe { CloseHandle(process) };
    alive
}

#[cfg(unix)]
pub fn process_alive(pid: u32) -> bool {
    // SAFETY: takes no pointers; signal 0 only checks that the process exists.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}
