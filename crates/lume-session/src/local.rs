//! The backend for sessions on this machine: a client of the session host.
//!
//! The backend holds one connection to the host and speaks the wire protocol over it.
//! Requests go out on the control stream and are matched to their responses by id; each
//! attachment is a stream of its own. Two tasks do the I/O: one writes whatever is
//! queued, one reads and hands each frame to the request or attachment it belongs to.
//!
//! The host is either a task in this process, which ends with it, or a process of its
//! own that the backend starts when nothing serves its endpoint. A connection to the
//! latter that is lost is made again on the next request; what was attached through
//! the lost one has ended.
//!
//! The backend speaks the protocol's current version and the one before, so that an
//! application can use a host that an older application left running. What the older
//! version does not have is refused here, as a host that does not offer it would.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::backend::{ScreenText, TextFound};

use async_trait::async_trait;
use lume_host::{Host, HostConfig};
use lume_proto::{
    BackendCaps, Body, BytesKind, CellPx, ClientFacts, ClientKind, Control, ErrorCode,
    FrameDecoder, HydrateMode, HydratePayload, Message, Notification, PROTO_VERSION,
    PROTO_VERSION_MIN, ProtoError, ReceiveWindow, Request, RequestBody, Response, ResponseBody,
    Role, SessionId, SessionInfo, SpawnSpec, TermSize, encode, encode_bytes,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{AbortHandle, JoinHandle};

use crate::backend::Sink;
use crate::daemon::Daemon;
use crate::{
    Attachment, FrameStream, Hydrate, InputSink, OutputFrame, Result, SessionBackend, SessionError,
};

/// The first version of the protocol with the requests that read the screen, type into
/// a session, say what the host is doing and stop it, and the frames that start an
/// attachment again from the screen and let go of it.
const SCREEN_VERSION: u32 = 2;

/// The size of the in-memory pipe between the backend and a host in the same process.
const IN_PROCESS_PIPE_BYTES: usize = 256 * 1024;

/// How many pieces may wait to be written to the host. Input and credit are small and
/// the host reads them promptly, so this is rarely more than one deep.
const OUTGOING_PIECES: usize = 64;

/// The size of one read from the connection.
const READ_CHUNK: usize = 64 * 1024;

/// How long [`LocalBackend::shutdown`] waits for a host in this process to end its
/// sessions, and [`LocalBackend::restart_host`] for a host on its own to stop. A
/// program gets a few seconds to end by itself before the host forces it.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(8);

/// How often a connection that is closing, or an endpoint that is coming back, is
/// looked at.
const CLOSE_POLL: Duration = Duration::from_millis(20);

/// The backend for sessions on this machine.
pub struct LocalBackend {
    current: Mutex<Current>,
    /// Held while the connection is being made again, so that it is made once.
    reconnecting: tokio::sync::Mutex<()>,
    /// How to reach the host again once the connection is lost. A host in this process
    /// cannot be reached again: it ended with its connection.
    daemon: Option<Daemon>,
    /// The host, when it runs in this process.
    host: Mutex<Option<JoinHandle<()>>>,
}

/// The connection in use and what the host said about itself on it.
struct Current {
    connection: Arc<Connection>,
    hello: HostHello,
}

/// What a host says about itself when a connection is made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostHello {
    /// The protocol version the two sides agreed on.
    pub proto: u32,
    /// The host's own version.
    pub version: String,
    /// The host's process.
    pub pid: u32,
    pub caps: BackendCaps,
}

/// What a host that runs on its own is doing, as it says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostStatus {
    pub version: String,
    pub pid: u32,
    pub started_unix_ms: u64,
    pub persistent: bool,
    pub sessions: u32,
    pub clients: u32,
    pub idle_exit: Option<Duration>,
}

type Answer = std::result::Result<ResponseBody, ProtoError>;

struct Connection {
    state: Mutex<State>,
    next_request: AtomicU64,
    /// Stream ids are handed out in order and never reused on a connection.
    next_stream: AtomicU32,
    /// The protocol version agreed on, once hello has been answered.
    proto: AtomicU32,
}

