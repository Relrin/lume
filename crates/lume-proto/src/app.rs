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
use crate::event::SessionEvent;
use crate::frame::{Kind, encode_control_json};
use crate::types::{BackendId, ExitStatus, Launcher, SessionId, SessionInfo, TermSize};

/// The version of the control endpoint's messages this crate speaks.
pub const APP_PROTO_VERSION: u32 = 9;

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
    /// The session is in no pane of a window whose page could answer for it, and the
    /// request needs its screen or its keyboard.
    pub const NOT_SHOWN: &str = "not_shown";
    /// What was asked for is not something this version of the application can do.
    pub const UNSUPPORTED: &str = "unsupported";
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
    /// Something a session reported, sent to a client that asked to follow sessions.
    Event(AppEvent),
}

/// What there is to tell about a session: something it reported, or how its program
/// ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionNews {
    Event(SessionEvent),
    Exit(ExitStatus),
}

/// One piece of news about one session. In JSON the news sits beside the names:
/// `{"pane": "p3", "session": "…", "event": {"type": "cwd", …}}`, or `"exit"` in
/// place of `"event"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppEvent {
    /// The pane that shows the session, where one does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
    pub session: SessionId,
    #[serde(flatten)]
    pub what: SessionNews,
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
    /// The profile to start, by its id. Left out, it is the default profile. A program
    /// named in `launcher` is run as named, whatever the profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// The program to run. Left out, it is the profile's.
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

/// A direction on the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Where the pane of a new session goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpawnPlace {
    /// In a new tab of the window the user is working in.
    Tab {},
    /// Beside the pane that has the keyboard, on the side named.
    Split { dir: Direction },
    /// In a new window.
    Window {},
}

impl Default for SpawnPlace {
    fn default() -> Self {
        SpawnPlace::Tab {}
    }
}

impl SpawnPlace {
    fn is_tab(&self) -> bool {
        matches!(self, SpawnPlace::Tab {})
    }
}

/// What a client may ask of the application.
///
/// A request about one terminal names its pane, its session, or neither, which means
/// the pane that has the keyboard. A pane is named as the layout names it, as in `p12`.
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
    /// Does what starting the application again asks for: its window is brought to the
    /// front, the workspace named is switched to, and the program named is run in a new
    /// tab. A second copy of the application sends this and exits, so that there is one
    /// application however often it is started.
    Launch {
        /// The workspace to show. Left out, the one on show stays.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
        /// A program to run in a new tab, and its arguments. Empty, nothing is run.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        exec: Vec<String>,
        /// The directory the program is started in: where the command was typed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    /// Closes the application as closing its window would: its sessions are ended first.
    Quit {},
    /// Asks what the application is and how far its start has come.
    Status {},
    /// Starts a session in a new pane.
    Spawn {
        spec: OpenSpec,
        #[serde(default, skip_serializing_if = "SpawnPlace::is_tab")]
        place: SpawnPlace,
    },
    List {},
    Kill {
        session: SessionId,
    },
    /// Types `text` into the terminal, as bytes of UTF-8.
    Send {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<SessionId>,
        text: String,
    },
    /// Gives the terminal this size, whatever the size of its pane is.
    Resize {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<SessionId>,
        size: TermSize,
    },
    /// Waits for the terminal's program to end.
    WaitExit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<SessionId>,
        timeout_ms: u64,
    },
    /// Reads the text on the terminal's screen.
    Screen {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<SessionId>,
    },
    /// Waits until `text` is on the terminal's screen.
    WaitText {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<SessionId>,
        text: String,
        timeout_ms: u64,
    },
    /// Reads the layout: the windows, their tabs, and the panes in them.
    Layout {},
    /// Changes the layout. `intent` is one of the layout's intents, as JSON.
    LayoutApply {
        intent: serde_json::Value,
    },
    /// Lists the workspaces there are, and says which is on show.
    Workspaces {},
    /// Lists the profiles there are: the ways to start a terminal.
    Profiles {},
    /// Follows what sessions report: titles, directories, commands that start and
    /// end, the program that has the terminal, a program's status, and how each
    /// session ends. Of one pane or session if one is named, else of all.
    ///
    /// This is the one request that does not end with its answer. After the answer
    /// the application sends an [`AppMessage::Event`] for each report, beginning with
    /// the state of every session followed as it stands, until the client closes the
    /// connection. No further request is read from it.
    Events {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<SessionId>,
    },
    /// Shows a session to the client as a terminal of its own: after the answer this
    /// connection carries the attachment on stream 1, as the session protocol carries
    /// one. The application sends the screen to start from as replayed output, then the
    /// output, sizes, reports and the exit; the client sends input, sizes, its view,
    /// credit, and a detach to end it. No further request is read from the connection.
    /// The client sends nothing more until it has the answer.
    Attach {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<SessionId>,
        /// The size of the client's terminal.
        size: TermSize,
    },
    /// Lists the commands there are.
    Commands {},
    /// Runs a command, in the window named or the one the user is working in.
    Command {
        id: String,
        #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
        args: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        window: Option<String>,
    },
    /// Lists the plugins that are installed, in the order they load in, with whether
    /// each can be used, whether its code is running, and how it has fared.
    Plugins {},
    /// Starts a plugin's code, as one of its activation events would.
    PluginActivate {
        id: String,
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
    /// The user's configuration was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_ms: Option<u64>,
    /// The host of this machine's sessions was ready.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_ms: Option<u64>,
    /// The workspace to show was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panes_ms: Option<u64>,
    /// The application was built and began to set itself up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_ms: Option<u64>,
    /// The window was opened.
    pub window_ms: Option<u64>,
    /// The first shell was started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_ms: Option<u64>,
    /// The first page's code began to run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parsed_ms: Option<u64>,
    /// The first page's terminal emulation was ready.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vt_ms: Option<u64>,
    /// The first page's graphics card answered: its terminals may draw with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_ms: Option<u64>,
    /// The first page's terminal fonts were loaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fonts_ms: Option<u64>,
    /// The first page laid out its panes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_ms: Option<u64>,
    /// The first page drew a terminal's first frame, with or without output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_ms: Option<u64>,
    /// The first output of a session reached the first page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_output_ms: Option<u64>,
    /// The first output of a session was drawn in a page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drawn_ms: Option<u64>,
    /// When a window kept ready in the background was last shown, in milliseconds
    /// since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown_unix_ms: Option<u64>,
    /// When that window's page drew its first frame after being shown, in milliseconds
    /// since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown_drawn_unix_ms: Option<u64>,
    /// The page finished loading.
    pub page_ms: Option<u64>,
    /// The page opened its carrier.
    pub carrier_ms: Option<u64>,
    /// The page attached to its first session.
    pub attached_ms: Option<u64>,
    /// The first output of a session was handed to the page.
    pub output_ms: Option<u64>,
    /// The installed plugins were read, and those that start at once were started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugins_ms: Option<u64>,
    /// How many files of plugins were read before the first output.
    #[serde(default)]
    pub plugin_files_before_output: u32,
}

