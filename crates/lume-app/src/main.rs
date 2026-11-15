//! `lume-gui`: the Lume application.
//!
//! It opens one window whose webview shows a terminal. The session behind the terminal
//! runs on a session host inside this process, reached through the session protocol
//! like any other host would be; the webview reaches it through the commands in
//! [`commands`] and a byte carrier.
//!
//! `lume-gui --probe` starts no session. The page reports which webview engine it runs
//! on and what GPU paths the engine offers, the findings are printed to standard output
//! as one JSON object, and the process exits. CI uses this to check the app starts on
//! each platform.

// A release build on Windows is a GUI-subsystem program and opens no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod carrier;
mod clipboard;
mod commands;
mod error;
mod fonts;
mod opener;
mod state;
#[cfg(test)]
mod testing;
mod tuning;

use std::sync::Arc;

use lume_session::{HostConfig, LocalBackend, Router};
use tauri::webview::PageLoadEvent;
use tauri::{RunEvent, WindowEvent};

use crate::clipboard::ClipboardThread;
use crate::commands::ProbeMode;
use crate::fonts::Fonts;
use crate::state::AppState;

/// Starts the backend for sessions on this machine and puts it in a router.
fn local_sessions(router: &Router) -> Option<Arc<LocalBackend>> {
    // The backend's tasks live on Tauri's runtime, which is the one commands run on.
    let started = tauri::async_runtime::block_on(LocalBackend::in_process(HostConfig::default()));
    match started {
        Ok(local) => {
            let local = Arc::new(local);
            router.insert(Router::local_id(), local.clone());
            Some(local)
        }
        Err(error) => {
            // The window still opens, and says so when it fails to start a session.
            tracing::error!(%error, "the session host did not start");
            None
        }
    }
}

fn main() {
    let probe = std::env::args().any(|arg| arg == "--probe");

    let router = Router::new();
    let local = if probe { None } else { local_sessions(&router) };
    let runtime = tauri::async_runtime::handle().inner().clone();
    let state = AppState::new(runtime, router, local, tuning::REATTACH_GRACE);

    let loading = Arc::clone(&state);
    let app = tauri::Builder::default()
        .manage(ProbeMode(probe))
        .manage(Arc::clone(&state))
        .manage(ClipboardThread::start())
        .manage(Fonts::default())
        .on_page_load(move |webview, payload| {
            // A page that starts loading replaces whatever page the webview had, and
            // that page's carrier with it. Nothing tells the old page's channel that
            // its page is gone, so the carrier is retired here.
            if payload.event() == PageLoadEvent::Started {
                loading.retire(webview.label());
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::webview_info,
            commands::probe_done,
            commands::carrier_open,
            commands::carrier_send,
            commands::session_spawn,
            commands::session_attach,
            commands::session_kill,
            commands::session_list,
            commands::clipboard_read,
            commands::clipboard_write,
            commands::open_link,
            commands::font_families,
        ])
        .build(tauri::generate_context!())
        .expect("error while starting Lume");

    app.run(move |_app, event| match event {
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::Destroyed,
            ..
        } => state.retire(&label),
        // A program in a terminal can outlive a process that exits without closing the
        // terminal, so the sessions are ended before the process is.
        RunEvent::Exit => tauri::async_runtime::block_on(state.shutdown()),
        _ => {}
    });
}
