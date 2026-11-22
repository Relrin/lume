//! One terminal block as views are attached to it.
//!
//! A feed keeps a data connection open for the block's stream and passes on what it
//! holds: first a copy of the terminal to start from, then the output, with sizes and
//! the program's end in their places.
//!
//! A server gives a client one stream of a block, however many views of the terminal
//! the application shows. So a terminal has one feed, and each view is served from
//! it, with its own credit and its own line of what waits for it. A copy of the
//! terminal comes only at the head of a stream: when a view needs one, because it is
//! new or lost its place, the stream is begun again, and every view that follows it
//! starts again from the new copy.
//!
//! A server does not wait for a client that is slow: it sends what the terminal's
//! program writes as it is written. A view takes output only as fast as it grants
//! credit for it. So the connection is always read, and what a view has not taken
//! waits here. A view for which too much waits gets no more, and starts again from a
//! new copy once it takes output again. A view that lets go of its screen, or asks for
//! a new copy of it, is served the same way. A stream no view follows is closed.
//!
//! A connection that is lost is continued where it stopped. A stream the server has
//! let go of is begun again.

use std::collections::{HashMap, VecDeque};
use std::future::pending;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use lume_proto::tuning::{MAX_PAYLOAD, MAX_WINDOW};
use lume_proto::{CellPx, ClientFacts, ExitStatus, SessionId, TermSize};
use lume_session::{Attachment, FrameStream, Hydrate, InputSink, OutputFrame, SessionError, Sink};
use lume_vt::Terminal;
use serde_json::json;
use tokio::sync::{mpsc, oneshot};

use crate::api::{BlobPart, Formatted, Ticketed};
use crate::backend::{Link, Shared, backoff};
use crate::control::Control;
use crate::data::{DataConn, DataEnd, DataOpen, Reject, ResumeToken, Ticket};
use crate::error::RexError;
use crate::ids::BlockRef;
use crate::input::{self, Input};
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
pub(crate) async fn read_replay(control: &Control, block: &BlockRef) -> Result<Vec<u8>, RexError> {
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

/// What is asked of a terminal's feed.
enum Command {
    /// A new view of the terminal.
    Join {
        facts: ClientFacts,
        reply: oneshot::Sender<Result<Attachment, RexError>>,
    },
    /// A view granted credit.
    Credit(u64),
    Rehydrate(u64),
    Release(u64),
    /// A view is gone. The sender is let go of once nothing of the view is left: when
    /// it was the last, once the stream is closed.
    Detach(u64, oneshot::Sender<()>),
    /// The backend is going: every view ends.
    Close,
}

/// Something that waits for a view to take it.
enum Waiting {
    /// Output, which goes only against credit.
    Data(Vec<u8>),
    Frame(OutputFrame),
}

/// How a view stands with the stream.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Standing {
    /// It follows the stream.
    Following,
    /// It took too little for too long, and gets no more output. It follows again,
    /// from a new copy of the terminal, when it grants credit.
    Behind,
    /// It let go of its screen. It follows again, from a new copy, when it asks.
    Released,
}

/// One view of a terminal, as the terminal's feed keeps it.
struct View {
    frames: mpsc::UnboundedSender<OutputFrame>,
    /// How much output the view has granted and not been sent.
    credit: Arc<AtomicU64>,
    /// Set once the view is no longer served, for its sink to see.
    gone: Arc<AtomicBool>,
    waiting: VecDeque<Waiting>,
    /// How many bytes of output wait.
    waiting_bytes: usize,
    /// The size the view was last told.
    size: TermSize,
    standing: Standing,
}

