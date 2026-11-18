//! One session: a terminal, the program in it, its screen, and the client attached to it.
//!
//! Terminal I/O blocks, so each session has three threads: one reads the terminal, one
//! writes to it, one waits for the program to end. A fourth owns the session's screen
//! (see [`crate::screen`]). Everything else happens in one async task, which owns all of
//! the session's state and is the only thing that touches it.
//!
//! Every byte read goes to the screen first, so the screen is always as far as the
//! output. A client that attaches starts from a copy of the screen and then takes the
//! output that follows. Besides moving output, the task looks at it as it goes by for
//! what the program reports about itself (its title, its directory, its commands), asks
//! the system now and then which program has the terminal, and passes both on as events,
//! in their place in the output.
//!
//! Backpressure is the absence of work. The reading thread hands each read to the task
//! through a queue two reads long. With no client attached the task takes every read:
//! the program runs on, and its output goes to the screen alone. When a client is
//! attached and has granted no more credit, the task stops taking from that queue, the
//! thread blocks on it, the terminal's own buffer fills, and the program blocks on its
//! next write. Nothing is dropped and nothing grows.

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use lume_proto::{
    Body, BytesKind, CellPx, ClientFacts, ErrorCode, ExitStatus, Foreground, HydrateMode,
    HydratePayload, Message, Notification, ProgramState, ProgramStatus, ProtoError, ResponseBody,
    SendWindow, SessionEvent, SessionId, SessionInfo, SessionState, SessionStats, TermSize, encode,
    encode_bytes,
};
use lume_pty::{Child, ChildKiller, ForegroundProbe, ForegroundProcess, Pty, PtySize, ReadStop};
use tokio::sync::{mpsc, oneshot};

use crate::Host;
use crate::conn::Outbox;
use crate::pending::{Flush, Pending, Segment};
use crate::scan::{Report, Scanner};
use crate::screen::{Answers, Found, HydrateWith, Screen, ScreenText};
use crate::ssh_exit::{self, Tail};
use crate::tuning::{
    ACTIVITY_INTERVAL, COALESCE_MAX_BYTES, COMMAND_QUEUE, EVENT_WINDOW, EVENTS_PER_WINDOW_MAX,
    EXIT_DRAIN_TIMEOUT, EXITED_UNSEEN_LINGER, FOREGROUND_SETTLE, INPUT_BACKLOG_MAX, READ_CHUNK,
    READ_QUEUE_CHUNKS, SEND_BATCH_BYTES, STATE_STATUSES_MAX_BYTES, UNATTACHED_EXIT_LINGER,
};

/// Identifies one attachment of a session. A frame from a client names the attachment
/// it was sent for, so a frame that arrives after its attachment ended is recognised
/// and ignored instead of being applied to the next one.
pub(crate) type AttachmentId = u64;

/// Something for a session to do, from a connection.
pub(crate) enum Command {
    /// Attach a client. The session writes the response to `outbox` itself, so that it
    /// is on the wire before any frame of the new stream. `reply` says whether the
    /// client is now attached.
    Attach {
        request: u64,
        stream: u32,
        facts: ClientFacts,
        /// How the client is to be brought up to date, by what its engine can load.
        hydrate: HydrateWith,
        outbox: Outbox,
        reply: oneshot::Sender<Option<AttachmentId>>,
    },
    Input {
        attachment: AttachmentId,
        bytes: Vec<u8>,
    },
    Resize {
        attachment: AttachmentId,
        size: TermSize,
        cell_px: CellPx,
    },
    Facts {
        attachment: AttachmentId,
        facts: ClientFacts,
    },
    Credit {
        attachment: AttachmentId,
        bytes: u32,
    },
    Detach {
        attachment: AttachmentId,
    },
    /// Send the attachment a copy of the screen, behind the output already on its way
    /// to it, for the client to start again from.
    Rehydrate {
        attachment: AttachmentId,
    },
    /// Read the screen as text.
    Screen {
        reply: oneshot::Sender<Option<ScreenText>>,
    },
    /// Wait for text to be on the screen. `reply` is answered with where it was found,
    /// or with `None` at the deadline.
    WaitText {
        text: String,
        deadline: Instant,
        reply: oneshot::Sender<Option<Found>>,
    },
    /// End the program. The session reports the exit and then removes itself.
    Kill,
    /// The connection that started the session has gone, and with it any reason to keep
    /// the session.
    Abandon,
}

/// What the rest of the host holds of a session: a way to send it commands and a view
/// of its state for listings.
#[derive(Clone)]
pub(crate) struct SessionHandle {
    commands: mpsc::Sender<Command>,
    shared: Arc<Mutex<Shared>>,
}