struct State {
    /// The way to the writing task. `None` once the connection is closed.
    outgoing: Option<mpsc::Sender<Vec<u8>>>,
    /// The reading task, to be stopped when the connection is closed from this side.
    reading: Option<AbortHandle>,
    /// Requests waiting for their response.
    waiting: HashMap<u64, oneshot::Sender<Answer>>,
    streams: HashMap<u32, Stream>,
}

/// The receiving side of one attachment.
struct Stream {
    frames: mpsc::UnboundedSender<OutputFrame>,
    /// The credit granted to the host and not yet used. The host must stay within it.
    window: ReceiveWindow,
    /// Where the state to start from goes, while the attachment is waiting for it.
    hydrate: Option<oneshot::Sender<HydratePayload>>,
}

impl Connection {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn is_open(&self) -> bool {
        self.state().outgoing.is_some()
    }

    /// Whether the version agreed on has what `version` brought.
    fn speaks(&self, version: u32) -> bool {
        self.proto.load(Ordering::Relaxed) >= version
    }

    /// Closes the connection: nothing more is sent, every waiting request fails and
    /// every attachment's stream ends.
    fn close(&self) {
        let mut state = self.state();
        // Dropping the last sender ends the writing task, and stopping the reading task
        // lets go of the other half. Both must go: not every transport can close one
        // direction alone, and on those the host sees the connection end only when
        // nothing on this side holds it any more.
        state.outgoing = None;
        if let Some(reading) = state.reading.take() {
            reading.abort();
        }
        state.waiting.clear();
        state.streams.clear();
    }

    async fn send(&self, frames: Vec<u8>) -> Result<()> {
        let outgoing = self
            .state()
            .outgoing
            .clone()
            .ok_or(SessionError::Disconnected)?;
        outgoing
            .send(frames)
            .await
            .map_err(|_| SessionError::Disconnected)
    }

    async fn send_message(&self, message: &Message) -> Result<()> {
        let mut frames = Vec::new();
        encode(message, &mut frames).map_err(|error| SessionError::Protocol(error.to_string()))?;
        self.send(frames).await
    }

    async fn request(&self, body: RequestBody) -> Result<ResponseBody> {
        let id = self.next_request.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = oneshot::channel();
        {
            let mut state = self.state();
            if state.outgoing.is_none() {
                return Err(SessionError::Disconnected);
            }
            state.waiting.insert(id, answer);
        }
        let request = Message::control(Control::Request(Request { id, body }));
        if let Err(error) = self.send_message(&request).await {
            self.state().waiting.remove(&id);
            return Err(error);
        }
        match answered.await {
            Ok(answer) => answer.map_err(SessionError::Refused),
            // The connection closed before the response arrived.
            Err(_) => Err(SessionError::Disconnected),
        }
    }

    /// Hands a received message to whatever is waiting for it. An error means the host
    /// broke the protocol and the connection must be closed.
    fn dispatch(&self, message: Message) -> std::result::Result<(), String> {
        let mut state = self.state();
        let stream = message.stream;
        match message.body {
            Body::Control(Control::Response(Response { id, result })) => {
                if let Some(answer) = state.waiting.remove(&id) {
                    let _ = answer.send(result);
                }
            }
            Body::Control(Control::Notify(Notification::Fatal { code, message })) => {
                return Err(format!(
                    "the host is closing the connection: {code}: {message}"
                ));
            }
            // Each attachment learns of its session's end from its own stream.
            Body::Control(Control::Notify(Notification::SessionExited { .. })) => {}
            Body::Control(Control::Request(_)) => {
                return Err("the host sent a request".to_owned());
            }
            Body::Data(bytes) => {
                // Output for a stream that has been detached was sent before the host
                // knew, and is dropped.
                if let Some(open) = state.streams.get_mut(&stream) {
                    open.window
                        .received(bytes.len())
                        .map_err(|error| error.to_string())?;
                    let _ = open.frames.send(OutputFrame::Data(bytes));
                }
            }
            Body::Resized(size) => forward(&state, stream, OutputFrame::Resized(size)),
            Body::Event(event) => forward(&state, stream, OutputFrame::Event(event)),
            Body::Exit(status) => forward(&state, stream, OutputFrame::Exit(status)),
            Body::Hydrate(payload) => {
                let Some(open) = state.streams.get_mut(&stream) else {
                    return Ok(());
                };
                match open.hydrate.take() {
                    // The state an attachment starts from.
                    Some(waiting) => {
                        let _ = waiting.send(payload);
                    }
                    // The state to start again from, asked for later.
                    None => {
                        let _ = open.frames.send(OutputFrame::Hydrate(hydrate_of(payload)));
                    }
                }
            }
            // The decoder only passes what a host may send, so these cannot arrive.
            Body::Input(_)
            | Body::Resize { .. }
            | Body::Facts(_)
            | Body::Credit(_)
            | Body::Detach
            | Body::Rehydrate
            | Body::Release => {}
        }
        Ok(())
    }
}