impl View {
    /// Sends the view what waits, as far as its credit goes. Returns whether the view
    /// goes on: it is over once the program's end was sent, or nobody takes its frames
    /// any more.
    fn pump(&mut self) -> bool {
        while let Some(front) = self.waiting.front_mut() {
            let frame = match front {
                Waiting::Data(bytes) => {
                    let credit =
                        usize::try_from(self.credit.load(Ordering::Acquire)).unwrap_or(usize::MAX);
                    if credit == 0 {
                        break;
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
                    _ => break,
                },
            };
            let last = matches!(frame, OutputFrame::Exit(_));
            if self.frames.send(frame).is_err() || last {
                return false;
            }
        }
        !self.frames.is_closed()
    }

    fn frame(&mut self, frame: OutputFrame) {
        self.waiting.push_back(Waiting::Frame(frame));
    }

    /// Lets go of the output that waits. Sizes and the program's end stay in line.
    fn drop_output(&mut self) {
        self.waiting
            .retain(|waiting| matches!(waiting, Waiting::Frame(_)));
        self.waiting_bytes = 0;
    }

    /// Tells the view the terminal's size, if it is not the one it was told last.
    fn tell_size(&mut self, size: TermSize) {
        if size != self.size {
            self.size = size;
            self.frame(OutputFrame::Resized(size));
        }
    }

    /// Starts the view again from a new copy of a terminal of `size`. The size goes
    /// first, so that the copy is shown at its own size.
    fn start_again(&mut self, size: TermSize, copy: Hydrate) {
        self.drop_output();
        self.tell_size(size);
        self.frame(OutputFrame::Hydrate(copy));
        self.standing = Standing::Following;
    }
}

/// How beginning the stream again went.
enum Again {
    /// It began, from this copy.
    Begun(Hydrate),
    /// The block is no longer there.
    Gone,
    /// It could not be begun, however often it was tried.
    GivenUp,
}

/// A terminal block's stream, and the views it is passed on to.
struct Feed {
    shared: Arc<Shared>,
    block: BlockRef,
    commands: mpsc::UnboundedReceiver<Command>,
    /// The way to this feed, for the views' sinks.
    tell: mpsc::UnboundedSender<Command>,
    input: Arc<Input>,
    views: HashMap<u64, View>,
    /// The number of the last view made.
    last_view: u64,
    live: Option<Live>,
    /// The terminal's size, as the stream last said it.
    size: Option<TermSize>,
    /// What a stream that has just begun held after its copy, to be passed on once
    /// the view it was begun for is among the views.
    begun_with: Vec<Piece>,
    /// Views that left last, to be told when the stream is closed.
    leaving: Vec<oneshot::Sender<()>>,
}

impl Feed {
    fn following(&self) -> bool {
        self.views
            .values()
            .any(|view| view.standing == Standing::Following)
    }

    /// A view is no longer served.
    fn remove(&mut self, id: u64) {
        if let Some(view) = self.views.remove(&id) {
            view.gone.store(true, Ordering::Release);
            self.input.leave(id);
        }
    }

    /// Makes a view and the attachment that is its other end.
    fn admit(&mut self, id: u64, size: TermSize, hydrate: Hydrate) -> Attachment {
        let (frames, taken) = mpsc::unbounded_channel();
        let credit = Arc::new(AtomicU64::new(0));
        let gone = Arc::new(AtomicBool::new(false));
        self.views.insert(
            id,
            View {
                frames,
                credit: Arc::clone(&credit),
                gone: Arc::clone(&gone),
                waiting: VecDeque::new(),
                waiting_bytes: 0,
                size,
                standing: Standing::Following,
            },
        );
        let sink = RexSink {
            view: id,
            tell: self.tell.clone(),
            credit,
            gone,
            input: Arc::clone(&self.input),
        };
        Attachment {
            size,
            // The view's terminal is to change size exactly as the server's own does,
            // which is as their common engine does it.
            resize_pulls_scrollback: true,
            hydrate,
            output: FrameStream::new(taken),
            input: InputSink::new(Arc::new(sink)),
        }
    }

    /// Puts what the stream held in line for the views. Output goes to the views that
    /// follow the stream; sizes and the program's end go to all.
    fn take(&mut self, pieces: Vec<Piece>) {
        for piece in pieces {
            match piece {
                Piece::Output(bytes) => {
                    let mut following: Vec<&mut View> = self
                        .views
                        .values_mut()
                        .filter(|view| view.standing == Standing::Following)
                        .collect();
                    let mut bytes = Some(bytes);
                    let count = following.len();
                    for (nth, view) in following.iter_mut().enumerate() {
                        // The last view takes the bytes themselves.
                        let Some(data) = (if nth + 1 == count {
                            bytes.take()
                        } else {
                            bytes.clone()
                        }) else {
                            break;
                        };
                        view.waiting_bytes += data.len();
                        view.waiting.push_back(Waiting::Data(data));
                        if view.waiting_bytes > PENDING_MAX_BYTES {
                            view.drop_output();
                            view.standing = Standing::Behind;
                        }
                    }
                }
                Piece::Note(Note::Resize { size, .. }) => {
                    self.size = Some(size);
                    for view in self.views.values_mut() {
                        view.tell_size(size);
                    }
                }
                Piece::Note(Note::ChildExited { exit_code }) => {
                    for view in self.views.values_mut() {
                        view.frame(OutputFrame::Exit(exit_of(exit_code)));
                    }
                }
                // Nothing shows a server's colours yet.
                Piece::Note(Note::Theme(_)) => {}
            }
        }
    }