impl SessionHandle {
    /// Sends a command. `false` means the session has ended.
    pub(crate) async fn send(&self, command: Command) -> bool {
        self.commands.send(command).await.is_ok()
    }

    /// Resolves once the session has ended.
    pub(crate) async fn ended(&self) {
        self.commands.closed().await;
    }

    pub(crate) fn info(&self, id: SessionId) -> SessionInfo {
        let shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        SessionInfo {
            id,
            program: shared.program.clone(),
            pid: shared.pid,
            size: shared.size,
            created_unix_ms: shared.created_unix_ms,
            attached: shared.attached,
            exit: shared.exit,
            stats: shared.stats,
            title: shared.title.clone(),
            cwd: shared.cwd.clone(),
        }
    }
}

/// The part of a session's state that listings read.
struct Shared {
    program: String,
    pid: Option<u32>,
    size: TermSize,
    created_unix_ms: u64,
    attached: bool,
    exit: Option<ExitStatus>,
    stats: SessionStats,
    title: Option<String>,
    cwd: Option<String>,
}

/// Everything needed to start a session around a program that is already running.
pub(crate) struct Start {
    pub(crate) host: Arc<Host>,
    pub(crate) id: SessionId,
    pub(crate) program: String,
    pub(crate) pty: Box<dyn Pty>,
    pub(crate) child: Box<dyn Child>,
    pub(crate) size: TermSize,
    pub(crate) owner: Outbox,
    /// About how many lines of scrollback the screen keeps.
    pub(crate) scrollback: usize,
    pub(crate) resize_pulls_scrollback: bool,
    /// The program is the OpenSSH client, whose last words say how its connection
    /// ended.
    pub(crate) ssh: bool,
}

fn thread(name: &str, run: impl FnOnce() + Send + 'static) -> io::Result<()> {
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(run)
        .map(drop)
}

/// Starts the threads and the task of a session. Must be called within a Tokio runtime.
pub(crate) fn start(start: Start) -> io::Result<SessionHandle> {
    let Start {
        host,
        id,
        program,
        mut pty,
        mut child,
        size,
        owner,
        scrollback,
        ssh,
        ..
    } = start;
    let probe = pty.foreground_probe();
    let mut reader = pty.take_reader()?;
    let mut writer = pty.take_writer()?;
    let stopper = reader.stopper();
    let killer = child.killer();
    let pid = child.pid();

    let (chunk_tx, chunks) = mpsc::channel::<Vec<u8>>(READ_QUEUE_CHUNKS);
    thread("session-read", move || {
        let mut buf = vec![0u8; READ_CHUNK];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    // Blocks while the session is not taking output. An error means the
                    // session has ended.
                    if chunk_tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    tracing::debug!(session = %id, %error, "reading the terminal failed");
                    break;
                }
            }
        }
    })?;

    let (input, mut input_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let input_queued = Arc::new(AtomicUsize::new(0));
    let written = Arc::clone(&input_queued);
    let screen = Screen::start(
        "session",
        size,
        scrollback,
        Answers {
            input: input.clone(),
            queued: Arc::clone(&input_queued),
        },
    )?;
    thread("session-write", move || {
        while let Some(bytes) = input_rx.blocking_recv() {
            let result = writer.write_all(&bytes);
            written.fetch_sub(bytes.len(), Ordering::Relaxed);
            if let Err(error) = result {
                tracing::debug!(session = %id, %error, "writing to the terminal failed");
                break;
            }
        }
    })?;

    let (exit_tx, exit_rx) = oneshot::channel();
    thread("session-wait", move || {
        let status = match child.wait() {
            Ok(lume_pty::ExitStatus::Code(code)) => ExitStatus::Code(code),
            Ok(lume_pty::ExitStatus::Signal(signal)) => ExitStatus::Signal(signal),
            Ok(lume_pty::ExitStatus::Unknown) => ExitStatus::Unknown,
            Err(error) => {
                tracing::debug!(session = %id, %error, "waiting for the program failed");
                ExitStatus::Unknown
            }
        };
        let _ = exit_tx.send(status);
    })?;

    let created_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        });
    let shared = Arc::new(Mutex::new(Shared {
        program,
        pid: Some(pid),
        size,
        created_unix_ms,
        attached: false,
        exit: None,
        stats: SessionStats::default(),
        title: None,
        cwd: None,
    }));

    let (command_tx, commands) = mpsc::channel(COMMAND_QUEUE);
    let (poll_tx, polled) = mpsc::channel(1);
    let session = Session {
        host,
        id,
        pty,
        killer,
        stopper,
        size,
        resize_pulls_scrollback: start.resize_pulls_scrollback,
        screen,
        owner,
        commands,
        commands_closed: false,
        chunks,
        exit_rx,
        input,
        input_queued,
        input_dropped: false,
        pending: Pending::new(),
        attachment: None,
        next_attachment: 1,
        ever_attached: false,
        last_input: None,
        flush_at: None,
        drain_at: None,
        linger_at: None,
        linger_over: false,
        reader_done: false,
        exit: None,
        exit_queued: false,
        exit_delivered: false,
        ending: None,
        read_bytes: 0,
        sent_bytes: 0,
        shared: Arc::clone(&shared),
        scanner: Scanner::new(),
        found: Vec::new(),
        seen: SessionState::default(),
        tail: ssh.then(Tail::new),
        window_start: Instant::now(),
        window_events: 0,
        resync_at: None,
        last_activity: None,
        probe,
        poll_tx,
        polled,
        poll_at: None,
        polling: false,
    };
    tokio::spawn(session.run());

    Ok(SessionHandle {
        commands: command_tx,
        shared,
    })
}

