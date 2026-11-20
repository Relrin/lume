//! The byte carrier between the core and one webview, and the streams that travel on it.
//!
//! A webview shows sessions through attachments. Each attachment is a stream: the
//! session's output, resize marks, events and exit go to the webview as frames, and the
//! webview's input, size requests, facts, credit and detach come back as frames. The
//! framing is the session protocol's own, so both ends are checked against the same
//! recorded byte streams, and the bytes of a terminal are never re-encoded as text.
//!
//! A [`Link`] is one carrier with its streams. It keeps no credit: the webview's grants
//! go straight to the session, which sends no more than it was granted. What waits in
//! a link is therefore bounded by the credit the webview handed out.
//!
//! A stream may also be a pipe: one of a plugin's streams of bytes, which a view of
//! that plugin was let use. Its bytes travel as a session's output and input do, under
//! the same credit, but nothing else of a session's does: there is no size, no event
//! and no state to start from. A pipe is read and written on threads of its own, since
//! both may wait, and a page that writes faster than the pipe takes loses the pipe
//! rather than fill the application's memory.
//!
//! The carrier's stream 0 carries what the core has to say to the page itself. Some of
//! it is requests: what is on a session's screen is known only to the terminal in the
//! page, so the core asks, and the page answers through a command that names the
//! request it answers. The rest is notices, which tell the page that something it keeps
//! a copy of has changed. Both leave through the one writer, so a page receives them in
//! the order they were sent.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use lume_proto::app::{AppError, AppReplyBody, PageRequest, PageRequestBody, codes, encode_notice};
use lume_proto::{Body, FrameDecoder, Header, HydratePayload, Kind, Message, Role, encode};
use lume_session::{
    Attachment, BackendId, ExitStatus, Hydrate, InputSink, OutputFrame, SessionError, SessionId,
};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};
use tokio::task::AbortHandle;

use crate::error::CommandError;
use crate::marks::Marks;
use crate::sessions::SessionStates;
use crate::tuning::{CARRIER_BATCH_MAX, PIPE_READ_MAX, PIPE_WRITE_QUEUE_MAX};

/// The other end of the carrier is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CarrierClosed;

/// The way from the core to one webview.
pub trait Carrier: Send + Sync + 'static {
    /// Hands over whole frames. Successive calls are delivered in order.
    fn deliver(&self, frames: Vec<u8>) -> Result<(), CarrierClosed>;
}

/// A carrier over a Tauri channel, which takes raw bytes and keeps their order.
pub struct TauriChannelCarrier(pub tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>);

impl Carrier for TauriChannelCarrier {
    fn deliver(&self, frames: Vec<u8>) -> Result<(), CarrierClosed> {
        self.0
            .send(tauri::ipc::InvokeResponseBody::Raw(frames))
            .map_err(|_| CarrierClosed)
    }
}

/// Names a session wherever it runs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionKey {
    pub backend: BackendId,
    pub session: SessionId,
}

/// Called from the task that forwards a session's output: with the status when the
/// program's exit goes by, and with nothing when the output has ended.
pub type OnExit = Arc<dyn Fn(&SessionKey, Option<ExitStatus>) + Send + Sync>;

/// What a page answered, or why there is no answer.
pub type PageAnswer = Result<AppReplyBody, AppError>;

/// What waits to be written to the carrier.
enum Queued {
    /// A frame of a stream.
    Frame(Message),
    /// A request to the page, already encoded as the frames of a control message.
    Control(Vec<u8>),
}

/// One carrier and the streams on it.
pub struct Link {
    id: u32,
    outgoing: mpsc::UnboundedSender<Queued>,
    state: Mutex<LinkState>,
    writer: AbortHandle,
    runtime: Handle,
    on_exit: OnExit,
    marks: Arc<Marks>,
    /// Where what the sessions report is noted on its way to the page.
    reports: Arc<SessionStates>,
}

struct LinkState {
    streams: HashMap<u32, Stream>,
    /// The highest stream id the webview has used. Ids only go up, so one is never
    /// reused and a late frame cannot land on a newer attachment.
    last_stream: u32,
    retired: bool,
    /// Requests the page has not answered yet, by their id.
    asked: HashMap<u64, oneshot::Sender<PageAnswer>>,
    next_ask: u64,
}