    /// Sends every view what waits for it, and lets go of the views that are over.
    fn pump(&mut self) {
        let over: Vec<u64> = self
            .views
            .iter_mut()
            .filter_map(|(id, view)| (!view.pump()).then_some(*id))
            .collect();
        for id in over {
            self.remove(id);
        }
    }

    /// Lets go of the stream.
    async fn stop(&mut self) {
        if let Some(live) = self.live.take() {
            live.conn.close().await;
        }
    }

    /// The block is no longer there: every view is told the program ended, unless it
    /// was told already.
    fn block_gone(&mut self) {
        self.live = None;
        for view in self.views.values_mut() {
            let told = view
                .waiting
                .iter()
                .any(|waiting| matches!(waiting, Waiting::Frame(OutputFrame::Exit(_))));
            if !told {
                view.frame(OutputFrame::Exit(ExitStatus::Unknown));
            }
        }
    }

    /// What a stream that has begun means to the views. A server gives a client one
    /// stream of a block, and a copy of the terminal only where a stream begins. So
    /// every view that follows the stream starts again from the new copy along with
    /// `also`, the view the stream was begun again for: what was printed between the
    /// old stream and the new one is in the copy and nowhere else.
    fn begun(&mut self, started: Started, also: Option<u64>) -> Hydrate {
        self.size = Some(started.size);
        for (id, view) in &mut self.views {
            if view.standing == Standing::Following || Some(*id) == also {
                view.start_again(started.size, started.hydrate.clone());
            }
        }
        self.live = Some(started.live);
        self.begun_with = started.after;
        started.hydrate
    }

    /// Begins the stream again, from a new copy of the terminal.
    async fn restart(&mut self, also: Option<u64>) -> Again {
        self.stop().await;
        let mut link = self.shared.link();
        let mut attempts = 0;
        loop {
            match start(&self.shared, &self.block).await {
                Ok(started) => return Again::Begun(self.begun(started, also)),
                Err(error) if gone(&error) => return Again::Gone,
                Err(error) => {
                    attempts += 1;
                    if attempts >= RESTART_ATTEMPTS_MAX {
                        tracing::warn!(server = %self.shared.name, %error, "a terminal's stream could not be begun again");
                        return Again::GivenUp;
                    }
                    tracing::debug!(server = %self.shared.name, %error, attempts, "a terminal's stream will be begun again");
                    tokio::time::sleep(backoff(attempts)).await;
                    // With no connection to the server there is nothing to try.
                    let connected = link
                        .wait_for(|link| !matches!(link, Link::Connecting | Link::Waiting { .. }))
                        .await;
                    if !matches!(connected.as_deref(), Ok(Link::Connected(_))) {
                        return Again::GivenUp;
                    }
                }
            }
        }
    }

    /// Begins the stream again for the views there are, and acts on how that went.
    async fn restart_for(&mut self, also: Option<u64>) {
        match self.restart(also).await {
            Again::Begun(_) => {}
            Again::Gone => self.block_gone(),
            Again::GivenUp => self.end_all(),
        }
    }

    /// Ends every view.
    fn end_all(&mut self) {
        let all: Vec<u64> = self.views.keys().copied().collect();
        for id in all {
            self.remove(id);
        }
    }

