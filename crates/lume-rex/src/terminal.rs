//! One terminal block as a view is attached to it.
//!
//! An attachment keeps a data connection open for the block's stream and passes on
//! what it holds: first a copy of the terminal to start from, then the output, with
//! sizes and the program's end in their places.
//!
//! A server does not wait for a client that is slow: it sends what the terminal's
//! program writes as it is written. A view takes output only as fast as it grants
//! credit for it. So the connection is always read, and what the view has not taken
//! waits here. When too much waits, the stream is let go altogether, and begun again
//! from a new copy of the terminal once the view takes output again. A view that lets
//! go of its screen, or asks for a new copy of it, is served the same way.
//!
//! A connection that is lost is continued where it stopped. A stream the server has
//! let go of is begun again.

use std::collections::VecDeque;
use std::future::pending;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use lume_proto::tuning::{MAX_PAYLOAD, MAX_WINDOW};
use lume_proto::{CellPx, ClientFacts, ExitStatus, SessionId, TermSize};
use lume_session::{Attachment, FrameStream, Hydrate, InputSink, OutputFrame, SessionError, Sink};
use lume_vt::Terminal;
use serde_json::json;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::api::{BlobPart, Formatted, Ticketed};
use crate::backend::{Link, Shared, backoff, unsupported};
use crate::control::Control;
use crate::data::{DataConn, DataEnd, DataOpen, Reject, ResumeToken, Ticket};
use crate::error::RexError;
use crate::ids::BlockRef;
use crate::stream::{Note, Opening, Piece, Splitter};
use crate::tuning::{
    DATA_CLOSE_TIMEOUT, PENDING_MAX_BYTES, REPLAY_PART_BYTES, RESTART_ATTEMPTS_MAX,
    SNAPSHOT_MAX_BYTES, STREAM_NOTE_MAX_BYTES, STREAM_OPEN_TIMEOUT, TERMINAL_DATA_REF,
    TERMINAL_SUBPROTOCOL,
};

/// A stream that is open: its connection, and where the stream stands.
struct Live {
    conn: DataConn,
    splitter: Splitter,
    /// What continues the stream if the connection is lost.
    token: ResumeToken,
    /// How many bytes of the stream were taken, over all its connections.
    received: u64,
}

/// What beginning a stream gave.
struct Started {
    live: Live,
    /// What a view starts from.
    hydrate: Hydrate,
    /// The terminal's size.
    size: TermSize,
    /// What the stream held after that, so far.
    after: Vec<Piece>,
}

/// What the end of a connection means to someone waiting for its stream to begin.
fn not_begun(end: DataEnd) -> RexError {
    match end {
        DataEnd::Failed(error) => error,
        DataEnd::BlockClosed => RexError::Rejected(Reject::Unavailable),
        DataEnd::Ended | DataEnd::Dropped | DataEnd::Restart => RexError::Disconnected,
    }
}

/// Whether a failure says the block is no longer there.
fn gone(error: &RexError) -> bool {
    matches!(
        error,
        RexError::Rejected(Reject::Unavailable) | RexError::Call { status: 404, .. }
    )
}

fn exit_of(exit_code: i64) -> ExitStatus {
    u32::try_from(exit_code)
        .map(ExitStatus::Code)
        .unwrap_or(ExitStatus::Unknown)
}

/// Loads a copy of a terminal in this application's engine, off the threads that must
/// not wait. Returns the copy, and what followed it in the same bytes: the engine knows
/// where a copy ends. Fails if the engine cannot load it, as when a server's engine is
/// of another version.
async fn load(mut copy: Vec<u8>) -> Result<(Vec<u8>, Vec<u8>), String> {
    let loading = tokio::task::spawn_blocking(move || {
        let (_, length) =
            Terminal::from_snapshot_prefix(&copy).map_err(|error| error.to_string())?;
        let after = copy.split_off(length.min(copy.len()));
        Ok((copy, after))
    });
    loading.await.map_err(|error| error.to_string())?
}

