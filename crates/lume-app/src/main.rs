//! `lume-gui`: the Lume application.
//!
//! It opens the windows of a workspace, whose webviews show terminals in tabs and
//! panes. The sessions behind them run on a session host, reached through the session
//! protocol: a host inside this process, whose sessions end with it, or, if the settings
//! say so, `lume-host`, a process of its own that keeps them running after the
//! application closes and that the application starts when it is not running. Which
//! windows and panes there are is the layout's to say, the application opens the
//! windows and starts and ends the sessions, and each webview follows through commands
//! and a byte carrier.
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
//! that finds the endpoint already served hands what it was started for to the first:
//! its window comes to the front, the workspace named is shown and the program named
//! is run in a tab. Then the copy exits.

// A release build on Windows is a GUI-subsystem program and opens no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod args;
mod attention;
mod background;
mod bounds;
mod builtins;
mod carrier;
mod clipboard;
mod commands;
mod commands_config;
mod commands_layout;
mod commands_window;
mod configsvc;
mod control;
mod error;
mod fonts;
mod frames;
mod harness;
mod host;
mod log;
mod marks;
mod opener;
mod panes;
mod plugins;
mod profiles;
mod registry;
mod services;
mod sessions;
mod state;
mod synthetic;
#[cfg(test)]
mod testing;
mod tuning;
mod window;
mod workspaces;

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use lume_config::schema::{Confirm, HostMode, HostSettings};
use lume_config::{ColorScheme, Dirs};
use lume_ipc::Listener;
use lume_layout::{Intent, Store};
use lume_session::{BackendId, Daemon, HostConfig, LocalBackend, Router};
use tauri::webview::PageLoadEvent;
use tauri::{Manager, RunEvent, WindowEvent};

use crate::args::{Args, Mode, Suite};
use crate::clipboard::ClipboardThread;
use crate::configsvc::ConfigService;
use crate::control::{Control, Shell};
use crate::fonts::Fonts;
use crate::marks::Marks;
use crate::panes::Panes;
use crate::services::Services;
use crate::state::AppState;
use crate::synthetic::SyntheticPty;
use crate::window::WindowShell;

/// The backend for sessions on this machine, and what the user is to be told about it.
struct LocalSessions {
    backend: Option<Arc<LocalBackend>>,
    /// Why the sessions run inside this process when they were meant not to.
    warning: Option<String>,
}

/// Connects to the session host that runs on its own, starting it if it is not
/// running.
fn daemon_sessions(
    settings: &HostSettings,
    app_endpoint: Option<&str>,
) -> Result<LocalBackend, String> {
    let program = Daemon::beside_this_program()
        .map_err(|error| format!("the session host's program is missing ({error})"))?;
    let daemon = Daemon {
        endpoint: lume_proto::host_endpoint_name(app_endpoint),
        program,
        idle_exit: Duration::from_secs(u64::from(settings.idle_exit)),
        log: lume_config::host_log_file(),
        state: Dirs::resolve().map(|dirs| dirs.state),
    };
    // The backend's tasks live on Tauri's runtime, which is the one commands run on.
    tauri::async_runtime::block_on(LocalBackend::daemon(daemon)).map_err(|error| error.to_string())
}

