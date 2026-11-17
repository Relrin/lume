//! What a command returns when it fails.

use std::borrow::Cow;
use std::fmt;

use lume_config::ConfigError;
use lume_layout::LayoutError;
use lume_proto::app::AppError;
use lume_session::SessionError;
use serde::Serialize;

/// A failed command as the webview sees it: a stable code to match on and a message for
/// people. A request a backend refused keeps the backend's own code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CommandError {
    pub code: Cow<'static, str>,
    pub message: String,
}

impl CommandError {
    pub fn new(code: impl Into<Cow<'static, str>>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// What was asked for does not exist.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("not_found", message)
    }

    /// The page may not do this.
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new("forbidden", message)
    }

    /// The application cannot do this yet.
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new("unsupported", message)
    }

    /// The request is not valid as sent.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new("bad_request", message)
    }

    /// The webview has not opened a carrier.
    pub fn no_carrier() -> Self {
        Self::new("no_carrier", "this window has no open carrier")
    }

    /// The request names a carrier that a newer one has replaced.
    pub fn stale_carrier() -> Self {
        Self::new("stale_carrier", "the carrier has been replaced")
    }

    /// The stream id cannot be used for a new attachment.
    pub fn invalid_stream(stream: u32) -> Self {
        Self::new(
            "invalid_stream",
            format!("stream {stream} is not above every stream used before"),
        )
    }

    /// The core failed in a way the webview could not have caused.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", message)
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CommandError {}

impl From<CommandError> for lume_proto::app::AppError {
    /// The same code and message, for a caller on the control endpoint.
    fn from(error: CommandError) -> Self {
        Self::new(error.code, error.message)
    }
}

impl From<AppError> for CommandError {
    /// The same code and message, for a page that asked what the control endpoint
    /// also answers.
    fn from(error: AppError) -> Self {
        Self::new(error.code, error.message)
    }
}

impl From<LayoutError> for CommandError {
    /// A refused intent keeps the layout's own code.
    fn from(error: LayoutError) -> Self {
        Self::new(error.code().to_owned(), error.to_string())
    }
}

impl From<lume_layout::IntentError> for CommandError {
    fn from(error: lume_layout::IntentError) -> Self {
        Self::new(error.code(), error.to_string())
    }
}

impl From<ConfigError> for CommandError {
    fn from(error: ConfigError) -> Self {
        Self::new("config", error.to_string())
    }
}

impl From<SessionError> for CommandError {
    fn from(error: SessionError) -> Self {
        let code = match &error {
            SessionError::Refused(refused) => {
                return Self::new(refused.code.as_str(), refused.message.clone());
            }
            SessionError::Disconnected => "disconnected",
            SessionError::Detached => "detached",
            SessionError::Protocol(_) => "protocol_error",
            SessionError::CreditOverflow => "limit_exceeded",
            SessionError::UnknownBackend(_) => "not_found",
        };
        Self::new(code, error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use lume_proto::{ErrorCode, ProtoError};
    use lume_session::BackendId;

    use super::*;

    #[test]
    fn a_refused_request_keeps_the_backends_code_and_message() {
        let refused =
            SessionError::Refused(ProtoError::new(ErrorCode::NotFound, "no such session"));
        assert_eq!(
            CommandError::from(refused),
            CommandError::new("not_found", "no such session")
        );
    }

    #[test]
    fn every_other_failure_has_a_code_of_its_own() {
        let code = |error: SessionError| CommandError::from(error).code;
        assert_eq!(code(SessionError::Disconnected), "disconnected");
        assert_eq!(code(SessionError::Detached), "detached");
        assert_eq!(code(SessionError::Protocol("x".into())), "protocol_error");
        assert_eq!(code(SessionError::CreditOverflow), "limit_exceeded");
        assert_eq!(
            code(SessionError::UnknownBackend(BackendId("x".into()))),
            "not_found"
        );
    }

    #[test]
    fn an_error_reaches_the_webview_as_a_code_and_a_message() {
        let json = serde_json::to_value(CommandError::stale_carrier()).unwrap();
        assert_eq!(json["code"], "stale_carrier");
        assert!(json["message"].is_string());
    }
}