/// Reads a terminal's screen and scrollback as output that rebuilds them in any engine.
async fn read_replay(control: &Control, block: &BlockRef) -> Result<Vec<u8>, RexError> {
    let of_block = |args: serde_json::Value| json!({ "session_id": block.session_id, "block_id": block.block_id, "args": args });
    let (first, _) = control
        .call_as::<Formatted>(
            "com.superlogical.terminal.format",
            of_block(json!({
                "format": "vt",
                "max_bytes": REPLAY_PART_BYTES,
                "extra_charsets": true,
                "extra_cursor": true,
                "extra_hyperlink": true,
                "extra_keyboard": true,
                "extra_kitty_keyboard": true,
                "extra_modes": true,
                "extra_palette": true,
                "extra_protection": true,
                "extra_pwd": true,
                "extra_scrolling_region": true,
                "extra_style": true,
                "extra_tabstops": true,
            })),
        )
        .await?;
    let mut replay = first.content.into_bytes();
    let Some(id) = first.id else {
        return Ok(replay);
    };
    loop {
        let (part, _) = control
            .call_as::<BlobPart>(
                "com.superlogical.terminal.blob_read",
                of_block(
                    json!({ "id": id, "offset": replay.len(), "max_bytes": REPLAY_PART_BYTES }),
                ),
            )
            .await?;
        if part.data.is_empty() && !part.eof {
            return Err(RexError::Protocol(
                "a terminal's screen was read in parts that never end".to_owned(),
            ));
        }
        replay.extend_from_slice(part.data.as_bytes());
        if replay.len() > SNAPSHOT_MAX_BYTES {
            return Err(RexError::Protocol(
                "a terminal's screen is larger than is taken".to_owned(),
            ));
        }
        if part.eof {
            return Ok(replay);
        }
    }
}

/// What a view starts from when the copy at the head of a stream cannot be used: the
/// screen as the server writes it out, or nothing if that cannot be had either. The
/// screen is read a moment after the stream began, so a little output around that
/// moment can show twice or not at all until the program draws again.
async fn replay(shared: &Shared, control: &Control, block: &BlockRef, why: &str) -> Hydrate {
    match read_replay(control, block).await {
        Ok(bytes) => {
            tracing::debug!(server = %shared.name, why, "a terminal starts from its screen written out");
            Hydrate::VtReplay(bytes)
        }
        Err(error) => {
            tracing::warn!(server = %shared.name, why, %error, "a terminal starts blank: its screen could not be read");
            Hydrate::None
        }
    }
}

/// Begins a block's stream: a ticket, a data connection, the copy of the terminal and
/// the terminal's size.
async fn start(shared: &Shared, block: &BlockRef) -> Result<Started, RexError> {
    let control = shared.control()?;
    let (ticketed, _) = control
        .call_as::<Ticketed>(
            "data.connect",
            json!({
                "session_id": block.session_id,
                "block_id": block.block_id,
                "data_ref_id": TERMINAL_DATA_REF,
                "subprotocol": TERMINAL_SUBPROTOCOL,
            }),
        )
        .await?;
    let ticket = Ticket::read(&ticketed.ticket)?;
    let io = shared.dial().await?;
    let mut conn = DataConn::open(io, &shared.name, DataOpen::Start(&ticket)).await?;
    let nonce = conn.welcome().nonce;
    let mut splitter = Splitter::new(nonce);
    let mut received = 0u64;

    let is_size = |piece: &Piece| matches!(piece, Piece::Note(Note::Resize { .. }));
    let opening = async {
        let mut opening = Opening::new(nonce);
        let opened = loop {
            let bytes = conn.next().await.map_err(not_begun)?;
            received += bytes.len() as u64;
            if let Some(opened) = opening.feed(&bytes) {
                break opened;
            }
        };
        // The terminal's size is the first thing a server says after the copy.
        let mut pieces = splitter.feed(&opened.rest);
        let mut taken = opened.rest.len();
        while !pieces.iter().any(is_size) {
            if taken > 2 * STREAM_NOTE_MAX_BYTES {
                return Err(RexError::Protocol(
                    "a terminal's stream does not say the terminal's size".to_owned(),
                ));
            }
            let bytes = conn.next().await.map_err(not_begun)?;
            received += bytes.len() as u64;
            taken += bytes.len();
            pieces.extend(splitter.feed(&bytes));
        }
        Ok((opened.snapshot, pieces))
    };
    let (snapshot, pieces) = tokio::time::timeout(STREAM_OPEN_TIMEOUT, opening)
        .await
        .map_err(|_| RexError::Timeout("a terminal's stream".to_owned()))??;

    let (hydrate, before) = match snapshot {
        Some(copy) => match load(copy).await {
            Ok((copy, after)) => (
                Hydrate::Snapshot {
                    vt_version: lume_vt::library_version().to_owned(),
                    bytes: copy,
                },
                after,
            ),
            Err(error) => (replay(shared, &control, block, &error).await, Vec::new()),
        },
        None => (
            replay(shared, &control, block, "the copy is too large").await,
            Vec::new(),
        ),
    };

    let mut size = None;
    let mut after = Vec::new();
    if !before.is_empty() {
        after.push(Piece::Output(before));
    }
    for piece in pieces {
        match piece {
            Piece::Note(Note::Resize { size: first, .. }) if size.is_none() => size = Some(first),
            piece => after.push(piece),
        }
    }
    let size = size.ok_or_else(|| {
        RexError::Protocol("a terminal's stream does not say the terminal's size".to_owned())
    })?;
    let token = conn.welcome().token.clone();
    Ok(Started {
        live: Live {
            conn,
            splitter,
            token,
            received,
        },
        hydrate,
        size,
        after,
    })
}