fn hydrate_of(payload: HydratePayload) -> Hydrate {
    match payload {
        HydratePayload::Snapshot { vt_version, bytes } => Hydrate::Snapshot { vt_version, bytes },
        HydratePayload::VtReplay(bytes) => Hydrate::VtReplay(bytes),
    }
}

fn forward(state: &State, stream: u32, frame: OutputFrame) {
    if let Some(open) = state.streams.get(&stream) {
        let _ = open.frames.send(frame);
    }
}

async fn write_loop<W: AsyncWrite + Unpin>(mut writer: W, mut queued: mpsc::Receiver<Vec<u8>>) {
    while let Some(frames) = queued.recv().await {
        if writer.write_all(&frames).await.is_err() {
            break;
        }
        if queued.is_empty() && writer.flush().await.is_err() {
            break;
        }
    }
    let _ = writer.shutdown().await;
}

async fn read_loop<R: AsyncRead + Unpin>(mut reader: R, connection: Arc<Connection>) {
    let mut decoder = FrameDecoder::new(Role::Client);
    let mut buf = vec![0u8; READ_CHUNK];
    'connection: loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(read) => decoder.push(&buf[..read]),
        }
        loop {
            let broken = match decoder.next() {
                Ok(Some(message)) => connection.dispatch(message).err(),
                Ok(None) => break,
                Err(error) => Some(error.to_string()),
            };
            if let Some(reason) = broken {
                tracing::error!(reason, "closing the connection to the session host");
                break 'connection;
            }
        }
    }
    connection.close();
}

/// Opens a connection over `io` and agrees a protocol version with the host.
async fn handshake<S>(io: S) -> Result<(Arc<Connection>, HostHello)>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (reader, writer) = tokio::io::split(io);
    let (outgoing, queued) = mpsc::channel(OUTGOING_PIECES);
    let connection = Arc::new(Connection {
        state: Mutex::new(State {
            outgoing: Some(outgoing),
            reading: None,
            waiting: HashMap::new(),
            streams: HashMap::new(),
        }),
        next_request: AtomicU64::new(1),
        next_stream: AtomicU32::new(1),
        proto: AtomicU32::new(0),
    });
    tokio::spawn(write_loop(writer, queued));
    let reading = tokio::spawn(read_loop(reader, Arc::clone(&connection)));
    connection.state().reading = Some(reading.abort_handle());

    // The views run the same engine as the host, built from the same source, so they
    // can start from the host's snapshots.
    let hello = RequestBody::Hello {
        proto: PROTO_VERSION,
        proto_min: PROTO_VERSION_MIN,
        vt_version: Some(lume_host::vt_version().to_owned()),
        client_kind: ClientKind::App,
    };
    let answer = connection.request(hello).await;
    let hello = match answer {
        Ok(ResponseBody::HelloAck {
            proto,
            caps,
            host_version,
            pid,
            ..
        }) if (PROTO_VERSION_MIN..=PROTO_VERSION).contains(&proto) => HostHello {
            proto,
            version: host_version,
            pid,
            caps,
        },
        Ok(ResponseBody::HelloAck { proto, .. }) => {
            connection.close();
            return Err(SessionError::Protocol(format!(
                "the host chose protocol version {proto}, which was not offered"
            )));
        }
        Ok(_) => {
            connection.close();
            return Err(SessionError::Protocol(
                "hello was answered with something else".into(),
            ));
        }
        Err(error) => {
            connection.close();
            return Err(error);
        }
    };
    connection.proto.store(hello.proto, Ordering::Relaxed);
    if hello.proto < PROTO_VERSION {
        tracing::info!(
            proto = hello.proto,
            host_version = hello.version,
            "the session host speaks an older version of the protocol"
        );
    }
    Ok((connection, hello))
}

