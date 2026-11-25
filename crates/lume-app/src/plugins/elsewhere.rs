//! Programs a plugin runs on another machine: on the one that a session is on.
//!
//! Three things have to hold before anything is run there. The plugin's grant names the
//! program, under a permission of its own, apart from what it may run here. The user
//! has agreed to the plugin's running it on that machine, when asked in the window; the
//! answer is kept with the install until they take it back. And a session is there now.
//!
//! What comes back is what that machine says, within the limits a run here has.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use lume_plugin::broker::grant_of;
use lume_plugin::lock::OnMachine;
use lume_plugin::permissions::{Kind, can_do_anything};
use lume_plugin_api::types::{MachineEntry, MachinesAllow, SpawnOptions};
use lume_plugin_api::{ApiError, ErrorCode};
use lume_services::ServiceError;
use lume_services::posix::{Asked, Limits};
use lume_services::process::{RunOutput, Stop};
use serde_json::{Value, json};
use tokio::sync::Semaphore;

use super::asking::{REASON_MAX, plain, too_long};
use super::system::from_service;
use super::{Plugins, api_error};
use crate::machines::{Begun, Machine, Machines};

/// How many programs a plugin has running on a machine at once; more wait their turn.
const AT_ONCE: usize = 8;

/// How long a no is taken for an answer without asking again.
const NO_STANDS: Duration = Duration::from_secs(60);

/// The kinds of failure that are only another machine's.
const NOT_ALLOWED: &str = "machine_not_allowed";
const NOT_CONNECTED: &str = "not_connected";

/// What is kept of plugins' dealings with other machines.
#[derive(Default)]
pub(super) struct Elsewhere {
    pub(super) machines: Machines,
    /// What lets only so many programs run at once, by the plugin and the machine.
    turns: Mutex<HashMap<(String, String), Arc<Semaphore>>>,
    /// The questions on show, by the plugin and the machine.
    asking: Mutex<HashSet<(String, String)>>,
    /// When the user last said no, by the plugin and the machine.
    refused: Mutex<HashMap<(String, String), Instant>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Elsewhere {
    fn turn(&self, local: &str, machine: &str) -> Arc<Semaphore> {
        let key = (local.to_owned(), machine.to_owned());
        Arc::clone(
            lock(&self.turns)
                .entry(key)
                .or_insert_with(|| Arc::new(Semaphore::new(AT_ONCE))),
        )
    }

    fn refused_lately(&self, key: &(String, String)) -> bool {
        lock(&self.refused)
            .get(key)
            .is_some_and(|at| at.elapsed() < NO_STANDS)
    }
}

/// A question on show, until it is answered.
struct OnShow<'a> {
    asking: &'a Mutex<HashSet<(String, String)>>,
    key: (String, String),
}

impl<'a> OnShow<'a> {
    fn begin(asking: &'a Mutex<HashSet<(String, String)>>, key: &(String, String)) -> Option<Self> {
        lock(asking).insert(key.clone()).then(|| Self {
            asking,
            key: key.clone(),
        })
    }
}

impl Drop for OnShow<'_> {
    fn drop(&mut self) {
        lock(self.asking).remove(&self.key);
    }
}

/// The input of a program on another machine, which is closed.
pub(super) struct NoInput;

impl Write for NoInput {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "a program on another machine reads nothing",
        ))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A failure of a kind that only another machine has, with the machine.
fn machine_error(
    code: ErrorCode,
    kind: &str,
    message: String,
    machine: &str,
    label: &str,
) -> ApiError {
    ApiError {
        kind: Some(kind.to_owned()),
        data: Some(json!({ "machine": machine, "label": label })),
        ..api_error(code, message)
    }
}

/// Why nothing was run on a machine, as a plugin's code is told it: with the machine,
/// where what it was has a name.
fn from_machine(error: ServiceError, machine: &Machine) -> ApiError {
    let error = from_service(error);
    ApiError {
        data: error
            .kind
            .as_ref()
            .map(|_| json!({ "machine": machine.id, "label": machine.shown() })),
        ..error
    }
}

impl Plugins {
    /// The programs a plugin may run on other machines, once it may on a given one.
    fn programs_elsewhere(&self, local: &str) -> Vec<String> {
        self.inner()
            .loaded
            .get(local)
            .and_then(|loaded| {
                grant_of(&loaded.grants, Kind::ProcessRunElsewhere).map(|grant| grant.scope.clone())
            })
            .unwrap_or_default()
    }

