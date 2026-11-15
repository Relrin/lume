//! `lume` on its own: start the application, or bring the running one to the front.
//!
//! The application is the program beside this one. It is started so that it outlives
//! the console `lume` was typed in, and `lume` returns once the application answers on
//! its control endpoint, so a command that follows finds it running.

use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use lume_proto::app::AppRequestBody;

use crate::client::{ANSWER_TIMEOUT, Client, Failure, endpoint};

/// The application's file name.
const APPLICATION: &str = if cfg!(windows) {
    "lume-gui.exe"
} else {
    "lume-gui"
};

/// How long the application may take to start answering.
const START_TIMEOUT: Duration = Duration::from_secs(20);

/// How often the endpoint is tried while the application starts.
const START_POLL: Duration = Duration::from_millis(50);

/// Where the application is: beside this program.
fn application() -> io::Result<PathBuf> {
    let own = std::env::current_exe()?;
    let beside = own.with_file_name(APPLICATION);
    if beside.is_file() {
        Ok(beside)
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is not there", beside.display()),
        ))
    }
}

/// Starts the application so that nothing ties it to this process or its console.
fn spawn_detached(name: Option<&str>) -> io::Result<()> {
    let mut command = Command::new(application()?);
    if let Some(name) = name {
        command.args(["--endpoint", name]);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console of ours, and no share in the Ctrl+C meant for whoever started us.
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A group of its own: the terminal's signals for our group pass it by.
        command.process_group(0);
    }
    // The child is not waited for: it is the application, and outlives this process.
    command.spawn().map(drop)
}

/// Brings the running application to the front, or starts it and waits until it answers.
pub async fn start(name: Option<&str>) -> Result<(), String> {
    let endpoint = endpoint(name).map_err(|error| error.to_string())?;
    let activate = || async {
        let mut client = Client::connect(&endpoint).await?;
        client
            .ask(AppRequestBody::Activate {}, ANSWER_TIMEOUT)
            .await
            .map(drop)
    };
    match activate().await {
        Ok(()) => return Ok(()),
        Err(Failure::NotRunning(_)) => {}
        Err(Failure::Broken(error)) => {
            return Err(format!("the running application did not answer: {error}"));
        }
        Err(Failure::Refused(error)) => return Err(error.to_string()),
    }

    spawn_detached(name)
        .map_err(|error| format!("the application could not be started: {error}"))?;
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        match Client::connect(&endpoint).await {
            Ok(_) => return Ok(()),
            Err(Failure::NotRunning(_)) if Instant::now() < deadline => {
                tokio::time::sleep(START_POLL).await;
            }
            Err(Failure::NotRunning(_)) => {
                return Err("the application was started but does not answer".to_owned());
            }
            Err(Failure::Broken(error)) => return Err(error.to_string()),
            Err(Failure::Refused(error)) => return Err(error.to_string()),
        }
    }
}
