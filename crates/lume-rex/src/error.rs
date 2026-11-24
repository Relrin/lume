//! Why something asked of a Rex server did not happen.

use std::io;

use lume_proto::{ErrorCode, ProtoError};
use lume_session::SessionError;

use crate::data::Reject;

/// Why something asked of a Rex server did not happen.
#[derive(Debug, thiserror::Error)]
pub enum RexError {
    /// The address is not one this client can reach.
    #[error("`{0}` is not an address of a Rex server this application can reach")]
    BadEndpoint(String),
    /// The address is one this client does not reach, though it is an address.
    #[error("the Rex server at {endpoint} is not reached by this application: {why}")]
    Unsupported { endpoint: String, why: String },
    /// The machine was reached, and has no Rex.
    #[error("Rex was not found on the machine at {endpoint}")]
    NotInstalled { endpoint: String },
    /// Reaching the server would send everything unencrypted over a network, and the
    /// user has not agreed to that for this connection.
    #[error(
        "the connection to the Rex server at {endpoint} would not be encrypted, and was not agreed to"
    )]
    Unconfirmed { endpoint: String },
    /// The server's certificate does not show that it is the server its address names.
    #[error("the Rex server at {endpoint} could not be shown to be that server: {reason}")]
    Untrusted { endpoint: String, reason: String },
    /// The SSH client could not sign in to the server's machine: it was not let in, or the
    /// machine was not shown to be the one the user means.
    #[error("the machine of the Rex server at {endpoint} was not signed in to: {reason}")]
    Ssh { endpoint: String, reason: String },
    /// The user did not answer what signing in to the server's machine needs.
    #[error("signing in to the machine of the Rex server at {endpoint} was given up")]
    GivenUp { endpoint: String },
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
    /// The server refused a data connection at its handshake.
    #[error("the Rex server refused the data connection: {0}")]
    Rejected(Reject),
    /// The connection ended without a word.
    #[error("the connection to the Rex server was lost")]
    Disconnected,
    /// The server did not answer in time.
    #[error("the Rex server did not answer `{0}` in time")]
    Timeout(String),
}

impl RexError {
    /// A short, stable name for the kind of failure, for whoever reads it by program.
    ///
    /// The names are this client's own, and some of them decide what the user is asked.
    /// What a server calls a failure of its own is never one of them: it goes by the
    /// status that comes with it, and the server's word is in the failure itself.
    pub fn code(&self) -> &'static str {
        match self {
            RexError::BadEndpoint(_) => "bad_request",
            RexError::Unsupported { .. } => "unsupported",
            RexError::NotInstalled { .. } => "not_installed",
            RexError::Unconfirmed { .. } => "confirmation_required",
            RexError::Untrusted { .. } => "untrusted",
            RexError::Ssh { .. } => "ssh",
            RexError::GivenUp { .. } => "given_up",
            RexError::Unreachable { .. } => "unreachable",
            RexError::Refused { .. } => "forbidden",
            RexError::Revision(_) => "version_mismatch",
            RexError::Protocol(_) => "protocol_error",
            RexError::Call { status, .. } => match status {
                400 => "bad_request",
                404 => "not_found",
                409 => "conflict",
                413 => "too_large",
                504 => "timeout",
                _ => "refused",
            },
            RexError::Rejected(_) => "rejected",
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
            RexError::Refused { .. }
            | RexError::Unsupported { .. }
            | RexError::NotInstalled { .. }
            | RexError::Unconfirmed { .. }
            | RexError::Untrusted { .. }
            | RexError::Ssh { .. }
            | RexError::GivenUp { .. } => ErrorCode::Unsupported,
            RexError::Revision(_) => ErrorCode::VersionMismatch,
            RexError::Timeout(_) => ErrorCode::Timeout,
            RexError::Rejected(reason) => match reason {
                Reject::Unavailable => ErrorCode::NotFound,
                Reject::AlreadyConnected => ErrorCode::AlreadyAttached,
                Reject::Limit => ErrorCode::LimitExceeded,
                _ => ErrorCode::Internal,
            },
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
