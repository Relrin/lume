//! The machines that sessions are on, other than this one, where plugins can run
//! programs: the Linux distributions of this Windows machine, and the machines of the
//! servers that run sessions elsewhere.
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
use lume_session::{BackendId, Launcher, SessionId, wsl};

use crate::carrier::SessionKey;
use crate::state::AppState;

/// How long a connection that nothing uses is kept, and how often that is looked at.
const IDLE: Duration = Duration::from_secs(5 * 60);
const IDLE_LOOK: Duration = Duration::from_secs(60);

/// The longest folder a session is said to be in.
const FOLDER_MAX: usize = 4096;

/// What a run on another machine fails for when that machine cannot be used now, or
/// cannot run programs for this application at all.
pub const NOT_CONNECTED: &str = "not_connected";
pub const CANNOT_RUN: &str = "cannot_run";

/// A machine that runs sessions for the application, and programs where it can.
pub trait Served: Send + Sync {
    /// What the user calls it, now.
    fn label(&self) -> String;

    /// Where it is reached, as the user is shown it.
    fn address(&self) -> String;

    /// Whether it can be used now: it is connected, and a session of it is open here.
    fn usable(&self) -> bool;

    /// The folder a session of it is in, as a path of that machine, by its own word.
    fn folder(&self, session: SessionId) -> Option<String>;

    /// What runs programs there.
    fn runner(&self) -> Arc<dyn Runner>;
}

/// How programs are run on a machine.
#[derive(Clone)]
enum Reach {
    /// By something started here, as what a session runs is.
    Launched(Launcher),
    /// By what serves its sessions.
    Served(Arc<dyn Served>),
}

/// A machine that sessions are on, other than this one.
#[derive(Clone)]
pub struct Machine {
    pub id: String,
    /// What to call it.
    pub label: String,
    pub kind: MachineKind,
    reach: Reach,
}

impl Machine {
    /// The machine of what serves a backend's sessions.
    fn served(id: &BackendId, served: Arc<dyn Served>) -> Self {
        Self {
            id: id.0.clone(),
            label: served.label(),
            kind: MachineKind::Rex,
            reach: Reach::Served(served),
        }
    }

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
                    reach: Reach::Launched(launcher.clone()),
                })
            }
            _ => None,
        }
    }

    /// Its name with how it is reached, as the user is told of it. A server is named
    /// with its address too: two can be called the same.
    pub fn shown(&self) -> String {
        let how = match self.kind {
            MachineKind::Rex => "Rex server",
            MachineKind::Ssh => "SSH",
            MachineKind::Wsl => "WSL",
        };
        match &self.reach {
            Reach::Served(served) => {
                let address = served.address();
                if address == self.label {
                    format!("{address} ({how})")
                } else {
                    format!("{}, {address} ({how})", self.label)
                }
            }
            Reach::Launched(_) => format!("{} ({how})", self.label),
        }
    }

    /// Whether it says itself where its sessions are, so that nothing they print counts.
    pub fn says_where(&self) -> bool {
        matches!(self.reach, Reach::Served(_))
    }

    /// Where a session on it is. `reported` is the folder it printed, which counts only
    /// on a machine that does not say so itself.
    pub fn place(&self, session: SessionId, reported: Option<&str>) -> Place {
        Place {
            machine: self.id.clone(),
            label: self.label.clone(),
            kind: self.kind,
            folder: self.folder(session, reported),
        }
    }

    /// The folder a session on it is in, as a path over there: what the machine says
    /// itself, or else an absolute path that a shell printed and that names that machine
    /// and not one it connected to.
    fn folder(&self, session: SessionId, reported: Option<&str>) -> Option<String> {
        let folder = match &self.reach {
            Reach::Served(served) => return served.folder(session),
            // A distribution goes by the name of the machine it is on.
            Reach::Launched(Launcher::Wsl { .. }) => lume_session::cwd::path_here(reported?)?,
            Reach::Launched(_) => return None,
        };
        (folder.starts_with('/') && folder.len() <= FOLDER_MAX).then_some(folder)
    }

    fn door(&self) -> Result<Door, ServiceError> {
        match &self.reach {
            Reach::Launched(Launcher::Wsl { distro, user }) => {
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

    /// What runs programs on a machine. That of a server is its own, and never kept
    /// here: a new connection has a new one.
    pub fn runner(&self, machine: &Machine) -> Result<Arc<dyn Runner>, ServiceError> {
        if let Reach::Served(served) = &machine.reach {
            return Ok(served.runner());
        }
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

/// What serves the sessions of backends that are on other machines, by the backends.
#[derive(Default)]
pub struct Servers(Mutex<HashMap<BackendId, Arc<dyn Served>>>);

impl Servers {
    fn held(&self) -> MutexGuard<'_, HashMap<BackendId, Arc<dyn Served>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Notes what serves a backend's sessions, from now until it is let go of.
    pub fn insert(&self, backend: BackendId, served: Arc<dyn Served>) {
        self.held().insert(backend, served);
    }

    pub fn remove(&self, backend: &BackendId) {
        self.held().remove(backend);
    }

    fn get(&self, backend: &BackendId) -> Option<Arc<dyn Served>> {
        self.held().get(backend).cloned()
    }

    /// Every one, taken out of the lock: what they are asked may take others.
    fn all(&self) -> Vec<(BackendId, Arc<dyn Served>)> {
        let held = self.held();
        held.iter()
            .map(|(id, served)| (id.clone(), Arc::clone(served)))
            .collect()
    }
}

impl AppState {
    /// The machine a session is on, if it is another one that plugins can run programs
    /// on.
    pub fn machine_of(&self, key: &SessionKey) -> Option<Machine> {
        if !self.is_local(&key.backend) {
            let served = self.servers().get(&key.backend)?;
            return Some(Machine::served(&key.backend, served));
        }
        Machine::of(&self.reports().launched(key)?.launcher)
    }

    /// The machines that sessions are on, other than this one, by their ids. A server's
    /// is one while it can be used.
    pub fn machines(&self) -> BTreeMap<String, Machine> {
        let launched = self.reports().launchers();
        let served = self
            .servers()
            .all()
            .into_iter()
            .filter(|(_, served)| served.usable())
            .map(|(id, served)| Machine::served(&id, served));
        launched
            .iter()
            .filter_map(Machine::of)
            .chain(served)
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
            assert!(Machine::of(&here).is_none());
        }
    }

    #[test]
    fn a_folder_is_one_of_the_machines_only_when_its_shell_says_it_is_there() {
        let ubuntu = Machine::of(&wsl(None)).unwrap();
        let name = machine_name().unwrap();
        let session = SessionId::new(1).unwrap();
        let folder = |url: &str| ubuntu.place(session, Some(url)).folder;
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
        assert_eq!(ubuntu.place(session, None).folder, None);
    }
}