/// A refusal for what the version agreed on does not have.
fn too_old(what: &str) -> SessionError {
    SessionError::Refused(ProtoError::new(
        ErrorCode::Unsupported,
        format!("the session host is too old to {what}"),
    ))
}

/// Waits for `connection` to close, for `limit` at most.
async fn until_closed(connection: &Connection, limit: Duration) -> bool {
    let deadline = std::time::Instant::now() + limit;
    while connection.is_open() {
        if std::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(CLOSE_POLL).await;
    }
    true
}

impl LocalBackend {
    /// Connects to a host over `io` and agrees a protocol version with it.
    pub async fn connect<S>(io: S) -> Result<Self>
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let (connection, hello) = handshake(io).await?;
        Ok(Self::with(connection, hello, None))
    }

    fn with(connection: Arc<Connection>, hello: HostHello, daemon: Option<Daemon>) -> Self {
        Self {
            current: Mutex::new(Current { connection, hello }),
            reconnecting: tokio::sync::Mutex::new(()),
            daemon,
            host: Mutex::new(None),
        }
    }

    /// Connects to the host that runs on its own, starting it if nothing serves its
    /// endpoint. A connection that is lost later is made again, and the host started
    /// again if it has gone, when the backend is next used.
    pub async fn daemon(daemon: Daemon) -> Result<Self> {
        let (connection, hello) = Self::reach(&daemon).await?;
        Ok(Self::with(connection, hello, Some(daemon)))
    }

    async fn reach(daemon: &Daemon) -> Result<(Arc<Connection>, HostHello)> {
        let (stream, started) = daemon.connect().await.map_err(|error| {
            SessionError::Unreachable(format!(
                "the session host on `{}` cannot be reached: {error}",
                daemon.endpoint
            ))
        })?;
        let (connection, hello) = handshake(stream).await?;
        tracing::info!(
            pid = hello.pid,
            version = hello.version,
            started,
            "connected to the session host"
        );
        Ok((connection, hello))
    }

    /// Starts a host inside this process and connects to it over an in-memory pipe.
    /// The host is reached through the protocol all the same; nothing is shared.
    pub async fn in_process(config: HostConfig) -> Result<Self> {
        let (client_end, host_end) = tokio::io::duplex(IN_PROCESS_PIPE_BYTES);
        let host = Host::new(config);
        let serving = tokio::spawn(async move {
            if let Err(error) = host.serve(host_end).await {
                tracing::warn!(%error, "the session host stopped serving");
            }
        });
        let backend = Self::connect(client_end).await?;
        *backend.host.lock().unwrap_or_else(PoisonError::into_inner) = Some(serving);
        Ok(backend)
    }

    fn current(&self) -> MutexGuard<'_, Current> {
        self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What the host said about itself on the connection in use.
    pub fn hello(&self) -> HostHello {
        self.current().hello.clone()
    }

    /// Whether the host runs on its own, as opposed to inside this process.
    pub fn is_daemon(&self) -> bool {
        self.daemon.is_some()
    }

    /// The connection to use: the one there is, or, if it has been lost and the host
    /// can be reached again, a new one.
    async fn connection(&self) -> Result<Arc<Connection>> {
        let connection = Arc::clone(&self.current().connection);
        let Some(daemon) = &self.daemon else {
            return Ok(connection);
        };
        if connection.is_open() {
            return Ok(connection);
        }
        let _one_at_a_time = self.reconnecting.lock().await;
        let connection = Arc::clone(&self.current().connection);
        if connection.is_open() {
            return Ok(connection);
        }
        tracing::warn!("the connection to the session host was lost; making it again");
        let (connection, hello) = Self::reach(daemon).await?;
        *self.current() = Current {
            connection: Arc::clone(&connection),
            hello,
        };
        Ok(connection)
    }

    async fn request(&self, body: RequestBody) -> Result<ResponseBody> {
        self.connection().await?.request(body).await
    }

    /// What the host is doing, for a host that can say.
    pub async fn host_status(&self) -> Result<HostStatus> {
        let connection = self.connection().await?;
        if !connection.speaks(SCREEN_VERSION) {
            return Err(too_old("say what it is doing"));
        }
        match connection.request(RequestBody::HostStatus {}).await? {
            ResponseBody::HostStatus {
                host_version,
                pid,
                started_unix_ms,
                persistent,
                sessions,
                clients,
                idle_exit_ms,
            } => Ok(HostStatus {
                version: host_version,
                pid,
                started_unix_ms,
                persistent,
                sessions,
                clients,
                idle_exit: idle_exit_ms.map(Duration::from_millis),
            }),
            _ => Err(unexpected("host_status")),
        }
    }

    /// Opens an attachment; with `replay`, one that starts from replayed output.
    async fn attach_as(
        &self,
        id: SessionId,
        facts: ClientFacts,
        replay: bool,
    ) -> Result<Attachment> {
        let connection = &self.connection().await?;
        let stream = connection.next_stream.fetch_add(1, Ordering::Relaxed);
        let (frames, received) = mpsc::unbounded_channel();
        let (hydrate, hydrated) = oneshot::channel();
        // The stream is registered before the request goes out: the host may send
        // frames for it straight after the response, and they must find it.
        {
            let mut state = connection.state();
            if state.outgoing.is_none() {
                return Err(SessionError::Disconnected);
            }
            let open = Stream {
                frames,
                window: ReceiveWindow::new(),
                hydrate: Some(hydrate),
            };
            state.streams.insert(stream, open);
        }

        let request = RequestBody::Attach {
            session: id,
            stream,
            facts,
            replay,
        };
        let (size, mode, resize_pulls_scrollback) = match connection.request(request).await {
            Ok(ResponseBody::Attached {
                size,
                hydrate,
                resize_pulls_scrollback,
            }) => (size, hydrate, resize_pulls_scrollback),
            Ok(_) => {
                connection.state().streams.remove(&stream);
                return Err(unexpected("attach"));
            }
            Err(error) => {
                connection.state().streams.remove(&stream);
                return Err(error);
            }
        };

        let hydrate = if mode == HydrateMode::None {
            if let Some(open) = connection.state().streams.get_mut(&stream) {
                open.hydrate = None;
            }
            Hydrate::None
        } else {
            // The state to start from is the first thing the host sends on the stream.
            match hydrated.await {
                Ok(payload) => hydrate_of(payload),
                Err(_) => return Err(SessionError::Disconnected),
            }
        };

        let sink = LocalSink {
            connection: Arc::clone(connection),
            stream,
        };
        Ok(Attachment {
            size,
            resize_pulls_scrollback,
            hydrate,
            output: FrameStream::new(received),
            input: InputSink::new(Arc::new(sink)),
        })
    }

    /// Ends every session the host that runs on its own has, stops it, and starts a new
    /// one. Returns how many sessions were ended.
    pub async fn restart_host(&self) -> Result<u32> {
        let Some(daemon) = &self.daemon else {
            return Err(SessionError::Refused(ProtoError::new(
                ErrorCode::Unsupported,
                "the session host runs inside the application",
            )));
        };
        let _one_at_a_time = self.reconnecting.lock().await;
        let connection = Arc::clone(&self.current().connection);
        let ended = if !connection.is_open() {
            0
        } else if connection.speaks(SCREEN_VERSION) {
            let request = RequestBody::Shutdown { end_sessions: true };
            match connection.request(request).await? {
                ResponseBody::Stopping { ended } => ended,
                _ => return Err(unexpected("shutdown")),
            }
        } else {
            // A host that cannot be asked to stop is left to stop by itself once its
            // sessions have ended.
            return Err(too_old("be stopped"));
        };
        // The host closes the connection on its way out, and its endpoint with it.
        if !until_closed(&connection, SHUTDOWN_WAIT).await {
            tracing::warn!("the session host did not close its connection when it stopped");
        }
        connection.close();
        let (fresh, hello) = Self::reach_again(daemon).await?;
        *self.current() = Current {
            connection: fresh,
            hello,
        };
        Ok(ended)
    }

    /// Reaches a host on an endpoint that may still be closing.
    async fn reach_again(daemon: &Daemon) -> Result<(Arc<Connection>, HostHello)> {
        let deadline = std::time::Instant::now() + SHUTDOWN_WAIT;
        loop {
            match Self::reach(daemon).await {
                Ok(reached) => return Ok(reached),
                Err(error) if std::time::Instant::now() >= deadline => return Err(error),
                Err(_) => tokio::time::sleep(CLOSE_POLL).await,
            }
        }
    }

    /// Closes the connection and, when the host runs in this process, waits for it to
    /// end the sessions this backend started. A host that runs on its own keeps them.
    ///
    /// Call this before the process exits. A program in a terminal can outlive a
    /// process that exits without closing the terminal; a host in this process closes
    /// them when its connection closes, and this waits until it has.
    pub async fn shutdown(&self) {
        self.current().connection.close();
        let serving = self
            .host
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(serving) = serving
            && tokio::time::timeout(SHUTDOWN_WAIT, serving).await.is_err()
        {
            tracing::warn!("the session host was still ending sessions at shutdown");
        }
    }
}

