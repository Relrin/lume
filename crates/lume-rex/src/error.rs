//! Why something asked of a Rex server did not happen.

use std::io;

use lume_proto::{ErrorCode, ProtoError};
use lume_session::SessionError;

/// Why something asked of a Rex server did not happen.
#[derive(Debug, thiserror::Error)]
pub enum RexError {
    /// The address is not one this client can reach.
    #[error("`{0}` is not an address of a Rex server this application can reach")]
    BadEndpoint(String),
    /// No server answered at the address.
    #[error("no Rex server could be reached at {endpoint}: {source}")]
    Unreachable {
        endpoint: String,
        #[source]
        source: io::Error,
    },
    /// The server did not let this user in.
    #[error("the Rex server at {endpoint} refused the connection: {reason}")]
    Refused { endpoint: String, reason: String },
    /// The server and this client have no revision of the API in common.
    #[error("the Rex server and this application share no revision of its API: {0}")]
    Revision(String),
    /// The server said something this client could not make sense of.
    #[error("the Rex server broke its protocol: {0}")]
    Protocol(String),
    /// The server answered a request with an error.
    #[error("the Rex server refused `{method}`: {message}")]
    Call {
        method: String,
        /// The server's name for the kind of failure, such as `not_found`.
        code: String,
        message: String,
        /// A number with the meaning of an HTTP status.
        status: u16,
    },
    /// The server closed the connection.
    #[error("the Rex server closed the connection: {reason} (status {code})")]
    Closed { code: u16, reason: String },
    /// The connection ended without a word.
    #[error("the connection to the Rex server was lost")]
    Disconnected,
    /// The server did not answer in time.
    #[error("the Rex server did not answer `{0}` in time")]
    Timeout(String),
}

impl RexError {
    /// A short, stable name for the kind of failure, for whoever reads it by program.
    pub fn code(&self) -> &str {
        match self {
            RexError::BadEndpoint(_) => "bad_request",
            RexError::Unreachable { .. } => "unreachable",
            RexError::Refused { .. } => "forbidden",
            RexError::Revision(_) => "version_mismatch",
            RexError::Protocol(_) => "protocol_error",
            RexError::Call { code, .. } => code,
            RexError::Closed { .. } | RexError::Disconnected => "disconnected",
            RexError::Timeout(_) => "timeout",
        }
    }

    /// Whether trying again later could work without anything else changing.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            RexError::Unreachable { .. }
                | RexError::Closed { .. }
                | RexError::Disconnected
                | RexError::Timeout(_)
        )
    }
}

impl From<RexError> for SessionError {
    fn from(error: RexError) -> Self {
        let code = match &error {
            RexError::Unreachable { .. } => return SessionError::Unreachable(error.to_string()),
            RexError::Closed { .. } | RexError::Disconnected => return SessionError::Disconnected,
            RexError::Protocol(_) => return SessionError::Protocol(error.to_string()),
            RexError::BadEndpoint(_) => ErrorCode::BadRequest,
            RexError::Refused { .. } => ErrorCode::Unsupported,
            RexError::Revision(_) => ErrorCode::VersionMismatch,
            RexError::Timeout(_) => ErrorCode::Timeout,
            RexError::Call { status, .. } => match status {
                404 => ErrorCode::NotFound,
                400 => ErrorCode::BadRequest,
                504 => ErrorCode::Timeout,
                _ => ErrorCode::Internal,
            },
        };
        SessionError::Refused(ProtoError::new(code, error.to_string()))
    }
}
