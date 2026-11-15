//! Messages of the application's control endpoint.
//!
//! The application serves a local endpoint that the command-line tool, and a second copy
//! of the application, talk to. What travels on it is the control plane only: JSON
//! messages in `Control` frames on stream 0, in the same request and response shape the
//! session host uses, but with the application's own requests:
//!
//! ```json
//! {"request":  {"id": 3, "body": {"type": "screen"}}}
//! {"response": {"id": 3, "ok": {"effect": "read 24 lines", "body": {"type": "screen", …}}}}
//! {"response": {"id": 3, "err": {"code": "no_window", "message": "…"}}}
//! ```
//!
//! Every answer says in `effect` what the request changed, in words, so that a script's
//! log reads as a record of what happened. Requests that wait carry how long they may
//! wait; none waits for ever.
//!
//! Some requests can only be answered by the page that shows the session: what is on
//! the screen is known to the terminal in the page and nowhere else. The application
//! passes those on as a [`PageRequest`], and the page answers with a [`PageResult`].

use serde::{Deserialize, Serialize};

use crate::control::ClientKind;
use crate::error::{DecodeError, EncodeError};
use crate::frame::{Kind, encode_control_json};
use crate::types::{BackendId, ExitStatus, Launcher, SessionId, SessionInfo, TermSize};

/// The version of the control endpoint's messages this crate speaks.
pub const APP_PROTO_VERSION: u32 = 1;

/// The name the control endpoint is served under, and looked for, when none is given.
///
/// A build made for development has a name of its own. Otherwise starting it while an
/// installed copy runs would find that copy's endpoint, bring its window to the front
/// and exit, as a second start of the same application is meant to.
pub const ENDPOINT_NAME: &str = if cfg!(debug_assertions) {
    "app-dev"
} else {
    "app"
};

/// The codes an [`AppError`] carries beyond those of the session protocol, which pass
/// through unchanged when a backend refuses a request. Their spelling is part of the
/// protocol: scripts match on it.
pub mod codes {
    /// The request is not valid as sent.
    pub const BAD_REQUEST: &str = "bad_request";
    /// Something other than `hello` arrived first.
    pub const HELLO_REQUIRED: &str = "hello_required";
    /// The two sides share no version of these messages.
    pub const VERSION_MISMATCH: &str = "version_mismatch";
    /// No such session.
    pub const NOT_FOUND: &str = "not_found";
    /// What was waited for did not happen in the time allowed.
    pub const TIMEOUT: &str = "timeout";
    /// The application has no window with a page that could answer.
    pub const NO_WINDOW: &str = "no_window";
    /// The session is not the one a window shows, and the request needs its screen.
    pub const NOT_SHOWN: &str = "not_shown";
    /// The session's program has ended.
    pub const SESSION_EXITED: &str = "session_exited";
    /// The application failed in a way the client could not have caused.
    pub const INTERNAL: &str = "internal";
}

/// One message on the control endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppMessage {
    Request(AppRequest),
    Response(AppResponse),
}

impl AppMessage {
    /// Appends the message to `out` as the frames of one control message.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<(), EncodeError> {
        encode_json(self, out)
    }

    /// Reads a message from the JSON of one control message.
    pub fn decode(json: &[u8]) -> Result<Self, DecodeError> {
        serde_json::from_slice(json).map_err(|error| DecodeError::BadPayload {
            kind: Kind::Control,
            reason: error.to_string(),
        })
    }
}

fn encode_json<T: Serialize>(message: &T, out: &mut Vec<u8>) -> Result<(), EncodeError> {
    let json = serde_json::to_vec(message).map_err(|error| EncodeError::Unrepresentable {
        kind: Kind::Control,
        reason: error.to_string(),
    })?;
    encode_control_json(&json, out)
}

/// A request. The response carries the same `id`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppRequest {
    pub id: u64,
    pub body: AppRequestBody,
}

/// What to start in a window. Everything left out is chosen by the application.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenSpec {
    /// The program to run. Left out, it is the user's default shell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher: Option<Launcher>,
    /// The working directory. Left out, it is the user's home directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Variables added to the program's environment.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<(String, String)>,
    /// The terminal's size. Left out, it is the size that fits the window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<TermSize>,
}