/// A client attached to a session.
struct Attachment {
    id: AttachmentId,
    stream: u32,
    outbox: Outbox,
    /// The output bytes the client has said it can take.
    window: SendWindow,
    /// The client's view, kept for when the host answers a program on its behalf.
    facts: ClientFacts,
    /// How the client is brought up to date, by what its engine can load.
    hydrate: HydrateWith,
}

/// Why a session is being ended before its program chose to end.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ending {
    Killed,
    Abandoned,
}

struct Session {
    host: Arc<Host>,
    id: SessionId,
    pty: Box<dyn Pty>,
    killer: Box<dyn ChildKiller>,
    stopper: Box<dyn ReadStop>,
    size: TermSize,
    resize_pulls_scrollback: bool,
    /// The session's own copy of its screen.
    screen: Screen,
    /// The connection that started the session.
    owner: Outbox,

    commands: mpsc::Receiver<Command>,
    commands_closed: bool,
    chunks: mpsc::Receiver<Vec<u8>>,
    exit_rx: oneshot::Receiver<ExitStatus>,
    input: mpsc::UnboundedSender<Vec<u8>>,
    /// Input bytes handed to the writing thread and not yet written.
    input_queued: Arc<AtomicUsize>,
    input_dropped: bool,

    pending: Pending,
    attachment: Option<Attachment>,
    next_attachment: AttachmentId,
    ever_attached: bool,
    last_input: Option<Instant>,

    /// When waiting output must be sent.
    flush_at: Option<Instant>,
    /// When to stop waiting for more output from a program that has ended.
    drain_at: Option<Instant>,
    /// When to give up on a client ever attaching to a program that has ended.
    linger_at: Option<Instant>,
    linger_over: bool,

    reader_done: bool,
    exit: Option<ExitStatus>,
    /// The exit marker is in `pending`: no more output will follow.
    exit_queued: bool,
    /// The exit marker has been sent to a client.
    exit_delivered: bool,
    ending: Option<Ending>,

    read_bytes: u64,
    sent_bytes: u64,
    shared: Arc<Mutex<Shared>>,

    scanner: Scanner,
    /// What the scanner found in the read being handled.
    found: Vec<Report>,
    /// What the session's reports add up to, as far as the output has been read.
    seen: SessionState,
    /// The end of the output, kept where it will be read for how the session ended.
    tail: Option<Tail>,
    /// When the current count of passed-on reports began, and the count.
    window_start: Instant,
    window_events: u32,
    /// When to tell the client the whole state, because reports were left out.
    resync_at: Option<Instant>,
    /// When the session last said it printed while its view was not being typed in.
    last_activity: Option<Instant>,

    /// The way to ask which program has the terminal.
    probe: Option<Arc<dyn ForegroundProbe>>,
    poll_tx: mpsc::Sender<Option<ForegroundProcess>>,
    polled: mpsc::Receiver<Option<ForegroundProcess>>,
    /// When to ask next.
    poll_at: Option<Instant>,
    /// An answer is on its way.
    polling: bool,
}

/// The earlier of two times that may each be unset.
fn sooner(a: Option<Instant>, b: Option<Instant>) -> Option<Instant> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Roughly how many bytes a status record takes when it is sent. Text may need
/// escaping, which at worst doubles it.
fn weight(status: &ProgramStatus) -> usize {
    let len = |text: &Option<String>| text.as_ref().map_or(0, String::len);
    64 + len(&status.id) + len(&status.app) + 2 * (len(&status.title) + len(&status.message))
}