/// Continues a stream whose connection was lost, on a new connection.
async fn resume(shared: &Shared, lost: Live) -> Result<Live, RexError> {
    let io = shared.dial().await?;
    let how = DataOpen::Resume {
        token: &lost.token,
        received: lost.received,
    };
    let conn = DataConn::open(io, &shared.name, how).await?;
    let token = conn.welcome().token.clone();
    Ok(Live {
        conn,
        splitter: lost.splitter,
        token,
        received: lost.received,
    })
}

/// The next bytes of a stream that is open. Waits without end for one that is not.
async fn next_of(live: &mut Option<Live>) -> Result<Vec<u8>, DataEnd> {
    match live {
        Some(live) => {
            let bytes = live.conn.next().await?;
            live.received += bytes.len() as u64;
            Ok(bytes)
        }
        None => pending().await,
    }
}

/// What a view asks of its attachment that the attachment's task acts on.
enum Command {
    /// Credit was granted.
    Credit,
    Rehydrate,
    Release,
}

/// Something that waits for the view to take it.
enum Waiting {
    /// Output, which goes only against credit.
    Data(Vec<u8>),
    Frame(OutputFrame),
}

/// How an attachment stands with its stream.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Standing {
    /// The stream is followed.
    Following,
    /// The view took too little for too long, and the stream was let go. It begins
    /// again when the view grants credit.
    Behind,
    /// The view let go of its screen. The stream begins again when it asks.
    Released,
}

struct Task {
    shared: Arc<Shared>,
    block: BlockRef,
    frames: mpsc::UnboundedSender<OutputFrame>,
    commands: mpsc::UnboundedReceiver<Command>,
    /// How much output the view has granted and not been sent.
    credit: Arc<AtomicU64>,
    live: Option<Live>,
    waiting: VecDeque<Waiting>,
    /// How many bytes of output wait.
    waiting_bytes: usize,
    /// The size the view was last told.
    size: TermSize,
    standing: Standing,
}

impl Task {
    /// Puts what a stream held in line for the view.
    fn take(&mut self, pieces: Vec<Piece>) {
        for piece in pieces {
            match piece {
                Piece::Output(bytes) => {
                    self.waiting_bytes += bytes.len();
                    self.waiting.push_back(Waiting::Data(bytes));
                }
                Piece::Note(Note::Resize { size, .. }) => {
                    if size != self.size {
                        self.size = size;
                        self.waiting
                            .push_back(Waiting::Frame(OutputFrame::Resized(size)));
                    }
                }
                Piece::Note(Note::ChildExited { exit_code }) => {
                    self.waiting
                        .push_back(Waiting::Frame(OutputFrame::Exit(exit_of(exit_code))));
                }
                // Nothing shows a server's colours yet.
                Piece::Note(Note::Theme(_)) => {}
            }
        }
    }

    /// Sends the view what waits, as far as its credit goes. Returns whether the
    /// attachment goes on: it is over once the program's end was sent, or nobody takes
    /// the frames any more.
    fn pump(&mut self) -> bool {
        while let Some(front) = self.waiting.front_mut() {
            let frame = match front {
                Waiting::Data(bytes) => {
                    let credit =
                        usize::try_from(self.credit.load(Ordering::Acquire)).unwrap_or(usize::MAX);
                    if credit == 0 {
                        return true;
                    }
                    let count = bytes.len().min(credit).min(MAX_PAYLOAD);
                    let chunk: Vec<u8> = bytes.drain(..count).collect();
                    if bytes.is_empty() {
                        self.waiting.pop_front();
                    }
                    self.waiting_bytes -= count;
                    self.credit.fetch_sub(count as u64, Ordering::AcqRel);
                    OutputFrame::Data(chunk)
                }
                Waiting::Frame(_) => match self.waiting.pop_front() {
                    Some(Waiting::Frame(frame)) => frame,
                    _ => return true,
                },
            };
            let last = matches!(frame, OutputFrame::Exit(_));
            if self.frames.send(frame).is_err() || last {
                return false;
            }
        }
        true
    }

