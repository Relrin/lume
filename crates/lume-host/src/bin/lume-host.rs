//! `lume-host`: the session host as a process of its own.
//!
//! `lume-host serve --endpoint <name>` is the host the application uses. It serves any
//! number of clients on the named per-user endpoint, and the sessions it runs outlive
//! the clients that started them: the application can close, crash or be replaced by a
//! newer one, and the shells carry on. It exits when a client asks it to stop, or once
//! it has had no client and no running session for the idle time. A second host started
//! on the same endpoint finds it served and exits at once, with status 3.
//!
//! `lume-host --endpoint <name>` serves one client and exits when that client
//! disconnects. The sessions the client started end with it.
//!
//! The host logs what it does, never what passes through a terminal, to the file
//! `--log` names (nothing is logged without one). `LUME_LOG` sets how much, as it does
//! for the application.

// A release build on Windows is a GUI-subsystem program: it opens no console window,
// and none is left behind when it is started by a program that has one.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use lume_host::tuning::{IDLE_EXIT_DEFAULT, IDLE_RECHECK, LOG_FILE_MAX_BYTES, STOP_WAIT};
use lume_host::{Host, HostConfig};
use lume_ipc::{Endpoint, Listener};
use tokio::time::Instant;

const USAGE: &str = "\
usage: lume-host serve --endpoint <name> [--idle-exit <seconds>] [--log <file>] [--state <dir>]
       lume-host --endpoint <name>
       lume-host --version

serve runs the host for any number of clients; sessions outlive the clients that
started them. --idle-exit is how long it stays with no client and no running session
(60 if not given, 0 for ever). --log names the file it logs to. --state names a folder
of its own, where it writes the scripts that have shells report what they do.

With --endpoint alone it serves one client, and the sessions end with that client.";

/// The exit status for arguments that were not understood.
const EXIT_USAGE: u8 = 2;

/// The exit status when another host already serves the endpoint.
const EXIT_IN_USE: u8 = 3;

/// The folder under `--state` that the shell scripts go in.
const SHELL_INTEGRATION_DIR: &str = "shell-integration";

/// The variable that sets how much is logged.
const LEVEL_VARIABLE: &str = "LUME_LOG";

/// What the host was asked to do.
enum Run {
    /// Serve many clients until stopped or idle.
    Serve(Serve),
    /// Serve one client.
    One {
        endpoint: String,
    },
    Version,
}

