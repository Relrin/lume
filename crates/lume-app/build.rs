//! Tauri build step.
//!
//! Declaring the app's commands here makes Tauri generate a permission for each one, so a
//! command is callable from the webview only if a capability file grants it. Without this
//! every registered command would be allowed by default.

/// Every command the web UI may be granted.
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
];

fn main() {
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    tauri_build::try_build(attributes).expect("tauri build step");
}