    /// Lets go of the stream and of the output that waits. Sizes and the program's end
    /// stay in line.
    async fn stop(&mut self) {
        if let Some(live) = self.live.take() {
            live.conn.close().await;
        }
        self.waiting
            .retain(|waiting| matches!(waiting, Waiting::Frame(_)));
        self.waiting_bytes = 0;
    }

    /// The block is no longer there: the view is told its program ended, unless it was
    /// told already.
    fn block_gone(&mut self) {
        self.live = None;
        let told = self
            .waiting
            .iter()
            .any(|waiting| matches!(waiting, Waiting::Frame(OutputFrame::Exit(_))));
        if !told {
            self.waiting
                .push_back(Waiting::Frame(OutputFrame::Exit(ExitStatus::Unknown)));
        }
    }

    /// Begins the stream again, from a new copy of the terminal. Returns whether the
    /// attachment goes on.
    async fn restart(&mut self) -> bool {
        self.stop().await;
        let mut link = self.shared.link();
        let mut attempts = 0;
        loop {
            match start(&self.shared, &self.block).await {
                Ok(started) => {
                    // The size goes first, so that the copy is shown at its own size.
                    if started.size != self.size {
                        self.size = started.size;
                        self.waiting
                            .push_back(Waiting::Frame(OutputFrame::Resized(started.size)));
                    }
                    self.waiting
                        .push_back(Waiting::Frame(OutputFrame::Hydrate(started.hydrate)));
                    self.live = Some(started.live);
                    self.take(started.after);
                    self.standing = Standing::Following;
                    return true;
                }
                Err(error) if gone(&error) => {
                    self.block_gone();
                    self.standing = Standing::Following;
                    return true;
                }
                Err(error) => {
                    attempts += 1;
                    if attempts >= RESTART_ATTEMPTS_MAX {
                        tracing::warn!(server = %self.shared.name, %error, "a terminal's stream could not be begun again");
                        return false;
                    }
                    tracing::debug!(server = %self.shared.name, %error, attempts, "a terminal's stream will be begun again");
                    tokio::time::sleep(backoff(attempts)).await;
                    // With no connection to the server there is nothing to try.
                    let connected = link
                        .wait_for(|link| !matches!(link, Link::Connecting | Link::Waiting { .. }))
                        .await;
                    if !matches!(connected.as_deref(), Ok(Link::Connected(_))) {
                        return false;
                    }
                }
            }
        }
    }

    /// Acts on the end of a connection. Returns whether the attachment goes on.
    async fn ended(&mut self, end: DataEnd) -> bool {
        match end {
            DataEnd::Dropped => {
                let Some(lost) = self.live.take() else {
                    return true;
                };
                match resume(&self.shared, lost).await {
                    Ok(live) => {
                        self.live = Some(live);
                        true
                    }
                    Err(error) => {
                        tracing::debug!(server = %self.shared.name, %error, "a terminal's stream could not be continued");
                        self.restart().await
                    }
                }
            }
            DataEnd::Ended | DataEnd::Restart => {
                self.live = None;
                self.restart().await
            }
            DataEnd::BlockClosed => {
                self.block_gone();
                true
            }
            DataEnd::Failed(error) => {
                tracing::debug!(server = %self.shared.name, %error, "a terminal's stream failed");
                self.live = None;
                self.restart().await
            }
        }
    }

    async fn run(&mut self) {
        loop {
            if !self.pump() {
                return;
            }
            tokio::select! {
                command = self.commands.recv() => match command {
                    None => return,
                    Some(Command::Credit) => {
                        if self.standing == Standing::Behind && !self.restart().await {
                            return;
                        }
                    }
                    Some(Command::Rehydrate) => {
                        if !self.restart().await {
                            return;
                        }
                    }
                    Some(Command::Release) => {
                        self.stop().await;
                        self.standing = Standing::Released;
                    }
                },
                bytes = next_of(&mut self.live) => match bytes {
                    Ok(bytes) => {
                        let pieces = match &mut self.live {
                            Some(live) => live.splitter.feed(&bytes),
                            None => Vec::new(),
                        };
                        self.take(pieces);
                        if self.waiting_bytes > PENDING_MAX_BYTES {
                            self.stop().await;
                            self.standing = Standing::Behind;
                        }
                    }
                    Err(end) => {
                        if !self.ended(end).await {
                            return;
                        }
                    }
                },
                () = self.frames.closed() => return,
            }
        }
    }
}