    /// Takes in a new view. The first one begins the stream; a later one has it begun
    /// again, since only there is a copy of the terminal to start from.
    async fn join(&mut self, facts: &ClientFacts) -> Result<Attachment, RexError> {
        self.last_view += 1;
        let id = self.last_view;
        if self.views.is_empty() {
            // Said before the stream begins: the copy the view starts from is then
            // already of the view's size, where the server lets this client decide.
            self.input.arrive(id, facts).await;
            self.stop().await;
            match start(&self.shared, &self.block).await {
                Ok(started) => {
                    let size = started.size;
                    let hydrate = self.begun(started, None);
                    Ok(self.admit(id, size, hydrate))
                }
                Err(error) => {
                    self.input.leave(id);
                    Err(error)
                }
            }
        } else {
            self.input.join(id, facts);
            match self.restart(None).await {
                Again::Begun(hydrate) => {
                    // The stream says its size before anything else.
                    let size = self.size.unwrap_or(facts.size);
                    Ok(self.admit(id, size, hydrate))
                }
                Again::Gone => {
                    self.input.leave(id);
                    self.block_gone();
                    Err(RexError::Rejected(Reject::Unavailable))
                }
                Again::GivenUp => {
                    self.input.leave(id);
                    self.end_all();
                    Err(RexError::Disconnected)
                }
            }
        }
    }

    /// Acts on the end of a connection.
    async fn ended(&mut self, end: DataEnd) {
        match end {
            DataEnd::Dropped => {
                let Some(lost) = self.live.take() else {
                    return;
                };
                match resume(&self.shared, lost).await {
                    Ok(live) => self.live = Some(live),
                    Err(error) => {
                        tracing::debug!(server = %self.shared.name, %error, "a terminal's stream could not be continued");
                        self.restart_for(None).await;
                    }
                }
            }
            DataEnd::Ended | DataEnd::Restart => {
                self.live = None;
                self.restart_for(None).await;
            }
            DataEnd::BlockClosed => self.block_gone(),
            DataEnd::Failed(error) => {
                tracing::debug!(server = %self.shared.name, %error, "a terminal's stream failed");
                self.live = None;
                self.restart_for(None).await;
            }
        }
    }

    async fn act(&mut self, command: Command) {
        match command {
            Command::Join { facts, reply } => {
                let joined = self.join(&facts).await;
                // Whoever asked may have stopped waiting: the view then has no other
                // end, and goes with the next look at it.
                let _ = reply.send(joined);
            }
            Command::Credit(id) => {
                let behind = self
                    .views
                    .get(&id)
                    .is_some_and(|view| view.standing == Standing::Behind);
                if behind {
                    self.restart_for(Some(id)).await;
                }
            }
            Command::Rehydrate(id) => {
                if self.views.contains_key(&id) {
                    self.restart_for(Some(id)).await;
                }
            }
            Command::Release(id) => {
                if let Some(view) = self.views.get_mut(&id) {
                    view.drop_output();
                    view.standing = Standing::Released;
                }
            }
            Command::Detach(id, done) => {
                self.remove(id);
                if self.views.is_empty() {
                    self.leaving.push(done);
                }
            }
            Command::Close => self.end_all(),
        }
    }

    /// Serves the views until there are none.
    async fn run(&mut self) {
        loop {
            tokio::select! {
                // What the views ask is done first: a view that let go of its screen
                // is then sent no output that came after it asked.
                biased;
                command = self.commands.recv() => match command {
                    Some(command) => self.act(command).await,
                    None => return,
                },
                bytes = next_of(&mut self.live) => match bytes {
                    Ok(bytes) => {
                        let pieces = match &mut self.live {
                            Some(live) => live.splitter.feed(&bytes),
                            None => Vec::new(),
                        };
                        self.take(pieces);
                    }
                    Err(end) => self.ended(end).await,
                },
            }
            let begun_with = std::mem::take(&mut self.begun_with);
            self.take(begun_with);
            self.pump();
            if self.views.is_empty() {
                return;
            }
            // A stream nobody follows is not kept: a server sends all of it.
            if !self.following() {
                self.stop().await;
            }
        }
    }
}

/// Where a feed's answer to a view that asked to join comes.
pub(crate) type Joining = oneshot::Receiver<Result<Attachment, RexError>>;

/// The way to a terminal's feed, as its backend keeps it.
pub(crate) struct FeedRef {
    /// Tells this feed from another of the same terminal, earlier or later.
    pub(crate) serial: u64,
    tell: mpsc::UnboundedSender<Command>,
}

impl FeedRef {
    /// Starts the feed of a block. It serves the views that join it and ends with the
    /// last of them, or at once if the first cannot be served. `id` is the block as
    /// the application names it. Must be called inside a Tokio runtime.
    pub(crate) fn start(shared: Arc<Shared>, id: SessionId, serial: u64, block: BlockRef) -> Self {
        let (tell, commands) = mpsc::unbounded_channel();
        let (input, typing) = Input::start(Arc::clone(&shared), block.clone());
        let mut feed = Feed {
            shared,
            block,
            commands,
            tell: tell.clone(),
            input: Arc::new(input),
            views: HashMap::new(),
            last_view: 0,
            live: None,
            size: None,
            begun_with: Vec::new(),
            leaving: Vec::new(),
        };
        tokio::spawn(async move {
            feed.run().await;
            feed.end_all();
            // Closed properly, the stream ends on the server at once.
            feed.stop().await;
            typing.abort();
            let leaving = input::give_up(&feed.shared, &feed.block);
            let _ = tokio::time::timeout(DATA_CLOSE_TIMEOUT, leaving).await;
            // Only now can the terminal be attached to anew: its stream is free.
            feed.shared.forget(id, serial);
        });
        Self { serial, tell }
    }