/// What a client may ask of the application.
///
/// Where a request names no session, it means the one the window shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AppRequestBody {
    /// Must be the first request on a connection.
    Hello {
        /// The newest version of these messages the client speaks.
        proto: u32,
        /// The oldest version the client speaks.
        proto_min: u32,
        client_kind: ClientKind,
    },
    /// Brings the application's window to the front. A second copy of the application
    /// sends this and exits, so that there is one application however often it is started.
    Activate {},
    /// Closes the application as closing its window would: its sessions are ended first.
    Quit {},
    /// Asks what the application is and how far its start has come.
    Status {},
    /// Starts a session and shows it in the window, in place of the one shown before,
    /// which keeps running.
    Spawn {
        spec: OpenSpec,
    },
    List {},
    Kill {
        session: SessionId,
    },
    /// Types `text` into the session, as bytes of UTF-8.
    Send {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<SessionId>,
        text: String,
    },
    /// Gives the session's terminal this size, whatever the window's size is.
    Resize {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<SessionId>,
        size: TermSize,
    },
    /// Waits for the session's program to end.
    WaitExit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<SessionId>,
        timeout_ms: u64,
    },
    /// Reads the text on the session's screen.
    Screen {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<SessionId>,
    },
    /// Waits until `text` is on the session's screen.
    WaitText {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<SessionId>,
        text: String,
        timeout_ms: u64,
    },
}

/// The answer to a request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawAppResponse", into = "RawAppResponse")]
pub struct AppResponse {
    pub id: u64,
    pub result: Result<AppReply, AppError>,
}

/// A response as it is spelt in JSON: `ok` or `err`, never both and never neither.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAppResponse {
    id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ok: Option<AppReply>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    err: Option<AppError>,
}

impl TryFrom<RawAppResponse> for AppResponse {
    type Error = &'static str;

    fn try_from(raw: RawAppResponse) -> Result<Self, Self::Error> {
        let result = match (raw.ok, raw.err) {
            (Some(reply), None) => Ok(reply),
            (None, Some(error)) => Err(error),
            _ => return Err("a response carries exactly one of `ok` and `err`"),
        };
        Ok(Self { id: raw.id, result })
    }
}

impl From<AppResponse> for RawAppResponse {
    fn from(response: AppResponse) -> Self {
        let (ok, err) = match response.result {
            Ok(reply) => (Some(reply), None),
            Err(error) => (None, Some(error)),
        };
        Self {
            id: response.id,
            ok,
            err,
        }
    }
}

/// A request that was carried out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppReply {
    /// What the request changed, in words.
    pub effect: String,
    pub body: AppReplyBody,
}

/// A request that was refused or failed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
#[serde(deny_unknown_fields)]
pub struct AppError {
    /// One of [`codes`], or a code of the session protocol.
    pub code: String,
    pub message: String,
}

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// How far the application's start has come: milliseconds from the start of the process
/// to each point, for the points that have been reached.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartMarks {
    /// The window was opened.
    pub window_ms: Option<u64>,
    /// The page finished loading.
    pub page_ms: Option<u64>,
    /// The page opened its carrier.
    pub carrier_ms: Option<u64>,
    /// The page attached to its first session.
    pub attached_ms: Option<u64>,
    /// The first output of a session was handed to the page.
    pub output_ms: Option<u64>,
}

/// One session as the application sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppSession {
    pub backend: BackendId,
    /// Whether a window is showing the session.
    pub shown: bool,
    pub info: SessionInfo,
}

/// Where the cursor is on a screen. Columns and rows count from zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenCursor {
    pub col: u16,
    pub row: u16,
    pub visible: bool,
}

/// What a request's answer carries, by request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AppReplyBody {
    Hello {
        /// The version of these messages both sides now speak.
        proto: u32,
        app_version: String,
        pid: u32,
    },
    Activated {},
    Quitting {},
    Status {
        app_version: String,
        pid: u32,
        /// Whether the application serves its own copy of the page, as an installed one
        /// does, rather than loading it from a development server.
        bundled_ui: bool,
        /// Whether a window's page is connected and can answer for its sessions.
        window: bool,
        /// The session the window shows.
        shown: Option<SessionId>,
        marks: StartMarks,
    },
    Spawned {
        session: SessionId,
        /// The session the window showed before, which is still running.
        previous: Option<SessionId>,
    },
    Sessions {
        sessions: Vec<AppSession>,
    },
    Killed {},
    Sent {
        /// How many bytes were written to the session.
        bytes: u64,
    },
    Resized {
        size: TermSize,
    },
    Exited {
        status: ExitStatus,
    },
    /// The visible screen, one string for each row with the blanks at its end removed.
    Screen {
        session: SessionId,
        size: TermSize,
        cursor: ScreenCursor,
        /// Whether a full-screen program has switched to the alternate screen.
        alternate: bool,
        lines: Vec<String>,
    },
    /// The text waited for is on the screen.
    Matched {
        session: SessionId,
        /// The first row that holds it.
        row: u16,
        /// That row's text.
        line: String,
    },
}

