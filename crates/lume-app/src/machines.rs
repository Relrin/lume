//! The machines that sessions are on, other than this one, where plugins can run
//! programs: the Linux distributions of this Windows machine, the machines that SSH
//! profiles sign in to, and the machines of the servers that run sessions elsewhere.
//!
//! A machine has an id, which is made of how it is reached and of nothing it says of
//! itself: what a user agreed to a plugin's running on a machine is kept under it. A
//! machine can be used while a session of this application is on it, and not otherwise:
//! a plugin that holds an id from some other day cannot have a machine reached that
//! nobody is using.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use lume_plugin_api::types::{MachineKind, Place};
use lume_services::posix::{self, Asked, Door, Limits, Shell};
use lume_services::process::{RunOutput, Running, Stop};
use lume_services::{ErrorKind, ServiceError};
use lume_session::{BackendId, Launcher, SessionId, wsl};

use crate::carrier::SessionKey;
use crate::sessions::Launched;
use crate::state::AppState;

mod ssh;

/// How long a sign-in may wait for the other side once the user has answered, beside
/// the time they take to.
const SIGN_IN_WAIT: Duration = Duration::from_secs(10 * 60);

/// How long a connection that nothing uses is kept, and how often that is looked at.
const IDLE: Duration = Duration::from_secs(5 * 60);
const IDLE_LOOK: Duration = Duration::from_secs(60);

/// The longest folder a session is said to be in.
const FOLDER_MAX: usize = 4096;

/// What a run on another machine fails for when that machine cannot be used now, or
/// cannot run programs for this application at all.
pub const NOT_CONNECTED: &str = posix::NOT_CONNECTED;
pub const CANNOT_RUN: &str = posix::CANNOT_RUN;

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
    /// What it called itself when this application last reached it, where that counts.
    name: Option<String>,
}

impl Machine {
    /// The machine of what serves a backend's sessions.
    fn served(id: &BackendId, served: Arc<dyn Served>) -> Self {
        Self {
            id: id.0.clone(),
            label: served.label(),
            kind: MachineKind::Rex,
            reach: Reach::Served(served),
            name: None,
        }
    }

    /// The machine that what a session runs is on, if it is another one that plugins
    /// can run programs on. `profile` is what the profile it was started from is called.
    fn of(launcher: &Launcher, profile: Option<String>) -> Option<Self> {
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
                    name: None,
                })
            }
            Launcher::Ssh { .. } => {
                let reached = reached(launcher)?;
                Some(Self {
                    id: reached.id(),
                    label: profile.unwrap_or_else(|| reached.address()),
                    kind: MachineKind::Ssh,
                    reach: Reach::Launched(launcher.clone()),
                    name: None,
                })
            }
            _ => None,
        }
    }

    /// Where it is reached, as the user is shown it, where that is not its name.
    pub fn address(&self) -> Option<String> {
        match &self.reach {
            Reach::Served(served) => Some(served.address()),
            Reach::Launched(launcher) => reached(launcher).map(|reached| reached.address()),
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
        match self.address() {
            Some(address) if address != self.label => {
                format!("{}, {address} ({how})", self.label)
            }
            Some(address) => format!("{address} ({how})"),
            None => format!("{} ({how})", self.label),
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
            // Of the machine its own shell named; until it has, nothing counts.
            Reach::Launched(Launcher::Ssh { .. }) => {
                let (host, path) = lume_session::cwd::parse_file_url(reported?)?;
                ssh::same_machine(&host, self.name.as_deref()?).then_some(path)?
            }
            Reach::Launched(_) => return None,
        };
        (folder.starts_with('/') && folder.len() <= FOLDER_MAX).then_some(folder)
    }

    /// How its shell is reached by a client that may ask the user what signing in takes,
    /// through `asks`. Only a machine signed in to has one.
    pub fn asking_door(&self, asks: Vec<(String, String)>) -> Result<Door, ServiceError> {
        match &self.reach {
            Reach::Launched(launcher @ Launcher::Ssh { .. }) => reached(launcher)
                .ok_or_else(|| ServiceError::invalid("not a machine reached over SSH"))?
                .door(&self.label, Some(asks)),
            _ => Err(ServiceError::invalid(format!(
                "nobody signs in to {}",
                self.label
            ))),
        }
    }

    fn door(&self) -> Result<Door, ServiceError> {
        match &self.reach {
            Reach::Launched(launcher @ Launcher::Ssh { .. }) => reached(launcher)
                .ok_or_else(|| ServiceError::invalid("not a machine reached over SSH"))?
                .door(&self.label, None),
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
                    unreached: None,
                })
            }
            _ => Err(ServiceError::unavailable(format!(
                "programs cannot be run on {}",
                self.label
            ))),
        }
    }
}

