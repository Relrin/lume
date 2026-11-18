//! The backend for sessions on this machine: a client of the session host.
//!
//! The backend holds one connection to the host and speaks the wire protocol over it.
//! Requests go out on the control stream and are matched to their responses by id; each
//! attachment is a stream of its own. Two tasks do the I/O: one writes whatever is
//! queued, one reads and hands each frame to the request or attachment it belongs to.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::backend::{ScreenText, TextFound};

use async_trait::async_trait;
use lume_host::{Host, HostConfig};
use lume_proto::{
    BackendCaps, Body, BytesKind, CellPx, ClientFacts, ClientKind, Control, FrameDecoder,
    HydrateMode, HydratePayload, Message, Notification, PROTO_VERSION, ProtoError, ReceiveWindow,
    Request, RequestBody, Response, ResponseBody, Role, SessionId, SessionInfo, SpawnSpec,
    TermSize, encode, encode_bytes,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{AbortHandle, JoinHandle};

use crate::backend::Sink;
use crate::{
    Attachment, FrameStream, Hydrate, InputSink, OutputFrame, Result, SessionBackend, SessionError,
};

/// The size of the in-memory pipe between the backend and a host in the same process.
const IN_PROCESS_PIPE_BYTES: usize = 256 * 1024;

/// How many pieces may wait to be written to the host. Input and credit are small and
/// the host reads them promptly, so this is rarely more than one deep.
const OUTGOING_PIECES: usize = 64;

/// The size of one read from the connection.
const READ_CHUNK: usize = 64 * 1024;

/// How long [`LocalBackend::shutdown`] waits for a host in this process to end its
/// sessions. A program gets a few seconds to end by itself before the host forces it.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(8);

/// The backend for sessions on this machine.
pub struct LocalBackend {
    connection: Arc<Connection>,
    caps: BackendCaps,
    /// The host, when it runs in this process.
    host: Mutex<Option<JoinHandle<()>>>,
}

type Answer = std::result::Result<ResponseBody, ProtoError>;

struct Connection {
    state: Mutex<State>,
    next_request: AtomicU64,
    /// Stream ids are handed out in order and never reused on a connection.
    next_stream: AtomicU32,
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
            | Body::Rehydrate => {}
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

impl LocalBackend {
    /// Connects to a host over `io` and agrees a protocol version with it.
    pub async fn connect<S>(io: S) -> Result<Self>
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
        });
        tokio::spawn(write_loop(writer, queued));
        let reading = tokio::spawn(read_loop(reader, Arc::clone(&connection)));
        connection.state().reading = Some(reading.abort_handle());

        // The views run the same engine as the host, built from the same source, so they
        // can start from the host's snapshots.
        let hello = RequestBody::Hello {
            proto: PROTO_VERSION,
            proto_min: PROTO_VERSION,
            vt_version: Some(lume_host::vt_version().to_owned()),
            client_kind: ClientKind::App,
        };
        let caps = match connection.request(hello).await {
            Ok(ResponseBody::HelloAck { proto, caps, .. }) if proto == PROTO_VERSION => caps,
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
        Ok(Self {
            connection,
            caps,
            host: Mutex::new(None),
        })
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

    /// Closes the connection and, when the host runs in this process, waits for it to
    /// end the sessions this backend started.
    ///
    /// Call this before the process exits. A program in a terminal can outlive a
    /// process that exits without closing the terminal; the host closes them when its
    /// connection closes, and this waits until it has.
    pub async fn shutdown(&self) {
        self.connection.close();
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
        self.connection.close();
    }
}

#[async_trait]
impl SessionBackend for LocalBackend {
    fn caps(&self) -> BackendCaps {
        self.caps
    }

    async fn list(&self) -> Result<Vec<SessionInfo>> {
        match self.connection.request(RequestBody::List {}).await? {
            ResponseBody::Sessions { sessions } => Ok(sessions),
            _ => Err(unexpected("list")),
        }
    }

    async fn spawn(&self, spec: SpawnSpec) -> Result<SessionId> {
        match self.connection.request(RequestBody::Spawn { spec }).await? {
            ResponseBody::Spawned { session } => Ok(session),
            _ => Err(unexpected("spawn")),
        }
    }

    async fn attach(&self, id: SessionId, facts: ClientFacts) -> Result<Attachment> {
        let connection = &self.connection;
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

    async fn kill(&self, id: SessionId) -> Result<()> {
        match self
            .connection
            .request(RequestBody::Kill { session: id })
            .await?
        {
            ResponseBody::Killed {} => Ok(()),
            _ => Err(unexpected("kill")),
        }
    }

    async fn screen(&self, id: SessionId) -> Result<ScreenText> {
        match self
            .connection
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
        match self.connection.request(request).await? {
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
        self.send(Body::Rehydrate).await
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