enum Stream {
    /// An attachment to a session.
    Session {
        session: SessionKey,
        input: InputSink,
        pump: AbortHandle,
    },
    /// One of a plugin's streams, used by one of its views.
    Pipe(Arc<PipeFlow>),
}

impl Stream {
    /// Stops what forwards the stream's output. A session's input is detached apart.
    fn stop(&self) {
        match self {
            Stream::Session { pump, .. } => pump.abort(),
            Stream::Pipe(flow) => flow.close(),
        }
    }

    fn into_input(self) -> Option<InputSink> {
        match self {
            Stream::Session { input, .. } => Some(input),
            Stream::Pipe(_) => None,
        }
    }
}

/// Where the frames a page sends on a stream go.
enum Target {
    Session(InputSink),
    Pipe(Arc<PipeFlow>),
}

/// A stream of bytes a link reads for a page and writes for it. Every call may wait,
/// and is made on a thread kept for it.
pub trait Pipe: Send + Sync + 'static {
    /// Whether there is anything to read from it, and whether it can be written.
    fn ways(&self) -> (bool, bool);
    /// The next bytes, `max` at most, waiting for at least one; nothing at its end.
    fn read(&self, max: usize) -> Result<Option<Vec<u8>>, String>;
    /// Writes all of `bytes`, waiting while it is full.
    fn write(&self, bytes: &[u8]) -> Result<(), String>;
    /// Nothing more will be written: a program reading it sees its input end.
    fn finish(&self);
}

/// Called once when a pipe is over, however that came about.
pub type PipeEnded = Box<dyn FnOnce() + Send>;

/// What a pipe's two threads and its link share.
struct PipeFlow {
    state: Mutex<PipeState>,
    /// Rung when credit arrives, when there is something to write, and at the end.
    changed: Condvar,
}

struct PipeState {
    /// How many bytes the page said it will take.
    credit: u64,
    /// What the page wrote and the pipe has not taken yet; an empty piece means that
    /// nothing more will be written.
    writes: VecDeque<Vec<u8>>,
    queued: usize,
    closed: bool,
    ended: Option<PipeEnded>,
}

impl PipeFlow {
    fn state(&self) -> MutexGuard<'_, PipeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Ends the flow: both threads stop as soon as the call each is in returns.
    fn close(&self) {
        let ended = {
            let mut state = self.state();
            state.closed = true;
            state.writes.clear();
            state.queued = 0;
            state.ended.take()
        };
        self.changed.notify_all();
        if let Some(ended) = ended {
            ended();
        }
    }

    fn grant(&self, bytes: u32) {
        let mut state = self.state();
        state.credit = state.credit.saturating_add(u64::from(bytes));
        drop(state);
        self.changed.notify_all();
    }

    /// Takes in what the page wrote. Refused when more waits than a pipe may hold.
    fn push(&self, bytes: Vec<u8>) -> Result<(), ()> {
        let mut state = self.state();
        if state.closed {
            return Ok(());
        }
        if state.queued + bytes.len() > PIPE_WRITE_QUEUE_MAX {
            return Err(());
        }
        state.queued += bytes.len();
        state.writes.push_back(bytes);
        drop(state);
        self.changed.notify_all();
        Ok(())
    }
}