/// One session as the application sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppSession {
    pub backend: BackendId,
    /// The pane that shows the session, if one does.
    pub pane: Option<String>,
    pub info: SessionInfo,
    /// Whether something runs in the session that closing its pane would end, as far as
    /// what the session reported tells.
    #[serde(default)]
    pub busy: bool,
}

/// One workspace there is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceEntry {
    pub name: String,
    /// The folder the workspace's new terminals start in, if it has one.
    pub root: Option<String>,
}

/// The pane that has the keyboard in the window the user is working in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Focused {
    pub pane: String,
    /// The session the pane shows, once it has one.
    pub session: Option<SessionId>,
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
    /// The window is at the front, and what else was asked has been done.
    Launched {
        /// The workspace on show afterwards.
        workspace: String,
        /// The pane the program named runs in, if one was named.
        pane: Option<String>,
    },
    Quitting {},
    Status {
        app_version: String,
        pid: u32,
        /// Whether the application serves its own copy of the page, as an installed one
        /// does, rather than loading it from a development server.
        bundled_ui: bool,
        /// Whether a window's page is connected and can answer for its sessions.
        window: bool,
        /// The pane that has the keyboard.
        focused: Option<Focused>,
        marks: Box<StartMarks>,
    },
    Spawned {
        session: SessionId,
        /// The pane that was made for the session.
        pane: String,
        /// The tab that was made for it, if one was.
        tab: Option<String>,
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
        pane: String,
        session: SessionId,
        size: TermSize,
        cursor: ScreenCursor,
        /// Whether a full-screen program has switched to the alternate screen.
        alternate: bool,
        lines: Vec<String>,
    },
    /// The text waited for is on the screen.
    Matched {
        pane: String,
        session: SessionId,
        /// The first row that holds it.
        row: u16,
        /// That row's text.
        line: String,
    },
    /// The layout as it stands.
    Layout {
        /// The window the user is working in.
        window: Option<String>,
        epoch: u32,
        revision: u64,
        /// The workspace, as the layout writes it.
        workspace: serde_json::Value,
    },
    /// An intent was applied.
    Applied {
        epoch: u32,
        /// The layout's revision afterwards. An intent that changed nothing leaves it
        /// as it was.
        revision: u64,
        /// The ids of what the intent made: `pane`, `tab`, `window`.
        created: serde_json::Value,
    },
    Workspaces {
        /// The workspace on show.
        active: String,
        workspaces: Vec<WorkspaceEntry>,
    },
    /// The profiles in effect, the id of the default one, and what there is to say
    /// about ones that might be expected and are not there.
    Profiles {
        profiles: serde_json::Value,
    },
    Commands {
        commands: serde_json::Value,
    },
    Plugins {
        plugins: serde_json::Value,
    },
    /// A plugin's code was started, or was running already.
    PluginActivated {
        id: String,
    },
    /// A command was run.
    Ran {
        command: String,
    },
    /// The sessions are being followed: their reports come next on this connection.
    Following {},
    /// The attachment is open on stream 1 of this connection.
    Attached {
        /// The pane that shows the session, if one does.
        pane: Option<String>,
        session: SessionId,
        /// The terminal's size now.
        size: TermSize,
    },
    /// The grid a pane that is about to be made will have. Only a page answers this.
    Estimated {
        size: TermSize,
    },
    /// What the user answered to something a plugin asked. Only a page answers this.
    Answered {
        answer: serde_json::Value,
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

/// What the application may ask of a page. A request about a terminal names its pane,
/// its session, or neither, which means the pane that has the keyboard.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PageRequestBody {
    /// How large the pane an intent would make is going to be: only the page knows
    /// the window's pixels and the font's. Answered with [`AppReplyBody::Estimated`].
    Estimate { intent: serde_json::Value },
    /// Run a command that is carried out in the page. Answered with
    /// [`AppReplyBody::Ran`].
    Command {
        id: String,
        #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
        args: serde_json::Value,
    },
    /// Answered with [`AppReplyBody::Screen`].
    Screen {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<SessionId>,
    },
    /// Answered with [`AppReplyBody::Matched`], or with a `timeout` error.
    WaitText {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<SessionId>,
        text: String,
        timeout_ms: u64,
    },
    /// Answered with [`AppReplyBody::Resized`] once the terminal has that size.
    Resize {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<SessionId>,
        size: TermSize,
    },
    /// Put a question to the user on a plugin's behalf: a permission it asks for, a
    /// use it needs approved, a confirmation or a choice. The page draws it under the
    /// plugin's name; the plugin cannot draw, fill in or dismiss it. Answered with
    /// [`AppReplyBody::Answered`].
    PluginAsk { ask: serde_json::Value },
}

