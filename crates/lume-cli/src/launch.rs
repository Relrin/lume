//! `lume` on its own: start the application, or bring the running one to the front.
//!
//! The application is the program beside this one. It is started so that it outlives
//! the console `lume` was typed in, and `lume` returns once the application answers on
//! its control endpoint, so a command that follows finds it running.
//!
//! A workspace to show, a Rex server to show a session of and a program to run in a tab
//! go to the application either way: as a request if it is running, on its command line if it is started here.

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

/// What the application is started for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Start {
    /// The name its control endpoint is served under, when it is not the default.
    pub endpoint: Option<String>,
    /// The workspace to show.
    pub workspace: Option<String>,
    /// The Rex server to show a session of; empty for the one of this machine.
    pub rex: Option<String>,
    /// A program to run in a new tab, and its arguments. Empty for none.
    pub exec: Vec<String>,
}

/// The directory a program given to run is started in: the one this was typed in.
fn here() -> Option<String> {
    std::env::current_dir()
        .ok()
        .map(|dir| dir.to_string_lossy().into_owned())
}

/// What the application is given on its command line for a start.
fn arguments(start: &Start, cwd: Option<&str>) -> Vec<String> {
    let mut arguments = Vec::new();
    // Joined with `=`, so that a value that begins with a dash is still a value.
    if let Some(name) = &start.endpoint {
        arguments.push(format!("--endpoint={name}"));
    }
    if let Some(workspace) = &start.workspace {
        arguments.push(format!("--workspace={workspace}"));
    }
    match start.rex.as_deref() {
        None => {}
        Some("") => arguments.push("--rex".to_owned()),
        Some(server) => arguments.push(format!("--rex={server}")),
    }
    if !start.exec.is_empty() {
        if let Some(cwd) = cwd {
            arguments.push(format!("--cwd={cwd}"));
        }
        // Last: everything after it is the program's.
        arguments.push("-e".to_owned());
        arguments.extend(start.exec.iter().cloned());
    }
    arguments
}

/// Starts the application so that nothing ties it to this process or its console.
fn spawn_detached(start: &Start, cwd: Option<&str>) -> io::Result<()> {
    let mut command = Command::new(application()?);
    command.args(arguments(start, cwd));
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
    // Nor does it keep a copy of what this process prints into: whoever reads that
    // would wait for the application to end.
    lume_ipc::keep_standard_handles();
    // The child is not waited for: it is the application, and outlives this process.
    command.spawn().map(drop)
}

/// Brings the running application to the front, or starts it and waits until it
/// answers. Either way it shows the workspace named and runs the program named.
pub async fn start(start: Start) -> Result<(), String> {
    let endpoint = endpoint(start.endpoint.as_deref()).map_err(|error| error.to_string())?;
    let cwd = if start.exec.is_empty() { None } else { here() };
    let launch = || async {
        let mut client = Client::connect(&endpoint).await?;
        let request = AppRequestBody::Launch {
            workspace: start.workspace.clone(),
            exec: start.exec.clone(),
            cwd: cwd.clone(),
            rex: start.rex.clone(),
        };
        client.ask(request, ANSWER_TIMEOUT).await.map(drop)
    };
    match launch().await {
        Ok(()) => return Ok(()),
        Err(Failure::NotRunning(_)) => {}
        Err(Failure::Broken(error)) => {
            return Err(format!("the running application did not answer: {error}"));
        }
        Err(Failure::Refused(error)) => return Err(error.to_string()),
    }

    spawn_detached(&start, cwd.as_deref())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_application_is_given_what_it_was_started_for() {
        assert_eq!(
            arguments(&Start::default(), Some("/work")),
            Vec::<String>::new()
        );
        let start = Start {
            endpoint: Some("trial".to_owned()),
            workspace: Some("site v2".to_owned()),
            rex: None,
            exec: vec!["htop".to_owned(), "--delay".to_owned(), "5".to_owned()],
        };
        assert_eq!(
            arguments(&start, Some("/work")),
            [
                "--endpoint=trial",
                "--workspace=site v2",
                "--cwd=/work",
                "-e",
                "htop",
                "--delay",
                "5"
            ]
        );
        // Without a program there is no directory to start it in.
        let start = Start {
            exec: Vec::new(),
            ..start
        };
        assert_eq!(
            arguments(&start, Some("/work")),
            ["--endpoint=trial", "--workspace=site v2"]
        );
        for (rex, argument) in [("", "--rex"), ("-odd name", "--rex=-odd name")] {
            let start = Start {
                rex: Some(rex.to_owned()),
                ..Start::default()
            };
            assert_eq!(arguments(&start, None), [argument]);
        }
    }
}