/// A request from the application to a window's page. It travels to the page as a
/// control message on the page's carrier: `{"request": {"id": …, "body": …}}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageRequest {
    pub id: u64,
    pub body: PageRequestBody,
}

impl PageRequest {
    /// Appends the request to `out` as the frames of one control message.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<(), EncodeError> {
        #[derive(Serialize)]
        #[serde(rename_all = "snake_case")]
        enum Envelope<'a> {
            Request(&'a PageRequest),
        }
        encode_json(&Envelope::Request(self), out)
    }
}

/// What the application may ask of a page. A request that names no session means the
/// one the page shows; one that names another is refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PageRequestBody {
    /// Start a session and show it. Answered with [`AppReplyBody::Spawned`].
    Open { spec: OpenSpec },
    /// Answered with [`AppReplyBody::Screen`].
    Screen {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<SessionId>,
    },
    /// Answered with [`AppReplyBody::Matched`], or with a `timeout` error.
    WaitText {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<SessionId>,
        text: String,
        timeout_ms: u64,
    },
    /// Answered with [`AppReplyBody::Resized`] once the terminal has that size.
    Resize {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<SessionId>,
        size: TermSize,
    },
}

/// A page's answer to a [`PageRequest`]: `{"ok": …}` or `{"err": …}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageResult {
    Ok(AppReplyBody),
    Err(AppError),
}