/// How an SSH launcher reaches its machine.
fn reached(launcher: &Launcher) -> Option<ssh::Reached<'_>> {
    match launcher {
        Launcher::Ssh {
            host,
            user,
            port,
            extra_args,
            client,
        } => Some(ssh::Reached {
            host,
            user: user.as_deref(),
            port: *port,
            extra_args,
            client: client.as_deref(),
        }),
        _ => None,
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

    /// Opens the connection through `door`, which may ask the user what signing in
    /// takes, and keeps it. Only a machine one signs in to has a way to.
    async fn sign_in(&self, door: Door, stop: Arc<Stop>) -> Result<(), ServiceError> {
        let _ = (door, stop);
        Err(ServiceError::unavailable("nobody signs in there").because(CANNOT_RUN))
    }

    /// Whether the connection there now was opened by signing in.
    fn signed_in(&self) -> bool {
        false
    }

    /// What the machine called itself, while there is a connection to it.
    fn name(&self) -> Option<String> {
        None
    }
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
        self.watch();
        Ok(Begun {
            stdout: started.stdout,
            stderr: started.stderr,
            child: started.child,
        })
    }

    fn close(&self) {
        self.shell.close();
    }

    async fn sign_in(&self, door: Door, stop: Arc<Stop>) -> Result<(), ServiceError> {
        let shell = Arc::clone(&self.shell);
        let signed = tokio::task::spawn_blocking(move || shell.sign_in(&door, &stop, SIGN_IN_WAIT))
            .await
            .unwrap_or_else(|error| Err(gone(error)));
        self.watch();
        signed
    }

    fn signed_in(&self) -> bool {
        self.shell.signed_in()
    }

    fn name(&self) -> Option<String> {
        self.shell.name()
    }
}

/// A runner, and how its machine was reached when it was made.
struct Held {
    runner: Arc<dyn Runner>,
    /// The program and words of its door, so that a machine now reached otherwise, as
    /// when its profile changed, has a new one.
    door: Option<(std::path::PathBuf, Vec<String>)>,
}

/// The machines in use, and what runs programs on them.
#[derive(Default)]
pub struct Machines {
    runners: Mutex<HashMap<String, Held>>,
    /// What each machine reached over SSH last called itself, by its id.
    names: Mutex<HashMap<String, String>>,
    /// The machines that want the user to sign in before anything is run there.
    wanting: Mutex<HashSet<String>>,
}

