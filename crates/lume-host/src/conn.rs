//! One client connection: reading frames, answering requests, and writing everything
//! that goes back.
//!
//! A connection has one writer. Sessions and the request handler hand it finished
//! frames through a bounded queue, so frames of different streams are never interleaved
//! mid-frame. A connection that is slow to take bytes holds back what goes to it; a
//! session with clients on other connections goes on serving them meanwhile.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lume_proto::app::ScreenCursor;
use lume_proto::tuning::MAX_PAYLOAD;
use lume_proto::{
    Body, CONTROL_STREAM, ClientFacts, ClientKind, Control, ErrorCode, FrameDecoder, Message,
    Notification, PROTO_VERSION, ProtoError, Request, RequestBody, Response, ResponseBody, Role,
    SessionId, encode, negotiate,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

use crate::screen::{HydrateWith, vt_version};
use crate::session::{AttachmentId, Command, SessionHandle};
use crate::tuning::{ABANDON_WAIT, OUTBOX_PIECES, READ_CHUNK, WAIT_TEXT_MAX};
use crate::{Host, ServeError};

/// Why frames could not be queued at once.
pub(crate) enum Behind {
    /// The connection is behind; here are the frames back.
    Full(Vec<u8>),
    /// The connection has gone.
    Gone,
}

enum Out {
    /// Whole frames, to be written as they are.
    Frames(Vec<u8>),
    /// Write nothing more and close.
    Close,
}

/// The way to send to a connection. Cheap to clone; every session attached to or
/// started by the connection holds one.
#[derive(Clone)]
pub(crate) struct Outbox {
    queue: mpsc::Sender<Out>,
}

impl Outbox {
    /// Queues whole frames. Waits while the connection is behind. `false` means the
    /// connection has gone.
    pub(crate) async fn frames(&self, frames: Vec<u8>) -> bool {
        self.queue.send(Out::Frames(frames)).await.is_ok()
    }

    /// Queues whole frames if there is room now. When there is not, the frames come
    /// back, to be sent with [`Outbox::frames`] by something that can wait.
    pub(crate) fn try_frames(&self, frames: Vec<u8>) -> Result<(), Behind> {
        match self.queue.try_send(Out::Frames(frames)) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(Out::Frames(frames))) => Err(Behind::Full(frames)),
            Err(_) => Err(Behind::Gone),
        }
    }

    async fn control(&self, control: Control) -> bool {
        let mut frames = Vec::new();
        match encode(&Message::control(control), &mut frames) {
            Ok(()) => self.frames(frames).await,
            Err(error) => {
                tracing::error!(%error, "a control message could not be encoded");
                false
            }
        }
    }

    pub(crate) async fn respond(&self, id: u64, result: Result<ResponseBody, ProtoError>) -> bool {
        self.control(Control::Response(Response { id, result }))
            .await
    }

    pub(crate) async fn notify(&self, notification: Notification) -> bool {
        self.control(Control::Notify(notification)).await
    }

    /// Tells the client why, then closes the connection.
    pub(crate) async fn fatal(&self, code: ErrorCode, message: impl Into<String>) {
        self.notify(Notification::Fatal {
            code,
            message: message.into(),
        })
        .await;
        self.close().await;
    }

    pub(crate) async fn close(&self) {
        let _ = self.queue.send(Out::Close).await;
    }
}

pub(crate) async fn serve<S>(host: Arc<Host>, io: S) -> Result<(), ServeError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (mut reader, writer) = tokio::io::split(io);
    let (queue, queued) = mpsc::channel(OUTBOX_PIECES);
    let outbox = Outbox { queue };
    let writing = tokio::spawn(write_loop(writer, queued));
    let serving = Arc::clone(&host);

    let mut connection = Connection {
        host,
        outbox: outbox.clone(),
        decoder: FrameDecoder::new(Role::Host),
        greeted: false,
        counted: false,
        hydrate: HydrateWith::Replay,
        streams: HashMap::new(),
        used_streams: HashSet::new(),
        started: Vec::new(),
    };
    // A host that is asked to stop closes every connection.
    let mut stopping = serving.stopping();
    // A connection that closes before its hello is answered was never counted; one
    // that says hello as a client that keeps the host busy is counted until it closes.
    let result = tokio::select! {
        result = connection.read_loop(&mut reader) => result,
        _ = stopping.wait_for(|stopping| *stopping) => Ok(()),
    };
    connection.finish().await;
    outbox.close().await;
    let _ = writing.await;
    if connection.counted {
        serving.client_went();
    }
    result
}