/// A session's whole state as one event. A state with more status records than one
/// event can carry goes without the ones reported longest ago.
fn whole(state: &SessionState) -> SessionEvent {
    let mut state = state.clone();
    let mut total: usize = state.statuses.iter().map(weight).sum();
    while total > STATE_STATUSES_MAX_BYTES && !state.statuses.is_empty() {
        total -= weight(&state.statuses.remove(0));
    }
    SessionEvent::State { state }
}

/// Resolves at `deadline`, or never if there is none.
async fn at(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
        None => std::future::pending().await,
    }
}

impl Session {
    async fn run(mut self) {
        loop {
            self.publish();
            self.arm_linger();
            if self.finished() {
                break;
            }
            let ingest = self.can_ingest();
            let poll = self.can_poll();
            // In order of urgency. Commands come first so that credit and kills are seen
            // even while output floods in; output comes before the timers so that a timer
            // which fell due while output was held back does not fire ahead of it.
            tokio::select! {
                biased;
                command = self.commands.recv(), if !self.commands_closed => match command {
                    Some(command) => self.on_command(command).await,
                    None => {
                        self.commands_closed = true;
                        self.end(Ending::Abandoned);
                    }
                },
                chunk = self.chunks.recv(), if ingest => match chunk {
                    Some(bytes) => self.on_chunk(bytes).await,
                    None => self.on_reader_end().await,
                },
                status = &mut self.exit_rx, if self.exit.is_none() => {
                    self.on_exit(status.unwrap_or(ExitStatus::Unknown)).await;
                }
                () = at(self.flush_at) => self.flush().await,
                found = self.polled.recv() => self.on_polled(found.flatten()).await,
                () = at(self.poll_at), if poll => self.poll(),
                () = at(self.resync_at) => self.resync().await,
                () = at(self.drain_at), if ingest => self.on_drain_timeout().await,
                () = at(self.linger_at) => {
                    self.linger_at = None;
                    self.linger_over = true;
                }
            }
        }

        self.host.remove(self.id);
        // A program left behind may still hold the terminal open; do not leave the
        // reading thread waiting on it.
        self.stopper.stop();
        tracing::debug!(session = %self.id, "session ended");
    }

    /// Whether the session has nothing left to do and can be removed.
    fn finished(&self) -> bool {
        if !self.exit_queued {
            return false;
        }
        if self.ending.is_some() {
            // Nobody is owed anything but the exit itself.
            return self.attachment.is_none() || self.exit_delivered;
        }
        match self.attachment {
            // The client is still showing the session; it goes when the client detaches.
            Some(_) => false,
            // A client saw how it ended, or nobody came back for it in time.
            None => self.exit_delivered || self.linger_over,
        }
    }

    /// Starts the wait for a client to come back to a session that ended while nobody
    /// was there to see it.
    fn arm_linger(&mut self) {
        let unseen = self.exit_queued
            && !self.exit_delivered
            && self.attachment.is_none()
            && self.ending.is_none();
        if unseen && self.linger_at.is_none() && !self.linger_over {
            let linger = if self.host.persistent() {
                EXITED_UNSEEN_LINGER
            } else {
                UNATTACHED_EXIT_LINGER
            };
            self.linger_at = Some(Instant::now() + linger);
        }
    }

    /// Whether to take more output from the reading thread. Not taking it is what stops
    /// the program. With no client attached everything is taken: the program runs on,
    /// and its output goes to the screen alone. A client is sent no more than one send
    /// can carry ahead of its credit.
    fn can_ingest(&self) -> bool {
        !self.reader_done
            && (self.ending.is_some()
                || self.attachment.is_none()
                || self.pending.bytes() < COALESCE_MAX_BYTES)
    }

    /// Makes the session's state visible to listings.
    fn publish(&self) {
        let mut shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        shared.size = self.size;
        shared.attached = self.attachment.is_some();
        shared.exit = self.exit;
        if shared.title != self.seen.title {
            shared.title.clone_from(&self.seen.title);
        }
        if shared.cwd != self.seen.cwd {
            shared.cwd.clone_from(&self.seen.cwd);
        }
        shared.stats = SessionStats {
            read_bytes: self.read_bytes,
            sent_bytes: self.sent_bytes,
            pending_bytes: u32::try_from(self.pending.bytes()).unwrap_or(u32::MAX),
            paused: !self.reader_done && !self.can_ingest(),
        };
    }

