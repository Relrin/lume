//! The machines that sessions are on, other than this one, where plugins can run
//! programs: for now the Linux distributions of this Windows machine.
//!
//! A machine has an id, which is made of how it is reached and of nothing it says of
//! itself: what a user agreed to a plugin's running on a machine is kept under it. A
//! machine can be used while a session of this application is on it, and not otherwise:
//! a plugin that holds an id from some other day cannot have a machine reached that
//! nobody is using.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use lume_plugin_api::types::{MachineKind, Place};
use lume_services::posix::{Asked, Door, Limits, Shell};
use lume_services::process::{RunOutput, Running, Stop};
use lume_services::{ErrorKind, ServiceError};
use lume_session::{Launcher, wsl};

use crate::carrier::SessionKey;
use crate::state::AppState;

/// How long a connection that nothing uses is kept, and how often that is looked at.
const IDLE: Duration = Duration::from_secs(5 * 60);
const IDLE_LOOK: Duration = Duration::from_secs(60);

/// The longest folder a session is said to be in.
const FOLDER_MAX: usize = 4096;

/// A machine that sessions are on, other than this one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Machine {
    pub id: String,
    /// What to call it.
    pub label: String,
    pub kind: MachineKind,
    /// How programs are run there.
    reach: Launcher,
}

impl Machine {
    /// The machine that what a session runs is on, if it is another one that plugins
    /// can run programs on.
    fn of(launcher: &Launcher) -> Option<Self> {
        match launcher {
            Launcher::Wsl { distro, user } => {
                let label = match user {
                    Some(user) => format!("{user}@{distro}"),
                    None => distro.clone(),
                };
                Some(Self {
                    id: format!("wsl:{label}"),
                    label,
                    kind: MachineKind::Wsl,
                    reach: launcher.clone(),
                })
            }
            _ => None,
        }
    }

    /// Its name with how it is reached, as the user is told of it.
    pub fn shown(&self) -> String {
        let how = match self.kind {
            MachineKind::Rex => "Rex server",
            MachineKind::Ssh => "SSH",
            MachineKind::Wsl => "WSL",
        };
        format!("{} ({how})", self.label)
    }

    /// Where a session on it is, going by the folder it reported, if it did.
    pub fn place(&self, reported: Option<&str>) -> Place {
        Place {
            machine: self.id.clone(),
            label: self.label.clone(),
            kind: self.kind,
            folder: reported.and_then(|url| self.folder(url)),
        }
    }

    /// The folder a session on it reported, as a path over there: an absolute one, named
    /// by a shell that says it is on that machine and not on one it connected to.
    fn folder(&self, url: &str) -> Option<String> {
        let folder = match self.reach {
            // A distribution goes by the name of the machine it is on.
            Launcher::Wsl { .. } => lume_session::cwd::path_here(url)?,
            _ => return None,
        };
        (folder.starts_with('/') && folder.len() <= FOLDER_MAX).then_some(folder)
    }

    fn door(&self) -> Result<Door, ServiceError> {
        match &self.reach {
            Launcher::Wsl { distro, user } => {
                let program = wsl::program()
                    .ok_or_else(|| ServiceError::unavailable("WSL is not installed"))?;
                let mut args = vec!["-d".to_owned(), distro.clone()];
                if let Some(user) = user {
                    args.extend(["-u".to_owned(), user.clone()]);
                }
                // A folder that is not there has `wsl.exe` run things in the root with
                // nothing said of it: it is given one that is, and the shell goes to the
                // folder that is meant.
                args.extend(["--cd", "/", "--exec"].map(str::to_owned));
                Ok(Door {
                    program,
                    args,
                    env: vec![("WSL_UTF8".to_owned(), "1".to_owned())],
                    one_line: false,
                    label: self.label.clone(),
                })
            }
            _ => Err(ServiceError::unavailable(format!(
                "programs cannot be run on {}",
                self.label
            ))),
        }
    }
}

/// A program started on a machine, with its output as it comes.
pub struct Begun {
    pub stdout: Box<dyn Read + Send>,
    pub stderr: Box<dyn Read + Send>,
    pub child: Arc<dyn Running>,
}

/// Something that runs programs on another machine.
#[async_trait]
pub trait Runner: Send + Sync {
    /// Runs a program to its end, or until its time is up or it is told to stop.
    async fn run(&self, asked: Asked, limits: Limits) -> Result<RunOutput, ServiceError>;

    /// Starts a program, which reads nothing.
    async fn spawn(&self, asked: Asked, stop: Arc<Stop>) -> Result<Begun, ServiceError>;

    /// Lets go of what it holds open.
    fn close(&self);
}

fn gone(error: tokio::task::JoinError) -> ServiceError {
    ServiceError::new(ErrorKind::Failed, error.to_string())
}

/// A machine reached through its shell.
struct Posix {
    shell: Arc<Shell>,
    /// Whether something looks after a connection that is not used.
    watched: Arc<AtomicBool>,
}

impl Posix {
    fn new(door: Door) -> Self {
        Self {
            shell: Arc::new(Shell::new(door)),
            watched: Arc::default(),
        }
    }