/// Reads a pipe for as long as the page grants credit, and sends what it read.
fn pipe_reader(
    stream: u32,
    pipe: &dyn Pipe,
    flow: &PipeFlow,
    outgoing: &mpsc::UnboundedSender<Queued>,
) {
    let status = loop {
        let allowed = {
            let mut state = flow.state();
            while state.credit == 0 && !state.closed {
                state = flow
                    .changed
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            if state.closed {
                return;
            }
            usize::try_from(state.credit)
                .unwrap_or(usize::MAX)
                .min(PIPE_READ_MAX)
        };
        match pipe.read(allowed) {
            Ok(Some(bytes)) => {
                {
                    let mut state = flow.state();
                    // Closed while the read waited: what it read has nowhere to go.
                    if state.closed {
                        return;
                    }
                    state.credit = state.credit.saturating_sub(bytes.len() as u64);
                }
                if outgoing
                    .send(Queued::Frame(Message::new(stream, Body::Data(bytes))))
                    .is_err()
                {
                    break None;
                }
            }
            Ok(None) => break Some(ExitStatus::Code(0)),
            Err(problem) => {
                tracing::debug!(stream, problem, "a pipe could not be read");
                break Some(ExitStatus::Unknown);
            }
        }
    };
    if let Some(status) = status {
        let _ = outgoing.send(Queued::Frame(Message::new(stream, Body::Exit(status))));
    }
    flow.close();
}

/// Writes what the page sent to a pipe, in order.
fn pipe_writer(
    stream: u32,
    pipe: &dyn Pipe,
    flow: &PipeFlow,
    readable: bool,
    outgoing: &mpsc::UnboundedSender<Queued>,
) {
    loop {
        let next = {
            let mut state = flow.state();
            loop {
                if state.closed {
                    return;
                }
                if let Some(next) = state.writes.pop_front() {
                    state.queued -= next.len();
                    break next;
                }
                state = flow
                    .changed
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        if next.is_empty() {
            pipe.finish();
            // A pipe that is only written is over once its input has ended.
            if !readable {
                let _ = outgoing.send(Queued::Frame(Message::new(
                    stream,
                    Body::Exit(ExitStatus::Code(0)),
                )));
                flow.close();
            }
            return;
        }
        if let Err(problem) = pipe.write(&next) {
            tracing::debug!(stream, problem, "a pipe could not be written");
            let _ = outgoing.send(Queued::Frame(Message::new(
                stream,
                Body::Exit(ExitStatus::Unknown),
            )));
            flow.close();
            return;
        }
    }
}

fn page_gone() -> AppError {
    AppError::new(codes::NO_WINDOW, "the page that was asked has gone")
}

impl Link {
    /// Starts a link that delivers to `carrier`. `id` is how the webview names it.
    pub fn open(
        id: u32,
        carrier: Arc<dyn Carrier>,
        runtime: &Handle,
        on_exit: OnExit,
        marks: Arc<Marks>,
        reports: Arc<SessionStates>,
    ) -> Arc<Self> {
        let (outgoing, queued) = mpsc::unbounded_channel();
        let writer = runtime.spawn(write_loop(queued, carrier)).abort_handle();
        Arc::new(Self {
            id,
            outgoing,
            state: Mutex::new(LinkState {
                streams: HashMap::new(),
                last_stream: 0,
                retired: false,
                asked: HashMap::new(),
                next_ask: 1,
            }),
            writer,
            runtime: runtime.clone(),
            on_exit,
            marks,
            reports,
        })
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    fn state(&self) -> MutexGuard<'_, LinkState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Claims a stream id for an attachment that is about to be made. The webview
    /// chooses the id, so that it can be ready for frames before the attach returns;
    /// the id must be above every one it has used on this link.
    pub fn reserve(&self, stream: u32) -> Result<(), CommandError> {
        let mut state = self.state();
        if state.retired {
            return Err(CommandError::stale_carrier());
        }
        if stream <= state.last_stream {
            return Err(CommandError::invalid_stream(stream));
        }
        state.last_stream = stream;
        Ok(())
    }

    /// Puts an attachment on a reserved stream and starts forwarding its output.
    ///
    /// If the link was retired while the attachment was being made, the attachment is
    /// detached again, so the session is free for whoever asks next.
    pub async fn bind(
        &self,
        stream: u32,
        session: SessionKey,
        attachment: Attachment,
    ) -> Result<(), CommandError> {
        let Attachment {
            hydrate,
            output,
            input,
            ..
        } = attachment;
        {
            let mut state = self.state();
            if !state.retired {
                // The state to start from goes first, ahead of anything the pump sends.
                if let Some(start) = payload_of(hydrate) {
                    let _ = self
                        .outgoing
                        .send(Queued::Frame(Message::new(stream, Body::Hydrate(start))));
                }
                let forwarding = pump(
                    stream,
                    session.clone(),
                    output,
                    self.outgoing.clone(),
                    Arc::clone(&self.on_exit),
                    Arc::clone(&self.marks),
                    Arc::clone(&self.reports),
                );
                let pump = self.runtime.spawn(forwarding).abort_handle();
                state.streams.insert(
                    stream,
                    Stream::Session {
                        session,
                        input,
                        pump,
                    },
                );
                return Ok(());
            }
        }
        let _ = input.detach().await;
        Err(CommandError::stale_carrier())
    }

    /// Puts one of a plugin's streams on a reserved stream and starts carrying its
    /// bytes: what is read from it goes to the page as output, within the credit the
    /// page grants, and the page's input is written to it. `ended` is called once when
    /// the pipe is over: it ran out, failed, the page let go of it, or the link went.
    pub fn bind_pipe(
        &self,
        stream: u32,
        pipe: Arc<dyn Pipe>,
        ended: PipeEnded,
    ) -> Result<(), CommandError> {
        let (readable, writable) = pipe.ways();
        let flow = Arc::new(PipeFlow {
            state: Mutex::new(PipeState {
                credit: 0,
                writes: VecDeque::new(),
                queued: 0,
                closed: false,
                ended: Some(ended),
            }),
            changed: Condvar::new(),
        });
        {
            let mut state = self.state();
            if state.retired {
                drop(state);
                flow.close();
                return Err(CommandError::stale_carrier());
            }
            state
                .streams
                .insert(stream, Stream::Pipe(Arc::clone(&flow)));
        }
        let started = |name: &str, work: Box<dyn FnOnce() + Send>| {
            std::thread::Builder::new()
                .name(name.to_owned())
                .spawn(work)
                .map(drop)
        };
        let mut threads = Ok(());
        if readable {
            let (pipe, flow, outgoing) =
                (Arc::clone(&pipe), Arc::clone(&flow), self.outgoing.clone());
            threads = started(
                "lume-pipe-read",
                Box::new(move || pipe_reader(stream, pipe.as_ref(), &flow, &outgoing)),
            );
        }
        if writable && threads.is_ok() {
            let (pipe, flow, outgoing) =
                (Arc::clone(&pipe), Arc::clone(&flow), self.outgoing.clone());
            threads = started(
                "lume-pipe-write",
                Box::new(move || pipe_writer(stream, pipe.as_ref(), &flow, readable, &outgoing)),
            );
        }
        if let Err(error) = threads {
            self.end_pipe(stream);
            return Err(CommandError::internal(format!(
                "the stream could not be started: {error}"
            )));
        }
        Ok(())
    }

    /// Ends a pipe from the application's side, as when its grant is taken back: the
    /// page is told the stream is over.
    pub fn end_pipe(&self, stream: u32) {
        let ended = {
            let mut state = self.state();
            match state.streams.get(&stream) {
                Some(Stream::Pipe(_)) => state.streams.remove(&stream),
                _ => None,
            }
        };
        if let Some(Stream::Pipe(flow)) = ended {
            flow.close();
            let _ = self.outgoing.send(Queued::Frame(Message::new(
                stream,
                Body::Exit(ExitStatus::Unknown),
            )));
        }
    }

    /// Acts on what a page sent on a pipe.
    fn apply_to_pipe(&self, stream: u32, flow: &PipeFlow, body: Body) {
        match body {
            Body::Input(bytes) => {
                if flow.push(bytes).is_err() {
                    tracing::debug!(stream, "a page wrote more than a pipe holds");
                    self.end_pipe(stream);
                }
            }
            Body::Credit(bytes) => flow.grant(bytes),
            Body::Detach => {
                let gone = self.state().streams.remove(&stream);
                if let Some(gone) = gone {
                    gone.stop();
                }
            }
            // A pipe has no size, no state and no facts.
            _ => {}
        }
    }

    /// Acts on the frames the webview sent, in order.
    ///
    /// `frames` must be whole frames of the kinds a view sends on a stream. Frames for
    /// a stream this link does not have are dropped: the webview may send one last
    /// frame to a stream that has just ended.
    pub async fn apply(&self, frames: &[u8]) -> Result<(), CommandError> {
        for message in decode_client_frames(frames)? {
            let Message { stream, body } = message;
            let target = self.state().streams.get(&stream).map(|found| match found {
                Stream::Session { input, .. } => Target::Session(input.clone()),
                Stream::Pipe(flow) => Target::Pipe(Arc::clone(flow)),
            });
            let input = match target {
                Some(Target::Session(input)) => input,
                Some(Target::Pipe(flow)) => {
                    self.apply_to_pipe(stream, &flow, body);
                    continue;
                }
                None => continue,
            };
            let result = match body {
                Body::Input(bytes) if bytes.is_empty() => Ok(()),
                Body::Input(bytes) => input.write(&bytes).await,
                Body::Resize { size, cell_px } => input.resize(size, cell_px).await,
                Body::Facts(facts) => input.facts(&facts).await,
                Body::Credit(bytes) => input.credit(bytes).await,
                Body::Rehydrate => input.rehydrate().await,
                Body::Release => input.release().await,
                Body::Detach => {
                    self.close(stream).await;
                    Ok(())
                }
                // The decoder accepts nothing else from a view.
                _ => Ok(()),
            };
            match result {
                // Detached under us: the stream ended while this frame was on its way.
                Ok(()) | Err(SessionError::Detached) => {}
                Err(SessionError::CreditOverflow) => {
                    // A view that grants without bound has lost count; nothing it sends
                    // on this stream can be trusted to keep the flow bounded.
                    self.close(stream).await;
                    return Err(SessionError::CreditOverflow.into());
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    /// Tells the page something it did not ask for. A notice for a page that has gone
    /// is dropped: its successor asks for the whole of whatever the notice was about.
    pub fn notify(&self, notice: &serde_json::Value) {
        if let Some(frames) = encoded_notice(notice) {
            self.notify_encoded(frames);
        }
    }

    /// Tells the page a notice already put into frames by [`encoded_notice`]: a notice
    /// for several pages is encoded once.
    pub fn notify_encoded(&self, frames: Vec<u8>) {
        if self.state().retired {
            return;
        }
        let _ = self.outgoing.send(Queued::Control(frames));
    }

    /// The way to type into a session, if the page is attached to it.
    pub fn input_of(&self, session: &SessionKey) -> Option<InputSink> {
        let state = self.state();
        state
            .streams
            .iter()
            .filter_map(|(id, stream)| match stream {
                Stream::Session {
                    session: shown,
                    input,
                    ..
                } if shown == session => Some((id, input)),
                _ => None,
            })
            .max_by_key(|(id, _)| **id)
            .map(|(_, input)| input.clone())
    }

    /// Asks the page something and waits for its answer, for `limit` at most.
    ///
    /// A page that is replaced or closed while it is being asked gives no answer; that
    /// is reported at once rather than when the time runs out.
    pub async fn ask(&self, body: PageRequestBody, limit: Duration) -> PageAnswer {
        let (answer, answered) = oneshot::channel();
        let id = {
            let mut state = self.state();
            if state.retired {
                return Err(page_gone());
            }
            let id = state.next_ask;
            state.next_ask += 1;
            state.asked.insert(id, answer);
            id
        };
        let mut frames = Vec::new();
        let sent = PageRequest { id, body }
            .encode(&mut frames)
            .map_err(|error| AppError::new(codes::BAD_REQUEST, error.to_string()))
            .and_then(|()| {
                self.outgoing
                    .send(Queued::Control(frames))
                    .map_err(|_| page_gone())
            });
        if let Err(error) = sent {
            self.state().asked.remove(&id);
            return Err(error);
        }
        match tokio::time::timeout(limit, answered).await {
            Ok(Ok(answer)) => answer,
            // The link was retired, which dropped the other end.
            Ok(Err(_)) => Err(page_gone()),
            Err(_) => {
                self.state().asked.remove(&id);
                Err(AppError::new(
                    codes::TIMEOUT,
                    "the page did not answer in time",
                ))
            }
        }
    }

    /// Takes the page's answer to the request with this id. An answer nobody waits for
    /// any more, or one with an id that was never asked, is dropped.
    pub fn answer(&self, id: u64, answer: PageAnswer) {
        let waiting = self.state().asked.remove(&id);
        if let Some(waiting) = waiting {
            let _ = waiting.send(answer);
        }
    }

    /// Ends one stream: its output stops and its session is free for another attachment.
    async fn close(&self, stream: u32) {
        let closed = self.state().streams.remove(&stream);
        if let Some(closed) = closed {
            closed.stop();
            if let Some(input) = closed.into_input() {
                let _ = input.detach().await;
            }
        }
    }

    /// Ends the link: nothing more is delivered and every stream is detached.
    ///
    /// When this returns, each detach has been handed to its backend, so an attach
    /// made afterwards finds the sessions free.
    pub async fn retire(&self) {
        let streams = {
            let mut state = self.state();
            state.retired = true;
            // Whoever waits for the page's answer learns now that none will come.
            state.asked.clear();
            std::mem::take(&mut state.streams)
        };
        self.writer.abort();
        for stream in streams.values() {
            stream.stop();
        }
        for stream in streams.into_values() {
            if let Some(input) = stream.into_input() {
                let _ = input.detach().await;
            }
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.writer.abort();
        let state = self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        for stream in state.streams.values() {
            stream.stop();
        }
    }
}

/// Reads `frames` as the whole frames of a view's streams.
fn decode_client_frames(frames: &[u8]) -> Result<Vec<Message>, CommandError> {
    // The decoder waits for more when a frame is cut short and for the rest of a split
    // message, and there is no more to come here. So the frames are measured first.
    let mut rest = frames;
    while !rest.is_empty() {
        let header = rest
            .first_chunk::<{ Header::LEN }>()
            .ok_or_else(|| CommandError::bad_request("a frame header is cut short"))
            .and_then(|bytes| {
                Header::parse(bytes).map_err(|error| CommandError::bad_request(error.to_string()))
            })?;
        if header.kind == Kind::Control {
            return Err(CommandError::bad_request(
                "control messages do not travel on the carrier",
            ));
        }
        rest = rest
            .get(Header::LEN + header.len as usize..)
            .ok_or_else(|| CommandError::bad_request("a frame is cut short"))?;
    }

    let mut decoder = FrameDecoder::new(Role::Host);
    decoder.push(frames);
    let mut messages = Vec::new();
    loop {
        match decoder.next() {
            Ok(Some(message)) => messages.push(message),
            Ok(None) => return Ok(messages),
            Err(error) => return Err(CommandError::bad_request(error.to_string())),
        }
    }
}

/// Forwards one attachment's frames to the link's writer under the webview's stream id.
/// The state to start from as the wire carries it, if there is any.
fn payload_of(hydrate: Hydrate) -> Option<HydratePayload> {
    match hydrate {
        Hydrate::None => None,
        Hydrate::Snapshot { vt_version, bytes } => {
            Some(HydratePayload::Snapshot { vt_version, bytes })
        }
        Hydrate::VtReplay(bytes) => Some(HydratePayload::VtReplay(bytes)),
    }
}

async fn pump(
    stream: u32,
    session: SessionKey,
    mut output: lume_session::FrameStream,
    outgoing: mpsc::UnboundedSender<Queued>,
    on_exit: OnExit,
    marks: Arc<Marks>,
    reports: Arc<SessionStates>,
) {
    let mut exited = false;
    while let Some(frame) = output.next().await {
        let body = match frame {
            OutputFrame::Data(bytes) => {
                marks.output();
                Body::Data(bytes)
            }
            OutputFrame::Resized(size) => Body::Resized(size),
            OutputFrame::Event(event) => {
                reports.on_event(&session, &event);
                Body::Event(event)
            }
            OutputFrame::Exit(status) => {
                exited = true;
                on_exit(&session, Some(status));
                Body::Exit(status)
            }
            OutputFrame::Hydrate(hydrate) => match payload_of(hydrate) {
                Some(payload) => Body::Hydrate(payload),
                None => continue,
            },
        };
        if outgoing
            .send(Queued::Frame(Message::new(stream, body)))
            .is_err()
        {
            return;
        }
    }
    // The attachment ended without the program's exit: the backend went away. The view
    // is told the session is over rather than left waiting for output for ever.
    if !exited {
        let _ = outgoing.send(Queued::Frame(Message::new(
            stream,
            Body::Exit(ExitStatus::Unknown),
        )));
    }
    on_exit(&session, None);
}

/// Sends queued frames to the carrier, as many as are ready in each message.
///
/// This is the only sender on a carrier, which is what keeps frames in order.
async fn write_loop(mut queued: mpsc::UnboundedReceiver<Queued>, carrier: Arc<dyn Carrier>) {
    let mut batch = Vec::new();
    let mut piece = Vec::new();
    while let Some(mut next) = queued.recv().await {
        loop {
            piece.clear();
            let encoded = match next {
                Queued::Frame(message) => encode(&message, &mut piece).map_err(|error| {
                    tracing::error!(%error, stream = message.stream, "a frame could not be encoded");
                }),
                Queued::Control(frames) => {
                    piece = frames;
                    Ok(())
                }
            };
            if encoded.is_err() {
                // Reported above; the frames around it still go.
            } else if !batch.is_empty() && batch.len() + piece.len() > CARRIER_BATCH_MAX {
                // A frame is never split across messages: this one starts the next.
                if carrier.deliver(std::mem::take(&mut batch)).is_err() {
                    return;
                }
                batch.extend_from_slice(&piece);
            } else {
                batch.extend_from_slice(&piece);
            }
            if batch.len() >= CARRIER_BATCH_MAX {
                break;
            }
            match queued.try_recv() {
                Ok(following) => next = following,
                Err(_) => break,
            }
        }
        if !batch.is_empty() && carrier.deliver(std::mem::take(&mut batch)).is_err() {
            return;
        }
    }
}

/// A notice put into the frames a page is sent, or nothing if it cannot be.
pub fn encoded_notice(notice: &serde_json::Value) -> Option<Vec<u8>> {
    let mut frames = Vec::new();
    match encode_notice(notice, &mut frames) {
        Ok(()) => Some(frames),
        Err(error) => {
            tracing::error!(%error, "a notice could not be encoded");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use lume_proto::{CellPx, TermSize};

    use super::*;
    use crate::testing::RecordingCarrier;

    fn data(stream: u32, len: usize) -> Message {
        Message::new(stream, Body::Data(vec![b'x'; len]))
    }

    async fn written(messages: Vec<Message>) -> Arc<RecordingCarrier> {
        let carrier = RecordingCarrier::new();
        let (outgoing, queued) = mpsc::unbounded_channel();
        for message in messages {
            outgoing.send(Queued::Frame(message)).unwrap();
        }
        drop(outgoing);
        write_loop(queued, carrier.clone()).await;
        carrier
    }

    #[tokio::test]
    async fn frames_that_are_ready_together_travel_in_one_message() {
        let size = TermSize::new(100, 30).unwrap();
        let messages = vec![
            data(1, 10),
            Message::new(1, Body::Resized(size)),
            data(2, 2000),
            Message::new(1, Body::Exit(ExitStatus::Code(0))),
        ];
        let carrier = written(messages.clone()).await;
        assert_eq!(carrier.batches().len(), 1);
        assert_eq!(carrier.messages(), messages);
    }

    #[tokio::test]
    async fn a_message_stays_within_the_limit_and_never_splits_a_frame() {
        let messages: Vec<Message> = (0..20).map(|_| data(1, 60_000)).collect();
        let carrier = written(messages.clone()).await;
        let batches = carrier.batches();
        assert!(batches.len() > 1);
        for batch in &batches {
            assert!(batch.len() <= CARRIER_BATCH_MAX, "{} bytes", batch.len());
            // Each message holds whole frames, so each decodes on its own.
            let mut decoder = FrameDecoder::new(Role::Client);
            decoder.push(batch);
            while decoder.next().unwrap().is_some() {}
        }
        assert_eq!(carrier.messages(), messages);
    }

    #[tokio::test]
    async fn writing_stops_when_the_carrier_is_closed() {
        let carrier = RecordingCarrier::new();
        carrier.close();
        let (outgoing, queued) = mpsc::unbounded_channel();
        outgoing.send(Queued::Frame(data(1, 1))).unwrap();
        // The sender is kept, so only the closed carrier can end the loop.
        write_loop(queued, carrier.clone()).await;
        assert!(carrier.batches().is_empty());
        drop(outgoing);
    }

    fn client_frames(messages: &[Message]) -> Vec<u8> {
        let mut out = Vec::new();
        for message in messages {
            encode(message, &mut out).unwrap();
        }
        out
    }

    fn link(carrier: Arc<RecordingCarrier>) -> Arc<Link> {
        Link::open(
            7,
            carrier,
            &Handle::current(),
            Arc::new(|_, _| {}),
            Arc::new(Marks::start()),
            SessionStates::new(),
        )
    }

    async fn asked(carrier: &RecordingCarrier, count: usize) -> Vec<serde_json::Value> {
        for _ in 0..200 {
            let requests = carrier.requests();
            if requests.len() >= count {
                return requests;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("the page was not asked");
    }

    #[tokio::test]
    async fn a_request_reaches_the_page_and_its_answer_comes_back() {
        let carrier = RecordingCarrier::new();
        let link = link(carrier.clone());
        let asking = {
            let link = Arc::clone(&link);
            tokio::spawn(async move {
                link.ask(
                    PageRequestBody::Screen {
                        pane: None,
                        session: None,
                    },
                    Duration::from_secs(20),
                )
                .await
            })
        };
        let requests = asked(&carrier, 1).await;
        assert_eq!(
            requests[0],
            serde_json::json!({"request": {"id": 1, "body": {"type": "screen"}}})
        );

        // An answer to a request that was never made changes nothing.
        link.answer(99, Ok(AppReplyBody::Killed {}));
        let size = TermSize::new(90, 20).unwrap();
        link.answer(1, Ok(AppReplyBody::Resized { size }));
        assert_eq!(asking.await.unwrap(), Ok(AppReplyBody::Resized { size }));
    }

    #[tokio::test]
    async fn a_page_that_does_not_answer_is_given_up_on() {
        let carrier = RecordingCarrier::new();
        let link = link(carrier.clone());
        let answer = link
            .ask(
                PageRequestBody::Screen {
                    pane: None,
                    session: None,
                },
                Duration::from_millis(30),
            )
            .await;
        assert_eq!(answer.unwrap_err().code, codes::TIMEOUT);
        // What arrives afterwards is too late and is dropped.
        link.answer(1, Ok(AppReplyBody::Killed {}));
    }

    #[tokio::test]
    async fn retiring_a_link_ends_the_wait_for_its_page() {
        let carrier = RecordingCarrier::new();
        let link = link(carrier.clone());
        let asking = {
            let link = Arc::clone(&link);
            tokio::spawn(async move {
                link.ask(
                    PageRequestBody::Screen {
                        pane: None,
                        session: None,
                    },
                    Duration::from_secs(20),
                )
                .await
            })
        };
        asked(&carrier, 1).await;
        link.retire().await;
        assert_eq!(asking.await.unwrap().unwrap_err().code, codes::NO_WINDOW);

        let answer = link
            .ask(
                PageRequestBody::Screen {
                    pane: None,
                    session: None,
                },
                Duration::from_secs(20),
            )
            .await;
        assert_eq!(answer.unwrap_err().code, codes::NO_WINDOW);
    }

    #[tokio::test]
    async fn notices_reach_the_page_in_the_order_they_were_sent() {
        let carrier = RecordingCarrier::new();
        let link = link(carrier.clone());
        for number in 0..3 {
            link.notify(&serde_json::json!({"type": "notice", "number": number}));
        }
        let received = asked(&carrier, 3).await;
        let numbers: Vec<u64> = received
            .iter()
            .map(|message| message["notify"]["number"].as_u64().unwrap())
            .collect();
        assert_eq!(numbers, vec![0, 1, 2]);

        // A page that has gone is told nothing more.
        link.retire().await;
        link.notify(&serde_json::json!({"type": "notice", "number": 3}));
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(carrier.requests().len(), 3);
    }

    #[test]
    fn whole_frames_from_a_view_are_read_in_order() {
        let size = TermSize::new(80, 24).unwrap();
        let messages = vec![
            Message::new(3, Body::Credit(1000)),
            Message::new(3, Body::Input(b"ls\r".to_vec())),
            Message::new(
                3,
                Body::Resize {
                    size,
                    cell_px: CellPx { w: 8, h: 16 },
                },
            ),
            Message::new(3, Body::Detach),
        ];
        assert_eq!(
            decode_client_frames(&client_frames(&messages)).unwrap(),
            messages
        );
        assert_eq!(decode_client_frames(&[]).unwrap(), vec![]);
    }

    #[test]
    fn a_frame_cut_short_is_refused() {
        let frames = client_frames(&[Message::new(3, Body::Input(b"hello".to_vec()))]);
        for cut in 1..frames.len() {
            let error = decode_client_frames(&frames[..cut]).unwrap_err();
            assert_eq!(error.code, "bad_request", "cut at {cut}");
        }
    }

    #[test]
    fn frames_a_view_may_not_send_are_refused() {
        // Output travels the other way.
        let mut from_host = Vec::new();
        encode(&data(3, 1), &mut from_host).unwrap();
        assert_eq!(
            decode_client_frames(&from_host).unwrap_err().code,
            "bad_request"
        );

        // Control messages belong to commands, whole or split.
        for kind in [Kind::Control as u8, Kind::Control as u8 | Kind::MORE] {
            let mut control = Vec::new();
            control.extend_from_slice(&0u32.to_le_bytes());
            control.push(kind);
            control.extend_from_slice(&2u32.to_le_bytes());
            control.extend_from_slice(b"{}");
            assert_eq!(
                decode_client_frames(&control).unwrap_err().code,
                "bad_request"
            );
        }
    }
}