    /// Every byte read from the terminal passes through here, in the order it was
    /// read, before anything else is done with it. It may look and must not change or
    /// hold back: the bytes go on to the client exactly as they came.
    fn observe(&mut self, bytes: &[u8]) {
        self.read_bytes += bytes.len() as u64;
        if let Some(tail) = &mut self.tail {
            tail.push(bytes);
        }
        if self.ending.is_some() {
            return;
        }
        let Self { scanner, found, .. } = self;
        scanner.feed(bytes, |report| {
            // A run of bells is one bell.
            if report != Report::Bell || found.last() != Some(&Report::Bell) {
                found.push(report);
            }
        });
    }

    /// Passes on what was found in the output read last. Called once that output is
    /// queued, so that a report follows the bytes that made it.
    fn report(&mut self, now: Instant) {
        let mut found = std::mem::take(&mut self.found);
        for report in found.drain(..) {
            self.on_report(report, now);
        }
        // Kept for the next read, so that it is not allocated anew each time.
        self.found = found;

        let unwatched = self
            .attachment
            .as_ref()
            .is_some_and(|attachment| !attachment.facts.focused);
        let due = self
            .last_activity
            .is_none_or(|last| now.saturating_duration_since(last) >= ACTIVITY_INTERVAL);
        if unwatched && due {
            self.last_activity = Some(now);
            self.emit(SessionEvent::Activity, now);
            // A session nobody sees is not looked at now and then, and what prints may
            // be a program that has just started.
            if !self.shown() {
                self.poll_soon(now);
            }
        }
    }

    fn on_report(&mut self, report: Report, now: Instant) {
        match report {
            // Shells set the same title and directory at every prompt; only a change
            // is worth telling.
            Report::Title(title) => {
                if self.seen.title.as_deref().unwrap_or_default() != title {
                    self.emit(SessionEvent::Title { title }, now);
                }
            }
            Report::Cwd(url) => {
                if self.seen.cwd.as_deref() != Some(url.as_str()) {
                    self.emit(SessionEvent::Cwd { url }, now);
                }
            }
            Report::Bell => self.emit(SessionEvent::Bell, now),
            Report::Prompt => {
                // A prompt means the command line is over, whether or not the shell
                // said so, and with it the work a program said it was in the middle of.
                if self.seen.command_running() {
                    self.emit(SessionEvent::CommandFinished { exit: None }, now);
                }
                let unfinished = |status: &ProgramStatus| {
                    matches!(status.state, ProgramState::Working | ProgramState::Blocked)
                };
                if self.seen.statuses.iter().any(unfinished) {
                    self.seen.statuses.retain(|status| !unfinished(status));
                    self.emit(whole(&self.seen), now);
                }
                self.poll_soon(now);
            }
            Report::CommandStarted(command) => {
                self.emit(SessionEvent::CommandStarted { command }, now);
                self.poll_soon(now);
            }
            Report::CommandFinished(exit) => {
                // A shell that says a command ended when none began has nothing to say.
                if self.seen.command_running() {
                    self.emit(SessionEvent::CommandFinished { exit }, now);
                }
                self.poll_soon(now);
            }
            Report::Status(status) => self.emit(SessionEvent::ProgramStatus(status), now),
        }
    }

    /// Takes a report into the session's state and queues it for the client, unless
    /// too many were queued lately; then the client is told the whole state once the
    /// rush is over.
    fn emit(&mut self, event: SessionEvent, now: Instant) {
        self.seen.apply(&event);
        if now.saturating_duration_since(self.window_start) >= EVENT_WINDOW {
            self.window_start = now;
            self.window_events = 0;
        }
        if self.window_events >= EVENTS_PER_WINDOW_MAX {
            self.resync_at
                .get_or_insert(self.window_start + EVENT_WINDOW);
            return;
        }
        self.window_events += 1;
        self.pending.push_event(event);
    }

    /// Tells the client the whole state, after reports had to be left out.
    async fn resync(&mut self) {
        self.resync_at = None;
        self.pending.push_event(whole(&self.seen));
        self.flush().await;
    }

    fn shown(&self) -> bool {
        self.attachment
            .as_ref()
            .is_some_and(|attachment| attachment.facts.visible)
    }

    /// Whether to ask which program has the terminal when the time comes.
    fn can_poll(&self) -> bool {
        !self.polling && self.probe.is_some() && self.exit.is_none() && self.ending.is_none()
    }

    /// Has the program that has the terminal looked up shortly: something just
    /// happened that usually changes it.
    fn poll_soon(&mut self, now: Instant) {
        self.poll_at = sooner(self.poll_at, Some(now + FOREGROUND_SETTLE));
    }

