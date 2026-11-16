//! Sessions as the rest of the application sees them.
//!
//! A [`SessionBackend`] is something that runs terminal sessions: it can list them,
//! start one, attach to one and end one. Attaching yields a stream of output frames and
//! a sink for input, and nothing else connects a view to its session: no terminal
//! handle and no pointer into the session ever crosses this interface, so a backend
//! is free to live in another process or on another machine.
//!
//! [`LocalBackend`] is the backend for sessions on this machine. It is a client of the
//! session host and speaks the wire protocol to it, even when the host runs inside the
//! same process.
//!
//! A [`Router`] holds the backends by id. A view names its session by backend id and
//! session id, and asks the router for the backend.

mod backend;
mod local;
mod router;
mod shell;

use lume_proto::ProtoError;

pub use backend::{Attachment, FrameStream, Hydrate, InputSink, OutputFrame, SessionBackend};
pub use local::LocalBackend;
/// What an in-process host is built from, for [`LocalBackend::in_process`].
pub use lume_host::HostConfig;
pub use lume_proto::{
    BackendCaps, BackendId, CellPx, ClientFacts, ColorScheme, ErrorCode, ExitStatus, HydrateKinds,
    Launcher, Palette, Responder, Rgb, SessionEvent, SessionId, SessionInfo, SessionStats,
    SpawnSpec, TermSize,
};
pub use router::Router;
pub use shell::{KnownShell, default_shell, known_shells};

/// Why a session operation failed.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// The backend understood the request and refused it. The code says why.
    #[error(transparent)]
    Refused(#[from] ProtoError),
    /// The connection to the backend is closed.
    #[error("the connection to the session backend is closed")]
    Disconnected,
    /// The attachment was detached and can no longer be used.
    #[error("the attachment has been detached")]
    Detached,
    /// The backend did something the protocol does not allow.
    #[error("the session backend broke the protocol: {0}")]
    Protocol(String),
    /// More credit was granted than a stream may hold.
    #[error("more credit was granted than a stream may hold")]
    CreditOverflow,
    #[error("there is no session backend called `{0}`")]
    UnknownBackend(BackendId),
}

pub type Result<T> = std::result::Result<T, SessionError>;