    /// Has the connection let go of once nothing has used it for a while.
    fn watch(&self) {
        if self.watched.swap(true, Ordering::AcqRel) {
            return;
        }
        let shell = Arc::downgrade(&self.shell);
        let watched = Arc::clone(&self.watched);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(IDLE_LOOK).await;
                let Some(shell) = shell.upgrade() else {
                    return;
                };
                shell.close_idle(IDLE);
                if shell.name().is_none() {
                    watched.store(false, Ordering::Release);
                    return;
                }
            }
        });
    }
}

#[async_trait]
impl Runner for Posix {
    async fn run(&self, asked: Asked, limits: Limits) -> Result<RunOutput, ServiceError> {
        let shell = Arc::clone(&self.shell);
        let output = tokio::task::spawn_blocking(move || shell.run(&asked, limits))
            .await
            .unwrap_or_else(|error| Err(gone(error)));
        self.watch();
        output
    }

    async fn spawn(&self, asked: Asked, stop: Arc<Stop>) -> Result<Begun, ServiceError> {
        let shell = Arc::clone(&self.shell);
        let started = tokio::task::spawn_blocking(move || shell.spawn(&asked, &stop))
            .await
            .unwrap_or_else(|error| Err(gone(error)))?;
        Ok(Begun {
            stdout: Box::new(started.stdout),
            stderr: Box::new(started.stderr),
            child: Arc::new(started.child),
        })
    }

    fn close(&self) {
        self.shell.close();
    }
}

/// The machines in use, and what runs programs on them.
#[derive(Default)]
pub struct Machines {
    runners: Mutex<HashMap<String, Arc<dyn Runner>>>,
}

impl Machines {
    fn runners(&self) -> MutexGuard<'_, HashMap<String, Arc<dyn Runner>>> {
        self.runners.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What runs programs on a machine.
    pub fn runner(&self, machine: &Machine) -> Result<Arc<dyn Runner>, ServiceError> {
        let mut runners = self.runners();
        if let Some(runner) = runners.get(&machine.id) {
            return Ok(Arc::clone(runner));
        }
        let runner: Arc<dyn Runner> = Arc::new(Posix::new(machine.door()?));
        runners.insert(machine.id.clone(), Arc::clone(&runner));
        Ok(runner)
    }

    /// Ends every connection.
    pub fn close(&self) {
        for (_, runner) in self.runners().drain() {
            runner.close();
        }
    }

    /// Has `runner` run what is to be run on the machine with the id `machine`.
    #[cfg(test)]
    pub fn put(&self, machine: &str, runner: Arc<dyn Runner>) {
        self.runners().insert(machine.to_owned(), runner);
    }
}

impl AppState {
    /// The machine a session is on, if it is another one that plugins can run programs
    /// on.
    pub fn machine_of(&self, key: &SessionKey) -> Option<Machine> {
        if !self.is_local(&key.backend) {
            return None;
        }
        Machine::of(&self.reports().launched(key)?.launcher)
    }

    /// The machines that sessions are on, other than this one, by their ids.
    pub fn machines(&self) -> BTreeMap<String, Machine> {
        self.reports()
            .launchers()
            .iter()
            .filter_map(Machine::of)
            .map(|machine| (machine.id.clone(), machine))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use lume_session::cwd::machine_name;

    use super::*;

    fn wsl(user: Option<&str>) -> Launcher {
        Launcher::Wsl {
            distro: "Ubuntu".to_owned(),
            user: user.map(str::to_owned),
        }
    }

    #[test]
    fn a_machine_is_known_by_how_it_is_reached() {
        let ubuntu = Machine::of(&wsl(None)).unwrap();
        assert_eq!(
            (ubuntu.id.as_str(), ubuntu.shown().as_str()),
            ("wsl:Ubuntu", "Ubuntu (WSL)")
        );
        let root = Machine::of(&wsl(Some("root"))).unwrap();
        assert_eq!(root.id, "wsl:root@Ubuntu");
        for here in [
            Launcher::Shell {
                program: "wsl.exe".to_owned(),
                args: vec!["-d".to_owned(), "Ubuntu".to_owned()],
                login: false,
            },
            Launcher::Ssh {
                host: "box".to_owned(),
                user: None,
                port: None,
                extra_args: Vec::new(),
                client: None,
            },
        ] {
            assert_eq!(Machine::of(&here), None);
        }
    }

    #[test]
    fn a_folder_is_one_of_the_machines_only_when_its_shell_says_it_is_there() {
        let ubuntu = Machine::of(&wsl(None)).unwrap();
        let name = machine_name().unwrap();
        let folder = |url: &str| ubuntu.place(Some(url)).folder;
        assert_eq!(
            folder(&format!("file://{name}/home/me/a%20b")).as_deref(),
            Some("/home/me/a b")
        );
        assert_eq!(
            folder(&format!("file://{name}/mnt/c/Users")).as_deref(),
            Some("/mnt/c/Users")
        );
        for url in [
            "file://some-other-box/home/me",
            "https://example.org/home",
            "file:///home/%0Ame",
            "file:///home/%zz",
            &format!("file://{name}/{}", "x".repeat(FOLDER_MAX)),
        ] {
            assert_eq!(folder(url), None, "{url}");
        }
        assert_eq!(ubuntu.place(None).folder, None);
    }
}