    /// What the user agreed to a plugin's running on a machine.
    fn agreed_on(&self, local: &str, machine: &str) -> Option<OnMachine> {
        self.inner()
            .loaded
            .get(local)?
            .installed
            .entry
            .machines
            .get(machine)
            .cloned()
    }

    /// Whether a plugin may run all its programs on a machine.
    fn allowed_on(&self, local: &str, machine: &str) -> bool {
        let wanted = self.programs_elsewhere(local);
        !wanted.is_empty()
            && self.agreed_on(local, machine).is_some_and(|agreed| {
                wanted
                    .iter()
                    .all(|program| agreed.programs.contains(program))
            })
    }

    /// The other machines that sessions are on, as a plugin is told of them.
    pub(super) fn machines_list(&self, local: &str) -> Vec<MachineEntry> {
        let machines = self.state.machines();
        machines
            .into_values()
            .map(|machine| MachineEntry {
                allowed: self.allowed_on(local, &machine.id),
                machine: machine.id,
                label: machine.label,
                kind: machine.kind,
            })
            .collect()
    }

    fn machine(&self, id: &str) -> Result<Machine, ApiError> {
        self.state.machines().remove(id).ok_or_else(|| {
            machine_error(
                ErrorCode::Unavailable,
                NOT_CONNECTED,
                format!("no session is on {id}"),
                id,
                id,
            )
        })
    }

    /// Asks the user to let a plugin run its programs on a machine, and keeps their yes.
    pub(super) async fn allow_machine(
        &self,
        local: &str,
        asked: MachinesAllow,
    ) -> Result<bool, ApiError> {
        let machine = self.machine(&asked.machine)?;
        if self.allowed_on(local, &machine.id) {
            return Ok(true);
        }
        let reason = asked.reason.unwrap_or_default();
        if reason.chars().count() > REASON_MAX {
            return Err(too_long("the reason", REASON_MAX));
        }
        let key = (local.to_owned(), machine.id.clone());
        if self.elsewhere.refused_lately(&key) {
            return Ok(false);
        }
        // Asked twice, it is on show once.
        let Some(_on_show) = OnShow::begin(&self.elsewhere.asking, &key) else {
            return Ok(false);
        };
        let programs = self.programs_elsewhere(local);
        let warning = can_do_anything(&programs).then_some(
            "A program such as a shell or git can do anything you can there, so running it is \
             close to trusting this plugin with your account on that machine.",
        );
        let answer = self
            .ask_user(
                local,
                json!({
                    "kind": "permission",
                    "permission": Kind::ProcessRunElsewhere.as_str(),
                    "what": format!(
                        "Run on {}, with any arguments: {}",
                        machine.shown(),
                        programs.join(", ")
                    ),
                    "warning": warning,
                    "reason": plain(&reason, REASON_MAX),
                }),
            )
            .await?;
        let allowed = answer["allow"] == true;
        let scope = format!("{} on {}", programs.join(", "), machine.id);
        self.broker
            .record(local, Kind::ProcessRunElsewhere, Some(&scope), allowed);
        if !allowed {
            lock(&self.elsewhere.refused).insert(key, Instant::now());
            return Ok(false);
        }
        let agreed = OnMachine {
            label: machine.shown(),
            programs,
        };
        let store = self.store.clone();
        let (owned, id, kept) = (local.to_owned(), machine.id.clone(), agreed.clone());
        tokio::task::spawn_blocking(move || store.allow_on(&owned, &id, &kept))
            .await
            .map_err(|error| api_error(ErrorCode::Failed, error.to_string()))?
            .map_err(|error| api_error(ErrorCode::Failed, error.to_string()))?;
        // Held at once, before the lock file is read again.
        if let Some(loaded) = self.inner().loaded.get_mut(local) {
            loaded
                .installed
                .entry
                .machines
                .insert(machine.id.clone(), agreed);
        }
        Ok(true)
    }