async fn write_loop<W: AsyncWrite + Unpin>(mut writer: W, mut queued: mpsc::Receiver<Out>) {
    while let Some(Out::Frames(frames)) = queued.recv().await {
        if writer.write_all(&frames).await.is_err() {
            break;
        }
        // Flush once nothing more is waiting, not after every piece.
        if queued.is_empty() && writer.flush().await.is_err() {
            break;
        }
    }
    let _ = writer.shutdown().await;
    // Dropping the queue here makes every later send fail, which is how sessions learn
    // that the connection has gone.
}

/// A stream the client has attached to a session.
struct Stream {
    session: SessionHandle,
    attachment: AttachmentId,
}

struct Connection {
    host: Arc<Host>,
    outbox: Outbox,
    decoder: FrameDecoder,
    /// Whether the client has said hello and a version was agreed.
    greeted: bool,
    /// Whether the client is one of those that keep the host from being idle: any but
    /// a command-line tool asking a question.
    counted: bool,
    /// How the client's attachments start: from a snapshot when its terminal engine is
    /// the host's own version, from replayed output otherwise.
    hydrate: HydrateWith,
    streams: HashMap<u32, Stream>,
    /// Every stream id the client has used. An id is never used twice, so a frame for
    /// an old stream cannot be taken for one of a new stream.
    used_streams: HashSet<u32>,
    /// The sessions this connection started, which end with it.
    started: Vec<SessionId>,
}

impl Connection {
    async fn read_loop<R: AsyncRead + Unpin>(&mut self, reader: &mut R) -> Result<(), ServeError> {
        let mut buf = vec![0u8; READ_CHUNK];
        loop {
            let read = match reader.read(&mut buf).await {
                Ok(0) => return Ok(()),
                Ok(read) => read,
                // A client that goes away without closing cleanly is still just gone.
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::BrokenPipe
                            | io::ErrorKind::ConnectionReset
                            | io::ErrorKind::ConnectionAborted
                            | io::ErrorKind::UnexpectedEof
                    ) =>
                {
                    return Ok(());
                }
                Err(error) => return Err(error.into()),
            };
            self.decoder.push(&buf[..read]);
            loop {
                match self.decoder.next() {
                    Ok(Some(message)) => {
                        if !self.on_message(message).await {
                            return Ok(());
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        self.outbox
                            .fatal(ErrorCode::ProtocolError, error.to_string())
                            .await;
                        return Err(error.into());
                    }
                }
            }
        }
    }

    /// The connection is over: let go of what it attached to and, unless sessions
    /// outlive their clients, end what it started.
    async fn finish(&mut self) {
        for (_, stream) in self.streams.drain() {
            stream
                .session
                .send(Command::Detach {
                    attachment: stream.attachment,
                })
                .await;
        }
        if self.host.persistent() {
            return;
        }
        let mut abandoned = Vec::new();
        for id in self.started.drain(..) {
            if let Some(session) = self.host.session(id)
                && session.send(Command::Abandon).await
            {
                abandoned.push(session);
            }
        }
        // Wait for the programs to be gone. Whoever is serving this connection may exit
        // as soon as it returns, and programs still running then would be left behind.
        let all_ended = async {
            for session in &abandoned {
                session.ended().await;
            }
        };
        if tokio::time::timeout(ABANDON_WAIT, all_ended).await.is_err() {
            tracing::warn!("sessions were still ending when their connection closed");
        }
    }

    /// Handles one message. `false` closes the connection.
    async fn on_message(&mut self, message: Message) -> bool {
        let stream = message.stream;
        match message.body {
            Body::Control(Control::Request(request)) => self.on_request(request).await,
            Body::Control(Control::Response(_) | Control::Notify(_)) => {
                self.outbox
                    .fatal(
                        ErrorCode::ProtocolError,
                        "only requests may be sent to a host",
                    )
                    .await;
                false
            }
            Body::Input(bytes) => {
                self.forward(stream, |attachment| Command::Input { attachment, bytes })
                    .await
            }
            Body::Resize { size, cell_px } => {
                self.forward(stream, |attachment| Command::Resize {
                    attachment,
                    size,
                    cell_px,
                })
                .await
            }
            Body::Facts(facts) => {
                self.forward(stream, |attachment| Command::Facts { attachment, facts })
                    .await
            }
            Body::Credit(bytes) => {
                self.forward(stream, |attachment| Command::Credit { attachment, bytes })
                    .await
            }
            Body::Rehydrate => {
                self.forward(stream, |attachment| Command::Rehydrate { attachment })
                    .await
            }
            Body::Release => {
                self.forward(stream, |attachment| Command::Release { attachment })
                    .await
            }
            Body::Detach => {
                if let Some(stream) = self.streams.remove(&stream) {
                    stream
                        .session
                        .send(Command::Detach {
                            attachment: stream.attachment,
                        })
                        .await;
                }
                true
            }
            // The decoder only passes what a client may send, so these cannot arrive.
            Body::Hydrate(_)
            | Body::Data(_)
            | Body::Resized(_)
            | Body::Event(_)
            | Body::Exit(_) => true,
        }
    }

