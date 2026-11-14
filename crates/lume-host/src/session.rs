//! One session: a terminal, the program in it, and the client attached to it.
//!
//! Terminal I/O blocks, so each session has three threads: one reads the terminal, one
//! writes to it, one waits for the program to end. Everything else happens in one async
//! task, which owns all of the session's state and is the only thing that touches it.
//!
//! Backpressure is the absence of work. The reading thread hands each read to the task
//! through a queue two reads long. When the task has nowhere to put more output (no
//! client, or a client that has granted no credit) it stops taking from that queue, the
//! thread blocks on it, the terminal's own buffer fills, and the program blocks on its
//! next write. Nothing is dropped and nothing grows.

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use lume_proto::{
    BytesKind, CellPx, ClientFacts, ErrorCode, ExitStatus, HydrateMode, Message, Notification,
    ProtoError, ResponseBody, SendWindow, SessionId, SessionInfo, SessionStats, TermSize, encode,
    encode_bytes,
};
use lume_pty::{Child, ChildKiller, Pty, PtySize, ReadStop};
use tokio::sync::{mpsc, oneshot};

use crate::Host;
use crate::conn::Outbox;
use crate::pending::{Flush, Pending, Segment};
use crate::tuning::{
    COALESCE_MAX_BYTES, COMMAND_QUEUE, EXIT_DRAIN_TIMEOUT, INPUT_BACKLOG_MAX, READ_CHUNK,
    READ_QUEUE_CHUNKS, SEND_BATCH_BYTES, UNATTACHED_EXIT_LINGER,
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
    pub(crate) pre_attach_bytes: usize,
    pub(crate) resize_pulls_scrollback: bool,
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
        pre_attach_bytes,
        ..
    } = start;
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
    }));

    let (command_tx, commands) = mpsc::channel(COMMAND_QUEUE);
    let session = Session {
        host,
        id,
        pty,
        killer,
        stopper,
        size,
        resize_pulls_scrollback: start.resize_pulls_scrollback,
        pre_attach_bytes,
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
    pre_attach_bytes: usize,
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
            if self.finished() {
                break;
            }
            let ingest = self.can_ingest();
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
            None => self.ever_attached || self.linger_over,
        }
    }

    /// How much output may wait in the session. Before the first client attaches it
    /// holds the program's first output; after that, only what one send can carry.
    fn capacity(&self) -> usize {
        if self.ever_attached {
            COALESCE_MAX_BYTES
        } else {
            self.pre_attach_bytes
        }
    }

    /// Whether to take more output from the reading thread. Not taking it is what stops
    /// the program.
    fn can_ingest(&self) -> bool {
        !self.reader_done && (self.ending.is_some() || self.pending.bytes() < self.capacity())
    }

    /// Makes the session's state visible to listings.
    fn publish(&self) {
        let mut shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        shared.size = self.size;
        shared.attached = self.attachment.is_some();
        shared.exit = self.exit;
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
        match self.pending.push_bytes(bytes, now, self.last_input) {
            Flush::Now => self.flush().await,
            Flush::At(at) => self.flush_at = Some(self.flush_at.map_or(at, |set| set.min(at))),
        }
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
        self.pending.push_exit(status);
        self.owner
            .notify(Notification::SessionExited {
                session: self.id,
                status,
            })
            .await;
        if self.attachment.is_none() && !self.ever_attached && self.ending.is_none() {
            self.linger_at = Some(Instant::now() + UNATTACHED_EXIT_LINGER);
        }
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
                outbox,
                reply,
            } => {
                let attached = self.attach(request, stream, facts, outbox).await;
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
                if let Some(attachment) = self.current(attachment) {
                    tracing::trace!(
                        focused = facts.focused,
                        visible = facts.visible,
                        "client facts updated"
                    );
                    attachment.facts = facts;
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
                }
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

        let attached = ResponseBody::Attached {
            size: self.size,
            hydrate: HydrateMode::None,
            resize_pulls_scrollback: self.resize_pulls_scrollback,
        };
        if !outbox.respond(request, Ok(attached)).await {
            // The connection went away while this was being handled.
            return None;
        }
        let id = self.next_attachment;
        self.next_attachment += 1;
        self.attachment = Some(Attachment {
            id,
            stream,
            outbox,
            window: SendWindow::new(),
            facts,
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
                let _ = self.pending.push_bytes(bytes, now, self.last_input);
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
        self.pending.push_resized(size);
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
                    append(
                        &mut batch,
                        &Message::new(stream, lume_proto::Body::Resized(size)),
                    );
                }
                Segment::Exit(status) => {
                    append(
                        &mut batch,
                        &Message::new(stream, lume_proto::Body::Exit(status)),
                    );
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

/// Appends a fixed-size frame, which cannot fail to encode.
fn append(batch: &mut Vec<u8>, message: &Message) {
    if let Err(error) = encode(message, batch) {
        tracing::error!(%error, "a marker frame could not be encoded");
    }
}