    /// Checks that a plugin may run a program on a machine, has the user approve this
    /// use of it where each is approved, and gives back the machine.
    async fn may_run_on(
        &self,
        local: &str,
        id: &str,
        program: &str,
        args: &[String],
    ) -> Result<Machine, ApiError> {
        let scope = format!("{program} on {id}");
        let record = |allowed| {
            self.broker
                .record(local, Kind::ProcessRunElsewhere, Some(&scope), allowed);
        };
        if !self
            .programs_elsewhere(local)
            .iter()
            .any(|granted| granted == program)
        {
            record(false);
            return Err(api_error(
                ErrorCode::Denied,
                format!("{local} has no permission to run {program} on other machines"),
            ));
        }
        let agreed = self
            .agreed_on(local, id)
            .is_some_and(|agreed| agreed.programs.iter().any(|held| held == program));
        let machine = self.machine(id);
        if !agreed {
            record(false);
            let label = machine.map_or_else(|_| id.to_owned(), |machine| machine.shown());
            return Err(machine_error(
                ErrorCode::Denied,
                NOT_ALLOWED,
                format!("{local} has not been allowed to run {program} on {label}"),
                id,
                &label,
            ));
        }
        let machine = machine?;
        record(true);
        self.approve_use(
            local,
            Kind::ProcessRunElsewhere,
            &super::system::command_line(program, args),
            Some(machine.shown()),
        )
        .await?;
        Ok(machine)
    }

