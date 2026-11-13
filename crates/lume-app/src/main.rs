//! `lume-gui`: the Lume application.
//!
//! For now it opens one window with one webview that reports which webview engine it runs
//! on and what GPU paths the engine offers.
//!
//! `lume-gui --probe` runs the same checks without interaction: the page reports its
//! findings, they are printed to standard output as one JSON object, and the process
//! exits. CI uses this to check the app starts on each platform.

// A release build on Windows is a GUI-subsystem program and opens no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Serialize;

/// What the core knows about the webview it is hosting.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebviewInfo {
    /// `WebView2`, `WKWebView` or `WebKitGTK`.
    engine: &'static str,
    /// The engine's version as the platform reports it, if it can be read.
    version: Option<String>,
    os: &'static str,
    arch: &'static str,
    app_version: &'static str,
    /// Whether the app was started with `--probe`.
    probe: bool,
}

struct ProbeMode(bool);

#[tauri::command]
fn webview_info(probe: tauri::State<'_, ProbeMode>) -> WebviewInfo {
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
        probe: probe.0,
    }
}

/// Called by the page when its checks are finished. In probe mode the report is printed
/// and the app exits; otherwise the call does nothing.
#[tauri::command]
fn probe_done(
    report: serde_json::Value,
    app: tauri::AppHandle,
    probe: tauri::State<'_, ProbeMode>,
) {
    if !probe.0 {
        return;
    }
    println!("{report}");
    let failed = report.get("ok").and_then(serde_json::Value::as_bool) == Some(false);
    app.exit(i32::from(failed));
}

fn main() {
    let probe = std::env::args().any(|arg| arg == "--probe");

    tauri::Builder::default()
        .manage(ProbeMode(probe))
        .invoke_handler(tauri::generate_handler![webview_info, probe_done])
        .run(tauri::generate_context!())
        .expect("error while running Lume");
}