impl Drop for LocalBackend {
    fn drop(&mut self) {
        self.current().connection.close();
    }
}

#[async_trait]
impl SessionBackend for LocalBackend {
    fn caps(&self) -> BackendCaps {
        self.current().hello.caps
    }

    async fn list(&self) -> Result<Vec<SessionInfo>> {
        match self.request(RequestBody::List {}).await? {
            ResponseBody::Sessions { sessions } => Ok(sessions),
            _ => Err(unexpected("list")),
        }
    }

    async fn spawn(&self, mut spec: SpawnSpec) -> Result<SessionId> {
        let connection = self.connection().await?;
        if !connection.speaks(SCREEN_VERSION) {
            // An older host keeps the scrollback it keeps and knows no other.
            spec.scrollback_lines = None;
        }
        match connection.request(RequestBody::Spawn { spec }).await? {
            ResponseBody::Spawned { session } => Ok(session),
            _ => Err(unexpected("spawn")),
        }
    }

    async fn attach(&self, id: SessionId, facts: ClientFacts) -> Result<Attachment> {
        self.attach_as(id, facts, false).await
    }

    async fn attach_replaying(&self, id: SessionId, facts: ClientFacts) -> Result<Attachment> {
        if !self.connection().await?.speaks(SCREEN_VERSION) {
            return Err(too_old("start an attachment from replayed output"));
        }
        self.attach_as(id, facts, true).await
    }

