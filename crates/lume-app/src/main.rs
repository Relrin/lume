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
//!
//! `lume-gui --harness <suite>` runs a test suite in the webview in the same way: the
//! window shows the harness page in place of the terminal, the page reports, and the
//! process exits with the suite's verdict.
//!
//! There is one application for each user. It serves a control endpoint, and a copy
//! that finds the endpoint already served asks the first to bring its window to the
//! front, and exits.

// A release build on Windows is a GUI-subsystem program and opens no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod args;
mod carrier;
mod clipboard;
mod commands;
mod control;
mod error;
mod fonts;
mod harness;
mod log;
mod marks;
mod opener;
mod state;
mod synthetic;
#[cfg(test)]
mod testing;
mod tuning;
mod window;

use std::process::ExitCode;
use std::sync::Arc;

use lume_ipc::Listener;
use lume_session::{BackendId, HostConfig, LocalBackend, Router};
use tauri::webview::PageLoadEvent;
use tauri::{RunEvent, WindowEvent};

use crate::args::{Args, Mode, Suite};
use crate::clipboard::ClipboardThread;
use crate::control::Control;
use crate::fonts::Fonts;
use crate::marks::Marks;
use crate::state::AppState;
use crate::synthetic::SyntheticPty;
use crate::window::WindowShell;

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

/// Starts the backend whose terminals run a benchmark's built-in programs. They are
/// served by a session host of their own, so that what is timed is the same path a real
/// session takes, with a source that is never the slow part.
fn bench_sessions(router: &Router) -> Option<Arc<LocalBackend>> {
    let config = HostConfig {
        pty: Arc::new(SyntheticPty),
        ..HostConfig::default()
    };
    match tauri::async_runtime::block_on(LocalBackend::in_process(config)) {
        Ok(bench) => {
            let bench = Arc::new(bench);
            router.insert(BackendId(tuning::BENCH_BACKEND.to_owned()), bench.clone());
            Some(bench)
        }
        Err(error) => {
            tracing::error!(%error, "the benchmark's session host did not start");
            None
        }
    }
}

/// The exit status for arguments that were not understood.
const EXIT_USAGE: u8 = 2;

/// Whether this process is the application, or another copy already is.
enum Instance {
    /// This is the application. It serves the endpoint, if it has one.
    First(Option<Listener>),
    /// Another copy is running and has been told to show itself.
    Second,
}

/// Claims the control endpoint, or hands over to the copy that holds it.
fn claim_instance(args: &Args) -> Result<Instance, String> {
    // The endpoint belongs to the terminal. A check of the webview starts no sessions
    // and must not stand in for the application, or be turned away by it.
    if args.mode != Mode::Terminal {
        return Ok(Instance::First(None));
    }
    let endpoint = control::endpoint(args.endpoint.as_deref())
        .map_err(|error| format!("the control endpoint cannot be named: {error}"))?;
    tauri::async_runtime::block_on(async {
        match control::bind(&endpoint) {
            Ok(Some(listener)) => Ok(Instance::First(Some(listener))),
            Ok(None) => control::activate_running(&endpoint)
                .await
                .map(|()| Instance::Second)
                .map_err(|error| {
                    format!("{endpoint} is in use, and what serves it did not answer: {error}")
                }),
            Err(error) => {
                // Without the endpoint the terminal still works; only the command-line
                // tool cannot reach it.
                tracing::error!(%error, "the control endpoint could not be served");
                Ok(Instance::First(None))
            }
        }
    })
}

fn main() -> ExitCode {
    let marks = Arc::new(Marks::start());
    log::install();
    let args = match args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(problem) => {
            eprintln!(
                "lume-gui: {problem}

{}",
                args::USAGE
            );
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let mode = args.mode;
    let listener = match claim_instance(&args) {
        Ok(Instance::First(listener)) => listener,
        Ok(Instance::Second) => return ExitCode::SUCCESS,
        Err(problem) => {
            eprintln!("lume-gui: {problem}");
            return ExitCode::FAILURE;
        }
    };

    let router = Router::new();
    // Only the terminal needs sessions on this machine.
    let locals: Vec<Arc<LocalBackend>> = match mode {
        Mode::Terminal => local_sessions(&router).into_iter().collect(),
        // A benchmark times real shells as well as the built-in programs.
        Mode::Harness(Suite::Bench) => local_sessions(&router)
            .into_iter()
            .chain(bench_sessions(&router))
            .collect(),
        Mode::Probe | Mode::Harness(_) => Vec::new(),
    };
    let runtime = tauri::async_runtime::handle().inner().clone();
    let state = AppState::new(
        runtime,
        router,
        locals,
        tuning::REATTACH_GRACE,
        Arc::clone(&marks),
    );

    let loading = Arc::clone(&state);
    let serving = Arc::clone(&state);
    let app = tauri::Builder::default()
        .manage(args)
        .manage(Arc::clone(&state))
        .manage(ClipboardThread::start())
        .manage(Fonts::default())
        .setup(move |app| {
            if matches!(mode, Mode::Harness(_)) {
                harness::start(app)?;
            }
            window::open(app, mode)?;
            marks.window();
            if let Some(listener) = listener {
                let shell = Arc::new(WindowShell::new(app.handle().clone()));
                let control = Control::new(serving, shell, tuning::PAGE_START_TIMEOUT);
                tauri::async_runtime::spawn(control::serve(listener, control));
            }
            Ok(())
        })
        .on_page_load(move |webview, payload| match payload.event() {
            // A page that starts loading replaces whatever page the webview had, and
            // that page's carrier with it. Nothing tells the old page's channel that
            // its page is gone, so the carrier is retired here.
            PageLoadEvent::Started => loading.retire(webview.label()),
            PageLoadEvent::Finished => loading.marks().page(),
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
            commands::page_reply,
            harness::harness_done,
            harness::harness_save,
            harness::harness_read,
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
    ExitCode::SUCCESS
}
