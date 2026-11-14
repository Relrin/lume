//! One client connection: reading frames, answering requests, and writing everything
//! that goes back.
//!
//! A connection has one writer. Sessions and the request handler hand it finished
//! frames through a bounded queue, so frames of different streams are never interleaved
//! mid-frame, and a connection that is slow to take bytes slows the sessions feeding it.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::Arc;

use lume_proto::tuning::MAX_PAYLOAD;
use lume_proto::{
    Body, CONTROL_STREAM, ClientFacts, Control, ErrorCode, FrameDecoder, Message, Notification,
    PROTO_VERSION, ProtoError, Request, RequestBody, Response, ResponseBody, Role, SessionId,
    encode, negotiate,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

use crate::session::{AttachmentId, Command, SessionHandle};
use crate::tuning::{ABANDON_WAIT, OUTBOX_PIECES, READ_CHUNK};
use crate::{Host, ServeError};

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

    let mut connection = Connection {
        host,
        outbox: outbox.clone(),
        decoder: FrameDecoder::new(Role::Host),
        greeted: false,
        streams: HashMap::new(),
        used_streams: HashSet::new(),
        started: Vec::new(),
    };
    let result = connection.read_loop(&mut reader).await;
    connection.finish().await;
    outbox.close().await;
    let _ = writing.await;
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

    /// The connection is over: end what it started and let go of what it attached to.
    async fn finish(&mut self) {
        for (_, stream) in self.streams.drain() {
            stream
                .session
                .send(Command::Detach {
                    attachment: stream.attachment,
                })
                .await;
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
                ..
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
            tracing::debug!(?client_kind, version, "client connected");
            let ack = ResponseBody::HelloAck {
                proto: version,
                host_version: env!("CARGO_PKG_VERSION").to_owned(),
                vt_version: None,
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
            } => {
                match self.attach(id, session, stream, facts).await {
                    // The session has answered the request itself.
                    None => return true,
                    Some(refusal) => Err(refusal),
                }
            }
            RequestBody::Kill { session } => match self.host.session(session) {
                Some(handle) if handle.send(Command::Kill).await => Ok(ResponseBody::Killed {}),
                _ => error(ErrorCode::NotFound, "no such session"),
            },
        };
        self.outbox.respond(id, result).await
    }

    /// Opens `stream` as an attachment to `session`. Returns the reason if the request
    /// is refused here; `None` means the session took the request and answered it.
    async fn attach(
        &mut self,
        request: u64,
        session: SessionId,
        stream: u32,
        facts: ClientFacts,
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