/// Starts the backend for sessions on this machine and puts it in a router: inside this
/// process, unless the settings ask for the session host that runs on its own and it
/// can be reached.
fn local_sessions(
    router: &Router,
    settings: Option<&HostSettings>,
    app_endpoint: Option<&str>,
) -> LocalSessions {
    let mut warning = None;
    if let Some(settings) = settings.filter(|settings| settings.mode == HostMode::Daemon) {
        match daemon_sessions(settings, app_endpoint) {
            Ok(local) => {
                let local = Arc::new(local);
                router.insert(Router::local_id(), local.clone());
                return LocalSessions {
                    backend: Some(local),
                    warning: None,
                };
            }
            Err(problem) => {
                tracing::error!(
                    problem,
                    "the session host could not be reached; sessions run inside the application"
                );
                warning = Some(format!("Programs will end when Lume closes: {problem}."));
            }
        }
    }
    let config = HostConfig {
        // Without a folder of its own the host asks no shell to report anything.
        shell_integration_dir: Dirs::resolve()
            .map(|dirs| dirs.state.join(tuning::SHELL_INTEGRATION_DIR)),
        ..HostConfig::default()
    };
    let started = tauri::async_runtime::block_on(LocalBackend::in_process(config));
    let backend = match started {
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
    };
    LocalSessions { backend, warning }
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

/// What an application that shows terminals has besides its sessions.
struct Terminals {
    panes: Arc<Panes>,
    config: Arc<ConfigService>,
    /// What the command line asked to have run, if the workspace was there already:
    /// the program and the directory to start it in. In a workspace that is begun by
    /// this start the program has the first pane instead.
    exec: Option<(Vec<String>, Option<String>)>,
}

/// What a start asks for besides the window: a workspace to show and a program to run.
#[derive(Clone, Debug, Default)]
struct Launch {
    workspace: Option<String>,
    exec: Vec<String>,
    cwd: Option<String>,
}

impl Launch {
    fn of(args: &Args) -> Self {
        // A program is started where the command was typed, unless it says otherwise.
        let here = || {
            std::env::current_dir()
                .ok()
                .map(|dir| dir.to_string_lossy().into_owned())
        };
        Self {
            workspace: args.workspace.clone(),
            exec: args.exec.clone(),
            cwd: if args.exec.is_empty() {
                None
            } else {
                args.cwd.clone().or_else(here)
            },
        }
    }
}

/// Reads the user's configuration. Which session host to use and which workspace to
/// show are the user's own settings, so they are read before any workspace's settings
/// are laid over the user's.
fn user_config() -> Result<Arc<ConfigService>, String> {
    let dirs = Dirs::resolve().ok_or_else(|| {
        "the folders for the configuration and the saved workspaces could not be found".to_owned()
    })?;
    let config = ConfigService::load(dirs, None);
    config.ensure_default_file();
    Ok(config)
}

/// Reads the workspace to show, and puts the workspace's panes in the application's
/// charge.
fn terminals(
    state: &Arc<AppState>,
    config: Arc<ConfigService>,
    launch: &Launch,
) -> Result<Terminals, String> {
    let dirs = config.dirs().clone();
    if let Some(name) = &launch.workspace
        && !lume_layout::valid_name(name)
    {
        return Err(format!("`{name}` cannot be a workspace's name"));
    }
    let name = workspaces::start_workspace(
        launch.workspace.as_deref(),
        &config.current().config.app.start_workspace,
        workspaces::last_workspace(&dirs),
    );
    config.set_workspace(Some(&name));
    let store = Store::new(dirs.state_workspaces_dir());
    let first = services::exec_request(&launch.exec, launch.cwd.clone());
    let (panes, fresh) = tauri::async_runtime::block_on(Panes::open(
        state,
        Arc::clone(&config),
        store,
        &name,
        first,
    ));
    workspaces::remember_workspace(&dirs, &panes.workspace().name);
    state.set_panes(Arc::clone(&panes));
    let exec =
        (!fresh && !launch.exec.is_empty()).then(|| (launch.exec.clone(), launch.cwd.clone()));
    Ok(Terminals {
        panes,
        config,
        exec,
    })
}

/// The colour scheme a window's theme is.
fn scheme_of(theme: tauri::Theme) -> ColorScheme {
    match theme {
        tauri::Theme::Light => ColorScheme::Light,
        _ => ColorScheme::Dark,
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
fn claim_instance(args: &Args, launch: &Launch) -> Result<Instance, String> {
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
            Ok(None) => control::launch_running(
                &endpoint,
                launch.workspace.clone(),
                launch.exec.clone(),
                launch.cwd.clone(),
            )
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
    let launch = Launch::of(&args);
    let listener = match claim_instance(&args, &launch) {
        Ok(Instance::First(listener)) => listener,
        Ok(Instance::Second) => return ExitCode::SUCCESS,
        Err(problem) => {
            eprintln!("lume-gui: {problem}");
            return ExitCode::FAILURE;
        }
    };

    let user = match mode {
        Mode::Terminal => match user_config() {
            Ok(config) => Some(config),
            Err(problem) => {
                eprintln!("lume-gui: {problem}");
                return ExitCode::FAILURE;
            }
        },
        Mode::Probe | Mode::Harness(_) => None,
    };
    let host_settings = user
        .as_ref()
        .map(|config| config.current().config.host.clone());

    let router = Router::new();
    // Only the terminal needs sessions on this machine. A benchmark times real shells
    // as well as the built-in programs, on hosts in this process, which it leaves as it
    // found them.
    let mut host_warning = None;
    let locals: Vec<Arc<LocalBackend>> = match mode {
        Mode::Terminal => {
            let local = local_sessions(&router, host_settings.as_ref(), args.endpoint.as_deref());
            host_warning = local.warning;
            local.backend.into_iter().collect()
        }
        Mode::Harness(Suite::Bench) => local_sessions(&router, None, None)
            .backend
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

    let terminals = match user {
        Some(config) => match terminals(&state, config, &launch) {
            Ok(terminals) => Some(terminals),
            Err(problem) => {
                eprintln!("lume-gui: {problem}");
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };
    if let (Some(terminals), Some(warning)) = (&terminals, host_warning) {
        terminals.panes.notice("warning", warning);
    }
    if let Some(terminals) = &terminals {
        host::tell_of_an_older_host(&state, &terminals.panes);
    }
    let config = terminals
        .as_ref()
        .map(|terminals| Arc::clone(&terminals.config));

    let loading = Arc::clone(&state);
    let serving = Arc::clone(&state);
    let app = tauri::Builder::default()
        // Notifications are shown from the application's side only; no page is granted
        // the plugin's commands.
        .plugin(tauri_plugin_notification::init())
        .manage(args)
        .manage(Arc::clone(&state))
        .manage(ClipboardThread::start())
        .manage(Fonts::default())
        .setup(move |app| {
            if matches!(mode, Mode::Harness(_)) {
                harness::start(app)?;
            }
            let Some(Terminals {
                panes,
                config,
                exec,
            }) = terminals
            else {
                window::open(app, mode)?;
                marks.window();
                return Ok(());
            };
            let windows = WindowShell::new(
                app.handle().clone(),
                Arc::clone(&serving),
                Arc::clone(&config),
            );
            app.manage(Arc::clone(&windows));
            let shell: Arc<dyn Shell> = windows;
            // The layout says which windows there are and where each was.
            for window in &panes.workspace().windows {
                shell.open_window(window.id, window.bounds);
            }
            marks.window();
            let registry = Arc::new(builtins::registry());
            let plugins = plugins::Plugins::new(
                config.dirs(),
                Arc::clone(&registry),
                Arc::clone(&serving),
                Arc::clone(&config),
            );
            let services = Arc::new(Services {
                state: Arc::clone(&serving),
                panes,
                config,
                registry,
                shell: Arc::clone(&shell),
                plugins: Arc::clone(&plugins),
            });
            app.manage(Arc::clone(&services));
            // Plugins are read once the first terminal has shown its output.
            plugins.start(&services);
            // The theme follows the system's scheme, which only a window can tell.
            let scheme = app
                .webview_windows()
                .values()
                .find_map(|window| window.theme().ok());
            if let Some(theme) = scheme {
                services.config.set_scheme(scheme_of(theme));
            }

            // A change of the configuration reaches the pages, and what the application
            // itself draws from it follows: the windows' frames, and where the list of
            // tabs is.
            let telling = Arc::clone(&serving);
            let following = Arc::downgrade(&services);
            let frames = frames::Frames::new(&services.config);
            services.config.watch(move |notice| {
                let changed = notice["type"] == "config_changed";
                telling.broadcast(&notice);
                let Some(services) = following.upgrade().filter(|_| changed) else {
                    return;
                };
                services.panes.profiles.config_changed();
                services.plugins.config_changed();
                frames.follow(&services.config, services.shell.as_ref());
                let panes = Arc::clone(&services.panes);
                telling
                    .runtime()
                    .spawn(async move { panes.sync_tab_panel().await });
            });
            // What is installed on the machine is looked for off to the side, and the
            // pages are told what was found.
            let telling = Arc::clone(&serving);
            services
                .panes
                .profiles
                .start(serving.runtime(), move |notice| telling.broadcast(&notice));
            let attending = Arc::clone(&shell);
            services.panes.run(shell);
            // A bell, or a long command that ends, in a window the user is not using.
            attention::watch(&services, attending);
            if let Some((exec, cwd)) = exec {
                let launching = Arc::clone(&services);
                tauri::async_runtime::spawn(async move {
                    let launched = launching
                        .launch(None, &exec, cwd, tuning::PAGE_START_TIMEOUT)
                        .await;
                    if let Err(error) = launched {
                        tracing::error!(%error, "the program the application was started with could not be run");
                        launching.panes.notice(
                            "error",
                            format!("{} could not be started: {}", exec[0], error.message),
                        );
                    }
                });
            }
            if let Some(listener) = listener {
                let control = Control::new(services, tuning::PAGE_START_TIMEOUT);
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
            commands_layout::layout_view,
            commands_layout::layout_apply,
            commands_layout::sessions_start,
            commands_layout::panes_busy,
            commands_layout::sessions_detached,
            commands_layout::commands_list,
            commands_layout::command_run,
            commands_config::config_view,
            commands_config::config_set,
            commands_config::config_unset,
            commands_config::config_open,
            commands_config::config_schema,
            commands_config::themes_list,
            commands_config::shells_list,
            commands_config::profiles_list,
            commands_config::background_image,
            commands_window::window_drag,
            commands_window::window_control,
            commands_window::window_state,
            commands_window::workspaces_list,
            harness::harness_done,
            harness::harness_save,
            harness::harness_read,
        ])
        .build(tauri::generate_context!())
        .expect("error while starting Lume");

    app.run(move |app, event| match event {
        RunEvent::WindowEvent { label, event, .. } => {
            let windows = app.try_state::<Arc<WindowShell>>();
            let windows = windows.as_ref().map(|windows| windows.inner());
            match event {
                WindowEvent::Destroyed => {
                    state.retire(&label);
                    if let Some(windows) = windows {
                        windows.forget(&label);
                        // The settings do not outlive the last window with terminals.
                        windows.close_settings_with(&label);
                    }
                }
                WindowEvent::ThemeChanged(theme) => {
                    if let Some(config) = &config {
                        config.set_scheme(scheme_of(theme));
                    }
                }
                WindowEvent::Focused(focused) => {
                    // The layout remembers which window the user is working in: a
                    // request that names no window means that one.
                    if focused
                        && let (Some(panes), Some(window)) =
                            (state.panes(), window::window_of(&label))
                    {
                        let panes = Arc::clone(panes);
                        tauri::async_runtime::spawn(async move {
                            let _ = panes.apply_core(Intent::WindowFocus { window }).await;
                        });
                    }
                    if let Some(windows) = windows {
                        windows.tell_state(&label);
                    }
                }
                WindowEvent::Resized(_) => {
                    if let Some(windows) = windows {
                        windows.tell_state(&label);
                        windows.moved(&label);
                    }
                }
                WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. } => {
                    if let Some(windows) = windows {
                        windows.moved(&label);
                    }
                }
                WindowEvent::CloseRequested { api, .. } => {
                    let (Some(panes), Some(window)) = (state.panes(), window::window_of(&label))
                    else {
                        return;
                    };
                    // A window in which something still runs may be set to ask first.
                    // Its page puts the question, and closes the window again with the
                    // user's agreement, which is used up here.
                    let always = config.as_ref().is_some_and(|config| {
                        config.current().config.window.confirm_close == Confirm::Always
                    });
                    let asked = windows.is_some_and(|windows| windows.take_confirmed(&label));
                    let page = state.page_of(&label);
                    // The last window closes the application, which ends nothing when the
                    // sessions run in a host of their own: they are there at the next
                    // start. Any other window ends what ran in it.
                    let last = panes.workspace().windows.len() <= 1;
                    let kept = last && state.locals().iter().any(|local| local.is_daemon());
                    if window::close_asks(
                        always,
                        panes.runs_something(window) && !kept,
                        page.is_some(),
                        asked,
                    ) {
                        api.prevent_close();
                        if let Some(page) = page {
                            page.notify(&serde_json::json!({ "type": "close_requested" }));
                        }
                        return;
                    }
                    if let Some(windows) = windows {
                        windows.keep_place(&label);
                    }
                    // The only window closes the application, and the layout stays as
                    // it is for the next start. Any other window is closed in the
                    // layout, which ends what ran in it and then closes it here.
                    if panes.workspace().windows.len() > 1 {
                        api.prevent_close();
                        let panes = Arc::clone(panes);
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) = panes.apply(Intent::WindowClose { window }).await {
                                tracing::debug!(%window, %error, "a window could not be closed");
                            }
                        });
                    }
                }
                _ => {}
            }
        }
        RunEvent::ExitRequested { .. } => {
            // Where each window is goes into the workspace's file with the rest.
            if let Some(windows) = app.try_state::<Arc<WindowShell>>() {
                for label in windows.layout_windows() {
                    windows.keep_place(&label);
                }
            }
        }
        RunEvent::Exit => {
            // The workspace is written while its panes are all still there, with the
            // sessions they show: a session host of its own keeps them running for
            // the next start, and a host in this process ends them next, because the
            // application is closing.
            if let Some(panes) = state.panes() {
                panes.close();
            }
            // Plugins' code hears that it is stopping.
            if let Some(services) = app.try_state::<Arc<Services>>() {
                services.plugins.shutdown();
            }
            // A program in a terminal can outlive a process that exits without closing
            // the terminal, so a host in this process ends its sessions before the
            // process exits. A host of its own is only let go of.
            tauri::async_runtime::block_on(state.shutdown());
        }
        _ => {}
    });
    ExitCode::SUCCESS
}