    /// Waits until a plugin may have one more program running on a machine.
    async fn turn_on(
        &self,
        local: &str,
        machine: &Machine,
        stop: &Stop,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, ApiError> {
        let stopped = || api_error(ErrorCode::Stopped, format!("{local} stopped"));
        let turn = self
            .elsewhere
            .turn(local, &machine.id)
            .acquire_owned()
            .await
            .map_err(|_| stopped())?;
        if stop.asked() {
            return Err(stopped());
        }
        Ok(turn)
    }

    /// Runs a program on a machine to its end.
    pub(super) async fn run_on(
        &self,
        local: &str,
        id: &str,
        asked: Asked,
        limits: Limits,
    ) -> Result<RunOutput, ApiError> {
        let machine = self
            .may_run_on(local, id, &asked.program, &asked.args)
            .await?;
        let _turn = self.turn_on(local, &machine, &limits.stop).await?;
        let runner = self
            .elsewhere
            .machines
            .runner(&machine)
            .map_err(|error| from_machine(error, &machine))?;
        runner
            .run(asked, limits)
            .await
            .map_err(|error| from_machine(error, &machine))
    }

    /// Starts a program on a machine.
    pub(super) async fn spawn_on(
        &self,
        local: &str,
        id: &str,
        program: String,
        args: Vec<String>,
        options: SpawnOptions,
        stop: Arc<Stop>,
    ) -> Result<Begun, ApiError> {
        let machine = self.may_run_on(local, id, &program, &args).await?;
        let runner = self
            .elsewhere
            .machines
            .runner(&machine)
            .map_err(|error| from_machine(error, &machine))?;
        let asked = asked_of(program, args, options.cwd, options.env);
        let begun = runner
            .spawn(asked, Arc::clone(&stop))
            .await
            .map_err(|error| from_machine(error, &machine))?;
        // Nobody waits for it any more.
        if stop.asked() {
            let _ = begun.child.kill();
            return Err(api_error(ErrorCode::Stopped, format!("{local} stopped")));
        }
        Ok(begun)
    }

    /// What installs may run where, as the settings window shows it.
    pub(super) fn machines_shown(entry: &lume_plugin::lock::Entry) -> Vec<Value> {
        entry
            .machines
            .iter()
            .map(|(id, agreed)| {
                json!({ "id": id, "label": agreed.label, "programs": agreed.programs })
            })
            .collect()
    }

    /// Takes back what an install may run on a machine.
    pub async fn revoke_machine(
        &self,
        local: &str,
        machine: &str,
    ) -> Result<(), crate::error::CommandError> {
        let store = self.store.clone();
        let (owned, id) = (local.to_owned(), machine.to_owned());
        super::manage::joined(
            tokio::task::spawn_blocking(move || store.revoke_on(&owned, &id)).await,
        )?;
        // Gone at once, before the lock file is read again.
        if let Some(loaded) = self.inner().loaded.get_mut(local) {
            loaded.installed.entry.machines.remove(machine);
        }
        Ok(())
    }
}

/// What is asked of a machine, from what the plugin sent. `${…}` stands for nothing
/// there: those are folders of this machine.
pub(super) fn asked_of(
    program: String,
    args: Vec<String>,
    folder: Option<String>,
    env: Option<BTreeMap<String, String>>,
) -> Asked {
    Asked {
        program,
        args,
        folder,
        env: env.unwrap_or_default().into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use lume_plugin::store::{InstallOptions, PackageSource};
    use lume_plugin_api::Call;
    use lume_plugin_api::types::SessionEntry;
    use lume_proto::SessionEvent;
    use lume_proto::app::SessionNews;
    use lume_session::Launcher;
    use lume_session::cwd::machine_name;

    use super::super::calls::tests::{running, says_it_is_in};
    use super::*;
    use crate::carrier::SessionKey;
    use crate::machines::Runner;
    use crate::services::Services;
    use crate::testing::{STEP, TempDir};

    const PLUGIN: &str = "acme.git";
    const UBUNTU: &str = "wsl:Ubuntu";

    /// A machine that notes what it is asked to run. What is to wait runs until it is
    /// told to stop, or to go on.
    #[derive(Default)]
    struct Noted {
        asked: Mutex<Vec<Asked>>,
        running: AtomicUsize,
        most: AtomicUsize,
        go_on: std::sync::atomic::AtomicBool,
    }

    #[async_trait]
    impl Runner for Noted {
        async fn run(&self, asked: Asked, limits: Limits) -> Result<RunOutput, ServiceError> {
            let waits = asked.args.iter().any(|arg| arg == "wait");
            let fails = asked.args.iter().any(|arg| arg == "fail");
            lock(&self.asked).push(asked);
            if fails {
                return Err(ServiceError::not_found("no such folder").because("folder_missing"));
            }
            let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
            self.most.fetch_max(now, Ordering::SeqCst);
            while waits && !limits.stop.asked() && !self.go_on.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            self.running.fetch_sub(1, Ordering::SeqCst);
            Ok(RunOutput {
                code: (!limits.stop.asked()).then_some(0),
                stdout: b"from there".to_vec(),
                stderr: Vec::new(),
                timed_out: false,
                truncated: false,
            })
        }

        async fn spawn(&self, _: Asked, _: Arc<Stop>) -> Result<Begun, ServiceError> {
            Err(ServiceError::unavailable("nothing is started here"))
        }

        fn close(&self) {}
    }

    /// The application with a plugin that may run git, elsewhere only, and a session on
    /// a Linux distribution.
    async fn with_plugin(dir: &TempDir) -> (Arc<Services>, SessionKey, Arc<Noted>) {
        let (services, key) = running(dir).await;
        let package = dir.path().join("package");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("plugin.toml"),
            format!(
                "schema = 1\nid = \"{PLUGIN}\"\nname = \"Git\"\nversion = \"1.0.0\"\n\
                 api = \"1\"\nmain = \"host.js\"\n[permissions]\n\
                 \"workspace.read\" = true\n\"process.run-elsewhere\" = [\"git\"]\n"
            ),
        )
        .unwrap();
        std::fs::write(package.join("host.js"), "").unwrap();
        let store = services.plugins.store.clone();
        let prepared = store
            .prepare(PackageSource::Path(package), InstallOptions::default())
            .unwrap();
        store.commit(prepared).unwrap();
        let loaded = || services.plugins.inner().loaded.contains_key(PLUGIN);
        let end = Instant::now() + STEP;
        while !loaded() {
            assert!(Instant::now() < end, "the plugin was not read");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let launcher = Launcher::Wsl {
            distro: "Ubuntu".to_owned(),
            user: None,
        };
        services
            .state
            .reports()
            .started(key.clone(), launcher, None);
        let noted = Arc::new(Noted::default());
        services
            .plugins
            .elsewhere
            .machines
            .put(UBUNTU, noted.clone());
        (services, key, noted)
    }

    fn git(args: &[&str], machine: Option<&str>) -> Call {
        let mut run = json!({ "program": "git", "args": args, "options": { "cwd": "/home/me" } });
        if let Some(machine) = machine {
            run["options"]["machine"] = json!(machine);
        }
        Call::decode("process.run", run).unwrap()
    }

    async fn run(services: &Services, call: Call) -> Result<Value, ApiError> {
        services.plugins.serve_system(PLUGIN, call).await
    }

    /// Has the user's yes be held, as it is once they gave it.
    fn allow(services: &Services, programs: &[&str]) {
        let agreed = OnMachine {
            label: "Ubuntu (WSL)".to_owned(),
            programs: programs.iter().map(|name| (*name).to_owned()).collect(),
        };
        let plugins = &services.plugins;
        plugins.store.allow_on(PLUGIN, UBUNTU, &agreed).unwrap();
        let mut inner = plugins.inner();
        let entry = &mut inner.loaded.get_mut(PLUGIN).unwrap().installed.entry;
        entry.machines.insert(UBUNTU.to_owned(), agreed);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_program_runs_elsewhere_only_where_the_user_agreed_and_a_session_is() {
        let dir = TempDir::new("plugin-elsewhere");
        let (services, key, noted) = with_plugin(&dir).await;
        let there = || git(&["status"], Some(UBUNTU));
        let why = |result: Result<Value, ApiError>| {
            let error = result.unwrap_err();
            (error.code, error.kind)
        };
        // What such a call needs is its own permission, and not the one for here.
        assert_eq!(there().permission(), Some("process.run-elsewhere"));
        assert_eq!(git(&[], None).permission(), Some("process.run"));
        let spawn = json!({ "program": "git", "options": { "machine": UBUNTU } });
        let spawn = Call::decode("process.spawn", spawn).unwrap();
        assert_eq!(spawn.permission(), Some("process.run-elsewhere"));

        let not_allowed = (ErrorCode::Denied, Some(NOT_ALLOWED.to_owned()));
        assert_eq!(why(run(&services, there()).await), not_allowed);
        assert_eq!(why(run(&services, spawn.clone()).await), not_allowed);
        // Here it may run nothing, whatever it may elsewhere.
        assert_eq!(
            why(run(&services, git(&[], None)).await).0,
            ErrorCode::Denied
        );
        let list = services.plugins.machines_list(PLUGIN);
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].machine.as_str(), list[0].allowed), (UBUNTU, false));

        allow(&services, &["git"]);
        assert!(services.plugins.machines_list(PLUGIN)[0].allowed);
        let answer = run(&services, there()).await.unwrap();
        assert_eq!(answer["stdout"], "from there");
        assert_eq!(answer["code"], 0);
        let asked = lock(&noted.asked).clone();
        assert_eq!(
            asked,
            [Asked {
                program: "git".to_owned(),
                args: vec!["status".to_owned()],
                folder: Some("/home/me".to_owned()),
                env: Vec::new(),
            }]
        );
        let failed = git(&["fail"], Some(UBUNTU));
        let error = run(&services, failed).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);
        assert_eq!(error.kind.as_deref(), Some("folder_missing"));
        let data = json!({ "machine": UBUNTU, "label": "Ubuntu (WSL)" });
        assert_eq!(error.data, Some(data));
        lock(&noted.asked).pop();
        // Another program, though the user once agreed to it there.
        allow(&services, &["git", "curl"]);
        let curl = json!({ "program": "curl", "options": { "machine": UBUNTU } });
        let curl = Call::decode("process.run", curl).unwrap();
        assert_eq!(why(run(&services, curl).await), (ErrorCode::Denied, None));
        // What the grant has grown by since is not agreed to.
        allow(&services, &[]);
        assert!(!services.plugins.machines_list(PLUGIN)[0].allowed);
        assert_eq!(why(run(&services, there()).await), not_allowed);

