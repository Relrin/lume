//! Programs that plugins run on the machine of a server elsewhere.
//!
//! A server that is connected to is one more machine that sessions are on. What it says
//! of where its sessions are is its own word, of a path of its own machine; what it runs
//! is run there, through the backend's runs. The server of this machine is not one:
//! its processes and folders are this machine's.

use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;
use lume_proto::BackendId;
use lume_rex::{Link, RexBackend, RunAsked, RunError, RunLimits, StartedRun};
use lume_services::ServiceError;
use lume_services::posix::{Asked, FOLDER_MISSING, Limits, PROGRAM_MISSING, Spelling};
use lume_services::process::{RunOutput, Running, Stop};
use lume_session::SessionId;

use super::hosts::Hosts;
use crate::machines::{Begun, CANNOT_RUN, NOT_CONNECTED, Runner, Served};

/// How often a run is looked at for whether it is still wanted.
const STOP_LOOK: Duration = Duration::from_millis(50);

/// A server that is connected to, as a machine that sessions are on.
pub(super) struct Server {
    pub(super) id: BackendId,
    pub(super) address: String,
    pub(super) backend: Arc<RexBackend>,
    pub(super) hosts: Weak<Hosts>,
}

impl Served for Server {
    fn label(&self) -> String {
        self.hosts
            .upgrade()
            .and_then(|hosts| hosts.label_of(&self.id.0))
            .unwrap_or_else(|| self.address.clone())
    }

    fn address(&self) -> String {
        self.address.clone()
    }

    fn usable(&self) -> bool {
        matches!(&*self.backend.link().borrow(), Link::Connected(_))
            && self.backend.sessions().iter().any(|session| session.opened)
    }

    fn folder(&self, session: SessionId) -> Option<String> {
        self.backend.folder_of(session)
    }

    fn runner(&self) -> Arc<dyn Runner> {
        Arc::new(Programs {
            backend: Arc::clone(&self.backend),
            label: self.label(),
        })
    }
}

/// What runs programs on a server's machine.
struct Programs {
    backend: Arc<RexBackend>,
    label: String,
}

impl Programs {
    /// What is asked, as the server is asked it, once it is known to be something that
    /// can be run there.
    fn asked(&self, asked: Asked) -> Result<RunAsked, ServiceError> {
        let spelling = if self.backend.on_windows() {
            Spelling::Windows
        } else {
            Spelling::Posix
        };
        asked.check(spelling)?;
        Ok(RunAsked {
            program: asked.program,
            args: asked.args,
            folder: asked.folder,
            env: asked.env,
        })
    }

    /// Why a program was not run, or not to its end.
    fn failed(&self, asked: &RunAsked, error: RunError) -> ServiceError {
        let label = &self.label;
        match error {
            RunError::NotConnected => {
                ServiceError::unavailable(format!("{label} is not connected"))
                    .because(NOT_CONNECTED)
            }
            RunError::CannotRun(why) => {
                ServiceError::unavailable(format!("programs cannot be run on {label}: {why}"))
                    .because(CANNOT_RUN)
            }
            RunError::NoFolder => ServiceError::not_found(format!(
                "the folder `{}` does not exist on {label}",
                asked.folder.as_deref().unwrap_or("~")
            ))
            .because(FOLDER_MISSING),
            RunError::NoProgram => {
                ServiceError::not_found(format!("`{}` was not found on {label}", asked.program))
                    .because(PROGRAM_MISSING)
            }
            RunError::TooLarge(why) => ServiceError::too_large(why),
            RunError::Busy => ServiceError::too_large(format!(
                "too many programs whose output is read are running on {label}"
            )),
            RunError::Failed(why) => {
                ServiceError::failed(format!("{label} could not run `{}`: {why}", asked.program))
            }
        }
    }
}

/// Comes back once a run is no longer wanted.
async fn stopped(stop: &Stop) {
    while !stop.asked() {
        tokio::time::sleep(STOP_LOOK).await;
    }
}

#[async_trait]
impl Runner for Programs {
    async fn run(&self, asked: Asked, limits: Limits) -> Result<RunOutput, ServiceError> {
        let asked = self.asked(asked)?;
        let run_limits = RunLimits {
            stdin: limits.stdin,
            timeout: limits.timeout,
            max_output: limits.max_output,
        };
        // Letting go of the run ends it there.
        let ran = tokio::select! {
            ran = self.backend.run(&asked, run_limits) => ran,
            () = stopped(&limits.stop) => {
                return Ok(RunOutput {
                    code: None,
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                    timed_out: false,
                    truncated: false,
                });
            }
        };
        let ran = ran.map_err(|error| self.failed(&asked, error))?;
        Ok(RunOutput {
            code: ran.code,
            stdout: ran.stdout,
            stderr: ran.stderr,
            timed_out: ran.timed_out,
            truncated: ran.truncated,
        })
    }

    async fn spawn(&self, asked: Asked, stop: Arc<Stop>) -> Result<Begun, ServiceError> {
        let asked = self.asked(asked)?;
        let started = tokio::select! {
            started = self.backend.start(&asked) => started,
            () = stopped(&stop) => return Err(ServiceError::failed("nobody waits for it any more")),
        };
        let started = started.map_err(|error| self.failed(&asked, error))?;
        Ok(Begun {
            stdout: Box::new(started.stdout),
            stderr: Box::new(started.stderr),
            child: Arc::new(Started(started.run)),
        })
    }

    /// Runs end with their connection: there is nothing of its own to let go of.
    fn close(&self) {}
}

/// A program started on a server's machine.
struct Started(StartedRun);

impl Running for Started {
    /// It has no number of this machine's.
    fn id(&self) -> u32 {
        0
    }

    fn try_wait(&self) -> Option<Option<i32>> {
        self.0.try_wait()
    }

    fn kill(&self) -> Result<(), ServiceError> {
        self.0.kill();
        Ok(())
    }
}
