//! `lume`: the console command.
//!
//! On its own it starts the application, or brings its window to the front if it is
//! running; it can name the workspace to show and a program to run in a tab. `lume ctl` drives the running application: it starts sessions, types into
//! them, waits for text or for an exit, reads the screen, and so on. Each command is one
//! request on the application's control endpoint, which only the user who runs the
//! application can reach.
//!
//! `lume host` says whether the session host runs, and starts or stops it.
//!
//! `lume plugin` installs plugins and changes what is installed.
//!
//! A word that is none of these is a verb a plugin adds: it is run in the application,
//! and what it prints is printed here.
//!
//! `lume config`, `lume keymap` and `lume theme` read and write the configuration's
//! files and need no application: they check a file, list the keys that are bound, and
//! make a theme from another terminal's colour scheme.

mod args;
mod attach;
mod client;
mod console;
mod ctl;
mod direct;
mod host;
mod launch;
mod offline;
mod plugin;
mod verb;

use std::process::ExitCode;

use crate::args::Command;

/// The exit status for arguments that were not understood.
const EXIT_USAGE: u8 = 2;

fn main() -> ExitCode {
    let command = match args::parse(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(problem) => {
            eprintln!("lume: {problem}\n\n{}", args::USAGE);
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let (start, ctl, verb) = match command {
        Command::Help => {
            println!("{}", args::USAGE);
            return ExitCode::SUCCESS;
        }
        Command::Version => {
            println!("lume {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Command::Offline(command) => {
            let dirs = lume_config::Dirs::resolve();
            let (mut out, mut err) = (std::io::stdout().lock(), std::io::stderr().lock());
            return ExitCode::from(offline::run(command, dirs.as_ref(), &mut out, &mut err));
        }
        Command::Plugin(command) => {
            use std::io::IsTerminal as _;
            let dirs = lume_config::Dirs::resolve();
            let interactive = std::io::stdin().is_terminal();
            let mut input = std::io::stdin().lock();
            let mut asker = plugin::Asker {
                input: &mut input,
                interactive,
            };
            let (mut out, mut err) = (std::io::stdout().lock(), std::io::stderr().lock());
            return ExitCode::from(plugin::run(
                command,
                dirs.as_ref(),
                &mut asker,
                &mut out,
                &mut err,
            ));
        }
        Command::Start {
            endpoint,
            workspace,
            exec,
        } => (
            Some(launch::Start {
                endpoint,
                workspace,
                exec,
            }),
            None,
            None,
        ),
        Command::Ctl(ctl) => (None, Some(ctl), None),
        Command::Verb { endpoint, words } => (None, None, Some((endpoint, words))),
        Command::Host(command) => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            return match runtime {
                Ok(runtime) => ExitCode::from(runtime.block_on(host::run(command))),
                Err(error) => {
                    eprintln!("lume: {error}");
                    ExitCode::FAILURE
                }
            };
        }
    };

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("lume: {error}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(async {
        if let Some(ctl) = ctl {
            return ExitCode::from(ctl::run(ctl).await);
        }
        if let Some((endpoint, words)) = verb {
            return ExitCode::from(verb::run(endpoint.as_deref(), words).await);
        }
        match launch::start(start.unwrap_or_default()).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(problem) => {
                eprintln!("lume: {problem}");
                ExitCode::FAILURE
            }
        }
    })
}