        allow(&services, &["git"]);
        services
            .plugins
            .revoke_machine(PLUGIN, UBUNTU)
            .await
            .unwrap();
        assert_eq!(why(run(&services, there()).await), not_allowed);

        // Agreed to, but nobody is there any more.
        allow(&services, &["git"]);
        services.state.reports().forget(&key);
        let not_connected = (ErrorCode::Unavailable, Some(NOT_CONNECTED.to_owned()));
        assert_eq!(why(run(&services, there()).await), not_connected);
        assert_eq!(why(run(&services, spawn).await), not_connected);
        assert!(services.plugins.machines_list(PLUGIN).is_empty());
        assert_eq!(lock(&noted.asked).len(), 1);
        services.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_question_is_not_put_again_for_a_while_after_a_no_nor_twice_at_once() {
        let dir = TempDir::new("plugin-elsewhere-asks");
        let (services, _, _) = with_plugin(&dir).await;
        let plugins = &services.plugins;
        let ask = || MachinesAllow {
            machine: UBUNTU.to_owned(),
            reason: None,
        };
        // Nobody can be shown a question here: that it fails is how one is told that it
        // would have been put.
        let error = plugins.allow_machine(PLUGIN, ask()).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::Failed);
        let key = (PLUGIN.to_owned(), UBUNTU.to_owned());
        lock(&plugins.elsewhere.refused).insert(key.clone(), Instant::now());
        assert!(!plugins.allow_machine(PLUGIN, ask()).await.unwrap());
        lock(&plugins.elsewhere.refused).clear();
        let on_show = OnShow::begin(&plugins.elsewhere.asking, &key).unwrap();
        assert!(!plugins.allow_machine(PLUGIN, ask()).await.unwrap());
        drop(on_show);
        assert!(plugins.allow_machine(PLUGIN, ask()).await.is_err());