/// Appends something the application tells a page without being asked to `out`, as the
/// frames of one control message: `{"notify": …}`. The value is whatever there is to
/// say, with a `type` naming what it is.
pub fn encode_notice(notice: &serde_json::Value, out: &mut Vec<u8>) -> Result<(), EncodeError> {
    #[derive(Serialize)]
    #[serde(rename_all = "snake_case")]
    enum Envelope<'a> {
        Notify(&'a serde_json::Value),
    }
    encode_json(&Envelope::Notify(notice), out)
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
            (
                AppRequestBody::Launch {
                    workspace: None,
                    exec: Vec::new(),
                    cwd: None,
                },
                r#"{"type":"launch"}"#,
            ),
            (
                AppRequestBody::Launch {
                    workspace: Some("site".to_owned()),
                    exec: vec!["htop".to_owned(), "-d".to_owned()],
                    cwd: Some("/work".to_owned()),
                },
                r#"{"type":"launch","workspace":"site","exec":["htop","-d"],"cwd":"/work"}"#,
            ),
            (AppRequestBody::Workspaces {}, r#"{"type":"workspaces"}"#),
            (AppRequestBody::Profiles {}, r#"{"type":"profiles"}"#),
            (
                AppRequestBody::Spawn {
                    spec: OpenSpec {
                        profile: Some("auto.wsl.Ubuntu".to_owned()),
                        ..OpenSpec::default()
                    },
                    place: SpawnPlace::Tab {},
                },
                r#"{"type":"spawn","spec":{"profile":"auto.wsl.Ubuntu"}}"#,
            ),
            (AppRequestBody::Quit {}, r#"{"type":"quit"}"#),
            (AppRequestBody::Status {}, r#"{"type":"status"}"#),
            (
                AppRequestBody::Spawn {
                    spec: OpenSpec::default(),
                    place: SpawnPlace::Tab {},
                },
                r#"{"type":"spawn","spec":{}}"#,
            ),
            (
                AppRequestBody::Spawn {
                    spec: OpenSpec::default(),
                    place: SpawnPlace::Split {
                        dir: Direction::Down,
                    },
                },
                r#"{"type":"spawn","spec":{},"place":{"type":"split","dir":"down"}}"#,
            ),
            (AppRequestBody::List {}, r#"{"type":"list"}"#),
            (
                AppRequestBody::Kill { session: session() },
                r#"{"type":"kill","session":"00000000000000ab"}"#,
            ),
            (
                AppRequestBody::Send {
                    pane: None,
                    session: None,
                    text: "ls\r".to_owned(),
                },
                r#"{"type":"send","text":"ls\r"}"#,
            ),
            (
                AppRequestBody::Send {
                    pane: Some("p3".to_owned()),
                    session: None,
                    text: "x".to_owned(),
                },
                r#"{"type":"send","pane":"p3","text":"x"}"#,
            ),
            (
                AppRequestBody::Resize {
                    pane: None,
                    session: Some(session()),
                    size: TermSize::new(100, 30).unwrap(),
                },
                r#"{"type":"resize","session":"00000000000000ab","size":{"cols":100,"rows":30}}"#,
            ),
            (
                AppRequestBody::WaitExit {
                    pane: None,
                    session: None,
                    timeout_ms: 5000,
                },
                r#"{"type":"wait_exit","timeout_ms":5000}"#,
            ),
            (
                AppRequestBody::Screen {
                    pane: None,
                    session: None,
                },
                r#"{"type":"screen"}"#,
            ),
            (
                AppRequestBody::WaitText {
                    pane: None,
                    session: None,
                    text: "$ ".to_owned(),
                    timeout_ms: 250,
                },
                r#"{"type":"wait_text","text":"$ ","timeout_ms":250}"#,
            ),
            (AppRequestBody::Layout {}, r#"{"type":"layout"}"#),
            (
                AppRequestBody::LayoutApply {
                    intent: serde_json::json!({"type": "focus_dir"}),
                },
                r#"{"type":"layout_apply","intent":{"type":"focus_dir"}}"#,
            ),
            (AppRequestBody::Commands {}, r#"{"type":"commands"}"#),
            (AppRequestBody::Plugins {}, r#"{"type":"plugins"}"#),
            (
                AppRequestBody::PluginActivate {
                    id: "acme.metrics".to_owned(),
                },
                r#"{"type":"plugin_activate","id":"acme.metrics"}"#,
            ),
            (
                AppRequestBody::Command {
                    id: "lume.tab.new".to_owned(),
                    args: serde_json::Value::Null,
                    window: None,
                },
                r#"{"type":"command","id":"lume.tab.new"}"#,
            ),
            (
                AppRequestBody::Command {
                    id: "lume.tab.activate".to_owned(),
                    args: serde_json::json!({"index": 2}),
                    window: Some("w1".to_owned()),
                },
                r#"{"type":"command","id":"lume.tab.activate","args":{"index":2},"window":"w1"}"#,
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
            profile: Some("mine".to_owned()),
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
            pane: "p1".to_owned(),
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
            r#"{"type":"screen","pane":"p1","session":"00000000000000ab","size":{"cols":4,"rows":2},"#,
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
                pane: None,
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
    fn a_notice_travels_in_the_notify_envelope() {
        let notice = serde_json::json!({"type": "notice", "level": "info", "message": "m"});
        let mut frames = Vec::new();
        encode_notice(&notice, &mut frames).unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&frames[crate::Header::LEN..]).unwrap();
        assert_eq!(json, serde_json::json!({ "notify": notice }));
    }

    #[test]
    fn a_page_is_asked_for_a_size_and_to_run_a_command() {
        let cases = [
            (
                PageRequestBody::Estimate {
                    intent: serde_json::json!({"type": "scratch_hide"}),
                },
                r#"{"type":"estimate","intent":{"type":"scratch_hide"}}"#,
            ),
            (
                PageRequestBody::Command {
                    id: "lume.terminal.copy".to_owned(),
                    args: serde_json::Value::Null,
                },
                r#"{"type":"command","id":"lume.terminal.copy"}"#,
            ),
        ];
        for (body, json) in cases {
            assert_eq!(serde_json::to_string(&body).unwrap(), json);
            assert_eq!(serde_json::from_str::<PageRequestBody>(json).unwrap(), body);
        }
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