    /// Asks which program has the terminal. The answer comes back to the task; asking
    /// may block for a moment, so it is done off the async threads.
    fn poll(&mut self) {
        self.poll_at = None;
        let Some(probe) = self.probe.clone() else {
            return;
        };
        self.polling = true;
        let answer = self.poll_tx.clone();
        tokio::task::spawn_blocking(move || {
            let _ = answer.blocking_send(probe.foreground());
        });
    }

    async fn on_polled(&mut self, found: Option<ForegroundProcess>) {
        self.polling = false;
        let now = Instant::now();
        // While a client shows the session it is asked again, and sooner if something
        // asked for it meanwhile.
        let next = self.shown().then(|| self.host.next_poll(now));
        self.poll_at = sooner(self.poll_at, next);
        let Some(ForegroundProcess { pid, name }) = found else {
            return;
        };
        let foreground = Foreground { pid, name };
        if self.seen.foreground.as_ref() != Some(&foreground) && self.ending.is_none() {
            self.emit(SessionEvent::Foreground(foreground), now);
            self.flush().await;
        }
    }

    async fn on_chunk(&mut self, bytes: Vec<u8>) {
        let now = Instant::now();
        if self.exit.is_some() {
            // Output is still coming, so keep waiting for the rest of it.
            self.drain_at = Some(now + EXIT_DRAIN_TIMEOUT);
        }
        self.observe(&bytes);
        if self.ending.is_some() {
            // Nobody wants the output of a session that is being ended.
            return;
        }
        let Some(flush) = self.take_output(bytes, now).await else {
            return;
        };
        match flush {
            Flush::Now => self.flush().await,
            Flush::At(at) => self.flush_at = Some(self.flush_at.map_or(at, |set| set.min(at))),
        }
    }

    /// Puts output that was read and observed on the screen and, when a client is
    /// attached, in the queue to it, followed by what was found in it. Says when to
    /// send, or `None` when nothing is queued for a client.
    async fn take_output(&mut self, bytes: Vec<u8>, now: Instant) -> Option<Flush> {
        if self.attachment.is_none() {
            self.screen.feed(bytes).await;
            self.report(now);
            return None;
        }
        self.screen.feed(bytes.clone()).await;
        let flush = self.pending.push_bytes(bytes, now, self.last_input);
        self.report(now);
        Some(flush)
    }

    async fn on_reader_end(&mut self) {
        self.reader_done = true;
        self.drain_at = None;
        if let Some(status) = self.exit {
            self.queue_exit(status).await;
        }
    }

    async fn on_exit(&mut self, status: ExitStatus) {
        self.exit = Some(status);
        // A host with nothing else running may now be idle.
        self.publish();
        self.host.touched();
        if self.reader_done {
            self.queue_exit(status).await;
        } else {
            // The terminal reports its end only when every program has let go of it.
            // Wait for that, but not for ever.
            self.drain_at = Some(Instant::now() + EXIT_DRAIN_TIMEOUT);
        }
    }

    async fn on_drain_timeout(&mut self) {
        self.drain_at = None;
        self.stopper.stop();
        self.reader_done = true;
        if let Some(status) = self.exit {
            self.queue_exit(status).await;
        }
    }

    /// Puts the exit marker behind whatever output is still waiting.
    async fn queue_exit(&mut self, status: ExitStatus) {
        if self.exit_queued {
            return;
        }
        self.exit_queued = true;
        if let Some(tail) = &self.tail
            && self.ending.is_none()
        {
            // Said whatever was said before: how a session ended is not left out.
            let ended = SessionEvent::Disconnected(ssh_exit::classify(status, tail));
            self.seen.apply(&ended);
            self.pending.push_event(ended);
        }
        self.pending.push_exit(status);
        self.owner
            .notify(Notification::SessionExited {
                session: self.id,
                status,
            })
            .await;
        self.flush().await;
    }

    /// Begins ending the session early. Output is of no further interest; the exit is.
    fn end(&mut self, ending: Ending) {
        if self.ending.is_none() {
            self.ending = Some(ending);
            self.pending.discard_bytes();
            self.flush_at = None;
            if self.exit.is_none()
                && let Err(error) = self.killer.kill()
            {
                tracing::warn!(session = %self.id, %error, "ending the program failed");
            }
        }
        if ending == Ending::Abandoned {
            self.attachment = None;
        }
    }

    fn current(&mut self, id: AttachmentId) -> Option<&mut Attachment> {
        self.attachment
            .as_mut()
            .filter(|attachment| attachment.id == id)
    }

