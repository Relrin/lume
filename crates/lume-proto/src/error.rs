//! Errors: the codes carried on the wire, and what encoding, decoding and credit
//! accounting can report.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::frame::Kind;

/// Why a request failed. The spelling of each code is part of the protocol: clients match
/// on it, so a code is never renamed or reused for another meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The request is not valid as sent.
    BadRequest,
    /// Something other than `hello` arrived first.
    HelloRequired,
    /// The two sides share no protocol version.
    VersionMismatch,
    /// No such session.
    NotFound,
    /// The session already has an attachment and takes only one.
    AlreadyAttached,
    /// The stream id is in use or was used before on this connection.
    StreamInUse,
    /// The stream id cannot carry an attachment.
    InvalidStream,
    /// The spawn request describes something that cannot be started.
    InvalidSpec,
    /// Starting the program failed.
    SpawnFailed,
    /// The host does not offer this.
    Unsupported,
    /// A limit of the host was reached.
    LimitExceeded,
    /// The session's program has ended.
    SessionExited,
    /// The peer broke the framing or flow-control rules.
    ProtocolError,
    /// The host failed in a way the client could not have caused.
    Internal,
}

impl ErrorCode {
    /// Every code, in declaration order.
    pub const ALL: [ErrorCode; 14] = [
        Self::BadRequest,
        Self::HelloRequired,
        Self::VersionMismatch,
        Self::NotFound,
        Self::AlreadyAttached,
        Self::StreamInUse,
        Self::InvalidStream,
        Self::InvalidSpec,
        Self::SpawnFailed,
        Self::Unsupported,
        Self::LimitExceeded,
        Self::SessionExited,
        Self::ProtocolError,
        Self::Internal,
    ];

    /// The code as it is spelt on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BadRequest => "bad_request",
            Self::HelloRequired => "hello_required",
            Self::VersionMismatch => "version_mismatch",
            Self::NotFound => "not_found",
            Self::AlreadyAttached => "already_attached",
            Self::StreamInUse => "stream_in_use",
            Self::InvalidStream => "invalid_stream",
            Self::InvalidSpec => "invalid_spec",
            Self::SpawnFailed => "spawn_failed",
            Self::Unsupported => "unsupported",
            Self::LimitExceeded => "limit_exceeded",
            Self::SessionExited => "session_exited",
            Self::ProtocolError => "protocol_error",
            Self::Internal => "internal",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A failed request as the wire carries it: a stable code and a message for people.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
#[serde(deny_unknown_fields)]
pub struct ProtoError {
    pub code: ErrorCode,
    pub message: String,
}

impl ProtoError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Why a byte stream is not a valid sequence of frames. Every one of these is fatal: after
/// it the position of the next frame is unknown, so the connection cannot continue.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("unknown frame kind {0:#04x}")]
    UnknownKind(u8),
    #[error("a frame payload of {0} bytes exceeds the limit")]
    PayloadTooLarge(u32),
    #[error("a {0:?} frame is not valid in this direction")]
    WrongDirection(Kind),
    #[error("a {kind:?} frame is not valid on stream {stream}")]
    WrongPlane { kind: Kind, stream: u32 },
    #[error("a {0:?} message cannot be split across frames")]
    NotFragmentable(Kind),
    #[error("a frame that announces a continuation carries no bytes")]
    EmptyFragment,
    #[error("a {got:?} frame on stream {stream} interrupts an unfinished {unfinished:?} message")]
    Interrupted {
        stream: u32,
        unfinished: Kind,
        got: Kind,
    },
    #[error("a {0:?} message exceeds its size limit once its frames are joined")]
    MessageTooLarge(Kind),
    #[error("a {kind:?} payload has {len} bytes where {expected} are required")]
    BadLength {
        kind: Kind,
        len: usize,
        expected: usize,
    },
    #[error("a {kind:?} payload is malformed: {reason}")]
    BadPayload { kind: Kind, reason: String },
}

/// Why a message cannot be put on the wire.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    #[error("a {kind:?} message cannot be sent on stream {stream}")]
    WrongPlane { kind: Kind, stream: u32 },
    #[error("a {0:?} message exceeds its size limit")]
    MessageTooLarge(Kind),
    #[error("a {kind:?} message cannot be represented: {reason}")]
    Unrepresentable { kind: Kind, reason: String },
}

/// A breach of the credit rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FlowError {
    #[error("the peer granted more credit than one stream may hold")]
    WindowOverflow,
    #[error("the peer sent {received} bytes with only {granted} bytes of credit")]
    GrantExceeded { granted: u32, received: usize },
}