    /// Passes a data-plane frame to the session its stream is attached to. A frame for
    /// a stream that is not open is ignored: it was sent before the client learnt the
    /// stream had closed.
    async fn forward(
        &mut self,
        stream: u32,
        command: impl FnOnce(AttachmentId) -> Command,
    ) -> bool {
        if let Some(open) = self.streams.get(&stream)
            && !open.session.send(command(open.attachment)).await
        {
            // The session has ended.
            self.streams.remove(&stream);
        }
        true
    }

    async fn on_request(&mut self, request: Request) -> bool {
        let Request { id, body } = request;
        let error = |code, message: &str| Err(ProtoError::new(code, message));

        if !self.greeted {
            let RequestBody::Hello {
                proto,
                proto_min,
                client_kind,
                vt_version: client_vt,
            } = body
            else {
                let refusal = error(ErrorCode::HelloRequired, "say hello before anything else");
                return self.outbox.respond(id, refusal).await;
            };
            let Some(version) = negotiate(PROTO_VERSION, proto, proto_min) else {
                let message = format!(
                    "the host speaks protocol version {PROTO_VERSION}; the client speaks {proto_min} to {proto}"
                );
                self.outbox
                    .respond(
                        id,
                        Err(ProtoError::new(ErrorCode::VersionMismatch, message)),
                    )
                    .await;
                self.outbox.close().await;
                return false;
            };
            self.greeted = true;
            if client_kind != ClientKind::Ctl {
                self.counted = true;
                self.host.client_came();
            }
            if client_vt.as_deref() == Some(vt_version()) {
                self.hydrate = HydrateWith::Snapshot;
            }
            tracing::debug!(?client_kind, version, ?client_vt, "client connected");
            let ack = ResponseBody::HelloAck {
                proto: version,
                host_version: env!("CARGO_PKG_VERSION").to_owned(),
                vt_version: Some(vt_version().to_owned()),
                caps: self.host.caps(),
                max_payload: MAX_PAYLOAD as u32,
                pid: std::process::id(),
            };
            return self.outbox.respond(id, Ok(ack)).await;
        }

        let result = match body {
            RequestBody::Hello { .. } => error(ErrorCode::BadRequest, "hello was already said"),
            RequestBody::Spawn { spec } => match self.host.spawn(spec, self.outbox.clone()).await {
                Ok(session) => {
                    self.started.push(session);
                    Ok(ResponseBody::Spawned { session })
                }
                Err(error) => Err(error),
            },
            RequestBody::List {} => Ok(ResponseBody::Sessions {
                sessions: self.host.list(),
            }),
            RequestBody::Attach {
                session,
                stream,
                facts,
                replay,
            } => {
                let hydrate = if replay {
                    HydrateWith::Replay
                } else {
                    self.hydrate
                };
                match self.attach(id, session, stream, facts, hydrate).await {
                    // The session has answered the request itself.
                    None => return true,
                    Some(refusal) => Err(refusal),
                }
            }
            RequestBody::Kill { session } => match self.host.session(session) {
                Some(handle) if handle.send(Command::Kill).await => Ok(ResponseBody::Killed {}),
                _ => error(ErrorCode::NotFound, "no such session"),
            },
            RequestBody::Send { session, text } => {
                let bytes = text.into_bytes();
                let len = bytes.len() as u64;
                match self.host.session(session) {
                    Some(handle) if handle.send(Command::Write { bytes }).await => {
                        Ok(ResponseBody::Sent { bytes: len })
                    }
                    _ => error(ErrorCode::NotFound, "no such session"),
                }
            }
            RequestBody::Screen { session } => self.screen(session).await,
            RequestBody::HostStatus {} => Ok(self.host.status()),
            RequestBody::Shutdown { end_sessions } => {
                match self.shutdown(end_sessions).await {
                    Ok(ended) => {
                        let stopping = ResponseBody::Stopping { ended };
                        let told = self.outbox.respond(id, Ok(stopping)).await;
                        // Told first: the connection closes as soon as the host stops.
                        self.host.stop();
                        return told;
                    }
                    Err(refusal) => Err(refusal),
                }
            }
            RequestBody::WaitText {
                session,
                text,
                timeout_ms,
            } => {
                // The answer may be a while coming; other requests go on meanwhile.
                let Some(handle) = self.host.session(session) else {
                    return self
                        .outbox
                        .respond(id, error(ErrorCode::NotFound, "no such session"))
                        .await;
                };
                let outbox = self.outbox.clone();
                let wait = Duration::from_millis(timeout_ms).min(WAIT_TEXT_MAX);
                tokio::spawn(async move {
                    let result = wait_text(&handle, text, wait).await;
                    outbox.respond(id, result).await;
                });
                return true;
            }
        };
        self.outbox.respond(id, result).await
    }

