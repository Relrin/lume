//! The commands the web UI may call.
//!
//! Each is declared in the build script and granted to the windows that may call it
//! in a capability file; a command missing from either cannot be called. The functions
//! here only take arguments apart and put results together: what they do lives in the
//! state.
//!
//! A terminal window's page is not granted the commands that start, end and list
//! sessions: the sessions of its panes are the application's to start. Those three are
//! for the page of a test suite, which measures sessions of its own.

use std::sync::Arc;

use lume_proto::app::PageResult;
use lume_proto::tuning::{CREDIT_WINDOW, MAX_CONTROL_MESSAGE, MAX_HYDRATE, MAX_PAYLOAD};
use lume_session::{
    BackendId, ClientFacts, Launcher, SessionId, SpawnSpec, TermSize, default_shell,
};
use serde::{Deserialize, Serialize};
use tauri::ipc::{Channel, InvokeBody, InvokeResponseBody, Request};
use tauri::{AppHandle, Manager, State, Webview};

use crate::args::{Args, Mode};
use crate::carrier::TauriChannelCarrier;
use crate::clipboard::ClipboardThread;
use crate::error::CommandError;
use crate::fonts::Fonts;
use crate::opener;
use crate::state::{AppState, Attached, Listed};

/// The request header that names the carrier a request belongs to. A request with a
/// raw body has no room for named arguments, so the id travels beside it.
const CARRIER_HEADER: &str = "lume-carrier";

type App<'a> = State<'a, Arc<AppState>>;

/// What the core knows about the webview it is hosting.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebviewInfo {
    /// `WebView2`, `WKWebView` or `WebKitGTK`.
    engine: &'static str,
    /// The engine's version as the platform reports it, if it can be read.
    version: Option<String>,
    os: &'static str,
    arch: &'static str,
    app_version: &'static str,
    /// Whether the app was started with `--probe`.
    probe: bool,
    /// The suite the app was started to run with `--harness`, if it was.
    harness: Option<&'static str>,
    /// Whether that suite may replace its reference files.
    update: bool,
}

// Run beside the main thread, which the windows need while they start.
#[tauri::command(async)]
pub fn webview_info(args: State<'_, Args>) -> WebviewInfo {
    let engine = if cfg!(windows) {
        "WebView2"
    } else if cfg!(target_os = "macos") {
        "WKWebView"
    } else {
        "WebKitGTK"
    };
    WebviewInfo {
        engine,
        version: tauri::webview_version().ok(),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        app_version: env!("CARGO_PKG_VERSION"),
        probe: args.mode == Mode::Probe,
        harness: match args.mode {
            Mode::Harness(suite) => Some(suite.name()),
            Mode::Terminal | Mode::Probe => None,
        },
        update: args.update,
    }
}

/// Called once by a page when it has drawn its first output, with the steps of its start
/// on its own clock and that clock's `now`. Only the first page's account is kept; the
/// control endpoint reports it among the start's marks.
#[tauri::command]
pub fn page_marks(app: App<'_>, steps: crate::marks::PageSteps, now: f64) {
    app.marks().page_steps(&steps, now);
}

/// Called by a page whose window was kept ready and shown again, at its first frame
/// since.
#[tauri::command]
pub fn page_shown(app: App<'_>) {
    app.marks().shown_drawn();
}

/// Called by the page when its checks are finished. In probe mode the report is printed
/// and the app exits; otherwise the call does nothing.
#[tauri::command]
pub fn probe_done(report: serde_json::Value, app: AppHandle, args: State<'_, Args>) {
    if args.mode != Mode::Probe {
        return;
    }
    println!("{report}");
    let failed = report.get("ok").and_then(serde_json::Value::as_bool) == Some(false);
    app.exit(i32::from(failed));
}

/// The protocol's sizes, so the page's codec uses the core's values rather than its own.
#[derive(Serialize)]
pub struct Limits {
    max_payload: usize,
    max_control_message: usize,
    max_hydrate: usize,
    credit_window: u32,
}

/// What opening a carrier tells the page.
#[derive(Serialize)]
pub struct Opened {
    /// Names this carrier in every later request.
    carrier: u32,
    limits: Limits,
    /// The backend new sessions start on.
    default_backend: BackendId,
}

/// Opens the page's carrier: session frames arrive on `channel` from here on. Any
/// carrier the webview had before belonged to a page that is gone, and is retired.
#[tauri::command]
pub async fn carrier_open(
    webview: Webview,
    state: App<'_>,
    channel: Channel<InvokeResponseBody>,
) -> Result<Opened, CommandError> {
    let carrier = Arc::new(TauriChannelCarrier(channel));
    Ok(Opened {
        carrier: state.open(webview.label(), carrier).await,
        limits: Limits {
            max_payload: MAX_PAYLOAD,
            max_control_message: MAX_CONTROL_MESSAGE,
            max_hydrate: MAX_HYDRATE,
            credit_window: CREDIT_WINDOW,
        },
        default_backend: state.default_backend(),
    })
}