    async fn send(&self, id: SessionId, text: &str) -> Result<()> {
        let connection = self.connection().await?;
        if !connection.speaks(SCREEN_VERSION) {
            return Err(too_old("type into a session it is not attached to"));
        }
        let request = RequestBody::Send {
            session: id,
            text: text.to_owned(),
        };
        match connection.request(request).await? {
            ResponseBody::Sent { .. } => Ok(()),
            _ => Err(unexpected("send")),
        }
    }

    async fn kill(&self, id: SessionId) -> Result<()> {
        match self.request(RequestBody::Kill { session: id }).await? {
            ResponseBody::Killed {} => Ok(()),
            _ => Err(unexpected("kill")),
        }
    }

    async fn screen(&self, id: SessionId) -> Result<ScreenText> {
        let connection = self.connection().await?;
        if !connection.speaks(SCREEN_VERSION) {
            return Err(too_old("read a screen"));
        }
        match connection
            .request(RequestBody::Screen { session: id })
            .await?
        {
            ResponseBody::Screen {
                size,
                cursor,
                alternate,
                lines,
            } => Ok(ScreenText {
                size,
                cursor,
                alternate,
                lines,
            }),
            _ => Err(unexpected("screen")),
        }
    }

    async fn wait_text(&self, id: SessionId, text: &str, timeout: Duration) -> Result<TextFound> {
        let request = RequestBody::WaitText {
            session: id,
            text: text.to_owned(),
            timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        };
        let connection = self.connection().await?;
        if !connection.speaks(SCREEN_VERSION) {
            return Err(too_old("wait for text on a screen"));
        }
        match connection.request(request).await? {
            ResponseBody::Matched { row, line } => Ok(TextFound { row, line }),
            _ => Err(unexpected("wait_text")),
        }
    }
}