    /// Ends the sessions, if asked to and if the host may stop at all. Returns how many
    /// were ended.
    async fn shutdown(&self, end_sessions: bool) -> Result<u32, ProtoError> {
        if !self.host.persistent() {
            return Err(ProtoError::new(
                ErrorCode::Unsupported,
                "this host stops with its client",
            ));
        }
        let running = self
            .host
            .list()
            .iter()
            .filter(|info| info.exit.is_none())
            .count();
        if running > 0 && !end_sessions {
            return Err(ProtoError::new(
                ErrorCode::BadRequest,
                format!(
                    "{running} session{} still running; ask again to end them",
                    if running == 1 { " is" } else { "s are" }
                ),
            ));
        }
        tracing::info!(sessions = running, "the host was asked to stop");
        Ok(self.host.end_all().await)
    }

    async fn screen(&self, session: SessionId) -> Result<ResponseBody, ProtoError> {
        let not_found = || ProtoError::new(ErrorCode::NotFound, "no such session");
        let handle = self.host.session(session).ok_or_else(not_found)?;
        let (reply, answer) = oneshot::channel();
        if !handle.send(Command::Screen { reply }).await {
            return Err(not_found());
        }
        let text = answer
            .await
            .map_err(|_| not_found())?
            .ok_or_else(|| ProtoError::new(ErrorCode::Internal, "the screen could not be read"))?;
        let (col, row, visible) = text.cursor;
        Ok(ResponseBody::Screen {
            size: text.size,
            cursor: ScreenCursor { col, row, visible },
            alternate: text.alternate,
            lines: text.lines,
        })
    }

    /// Opens `stream` as an attachment to `session`. Returns the reason if the request
    /// is refused here; `None` means the session took the request and answered it.
    async fn attach(
        &mut self,
        request: u64,
        session: SessionId,
        stream: u32,
        facts: ClientFacts,
        hydrate: HydrateWith,
    ) -> Option<ProtoError> {
        if stream == CONTROL_STREAM {
            return Some(ProtoError::new(
                ErrorCode::InvalidStream,
                "stream 0 carries the control plane",
            ));
        }
        if self.used_streams.contains(&stream) {
            return Some(ProtoError::new(
                ErrorCode::StreamInUse,
                format!("stream {stream} was already used on this connection"),
            ));
        }
        let not_found = || ProtoError::new(ErrorCode::NotFound, "no such session");
        let Some(handle) = self.host.session(session) else {
            return Some(not_found());
        };

        let (reply, answered) = oneshot::channel();
        let command = Command::Attach {
            request,
            stream,
            facts,
            hydrate,
            outbox: self.outbox.clone(),
            reply,
        };
        if !handle.send(command).await {
            return Some(not_found());
        }
        match answered.await {
            Ok(Some(attachment)) => {
                self.used_streams.insert(stream);
                self.streams.insert(
                    stream,
                    Stream {
                        session: handle,
                        attachment,
                    },
                );
                None
            }
            // The session refused and has said so.
            Ok(None) => None,
            // The session ended before it could answer.
            Err(_) => Some(not_found()),
        }
    }
}

/// Waits up to `wait` for `text` to be on a session's screen.
async fn wait_text(
    handle: &SessionHandle,
    text: String,
    wait: Duration,
) -> Result<ResponseBody, ProtoError> {
    let gone = || ProtoError::new(ErrorCode::NotFound, "the session has ended");
    let (reply, answer) = oneshot::channel();
    let command = Command::WaitText {
        text,
        deadline: Instant::now() + wait,
        reply,
    };
    if !handle.send(command).await {
        return Err(gone());
    }
    match answer.await.map_err(|_| gone())? {
        Some(found) => Ok(ResponseBody::Matched {
            row: found.row,
            line: found.line,
        }),
        None => Err(ProtoError::new(
            ErrorCode::Timeout,
            "the text did not appear on the screen in time",
        )),
    }
}