/// Takes the frames a page sends on its streams: input, size requests, facts, credit
/// and detach. The body is the frames themselves.
#[tauri::command]
pub async fn carrier_send(
    webview: Webview,
    state: App<'_>,
    request: Request<'_>,
) -> Result<(), CommandError> {
    let carrier = request
        .headers()
        .get(CARRIER_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| CommandError::bad_request("the request does not name its carrier"))?;
    match request.body() {
        InvokeBody::Raw(frames) => state.send(webview.label(), carrier, frames).await,
        // A webview that cannot post bytes sends them as a list of numbers instead.
        InvokeBody::Json(value) => {
            let frames: Vec<u8> = serde_json::from_value(value.clone())
                .map_err(|_| CommandError::bad_request("the body is not bytes"))?;
            state.send(webview.label(), carrier, &frames).await
        }
    }
}

/// What to start. Only the size is required.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnRequest {
    /// The program to run. Left out, it is the user's default shell.
    launcher: Option<Launcher>,
    /// The working directory. Left out, it is the user's home directory.
    cwd: Option<String>,
    #[serde(default)]
    env: Vec<(String, String)>,
    size: TermSize,
}

#[tauri::command]
pub async fn session_spawn(
    webview: Webview,
    state: App<'_>,
    carrier: u32,
    backend: BackendId,
    spec: SpawnRequest,
) -> Result<SessionId, CommandError> {
    let home = || {
        let home = webview.app_handle().path().home_dir().ok()?;
        Some(home.to_string_lossy().into_owned())
    };
    let spec = SpawnSpec {
        launcher: spec.launcher.unwrap_or_else(default_shell),
        cwd: spec.cwd.or_else(home),
        env: spec.env,
        size: spec.size,
        shell_integration: false,
        scrollback_lines: None,
    };
    state.spawn(webview.label(), carrier, &backend, spec).await
}

/// Attaches the page to a session. `stream` is the id the page chose for the
/// attachment's frames; it must be above every id the page has used on this carrier.
#[tauri::command]
pub async fn session_attach(
    webview: Webview,
    state: App<'_>,
    carrier: u32,
    backend: BackendId,
    session: SessionId,
    stream: u32,
    facts: ClientFacts,
) -> Result<Attached, CommandError> {
    state
        .attach(webview.label(), carrier, &backend, session, stream, facts)
        .await
}

#[tauri::command]
pub async fn session_kill(
    webview: Webview,
    state: App<'_>,
    carrier: u32,
    backend: BackendId,
    session: SessionId,
) -> Result<(), CommandError> {
    state
        .kill(webview.label(), carrier, &backend, session)
        .await
}

#[tauri::command]
pub async fn session_list(
    webview: Webview,
    state: App<'_>,
    carrier: u32,
) -> Result<Vec<Listed>, CommandError> {
    state.list(webview.label(), carrier).await
}

/// The text on the system clipboard, or nothing when it holds no text.
#[tauri::command]
pub async fn clipboard_read(
    clipboard: State<'_, ClipboardThread>,
) -> Result<Option<String>, CommandError> {
    clipboard.read().await
}

/// Puts text on the system clipboard.
#[tauri::command]
pub async fn clipboard_write(
    clipboard: State<'_, ClipboardThread>,
    text: String,
) -> Result<(), CommandError> {
    clipboard.write(text).await
}

/// Opens a link the user asked for, if it is of a kind that may be opened. What the
/// webview decided about the link counts for nothing here: it is checked again.
#[tauri::command]
pub async fn open_link(url: String) -> Result<(), CommandError> {
    tokio::task::spawn_blocking(move || opener::open(&url))
        .await
        .map_err(|error| CommandError::internal(error.to_string()))?
}

/// Takes the page's answer to a request the core sent it on its carrier. `id` is the
/// request's; an answer to nothing that was asked is dropped.
#[tauri::command]
pub fn page_reply(
    webview: Webview,
    state: App<'_>,
    carrier: u32,
    id: u64,
    result: PageResult,
) -> Result<(), CommandError> {
    state.answer(webview.label(), carrier, id, result.into())
}

/// The installed monospace font families, in alphabetical order.
#[tauri::command]
pub async fn font_families(fonts: State<'_, Fonts>) -> Result<Vec<String>, CommandError> {
    fonts.families().await
}
