//! `lume`: the console command.
//!
//! On its own it starts the application, or brings its window to the front if it is
//! running. `lume ctl` drives the running application: it starts sessions, types into
//! them, waits for text or for an exit, reads the screen, and so on. Each command is one
//! request on the application's control endpoint, which only the user who runs the
//! application can reach.

mod args;
mod client;
mod ctl;
mod launch;

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
    let (start, ctl) = match command {
        Command::Help => {
            println!("{}", args::USAGE);
            return ExitCode::SUCCESS;
        }
        Command::Version => {
            println!("lume {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Command::Start { endpoint } => (Some(endpoint), None),
        Command::Ctl(ctl) => (None, Some(ctl)),
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
        match launch::start(start.flatten().as_deref()).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(problem) => {
                eprintln!("lume: {problem}");
                ExitCode::FAILURE
            }
        }
    })
}