        allow(&services, &["git"]);
        assert!(plugins.allow_machine(PLUGIN, ask()).await.unwrap());
        let elsewhere = MachinesAllow {
            machine: "wsl:Debian".to_owned(),
            reason: None,
        };
        let error = plugins.allow_machine(PLUGIN, elsewhere).await.unwrap_err();
        assert_eq!(error.kind.as_deref(), Some(NOT_CONNECTED));
        services.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn so_many_run_at_once_and_what_runs_ends_with_its_plugin() {
        let dir = TempDir::new("plugin-elsewhere-ends");
        let (services, _, noted) = with_plugin(&dir).await;
        allow(&services, &["git"]);
        let start = || {
            let services = Arc::clone(&services);
            tokio::spawn(async move { run(&services, git(&["wait"], Some(UBUNTU))).await })
        };
        let runs: Vec<_> = (0..AT_ONCE + 3).map(|_| start()).collect();
        let end = Instant::now() + STEP;
        while noted.running.load(Ordering::SeqCst) < AT_ONCE {
            assert!(Instant::now() < end, "they do not run");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        services.plugins.release(PLUGIN);
        let mut ended = 0;
        for run in runs {
            match tokio::time::timeout(STEP, run).await.unwrap().unwrap() {
                Ok(answer) => {
                    assert!(answer["code"].is_null(), "{answer}");
                    ended += 1;
                }
                Err(error) => assert_eq!(error.code, ErrorCode::Stopped),
            }
        }
        assert_eq!(ended, AT_ONCE);
        assert_eq!(noted.most.load(Ordering::SeqCst), AT_ONCE);
        assert_eq!(lock(&noted.asked).len(), AT_ONCE);
        // Nothing is left of it that would end what it runs next.
        noted.go_on.store(true, Ordering::SeqCst);
        let answer = run(&services, git(&["wait"], Some(UBUNTU))).await.unwrap();
        assert_eq!(answer["code"], 0);
        services.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_session_on_another_machine_has_a_place_there_and_no_folder_here() {
        let dir = TempDir::new("plugin-elsewhere-place");
        let (services, key, _) = with_plugin(&dir).await;
        let entry = async || {
            let listed = services.plugins.session_list().await.unwrap();
            let listed: Vec<SessionEntry> = serde_json::from_value(listed).unwrap();
            let id = key.session.to_string();
            listed
                .into_iter()
                .find(|entry| entry.session == id)
                .unwrap()
        };
        assert_eq!(entry().await.place.unwrap().folder, None);
        let url = format!("file://{}/home/me", machine_name().unwrap());
        says_it_is_in(&services, &key, &url);
        let found = entry().await;
        assert_eq!(found.cwd, None);
        let place = found.place.unwrap();
        assert_eq!(
            (place.machine.as_str(), place.label.as_str()),
            (UBUNTU, "Ubuntu")
        );
        assert_eq!(place.folder.as_deref(), Some("/home/me"));

        // A plugin hears where it is as a report of its own kind, and never as a folder
        // of this machine.
        let told = |event| {
            services.plugins.reports_of(&crate::sessions::News {
                key: key.clone(),
                what: SessionNews::Event(event),
            })
        };
        let place = ("place".to_owned(), serde_json::to_value(&place).unwrap());
        assert_eq!(
            told(SessionEvent::Cwd { url }),
            std::slice::from_ref(&place)
        );
        let state = services.state.reports().state(&key).unwrap();
        let whole = told(SessionEvent::State { state });
        assert_eq!((whole.len(), &whole[1]), (2, &place));
        assert_eq!(whole[0].0, "state");
        assert!(!whole[0].1.to_string().contains("/home/me"), "{whole:?}");
        assert_eq!(told(SessionEvent::Bell).len(), 1);
        services.state.shutdown().await;
    }
}