fn unexpected(request: &str) -> SessionError {
    SessionError::Protocol(format!("{request} was answered with something else"))
}

/// The sending side of one attachment.
struct LocalSink {
    connection: Arc<Connection>,
    stream: u32,
}

impl LocalSink {
    /// Fails if this attachment can no longer send.
    fn check_open(&self) -> Result<()> {
        let state = self.connection.state();
        if state.outgoing.is_none() {
            Err(SessionError::Disconnected)
        } else if !state.streams.contains_key(&self.stream) {
            Err(SessionError::Detached)
        } else {
            Ok(())
        }
    }

    async fn send(&self, body: Body) -> Result<()> {
        self.check_open()?;
        self.connection
            .send_message(&Message::new(self.stream, body))
            .await
    }
}

#[async_trait]
impl Sink for LocalSink {
    async fn write(&self, bytes: &[u8]) -> Result<()> {
        self.check_open()?;
        let mut frames = Vec::with_capacity(bytes.len() + 16);
        encode_bytes(self.stream, BytesKind::Input, bytes, &mut frames);
        self.connection.send(frames).await
    }

    async fn resize(&self, size: TermSize, cell_px: CellPx) -> Result<()> {
        self.send(Body::Resize { size, cell_px }).await
    }

    async fn facts(&self, facts: &ClientFacts) -> Result<()> {
        self.send(Body::Facts(facts.clone())).await
    }

    async fn credit(&self, bytes: u32) -> Result<()> {
        {
            let mut state = self.connection.state();
            if state.outgoing.is_none() {
                return Err(SessionError::Disconnected);
            }
            let open = state
                .streams
                .get_mut(&self.stream)
                .ok_or(SessionError::Detached)?;
            // Recorded before it is sent: the host may use it the moment it arrives.
            open.window
                .granted(bytes)
                .map_err(|_| SessionError::CreditOverflow)?;
        }
        self.connection
            .send_message(&Message::new(self.stream, Body::Credit(bytes)))
            .await
    }

    async fn rehydrate(&self) -> Result<()> {
        if !self.connection.speaks(SCREEN_VERSION) {
            return Err(too_old("send a screen again"));
        }
        self.send(Body::Rehydrate).await
    }

    async fn release(&self) -> Result<()> {
        if !self.connection.speaks(SCREEN_VERSION) {
            return Err(too_old("let go of a screen"));
        }
        self.send(Body::Release).await
    }

    async fn detach(&self) -> Result<()> {
        // Removing the stream ends the attachment's frame stream.
        let was_open = self
            .connection
            .state()
            .streams
            .remove(&self.stream)
            .is_some();
        if !was_open {
            return Err(SessionError::Detached);
        }
        self.connection
            .send_message(&Message::new(self.stream, Body::Detach))
            .await
    }
}