    /// Asks the feed for a new view, and returns where the answer will come. No answer,
    /// here or there, means the feed is over: its terminal is to be asked again, of a
    /// new feed.
    pub(crate) fn join(&self, facts: ClientFacts) -> Option<Joining> {
        let (reply, joined) = oneshot::channel();
        self.tell.send(Command::Join { facts, reply }).ok()?;
        Some(joined)
    }

    /// Ends every view of the feed, without waiting for it.
    pub(crate) fn close(&self) {
        let _ = self.tell.send(Command::Close);
    }
}

/// A view's side of its attachment.
struct RexSink {
    view: u64,
    tell: mpsc::UnboundedSender<Command>,
    credit: Arc<AtomicU64>,
    /// Set once the view is detached or no longer served.
    gone: Arc<AtomicBool>,
    input: Arc<Input>,
}

impl RexSink {
    fn check_open(&self) -> Result<(), SessionError> {
        if self.gone.load(Ordering::Acquire) || self.tell.is_closed() {
            return Err(SessionError::Detached);
        }
        Ok(())
    }

    fn tell(&self, command: Command) -> Result<(), SessionError> {
        self.check_open()?;
        self.tell.send(command).map_err(|_| SessionError::Detached)
    }
}

#[async_trait]
impl Sink for RexSink {
    async fn write(&self, bytes: &[u8]) -> Result<(), SessionError> {
        self.check_open()?;
        self.input.write(self.view, bytes)
    }

    async fn resize(&self, size: TermSize, cell_px: CellPx) -> Result<(), SessionError> {
        self.check_open()?;
        self.input.resize(self.view, size, cell_px);
        Ok(())
    }

    /// Of what a view says of itself, a server is told whether it has the focus. A
    /// view's colours are its own: a server's terminal has one set for all its clients.
    async fn facts(&self, facts: &ClientFacts) -> Result<(), SessionError> {
        self.check_open()?;
        self.input.focus(self.view, facts.focused);
        Ok(())
    }

    async fn credit(&self, bytes: u32) -> Result<(), SessionError> {
        self.check_open()?;
        // Recorded before the feed hears of it: it may use it the moment it looks.
        self.credit
            .try_update(Ordering::AcqRel, Ordering::Acquire, |granted| {
                granted
                    .checked_add(u64::from(bytes))
                    .filter(|granted| *granted <= u64::from(MAX_WINDOW))
            })
            .map_err(|_| SessionError::CreditOverflow)?;
        self.tell(Command::Credit(self.view))
    }

    async fn rehydrate(&self) -> Result<(), SessionError> {
        self.tell(Command::Rehydrate(self.view))
    }

    async fn release(&self) -> Result<(), SessionError> {
        self.tell(Command::Release(self.view))
    }

    /// Ends the view. When it was the terminal's last, this waits until the stream is
    /// closed, so that attaching again at once finds the way free.
    async fn detach(&self) -> Result<(), SessionError> {
        if self.gone.swap(true, Ordering::AcqRel) {
            return Err(SessionError::Detached);
        }
        let (done, gone) = oneshot::channel();
        if self.tell.send(Command::Detach(self.view, done)).is_ok() {
            // Either word or silence says it is done: the feed lets go of the sender
            // when it is.
            let _ = tokio::time::timeout(3 * DATA_CLOSE_TIMEOUT, gone).await;
        }
        Ok(())
    }
}