struct Serve {
    endpoint: String,
    idle_exit: Option<Duration>,
    log: Option<PathBuf>,
    state: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Run, String> {
    let mut words = args.iter();
    let value = |words: &mut std::slice::Iter<'_, String>, option: &str| {
        words
            .next()
            .cloned()
            .ok_or_else(|| format!("{option} needs a value"))
    };
    match args.first().map(String::as_str) {
        Some("--version") if args.len() == 1 => return Ok(Run::Version),
        Some("--endpoint") if args.len() == 2 => {
            return Ok(Run::One {
                endpoint: args[1].clone(),
            });
        }
        Some("serve") => {
            words.next();
        }
        _ => return Err("not understood".to_owned()),
    }
    let mut serve = Serve {
        endpoint: String::new(),
        idle_exit: Some(IDLE_EXIT_DEFAULT),
        log: None,
        state: None,
    };
    while let Some(word) = words.next() {
        match word.as_str() {
            "--endpoint" => serve.endpoint = value(&mut words, "--endpoint")?,
            "--idle-exit" => {
                let text = value(&mut words, "--idle-exit")?;
                let seconds: u64 = text
                    .parse()
                    .map_err(|_| format!("{text} is not a whole number of seconds"))?;
                serve.idle_exit = (seconds > 0).then(|| Duration::from_secs(seconds));
            }
            "--log" => serve.log = Some(PathBuf::from(value(&mut words, "--log")?)),
            "--state" => serve.state = Some(PathBuf::from(value(&mut words, "--state")?)),
            other => return Err(format!("serve does not take {other}")),
        }
    }
    if serve.endpoint.is_empty() {
        return Err("serve needs --endpoint".to_owned());
    }
    Ok(Run::Serve(serve))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let run = match parse(&args) {
        Ok(run) => run,
        Err(problem) => {
            eprintln!("lume-host: {problem}\n\n{USAGE}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("lume-host: {error}");
            return ExitCode::FAILURE;
        }
    };
    let result = match run {
        Run::Version => {
            println!("lume-host {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Run::One { endpoint } => runtime.block_on(serve_one(&endpoint)),
        Run::Serve(serve) => {
            if let Some(log) = &serve.log {
                install_log(log);
            }
            runtime.block_on(serve_many(serve))
        }
    };
    // Whatever the sessions left running on their own threads goes with the process.
    runtime.shutdown_timeout(Duration::from_millis(100));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
            tracing::info!("another host already serves the endpoint");
            ExitCode::from(EXIT_IN_USE)
        }
        Err(error) => {
            tracing::error!(%error, "the host stopped");
            eprintln!("lume-host: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn serve_one(name: &str) -> io::Result<()> {
    let endpoint = Endpoint::named(name)?;
    let mut listener = Listener::bind(&endpoint)?;
    let client = listener.accept().await?;
    // One client only: stop listening so a second cannot wait for ever.
    drop(listener);
    Host::new(HostConfig::default())
        .serve(client)
        .await
        .map_err(io::Error::other)
}

async fn serve_many(serve: Serve) -> io::Result<()> {
    let endpoint = Endpoint::named(&serve.endpoint)?;
    let mut listener = Listener::bind(&endpoint)?;
    let host = Host::new(HostConfig {
        persistent: true,
        idle_exit: serve.idle_exit,
        shell_integration_dir: serve.state.map(|dir| dir.join(SHELL_INTEGRATION_DIR)),
        ..HostConfig::default()
    });
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        pid = std::process::id(),
        endpoint = endpoint.name(),
        idle_exit_s = serve.idle_exit.map(|wait| wait.as_secs()),
        "the session host is serving"
    );

    let mut changes = host.idle_changes();
    // When the host last became idle, while it still is.
    let mut idle_since = host.is_idle().then(Instant::now);
    // When to look again whether the host is still idle, while a connection that may
    // make it busy is open.
    let mut recheck: Option<Instant> = None;
    loop {
        let deadline = recheck.or_else(|| {
            serve
                .idle_exit
                .zip(idle_since)
                .map(|(wait, since)| since + wait)
        });
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(stream) => {
                    // Whether the client keeps the host busy is known once it has said
                    // what it is; the host does not exit while it is connected.
                    let serving = Arc::clone(&host);
                    tokio::spawn(async move {
                        if let Err(error) = serving.serve(stream).await {
                            tracing::warn!(%error, "a client's connection failed");
                        }
                    });
                }
                Err(error) => {
                    tracing::warn!(%error, "a client could not be accepted");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
            changed = changes.changed() => {
                if changed.is_err() {
                    break;
                }
                recheck = None;
                idle_since = if host.is_idle() {
                    idle_since.or_else(|| Some(Instant::now()))
                } else {
                    None
                };
            }
            () = sleep_until(deadline), if deadline.is_some() => {
                recheck = None;
                if !host.is_idle() {
                    idle_since = None;
                } else if host.connections() > 0 {
                    // Someone has just connected, or is asking a question: look again
                    // in a moment.
                    recheck = Some(Instant::now() + IDLE_RECHECK);
                } else {
                    tracing::info!("the host had nothing to do and is exiting");
                    break;
                }
            }
            () = host.stopped() => {
                tracing::info!("the host is stopping, as asked");
                break;
            }
        }
    }
    // Nobody new; those still connected are let go of, and given a moment to be.
    drop(listener);
    host.stop();
    let deadline = Instant::now() + STOP_WAIT;
    while host.clients() > 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// How much to log, as `LUME_LOG` spells it. `None` is no logging at all.
fn level(spelling: Option<&str>) -> Option<tracing::Level> {
    use tracing::Level;
    match spelling.map(str::trim).unwrap_or("info") {
        "off" | "none" => None,
        "error" => Some(Level::ERROR),
        "warn" => Some(Level::WARN),
        "debug" => Some(Level::DEBUG),
        "trace" => Some(Level::TRACE),
        _ => Some(Level::INFO),
    }
}

/// Opens the log file for appending. A file that has grown past its limit is set aside
/// first, replacing the one set aside before.
fn open_log(path: &Path) -> io::Result<File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    if fs::metadata(path).is_ok_and(|meta| meta.len() > LOG_FILE_MAX_BYTES) {
        let mut aside = path.as_os_str().to_owned();
        aside.push(".old");
        let _ = fs::rename(path, aside);
    }
    OpenOptions::new().create(true).append(true).open(path)
}

/// Logs to `path` for the rest of the process. A file that cannot be opened means no
/// log: the host runs on without one.
fn install_log(path: &Path) {
    let Some(level) = level(std::env::var(LEVEL_VARIABLE).ok().as_deref()) else {
        return;
    };
    let Ok(file) = open_log(path) else {
        return;
    };
    let file = Arc::new(Mutex::new(file));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_ansi(false)
        .with_writer(move || LogFile(Arc::clone(&file)))
        .finish();
    if tracing::subscriber::set_global_default(subscriber).is_ok() {
        // A host that fails this way has nobody watching its standard error.
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            tracing::error!(%panic, "the session host failed");
            default_hook(panic);
        }));
    }
}

struct LogFile(Arc<Mutex<File>>);

impl io::Write for LogFile {
    fn write(&mut self, line: &[u8]) -> io::Result<usize> {
        let mut file = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        // A log that cannot be written is not a reason to fail.
        let _ = file.write_all(line);
        Ok(line.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn the_arguments_say_how_to_serve() {
        let Ok(Run::Serve(serve)) = parse(&words(
            "serve --endpoint host-dev --idle-exit 5 --log h.log --state s",
        )) else {
            panic!("serve was not understood");
        };
        assert_eq!(serve.endpoint, "host-dev");
        assert_eq!(serve.idle_exit, Some(Duration::from_secs(5)));
        assert_eq!(serve.log, Some(PathBuf::from("h.log")));
        assert_eq!(serve.state, Some(PathBuf::from("s")));

        let Ok(Run::Serve(serve)) = parse(&words("serve --endpoint h --idle-exit 0")) else {
            panic!("serve was not understood");
        };
        assert_eq!(serve.idle_exit, None);
        let Ok(Run::Serve(serve)) = parse(&words("serve --endpoint h")) else {
            panic!("serve was not understood");
        };
        assert_eq!(serve.idle_exit, Some(IDLE_EXIT_DEFAULT));

        assert!(matches!(parse(&words("--endpoint h")), Ok(Run::One { .. })));
        assert!(matches!(parse(&words("--version")), Ok(Run::Version)));
        for wrong in [
            "",
            "serve",
            "serve --endpoint",
            "serve --endpoint h --idle-exit soon",
            "serve --endpoint h --loud",
            "--endpoint",
            "--endpoint h extra",
        ] {
            assert!(parse(&words(wrong)).is_err(), "{wrong:?} was understood");
        }
    }
}
