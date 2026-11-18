//! Reaching the session host that runs on its own, and starting it when nothing serves
//! its endpoint.
//!
//! The host is the `lume-host` program beside the running one. It is started detached,
//! so that it outlives the application that started it, and serves the endpoint until
//! it is asked to stop or has had nothing to do for its idle time. Two applications
//! that start it at the same moment are both served: the second host finds the
//! endpoint taken and exits, and both connect to the first.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use lume_ipc::{Endpoint, Stream};

/// The host program's file name.
pub const HOST_PROGRAM: &str = if cfg!(windows) {
    "lume-host.exe"
} else {
    "lume-host"
};

/// How long a host that was just started may take to serve its endpoint.
const START_TIMEOUT: Duration = Duration::from_secs(10);

/// The status a host exits with when another one already serves its endpoint.
pub const EXIT_IN_USE: i32 = 3;

/// How to reach the host, and how to start it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Daemon {
    /// The endpoint it serves.
    pub endpoint: String,
    /// The program to start when nothing serves the endpoint.
    pub program: PathBuf,
    /// How long it stays with no client and no running session. Zero is for ever.
    pub idle_exit: Duration,
    /// The file it logs to.
    pub log: Option<PathBuf>,
    /// A folder of its own, for the scripts that have shells report what they do.
    pub state: Option<PathBuf>,
}

impl Daemon {
    /// The host program beside the program that is running, if it is there.
    pub fn beside_this_program() -> io::Result<PathBuf> {
        let own = std::env::current_exe()?;
        let beside = own.with_file_name(HOST_PROGRAM);
        if beside.is_file() {
            Ok(beside)
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} is not there", beside.display()),
            ))
        }
    }

    /// The command line the host is started with.
    fn arguments(&self) -> Vec<String> {
        let mut arguments = vec![
            "serve".to_owned(),
            "--endpoint".to_owned(),
            self.endpoint.clone(),
            "--idle-exit".to_owned(),
            self.idle_exit.as_secs().to_string(),
        ];
        let mut path = |option: &str, path: &Option<PathBuf>| {
            if let Some(path) = path {
                arguments.push(option.to_owned());
                arguments.push(path.to_string_lossy().into_owned());
            }
        };
        path("--log", &self.log);
        path("--state", &self.state);
        arguments
    }

    /// Connects to the host, starting it first if nothing serves its endpoint. Says
    /// whether it was started here.
    pub async fn connect(&self) -> io::Result<(Stream, bool)> {
        let endpoint = Endpoint::named(&self.endpoint)?;
        match lume_ipc::connect(&endpoint).await {
            Ok(stream) => return Ok((stream, false)),
            Err(error) if lume_ipc::not_served(&error) => {}
            Err(error) => return Err(error),
        }
        self.start(&endpoint).await.map(|stream| (stream, true))
    }

    async fn start(&self, endpoint: &Endpoint) -> io::Result<Stream> {
        if !Path::new(&self.program).is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} is not there", self.program.display()),
            ));
        }
        let mut command = Command::new(&self.program);
        command.args(self.arguments());
        if let Some(state) = self.state.as_ref().filter(|dir| dir.is_dir()) {
            command.current_dir(state);
        }
        tracing::info!(endpoint = endpoint.name(), "starting the session host");
        lume_ipc::start_and_connect(endpoint, &mut command, EXIT_IN_USE, START_TIMEOUT).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_is_told_where_to_serve_log_and_keep_its_scripts() {
        let daemon = Daemon {
            endpoint: "host-dev".to_owned(),
            program: PathBuf::from("lume-host"),
            idle_exit: Duration::from_secs(60),
            log: Some(PathBuf::from("logs/host.log")),
            state: None,
        };
        assert_eq!(
            daemon.arguments(),
            [
                "serve",
                "--endpoint",
                "host-dev",
                "--idle-exit",
                "60",
                "--log",
                "logs/host.log"
            ]
        );
    }
}