    async fn on_command(&mut self, command: Command) {
        match command {
            Command::Attach {
                request,
                stream,
                facts,
                hydrate,
                outbox,
                reply,
            } => {
                let attached = self.attach(request, stream, facts, hydrate, outbox).await;
                let _ = reply.send(attached);
                if attached.is_some() {
                    // Markers need no credit, so anything already waiting that is not
                    // output can go now that the response is on its way.
                    self.flush().await;
                }
            }
            Command::Input { attachment, bytes } => {
                if self.current(attachment).is_some() {
                    self.last_input = Some(Instant::now());
                    self.write_input(bytes);
                }
            }
            Command::Resize {
                attachment,
                size,
                cell_px,
            } => {
                if self.current(attachment).is_some() {
                    self.resize(size, cell_px).await;
                }
            }
            Command::Facts { attachment, facts } => {
                let Some(attachment) = self.current(attachment) else {
                    return;
                };
                let was = std::mem::replace(&mut attachment.facts, facts.clone());
                let (focused, visible) = (attachment.facts.focused, attachment.facts.visible);
                self.screen.facts(facts).await;
                tracing::trace!(focused, visible, "client facts updated");
                if was.focused != focused {
                    // The next output while nobody types here is said again at once.
                    self.last_activity = None;
                }
                if visible && !was.visible {
                    // A session that comes on show is brought up to date.
                    self.poll_at = sooner(self.poll_at, Some(Instant::now()));
                } else if !visible && was.visible {
                    // Looked at once more as it goes out of sight, since it is not
                    // looked at again while nobody sees it: what was started a moment
                    // ago is known.
                    self.poll_soon(Instant::now());
                }
            }
            Command::Credit { attachment, bytes } => {
                let Some(attachment) = self.current(attachment) else {
                    return;
                };
                if attachment.window.grant(bytes).is_err() {
                    attachment
                        .outbox
                        .fatal(
                            ErrorCode::ProtocolError,
                            "more credit was granted than a stream may hold",
                        )
                        .await;
                    self.attachment = None;
                    return;
                }
                self.flush().await;
            }
            Command::Detach { attachment } => {
                if self.current(attachment).is_some() {
                    self.attachment = None;
                    self.screen.unfocused().await;
                }
            }
            Command::Rehydrate { attachment } => {
                let Some(with) = self.current(attachment).map(|open| open.hydrate) else {
                    return;
                };
                // Everything read so far is queued for the client and on the screen, so
                // the copy goes behind it: the client starts again exactly where the
                // output it has not yet taken ends.
                if let Some(payload) = self.screen.hydrate(with).await {
                    self.pending.push_hydrate(payload);
                    self.flush().await;
                }
            }
            Command::Screen { reply } => {
                let _ = reply.send(self.screen.text().await);
            }
            Command::WaitText {
                text,
                deadline,
                reply,
            } => {
                // The wait runs in the screen; the session goes on meanwhile.
                let screen = self.screen.waiter();
                tokio::spawn(async move {
                    let _ = reply.send(screen.wait_text(text, deadline).await);
                });
            }
            Command::Kill => self.end(Ending::Killed),
            Command::Abandon => self.end(Ending::Abandoned),
        }
    }

    async fn attach(
        &mut self,
        request: u64,
        stream: u32,
        facts: ClientFacts,
        hydrate: HydrateWith,
        outbox: Outbox,
    ) -> Option<AttachmentId> {
        let refusal = if self.ending.is_some() {
            Some(ProtoError::new(
                ErrorCode::SessionExited,
                "the session is ending",
            ))
        } else if self.attachment.is_some() {
            Some(ProtoError::new(
                ErrorCode::AlreadyAttached,
                "the session already has a client",
            ))
        } else {
            None
        };
        if let Some(refusal) = refusal {
            outbox.respond(request, Err(refusal)).await;
            return None;
        }

        // A copy of the screen as of every byte read so far. Nothing is read while this
        // runs, so the output that follows starts with the next byte.
        let payload = self.screen.hydrate(hydrate).await;
        let mode = match &payload {
            Some(HydratePayload::Snapshot { .. }) => HydrateMode::Snapshot,
            Some(HydratePayload::VtReplay(_)) => HydrateMode::VtReplay,
            None => HydrateMode::None,
        };
        let attached = ResponseBody::Attached {
            size: self.size,
            hydrate: mode,
            resize_pulls_scrollback: self.resize_pulls_scrollback,
        };
        if !outbox.respond(request, Ok(attached)).await {
            // The connection went away while this was being handled.
            return None;
        }
        // The screen goes first, then what the session has reported so far.
        let mut start = Vec::new();
        if let Some(payload) = payload {
            append(&mut start, &Message::new(stream, Body::Hydrate(payload)));
        }
        if !self.seen.is_empty() {
            append(
                &mut start,
                &Message::new(stream, Body::Event(whole(&self.seen))),
            );
        }
        if !start.is_empty() && !outbox.frames(start).await {
            return None;
        }
        // What was waiting was waiting for nobody: the copy and the state cover it.
        // Only the end of the program, if it has ended, is still to be told.
        self.pending.clear();
        if self.exit_queued
            && let Some(status) = self.exit
        {
            self.pending.push_exit(status);
        }
        self.screen.facts(facts.clone()).await;
        let id = self.next_attachment;
        self.next_attachment += 1;
        if facts.visible {
            self.poll_at = sooner(self.poll_at, Some(Instant::now()));
        }
        self.last_activity = None;
        self.attachment = Some(Attachment {
            id,
            stream,
            outbox,
            window: SendWindow::new(),
            facts,
            hydrate,
        });
        self.ever_attached = true;
        self.linger_at = None;
        Some(id)
    }