impl From<PageResult> for Result<AppReplyBody, AppError> {
    fn from(result: PageResult) -> Self {
        match result {
            PageResult::Ok(body) => Ok(body),
            PageResult::Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> SessionId {
        SessionId::new(0xab).unwrap()
    }

    fn message(json: &str) -> Result<AppMessage, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn requests_have_the_documented_shape() {
        let cases = [
            (
                AppRequestBody::Hello {
                    proto: 1,
                    proto_min: 1,
                    client_kind: ClientKind::Ctl,
                },
                r#"{"type":"hello","proto":1,"proto_min":1,"client_kind":"ctl"}"#,
            ),
            (AppRequestBody::Activate {}, r#"{"type":"activate"}"#),
            (AppRequestBody::Quit {}, r#"{"type":"quit"}"#),
            (AppRequestBody::Status {}, r#"{"type":"status"}"#),
            (
                AppRequestBody::Spawn {
                    spec: OpenSpec::default(),
                },
                r#"{"type":"spawn","spec":{}}"#,
            ),
            (AppRequestBody::List {}, r#"{"type":"list"}"#),
            (
                AppRequestBody::Kill { session: session() },
                r#"{"type":"kill","session":"00000000000000ab"}"#,
            ),
            (
                AppRequestBody::Send {
                    session: None,
                    text: "ls\r".to_owned(),
                },
                r#"{"type":"send","text":"ls\r"}"#,
            ),
            (
                AppRequestBody::Resize {
                    session: Some(session()),
                    size: TermSize::new(100, 30).unwrap(),
                },
                r#"{"type":"resize","session":"00000000000000ab","size":{"cols":100,"rows":30}}"#,
            ),
            (
                AppRequestBody::WaitExit {
                    session: None,
                    timeout_ms: 5000,
                },
                r#"{"type":"wait_exit","timeout_ms":5000}"#,
            ),
            (
                AppRequestBody::Screen { session: None },
                r#"{"type":"screen"}"#,
            ),
            (
                AppRequestBody::WaitText {
                    session: None,
                    text: "$ ".to_owned(),
                    timeout_ms: 250,
                },
                r#"{"type":"wait_text","text":"$ ","timeout_ms":250}"#,
            ),
        ];
        for (body, json) in cases {
            assert_eq!(serde_json::to_string(&body).unwrap(), json);
            assert_eq!(
                serde_json::from_str::<AppRequestBody>(json).unwrap(),
                body,
                "{json}"
            );
        }

        let request = AppMessage::Request(AppRequest {
            id: 3,
            body: AppRequestBody::List {},
        });
        let json = r#"{"request":{"id":3,"body":{"type":"list"}}}"#;
        assert_eq!(serde_json::to_string(&request).unwrap(), json);
        assert_eq!(message(json).unwrap(), request);
    }

    #[test]
    fn a_spawn_request_says_only_what_it_chooses() {
        let spec = OpenSpec {
            launcher: Some(Launcher::Shell {
                program: "cmd.exe".to_owned(),
                args: vec!["/d".to_owned()],
                login: false,
            }),
            cwd: Some("C:\\".to_owned()),
            env: vec![("A".to_owned(), "1".to_owned())],
            size: Some(TermSize::new(80, 24).unwrap()),
        };
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(serde_json::from_str::<OpenSpec>(&json).unwrap(), spec);
        assert_eq!(
            serde_json::from_str::<OpenSpec>("{}").unwrap(),
            OpenSpec::default()
        );
    }

    #[test]
    fn responses_carry_exactly_one_of_ok_and_err() {
        let ok = AppMessage::Response(AppResponse {
            id: 3,
            result: Ok(AppReply {
                effect: "ended the session".to_owned(),
                body: AppReplyBody::Killed {},
            }),
        });
        let json =
            r#"{"response":{"id":3,"ok":{"effect":"ended the session","body":{"type":"killed"}}}}"#;
        assert_eq!(serde_json::to_string(&ok).unwrap(), json);
        assert_eq!(message(json).unwrap(), ok);

        let err = AppMessage::Response(AppResponse {
            id: 3,
            result: Err(AppError::new(codes::NO_WINDOW, "no window is open")),
        });
        let json =
            r#"{"response":{"id":3,"err":{"code":"no_window","message":"no window is open"}}}"#;
        assert_eq!(serde_json::to_string(&err).unwrap(), json);
        assert_eq!(message(json).unwrap(), err);

        assert!(message(r#"{"response":{"id":3}}"#).is_err());
    }

    #[test]
    fn a_screen_is_rows_of_text_with_the_cursor() {
        let screen = AppReplyBody::Screen {
            session: session(),
            size: TermSize::new(4, 2).unwrap(),
            cursor: ScreenCursor {
                col: 2,
                row: 1,
                visible: true,
            },
            alternate: false,
            lines: vec!["ab".to_owned(), "$ ".to_owned()],
        };
        let json = concat!(
            r#"{"type":"screen","session":"00000000000000ab","size":{"cols":4,"rows":2},"#,
            r#""cursor":{"col":2,"row":1,"visible":true},"alternate":false,"lines":["ab","$ "]}"#
        );
        assert_eq!(serde_json::to_string(&screen).unwrap(), json);
        assert_eq!(serde_json::from_str::<AppReplyBody>(json).unwrap(), screen);
    }

    #[test]
    fn unknown_fields_and_requests_are_refused() {
        for json in [
            r#"{"request":{"id":1,"body":{"type":"list"},"extra":1}}"#,
            r#"{"request":{"id":1,"body":{"type":"list","extra":1}}}"#,
            r#"{"request":{"id":1,"body":{"type":"attach"}}}"#,
            r#"{"request":{"id":1,"body":{"type":"spawn","spec":{"shell":"sh"}}}}"#,
            r#"{"response":{"id":1,"ok":{"effect":"","body":{"type":"killed"},"extra":1}}}"#,
            r#"{"notify":{"type":"fatal","code":"internal","message":""}}"#,
        ] {
            assert!(message(json).is_err(), "accepted: {json}");
        }
    }

    #[test]
    fn a_page_request_travels_in_the_request_envelope() {
        let request = PageRequest {
            id: 9,
            body: PageRequestBody::WaitText {
                session: None,
                text: "ready".to_owned(),
                timeout_ms: 1000,
            },
        };
        let mut frames = Vec::new();
        request.encode(&mut frames).unwrap();
        let json = &frames[crate::Header::LEN..];
        assert_eq!(
            std::str::from_utf8(json).unwrap(),
            r#"{"request":{"id":9,"body":{"type":"wait_text","text":"ready","timeout_ms":1000}}}"#
        );
    }

    #[test]
    fn a_page_answers_with_ok_or_err() {
        let ok: PageResult =
            serde_json::from_str(r#"{"ok":{"type":"resized","size":{"cols":90,"rows":20}}}"#)
                .unwrap();
        assert_eq!(
            Result::from(ok),
            Ok(AppReplyBody::Resized {
                size: TermSize::new(90, 20).unwrap()
            })
        );
        let err: PageResult =
            serde_json::from_str(r#"{"err":{"code":"not_shown","message":"m"}}"#).unwrap();
        assert_eq!(Result::from(err), Err(AppError::new(codes::NOT_SHOWN, "m")));
    }
}
