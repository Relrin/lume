//! Tauri build step.
//!
//! Declaring the app's commands here makes Tauri generate a permission for each one, so a
//! command is callable from the webview only if a capability file grants it. Without this
//! every registered command would be allowed by default.

/// Every command the web UI may be granted. The capability files grant a terminal
/// window its commands and the reporting window its two; the three that start, end and
/// list sessions directly, and the last three, are granted only when the application
/// is started to run a test suite.
const COMMANDS: &[&str] = &[
    "webview_info",
    "probe_done",
    "carrier_open",
    "carrier_send",
    "session_spawn",
    "session_attach",
    "session_kill",
    "session_list",
    "clipboard_read",
    "clipboard_write",
    "open_link",
    "font_families",
    "page_reply",
    "layout_view",
    "layout_apply",
    "sessions_start",
    "commands_list",
    "command_run",
    "config_view",
    "config_set",
    "config_unset",
    "config_open",
    "window_drag",
    "window_control",
    "window_state",
    "workspaces_list",
    "harness_done",
    "harness_save",
    "harness_read",
];

fn main() {
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    tauri_build::try_build(attributes).expect("tauri build step");
}