    fn write_input(&mut self, bytes: Vec<u8>) {
        let queued = self.input_queued.load(Ordering::Relaxed);
        if queued.saturating_add(bytes.len()) > INPUT_BACKLOG_MAX {
            if !self.input_dropped {
                self.input_dropped = true;
                tracing::warn!(session = %self.id, "dropping input: the program is not reading it");
            }
            return;
        }
        self.input_queued.fetch_add(bytes.len(), Ordering::Relaxed);
        if self.input.send(bytes).is_err() {
            // The writing thread has stopped: the terminal no longer takes input.
            self.input_queued.store(0, Ordering::Relaxed);
        }
    }

    /// Resizes the terminal and marks the place in the output where it happened.
    async fn resize(&mut self, size: TermSize, cell_px: CellPx) {
        // What was read before the resize belongs before the marker.
        let now = Instant::now();
        while let Ok(bytes) = self.chunks.try_recv() {
            self.observe(&bytes);
            if self.ending.is_none() {
                let _ = self.take_output(bytes, now).await;
            }
        }

        let pixels = PtySize {
            cols: size.cols(),
            rows: size.rows(),
            px_width: size.cols().saturating_mul(cell_px.w),
            px_height: size.rows().saturating_mul(cell_px.h),
        };
        if let Err(error) = self.pty.resize(pixels) {
            // The terminal did not change size, so there is nothing to mark.
            tracing::debug!(session = %self.id, %error, "resizing the terminal failed");
            return;
        }
        self.size = size;
        self.screen.resize(size, cell_px).await;
        if self.attachment.is_some() {
            self.pending.push_resized(size);
        }
        if let Some(attachment) = &mut self.attachment {
            // The size requested is the size the client has; its facts follow it.
            attachment.facts.size = size;
            attachment.facts.cell_px = cell_px;
        }
        self.flush().await;
    }

    /// Sends what is waiting, as far as the client's credit allows.
    async fn flush(&mut self) {
        self.flush_at = None;
        let Some(attachment) = &mut self.attachment else {
            return;
        };
        let stream = attachment.stream;
        let mut batch = Vec::new();
        let mut connected = true;

        while connected {
            let Some(segment) = self.pending.pop(attachment.window.available() as usize) else {
                break;
            };
            match segment {
                Segment::Bytes(bytes) => {
                    attachment.window.take(bytes.len());
                    self.sent_bytes += bytes.len() as u64;
                    encode_bytes(stream, BytesKind::Data, &bytes, &mut batch);
                }
                Segment::Resized(size) => {
                    append(&mut batch, &Message::new(stream, Body::Resized(size)));
                }
                Segment::Event(event) => {
                    append(&mut batch, &Message::new(stream, Body::Event(event)));
                }
                Segment::Hydrate(payload) => {
                    append(&mut batch, &Message::new(stream, Body::Hydrate(payload)));
                }
                Segment::Exit(status) => {
                    append(&mut batch, &Message::new(stream, Body::Exit(status)));
                    self.exit_delivered = true;
                }
            }
            if batch.len() >= SEND_BATCH_BYTES {
                connected = attachment.outbox.frames(std::mem::take(&mut batch)).await;
            }
        }
        if connected && !batch.is_empty() {
            connected = attachment.outbox.frames(batch).await;
        }
        if !connected {
            // The connection has gone; the session carries on without a client.
            self.attachment = None;
        }
    }
}

/// Appends a marker's frame. Markers are small, and reports are kept within what one
/// frame carries, so this does not fail; if it did, the marker would be left out.
fn append(batch: &mut Vec<u8>, message: &Message) {
    if let Err(error) = encode(message, batch) {
        tracing::error!(%error, "a marker frame could not be encoded");
    }
}