fn held<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Machines {
    fn runners(&self) -> MutexGuard<'_, HashMap<String, Held>> {
        held(&self.runners)
    }

    /// What runs programs on a machine. That of a server is its own, and never kept
    /// here: a new connection has a new one.
    pub fn runner(&self, machine: &Machine) -> Result<Arc<dyn Runner>, ServiceError> {
        if let Reach::Served(served) = &machine.reach {
            return Ok(served.runner());
        }
        let mut runners = self.runners();
        let door = machine.door();
        let how = door
            .as_ref()
            .ok()
            .map(|door| (door.program.clone(), door.args.clone()));
        if let Some(kept) = runners.get(&machine.id) {
            // One put in its place by a test is never another.
            if kept.door.is_none() || kept.door == how {
                return Ok(Arc::clone(&kept.runner));
            }
        }
        let door = door?;
        let runner: Arc<dyn Runner> = Arc::new(Posix::new(door));
        if let Some(old) = runners.insert(
            machine.id.clone(),
            Held {
                runner: Arc::clone(&runner),
                door: how,
            },
        ) {
            old.runner.close();
        }
        Ok(runner)
    }

    /// The machine, with what it last called itself, where that counts.
    pub fn named(&self, mut machine: Machine) -> Machine {
        if machine.kind == MachineKind::Ssh {
            machine.name = held(&self.names).get(&machine.id).cloned();
        }
        machine
    }

    /// Notes what a run on a machine came to: whether it wants the user to sign in, and
    /// what it called itself.
    pub fn noted<T>(&self, machine: &Machine, runner: &dyn Runner, done: &Result<T, ServiceError>) {
        if machine.kind != MachineKind::Ssh {
            return;
        }
        // A run that came to its folder or its program got in.
        let why = done.as_ref().err().map(|error| error.why);
        match why {
            Some(Some(posix::SIGN_IN)) => {
                held(&self.wanting).insert(machine.id.clone());
            }
            None | Some(Some(posix::FOLDER_MISSING | posix::PROGRAM_MISSING)) => {
                held(&self.wanting).remove(&machine.id);
            }
            Some(_) => {}
        }
        if let Some(name) = runner.name() {
            held(&self.names).insert(machine.id.clone(), name);
        }
    }

    /// Whether a machine wants the user to sign in before anything can be run there.
    pub fn wants_sign_in(&self, machine: &str) -> bool {
        held(&self.wanting).contains(machine)
            && !self
                .runners()
                .get(machine)
                .is_some_and(|kept| kept.runner.signed_in())
    }

    /// Whether what a machine called itself is known.
    pub fn knows_name(&self, machine: &str) -> bool {
        held(&self.names).contains_key(machine)
    }

    /// Lets go of the connections to machines that are not among `kept`, and of what was
    /// known of them.
    pub fn keep_only(&self, kept: &BTreeMap<String, Machine>) {
        let gone: Vec<Held> = {
            let mut runners = self.runners();
            let ids: Vec<String> = runners
                .keys()
                .filter(|id| !kept.contains_key(*id))
                .cloned()
                .collect();
            ids.iter().filter_map(|id| runners.remove(id)).collect()
        };
        for gone in gone {
            gone.runner.close();
        }
        held(&self.names).retain(|id, _| kept.contains_key(id));
        held(&self.wanting).retain(|id| kept.contains_key(id));
    }

    /// Ends every connection.
    pub fn close(&self) {
        for (_, kept) in self.runners().drain() {
            kept.runner.close();
        }
    }

    /// Has `runner` run what is to be run on the machine with the id `machine`.
    #[cfg(test)]
    pub fn put(&self, machine: &str, runner: Arc<dyn Runner>) {
        self.runners()
            .insert(machine.to_owned(), Held { runner, door: None });
    }

    /// Whether a connection to a machine is held.
    #[cfg(test)]
    pub fn holds(&self, machine: &str) -> bool {
        self.runners().contains_key(machine)
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
        let launched = self.reports().launched(key)?;
        let profile = self.profile_name(&launched);
        Machine::of(&launched.launcher, profile)
    }

    /// What the profile a session was started from is called now.
    fn profile_name(&self, launched: &Launched) -> Option<String> {
        if !matches!(launched.launcher, Launcher::Ssh { .. }) {
            return None;
        }
        let profile = launched.profile.as_deref()?;
        Some(self.panes()?.profiles.find(profile)?.name)
    }

    /// The machines that sessions are on, other than this one, by their ids. A server's
    /// is one while it can be used.
    pub fn machines(&self) -> BTreeMap<String, Machine> {
        self.machines_but(None)
    }

    /// The same, as if a session that is ending were gone already.
    pub fn machines_but(&self, ending: Option<&SessionKey>) -> BTreeMap<String, Machine> {
        let launched: Vec<Machine> = self
            .reports()
            .launched_all()
            .into_iter()
            .filter(|(key, _)| Some(key) != ending)
            .filter_map(|(_, launched)| {
                let profile = self.profile_name(&launched);
                Machine::of(&launched.launcher, profile)
            })
            .collect();
        let served = self
            .servers()
            .all()
            .into_iter()
            .filter(|(_, served)| served.usable())
            .map(|(id, served)| Machine::served(&id, served));
        launched
            .into_iter()
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
        let ubuntu = Machine::of(&wsl(None), None).unwrap();
        assert_eq!(
            (ubuntu.id.as_str(), ubuntu.shown().as_str()),
            ("wsl:Ubuntu", "Ubuntu (WSL)")
        );
        let root = Machine::of(&wsl(Some("root")), None).unwrap();
        assert_eq!(root.id, "wsl:root@Ubuntu");
        // The SSH client run as a program of its own is run here.
        for here in [
            Launcher::Shell {
                program: "wsl.exe".to_owned(),
                args: vec!["-d".to_owned(), "Ubuntu".to_owned()],
                login: false,
            },
            Launcher::Shell {
                program: "ssh".to_owned(),
                args: vec!["box".to_owned()],
                login: false,
            },
        ] {
            assert!(Machine::of(&here, None).is_none());
        }
        let ssh = |user: Option<&str>| Launcher::Ssh {
            host: "box".to_owned(),
            user: user.map(str::to_owned),
            port: Some(2222),
            extra_args: Vec::new(),
            client: None,
        };
        let named = Machine::of(&ssh(Some("me")), Some("Build box".to_owned())).unwrap();
        assert_eq!(
            (named.id.as_str(), named.shown().as_str()),
            ("ssh:me@box:2222", "Build box, me@box:2222 (SSH)")
        );
        // Renamed, it is the same machine.
        let renamed = Machine::of(&ssh(Some("me")), Some("Builder".to_owned())).unwrap();
        assert_eq!(renamed.id, named.id);
        let plain = Machine::of(&ssh(None), None).unwrap();
        assert_eq!(plain.shown(), "box:2222 (SSH)");
    }

    #[test]
    fn a_folder_is_one_of_the_machines_only_when_its_shell_says_it_is_there() {
        let ubuntu = Machine::of(&wsl(None), None).unwrap();
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

    #[test]
    fn a_folder_of_a_machine_signed_in_to_counts_once_it_named_itself() {
        let launcher = Launcher::Ssh {
            host: "devbox".to_owned(),
            user: None,
            port: None,
            extra_args: Vec::new(),
            client: None,
        };
        let session = SessionId::new(1).unwrap();
        let unnamed = Machine::of(&launcher, None).unwrap();
        assert_eq!(
            unnamed.place(session, Some("file://devbox/home/me")).folder,
            None
        );
        let named = Machine {
            name: Some("devbox".to_owned()),
            ..unnamed
        };
        let folder = |url: &str| named.place(session, Some(url)).folder;
        assert_eq!(folder("file://devbox/home/me").as_deref(), Some("/home/me"));
        assert_eq!(
            folder("file://DEVBOX.lan/srv/a%20b").as_deref(),
            Some("/srv/a b")
        );
        let here = format!("file://{}/home/me", machine_name().unwrap());
        for url in [
            // A further machine that the user went on to, and this one.
            "file://other/home/me",
            &here,
            "file:///home/me",
            "file://localhost/home/me",
            "file://devbox/home/%0Ame",
            "https://devbox/home",
            &format!("file://devbox/{}", "x".repeat(FOLDER_MAX)),
        ] {
            assert_eq!(folder(url), None, "{url}");
        }
        assert_eq!(named.place(session, None).folder, None);
    }
}