/// An attachment as its backend holds it: the way to end it.
pub(crate) struct Handle {
    detached: watch::Sender<bool>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Handle {
    /// Ends the attachment and waits until its stream is closed, so that a new one to
    /// the same block finds the way free. Returns whether this is what ended it.
    pub(crate) async fn end(&self) -> bool {
        let first = !self.detached.send_replace(true);
        let task = self
            .task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(task) = task {
            let _ = tokio::time::timeout(2 * DATA_CLOSE_TIMEOUT, task).await;
        }
        first
    }

    /// Ends the attachment without waiting for it.
    pub(crate) fn end_soon(&self) {
        self.detached.send_replace(true);
    }
}

struct RexSink {
    handle: Arc<Handle>,
    commands: mpsc::UnboundedSender<Command>,
    credit: Arc<AtomicU64>,
}

impl RexSink {
    fn check_open(&self) -> Result<(), SessionError> {
        if *self.handle.detached.borrow() || self.commands.is_closed() {
            return Err(SessionError::Detached);
        }
        Ok(())
    }

    fn tell(&self, command: Command) -> Result<(), SessionError> {
        self.check_open()?;
        self.commands
            .send(command)
            .map_err(|_| SessionError::Detached)
    }
}

#[async_trait]
impl Sink for RexSink {
    async fn write(&self, _bytes: &[u8]) -> Result<(), SessionError> {
        self.check_open()?;
        Err(unsupported("be typed into"))
    }

    async fn resize(&self, _size: TermSize, _cell_px: CellPx) -> Result<(), SessionError> {
        self.check_open()
    }

    async fn facts(&self, _facts: &ClientFacts) -> Result<(), SessionError> {
        self.check_open()
    }

    async fn credit(&self, bytes: u32) -> Result<(), SessionError> {
        self.check_open()?;
        // Recorded before the task hears of it: it may use it the moment it looks.
        self.credit
            .try_update(Ordering::AcqRel, Ordering::Acquire, |granted| {
                granted
                    .checked_add(u64::from(bytes))
                    .filter(|granted| *granted <= u64::from(MAX_WINDOW))
            })
            .map_err(|_| SessionError::CreditOverflow)?;
        self.tell(Command::Credit)
    }

    async fn rehydrate(&self) -> Result<(), SessionError> {
        self.tell(Command::Rehydrate)
    }

    async fn release(&self) -> Result<(), SessionError> {
        self.tell(Command::Release)
    }

    async fn detach(&self) -> Result<(), SessionError> {
        if self.handle.end().await {
            Ok(())
        } else {
            Err(SessionError::Detached)
        }
    }
}

/// Attaches to a block: begins its stream and starts the task that follows it. `id` is
/// the block as the application names it, and `serial` tells this attachment from
/// others to the same block.
pub(crate) async fn attach(
    shared: Arc<Shared>,
    id: SessionId,
    serial: u64,
    block: BlockRef,
) -> Result<(Attachment, Arc<Handle>), RexError> {
    let started = start(&shared, &block).await?;
    let (frames, taken) = mpsc::unbounded_channel();
    let (commands, told) = mpsc::unbounded_channel();
    let (detached, mut is_detached) = watch::channel(false);
    let credit = Arc::new(AtomicU64::new(0));
    let size = started.size;
    let mut task = Task {
        shared: Arc::clone(&shared),
        block,
        frames,
        commands: told,
        credit: Arc::clone(&credit),
        live: Some(started.live),
        waiting: VecDeque::new(),
        waiting_bytes: 0,
        size,
        standing: Standing::Following,
    };
    task.take(started.after);

    let running = tokio::spawn(async move {
        tokio::select! {
            () = task.run() => {}
            _ = is_detached.wait_for(|detached| *detached) => {}
        }
        // Closed properly, the stream ends on the server at once.
        if let Some(live) = task.live.take() {
            live.conn.close().await;
        }
        task.shared.forget(id, serial);
    });
    let handle = Arc::new(Handle {
        detached,
        task: Mutex::new(Some(running)),
    });
    let sink = RexSink {
        handle: Arc::clone(&handle),
        commands,
        credit,
    };
    let attachment = Attachment {
        size,
        // The view's terminal is to change size exactly as the server's own does, which
        // is as their common engine does it.
        resize_pulls_scrollback: true,
        hydrate: started.hydrate,
        output: FrameStream::new(taken),
        input: InputSink::new(Arc::new(sink)),
    };
    Ok((attachment, handle))
}
