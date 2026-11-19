//! The installed plugins, while the application runs.
//!
//! Plugins are read only once the first terminal has shown its first output, so that
//! however many are installed the start is not slowed by them. Each plugin that can be
//! used adds its commands to the registry, and its code is started when one of its
//! activation events happens: the application started, one of its commands was run, a
//! session reported something it listens for. Its calls come here, pass the broker,
//! and are carried out.
//!
//! A plugin that goes over its limits is stopped; it starts again when it is next
//! wanted. Three times in a minute and it is turned off until the user turns it on.
//! Changes made with `lume plugin` reach the application through the lock file, which is
//! watched.
//!
//! What the plugins add to the windows (menus, status items, panels, pictures, motion,
//! cursor effects) is worked out here from their manifests, without their code, and
//! pages fetch it whole and are told when it changes. What their code shows in those
//! (a status item's value, a panel's rows) reaches pages at most ten times a second.
//! Pages say which of the plugins' items they have on screen: that starts a plugin
//! whose manifest waits for one of its items to be seen, and the machine is read for a
//! plugin's meters only while they are seen.
//!
//! What a plugin's code may reach beyond its own things (files, programs, servers, the
//! terminals, the workspace, the user's attention, other plugins) is carried out in the
//! modules below, each call only within what the plugin was granted.

mod asking;
mod calls;
mod manage;
mod system;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

use lume_config::Dirs;
use lume_plugin::Store;
use lume_plugin::broker::{Broker, allows, append, holds};
use lume_plugin::contributions::{self, Commands, Contributions, Install, PanelDef};
use lume_plugin::id::{check_name, full_id};
use lume_plugin::manifest::{Activation, Manifest};
use lume_plugin::order::{Resolution, State};
use lume_plugin::permissions::{Kind, Permissions, ScopeKind};
use lume_plugin::store::{Check, Installed};
use lume_plugin_api::types::{PanelRow, StatusValue, Topic};
use lume_plugin_api::{ApiError, Call, ErrorCode};
use lume_plugin_runtime::{Breach, Host, Level, Limits, Outbound, Plugin, Spec};
use lume_services::metrics::{Hold, Listening, Sampler};
use notify::{RecursiveMode, Watcher};
use serde_json::{Map, Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::clipboard::ClipboardThread;
use crate::configsvc::{ConfigService, PluginConfig};
use crate::error::CommandError;
use crate::registry::{CommandSpec, Owner, PluginAccess, Registry, Runs, plugin_access};
use crate::services::{Asker, Services};
use crate::sessions::News;
use crate::state::AppState;
use lume_proto::app::SessionNews;

/// How long after the start the plugins are read when no terminal shows any output.
const WITHOUT_OUTPUT: Duration = Duration::from_secs(2);

/// How long a plugin's code has to start, and to carry out a command.
const START_WAIT: Duration = Duration::from_secs(10);
const COMMAND_WAIT: Duration = Duration::from_secs(30);

/// How many breaches in how long turn a plugin off.
const BREACHES_TO_TURN_OFF: usize = 3;
const BREACH_WINDOW: Duration = Duration::from_secs(60);

/// How long a plugin may do nothing before its code is stopped, and how often that is
/// looked at.
const IDLE_UNLOAD: Duration = Duration::from_secs(5 * 60);
const IDLE_LOOK: Duration = Duration::from_secs(30);

/// How long after the lock file changes the plugins are read again.
const LOCK_SETTLE: Duration = Duration::from_millis(300);

/// The lines of each plugin's log kept in memory, how many it may write in a second,
/// and how large its log file grows before it is set aside.
const LOG_LINES_KEPT: usize = 200;
const LOG_LINES_PER_SECOND: u32 = 100;
const LOG_FILE_MAX: u64 = 1024 * 1024;

/// What a plugin may store with `lume.kv`, all together, and how long a key may be.
const KV_MAX_BYTES: usize = 1024 * 1024;
const KV_KEY_MAX: usize = 256;

/// The file `lume.kv` keeps in a plugin's data folder.
const KV_FILE: &str = "kv.json";

/// How often what plugins' code shows is told to pages, at most.
const SHOW_EVERY: Duration = Duration::from_millis(100);

/// How many rows a list panel may hold, how deep, and how long a row's words may be.
const PANEL_ROWS_MAX: usize = 5000;
const PANEL_DEPTH_MAX: usize = 8;
const PANEL_TEXT_MAX: usize = 200;

/// A message a plugin's thread sent, with which run of its code sent it.
struct Message {
    local: String,
    generation: u64,
    outbound: Outbound,
}

/// Where a running plugin's messages go: to the manager's queue.
struct Link {
    local: String,
    generation: u64,
    queue: mpsc::UnboundedSender<Message>,
}

impl Host for Link {
    fn send(&self, outbound: Outbound) {
        let _ = self.queue.send(Message {
            local: self.local.clone(),
            generation: self.generation,
            outbound,
        });
    }
}

/// A plugin's code, running.
struct Running {
    plugin: Arc<Plugin>,
    generation: u64,
    started: bool,
    waiting: Vec<oneshot::Sender<Result<(), String>>>,
}

/// How a plugin has fared.
#[derive(Default)]
struct Health {
    breaches: VecDeque<Instant>,
    last_problem: Option<String>,
    starts: u32,
    /// What of its manifest could not be done as it asks: a thing left out, a key not
    /// bound, an anchor not found.
    warnings: Vec<String>,
}

/// The rate a plugin's log is written at.
struct LogRate {
    second: Instant,
    lines: u32,
    left_out: u32,
}

/// One installed plugin, as the application has it.
struct Loaded {
    installed: Installed,
    state: State,
    grants: Permissions,
    running: Option<Running>,
    health: Health,
    /// What its status items show, by item and pane.
    status: BTreeMap<(String, Option<String>), Value>,
    contexts: BTreeMap<String, bool>,
    topics: BTreeSet<Topic>,
    log: VecDeque<String>,
    rate: LogRate,
    /// The settings it was last told of.
    told_settings: Option<Value>,
    /// What each of its list panels shows, by the panel's name.
    panels: BTreeMap<String, Value>,
}

impl Loaded {
    fn manifest(&self) -> Option<&Manifest> {
        self.installed.manifest.as_ref().ok()
    }

    fn is_ready(&self) -> bool {
        self.state == State::Ready
    }
}

/// A command given to a plugin: the plugin, the run of its code, and who waits.
type Invocation = (String, u64, oneshot::Sender<Result<Value, ApiError>>);

#[derive(Default)]
struct Inner {
    loaded: BTreeMap<String, Loaded>,
    resolution: Resolution,
    /// Commands given to plugins and not yet carried out: by invocation, the plugin,
    /// the run of its code, and who waits.
    invocations: HashMap<u64, Invocation>,
    /// The hashes of packages the user has been told were changed.
    told_modified: HashSet<String>,
    /// What the plugins add to the windows, as pages fetch it.
    shown: Arc<Shown>,
    /// The plugins' things each window has on screen, by the window's label.
    visible: HashMap<String, BTreeSet<String>>,
    /// The windows that are minimised, whose things nobody sees.
    minimised: HashSet<String>,
    /// What waits to be told to pages.
    pending: Pending,
    metrics: Metrics,
}

/// What the plugins add to the windows, and how many times that has changed.
#[derive(Default)]
struct Shown {
    revision: u64,
    contributions: Contributions,
}

/// What plugins' code showed that pages have not been told yet: status items' values by
/// (install, item, pane), panels' rows by (install, panel).
#[derive(Default)]
struct Pending {
    status: BTreeSet<(String, String, Option<String>)>,
    panels: BTreeSet<(String, String)>,
    /// Whether something is on its way to tell them.
    telling: bool,
}

/// The machine's readings plugins asked for.
#[derive(Default)]
struct Metrics {
    sampler: Option<Sampler>,
    hold: Option<Hold>,
    listening: Option<Listening>,
    subscriptions: BTreeMap<u64, Subscription>,
}

/// One plugin's wish for readings: the run of its code that asked, and the status items
/// of its own, by full id, they are for.
struct Subscription {
    local: String,
    generation: u64,
    visible: Vec<String>,
}

/// The application's commands, as what the plugins add needs to know of them.
struct Known<'a>(&'a Registry);

impl Commands for Known<'_> {
    fn builtin(&self, id: &str) -> Option<(String, bool)> {
        let spec = self.0.get(id)?;
        Some((spec.title, plugin_access(id) == PluginAccess::Open))
    }
}

/// The installed plugins and their running code.
pub struct Plugins {
    store: Store,
    broker: Broker,
    registry: Arc<Registry>,
    state: Arc<AppState>,
    config: Arc<ConfigService>,
    services: OnceLock<Weak<Services>>,
    inner: Mutex<Inner>,
    queue: mpsc::UnboundedSender<Message>,
    received: Mutex<Option<mpsc::UnboundedReceiver<Message>>>,
    next: AtomicU64,
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
    /// The machine's readings, from the sampler's thread.
    samples: mpsc::UnboundedSender<Value>,
    sampled: Mutex<Option<mpsc::UnboundedReceiver<Value>>>,
    /// The way to the clipboard, once the application has opened it.
    clipboard: OnceLock<ClipboardThread>,
    /// What plugins' code has open on the machine: streams, watches, programs.
    system: system::Held,
    /// The workspace plugins were last told of.
    told_workspace: Mutex<Option<Value>>,
    /// Packages the settings window is installing, waiting for the user's word.
    waiting: manage::Waiting,
}

fn api_error(code: ErrorCode, message: impl Into<String>) -> ApiError {
    ApiError::new(code, message)
}

impl Plugins {
    pub fn new(
        dirs: &Dirs,
        registry: Arc<Registry>,
        state: Arc<AppState>,
        config: Arc<ConfigService>,
    ) -> Arc<Self> {
        let store = Store::of(dirs);
        let broker = Broker::new(Some(store.audit_file()));
        let (queue, received) = mpsc::unbounded_channel();
        let (samples, sampled) = mpsc::unbounded_channel();
        Arc::new(Self {
            store,
            broker,
            registry,
            state,
            config,
            services: OnceLock::new(),
            inner: Mutex::new(Inner::default()),
            queue,
            received: Mutex::new(Some(received)),
            next: AtomicU64::new(1),
            watcher: Mutex::new(None),
            samples,
            sampled: Mutex::new(Some(sampled)),
            clipboard: OnceLock::new(),
            system: system::Held::default(),
            told_workspace: Mutex::new(None),
            waiting: manage::Waiting::default(),
        })
    }

    /// Gives plugins the way to the clipboard.
    pub fn set_clipboard(&self, clipboard: ClipboardThread) {
        let _ = self.clipboard.set(clipboard);
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn services(&self) -> Option<Arc<Services>> {
        self.services.get().and_then(Weak::upgrade)
    }

    fn next(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    fn notice(&self, level: &str, message: String) {
        match self.services() {
            Some(services) => services.panes.notice(level, message),
            None => tracing::info!(%message, "a notice about plugins had no window to go to"),
        }
    }

    /// Starts following the plugins: once the first output is out, they are read, and
    /// from then on their messages, the sessions' reports and the lock file are followed.
    pub fn start(self: &Arc<Self>, services: &Arc<Services>) {
        let _ = self.services.set(Arc::downgrade(services));
        let this = Arc::clone(self);
        self.state.runtime().spawn(async move {
            let began = Instant::now();
            while !this.state.marks().has_output() && began.elapsed() < WITHOUT_OUTPUT {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            if let Some(received) = this
                .received
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
            {
                tokio::spawn(Arc::clone(&this).follow_messages(received));
            }
            if let Some(sampled) = this
                .sampled
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
            {
                tokio::spawn(Arc::clone(&this).follow_samples(sampled));
            }
            this.reload().await;
            this.state.marks().plugins();
            this.watch_lock();
            tokio::spawn(Arc::clone(&this).follow_sessions());
            tokio::spawn(Arc::clone(&this).look_for_idle());
        });
    }

    /// Reads what is installed again and follows it: plugins that went or changed stop,
    /// new ones add their commands, those that start at once start.
    pub async fn reload(self: &Arc<Self>) {
        let store = self.store.clone();
        let marks_state = Arc::clone(&self.state);
        let read = tokio::task::spawn_blocking(move || {
            let read = store.resolve(true);
            if let Ok((installed, _)) = &read {
                for _ in installed {
                    marks_state.marks().plugin_file();
                }
            }
            read
        })
        .await;
        let (installed, resolution) = match read {
            Ok(Ok(read)) => read,
            Ok(Err(error)) => {
                tracing::warn!(%error, "the installed plugins could not be read");
                self.notice(
                    "error",
                    format!("The installed plugins could not be read: {error}"),
                );
                return;
            }
            Err(_) => return,
        };
        let starting = self.apply(installed, resolution);
        self.rebuild().await;
        self.state.broadcast(&json!({ "type": "commands_changed" }));
        self.state.broadcast(&json!({ "type": "plugins_changed" }));
        for local in starting {
            if let Err(problem) = self.activate(&local).await {
                tracing::warn!(plugin = %local, %problem, "a plugin that starts with the application did not start");
            }
        }
    }

    /// Works out again what the plugins that can be used add, from their manifests and
    /// packages, off the async threads; gives the configuration its part; tells pages.
    async fn rebuild(self: &Arc<Self>) {
        struct Owned {
            local: String,
            name: String,
            manifest: Manifest,
            dir: std::path::PathBuf,
            rank: i64,
            bound: BTreeMap<String, String>,
        }
        let (owned, enabled): (Vec<Owned>, BTreeSet<String>) = {
            let inner = self.inner();
            let owned = inner
                .resolution
                .order
                .iter()
                .filter_map(|local| inner.loaded.get(local).map(|loaded| (local, loaded)))
                .filter(|(_, loaded)| loaded.is_ready())
                .filter_map(|(local, loaded)| {
                    Some(Owned {
                        local: local.clone(),
                        name: loaded.installed.name(),
                        manifest: loaded.manifest()?.clone(),
                        dir: self.store.package_dir(&loaded.installed.entry),
                        rank: inner
                            .resolution
                            .rank(local)
                            .and_then(|rank| i64::try_from(rank).ok())
                            .unwrap_or(i64::MAX),
                        bound: inner
                            .resolution
                            .bound
                            .get(local)
                            .cloned()
                            .unwrap_or_default(),
                    })
                })
                .collect();
            let enabled = inner
                .loaded
                .iter()
                .filter(|(_, loaded)| loaded.installed.entry.enabled)
                .map(|(local, _)| local.clone())
                .collect();
            (owned, enabled)
        };
        let registry = Arc::clone(&self.registry);
        let built = tokio::task::spawn_blocking(move || {
            let installs: Vec<Install<'_>> = owned
                .iter()
                .map(|one| Install {
                    local: &one.local,
                    name: one.name.clone(),
                    manifest: &one.manifest,
                    dir: one.dir.clone(),
                    rank: one.rank,
                    bound: &one.bound,
                })
                .collect();
            contributions::build(&installs, &Known(&registry))
        })
        .await;
        let Ok(built) = built else {
            return;
        };
        let schema = contributions::compose_schema(lume_config::config_schema(), &built);
        let config = PluginConfig {
            themes: built.themes.clone(),
            keys: built
                .keys
                .iter()
                .map(|key| lume_config::PluginBinding {
                    plugin: key.plugin.clone(),
                    keys: key.key.clone(),
                    macos: key.macos.clone(),
                    command: key.command.clone(),
                    args: key.args.clone(),
                    when: key.when.clone(),
                })
                .collect(),
            fragments: built.settings.fragments.clone(),
            schema: Some(Arc::new(schema)),
            enabled,
        };
        let revision = {
            let mut inner = self.inner();
            for loaded in inner.loaded.values_mut() {
                loaded.health.warnings.clear();
            }
            for (local, warning) in &built.warnings {
                if let Some(loaded) = inner.loaded.get_mut(local) {
                    loaded.health.warnings.push(warning.clone());
                }
            }
            let revision = inner.shown.revision + 1;
            inner.shown = Arc::new(Shown {
                revision,
                contributions: built,
            });
            revision
        };
        self.config.set_plugins(config);
        // The keys the configuration could not bind for them.
        let dropped = self.config.dropped_keys();
        {
            let mut inner = self.inner();
            for key in dropped {
                if let Some(loaded) = inner.loaded.get_mut(&key.plugin) {
                    loaded.health.warnings.push(key.message);
                }
            }
        }
        self.state.broadcast(&json!({
            "type": "contributions_changed",
            "revision": revision,
        }));
        self.update_metrics();
    }

    /// Puts what was read in place of what was there. Returns the plugins to start now.
    fn apply(&self, installed: Vec<Installed>, resolution: Resolution) -> Vec<String> {
        let mut inner = self.inner();
        let mut old = std::mem::take(&mut inner.loaded);
        let mut starting = Vec::new();
        let mut released: Vec<String> = Vec::new();
        let mut modified = Vec::new();
        for one in installed {
            let local = one.entry.local.clone();
            let state = resolution
                .states
                .get(&local)
                .cloned()
                .unwrap_or(State::Inactive("it was not resolved".to_owned()));
            if one.check == Check::Modified
                && !inner.told_modified.contains(&one.entry.content_hash)
            {
                inner.told_modified.insert(one.entry.content_hash.clone());
                modified.push(one.name());
            }
            let grants =
                lume_plugin::permissions::parse_table(&one.entry.grants).unwrap_or_default();
            let previous = old.remove(&local);
            let same = previous.as_ref().is_some_and(|previous| {
                previous.installed.entry.content_hash == one.entry.content_hash
                    && previous.is_ready()
                    && state == State::Ready
            });
            let mut loaded = match previous {
                Some(mut previous) if same => {
                    previous.installed = one;
                    previous.grants = grants;
                    previous.state = state;
                    previous
                }
                previous => {
                    if let Some(mut previous) = previous
                        && let Some(running) = previous.running.take()
                    {
                        running.plugin.stop();
                        released.push(local.clone());
                    }
                    Loaded {
                        installed: one,
                        state,
                        grants,
                        running: None,
                        health: Health::default(),
                        status: BTreeMap::new(),
                        contexts: BTreeMap::new(),
                        topics: BTreeSet::new(),
                        log: VecDeque::new(),
                        rate: LogRate {
                            second: Instant::now(),
                            lines: 0,
                            left_out: 0,
                        },
                        told_settings: None,
                        panels: BTreeMap::new(),
                    }
                }
            };
            self.registry.unregister_plugin(&local);
            if loaded.is_ready()
                && let Some(manifest) = loaded.installed.manifest.as_ref().ok().cloned()
            {
                for command in &manifest.contributes.commands {
                    let spec = CommandSpec {
                        id: full_id(&local, &command.id),
                        title: command.title.clone(),
                        category: command
                            .category
                            .clone()
                            .unwrap_or_else(|| loaded.installed.name()),
                        runs: Runs::Plugin,
                        palette: command.palette,
                        args: None,
                    };
                    if let Err(error) = self.registry.register(Owner::Plugin(&local), spec) {
                        loaded.health.last_problem = Some(error.to_string());
                    }
                }
                if manifest.events.contains(&Activation::Startup) && loaded.running.is_none() {
                    starting.push(local.clone());
                }
            }
            inner.loaded.insert(local, loaded);
        }
        // Plugins that are no longer installed.
        for (local, mut gone) in old {
            self.registry.unregister_plugin(&local);
            if let Some(running) = gone.running.take() {
                running.plugin.stop();
                released.push(local);
            }
        }
        // In the order they load in.
        starting.sort_by_key(|local| resolution.rank(local));
        inner.resolution = resolution;
        drop(inner);
        // What the stopped ones had open on the machine goes with them.
        for local in released {
            self.release(&local);
        }
        for name in modified {
            self.notice(
                "error",
                format!(
                    "{name} is not used: its files have changed since it was installed. \
                     Install it again to use it."
                ),
            );
        }
        starting
    }

    /// Starts a plugin's code, and that of the plugins it depends on first, if it is not
    /// running yet.
    pub async fn activate(self: &Arc<Self>, local: &str) -> Result<(), String> {
        let dependencies: Vec<(String, bool)> = {
            let inner = self.inner();
            let loaded = inner
                .loaded
                .get(local)
                .ok_or_else(|| format!("no plugin is installed as {local}"))?;
            if let State::Inactive(why) = &loaded.state {
                return Err(format!("{} cannot be used: {why}", loaded.installed.name()));
            }
            let bound = inner
                .resolution
                .bound
                .get(local)
                .cloned()
                .unwrap_or_default();
            loaded
                .manifest()
                .map(|manifest| {
                    manifest
                        .dependencies
                        .iter()
                        .filter_map(|(wanted, dependency)| {
                            bound
                                .get(wanted)
                                .map(|target| (target.clone(), dependency.optional()))
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        for (dependency, optional) in dependencies {
            let has_code = self
                .inner()
                .loaded
                .get(&dependency)
                .and_then(Loaded::manifest)
                .is_some_and(|manifest| manifest.main.is_some());
            if !has_code {
                continue;
            }
            let started = Box::pin(self.activate(&dependency)).await;
            if let Err(problem) = started
                && !optional
            {
                return Err(format!(
                    "{dependency}, which it needs, did not start: {problem}"
                ));
            }
        }
        let waiting = self.begin(local)?;
        match waiting {
            None => Ok(()),
            Some(receiver) => match tokio::time::timeout(START_WAIT, receiver).await {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(_)) => Err("its code stopped while it started".to_owned()),
                Err(_) => Err("its code did not start in time".to_owned()),
            },
        }
    }

    /// Starts the code of a plugin whose dependencies run, unless it runs already.
    /// Returns what to wait on, or nothing when it has started.
    fn begin(&self, local: &str) -> Result<Option<oneshot::Receiver<Result<(), String>>>, String> {
        let (directory, manifest, name) = {
            let mut inner = self.inner();
            let loaded = inner
                .loaded
                .get_mut(local)
                .ok_or_else(|| format!("no plugin is installed as {local}"))?;
            match &mut loaded.running {
                Some(running) if running.started => return Ok(None),
                Some(running) => {
                    let (sender, receiver) = oneshot::channel();
                    running.waiting.push(sender);
                    return Ok(Some(receiver));
                }
                None => {}
            }
            let manifest = loaded
                .manifest()
                .cloned()
                .ok_or_else(|| "its manifest cannot be read".to_owned())?;
            (
                self.store.package_dir(&loaded.installed.entry),
                manifest,
                loaded.installed.name(),
            )
        };
        let Some(main) = manifest.main.clone() else {
            return Ok(None);
        };
        let code = std::fs::read_to_string(directory.join(&main))
            .map_err(|error| format!("its code cannot be read: {error}"))?;
        let source_map = std::fs::read_to_string(directory.join(format!("{main}.map"))).ok();
        self.state.marks().plugin_file();
        let generation = self.next();
        let spec = Spec {
            id: local.to_owned(),
            manifest_id: manifest.id.clone(),
            name,
            version: manifest.version.clone(),
            code,
            file: main,
            source_map,
            limits: Limits::with_memory_mib(manifest.limits.memory_mib()),
        };
        let link = Link {
            local: local.to_owned(),
            generation,
            queue: self.queue.clone(),
        };
        let mut inner = self.inner();
        let loaded = inner
            .loaded
            .get_mut(local)
            .ok_or_else(|| format!("{local} was removed while it started"))?;
        if let Some(running) = &mut loaded.running {
            // Another start got there while the code was read.
            if running.started {
                return Ok(None);
            }
            let (sender, receiver) = oneshot::channel();
            running.waiting.push(sender);
            return Ok(Some(receiver));
        }
        let plugin = Plugin::start(spec, Box::new(link)).map_err(|error| error.to_string())?;
        let (sender, receiver) = oneshot::channel();
        loaded.health.starts += 1;
        loaded.running = Some(Running {
            plugin: Arc::new(plugin),
            generation,
            started: false,
            waiting: vec![sender],
        });
        Ok(Some(receiver))
    }

    async fn follow_messages(self: Arc<Self>, mut received: mpsc::UnboundedReceiver<Message>) {
        while let Some(message) = received.recv().await {
            self.handle(message);
        }
    }

    /// The running code of a plugin, if this run of it is the current one.
    fn current(&self, local: &str, generation: u64) -> Option<Arc<Plugin>> {
        self.inner()
            .loaded
            .get(local)
            .and_then(|loaded| loaded.running.as_ref())
            .filter(|running| running.generation == generation)
            .map(|running| Arc::clone(&running.plugin))
    }

    fn handle(self: &Arc<Self>, message: Message) {
        let Message {
            local,
            generation,
            outbound,
        } = message;
        match outbound {
            Outbound::Log { level, message } => self.log(&local, level, &message),
            Outbound::Call { id, name, args } => {
                if let Some(plugin) = self.current(&local, generation) {
                    self.call(&local, plugin, id, &name, args);
                }
            }
            Outbound::CommandDone { invocation, result } => {
                if let Some((_, _, sender)) = self.inner().invocations.remove(&invocation) {
                    let _ = sender.send(result);
                }
            }
            Outbound::Started => {
                let mut inner = self.inner();
                if let Some(running) = inner
                    .loaded
                    .get_mut(&local)
                    .and_then(|loaded| loaded.running.as_mut())
                    .filter(|running| running.generation == generation)
                {
                    running.started = true;
                    for waiting in running.waiting.drain(..) {
                        let _ = waiting.send(Ok(()));
                    }
                }
            }
            Outbound::Failed { message } => {
                let name = self.ended(&local, generation, &message);
                if let Some(name) = name {
                    self.notice("error", format!("{name} could not start: {message}"));
                    self.tell_state(&local);
                }
            }
            Outbound::Breached { breach, detail } => {
                self.breached(&local, generation, breach, &detail)
            }
            Outbound::Hung => self.breached(
                &local,
                generation,
                Breach::Time,
                "it was busy for too long and could not be interrupted",
            ),
            Outbound::Stopped => {
                self.ended(&local, generation, "its code stopped");
            }
        }
    }

    /// The end of a run of a plugin's code: whoever waits is told. Returns the plugin's
    /// name when this was its current run.
    fn ended(&self, local: &str, generation: u64, why: &str) -> Option<String> {
        let mut inner = self.inner();
        let failed: Vec<u64> = inner
            .invocations
            .iter()
            .filter(|(_, (plugin, run, _))| plugin == local && *run == generation)
            .map(|(invocation, _)| *invocation)
            .collect();
        for invocation in failed {
            if let Some((_, _, sender)) = inner.invocations.remove(&invocation) {
                let _ = sender.send(Err(api_error(ErrorCode::Stopped, why)));
            }
        }
        let loaded = inner.loaded.get_mut(local)?;
        let running = loaded
            .running
            .take_if(|running| running.generation == generation)?;
        for waiting in running.waiting {
            let _ = waiting.send(Err(why.to_owned()));
        }
        if why != "its code stopped" {
            loaded.health.last_problem = Some(why.to_owned());
        }
        let name = loaded.installed.name();
        // Its wishes for readings ended with the run that made them.
        inner.metrics.subscriptions.retain(|_, subscription| {
            subscription.local != local || subscription.generation != generation
        });
        drop(inner);
        self.update_metrics();
        // So did what it had open on the machine.
        self.release(local);
        Some(name)
    }

    /// Tells pages how a plugin fares, so that its things can show it.
    fn tell_state(&self, local: &str) {
        let problem = self
            .inner()
            .loaded
            .get(local)
            .and_then(|loaded| loaded.health.last_problem.clone());
        self.state.broadcast(&json!({
            "type": "plugin_state",
            "plugin": local,
            "problem": problem,
        }));
    }

    fn breached(self: &Arc<Self>, local: &str, generation: u64, breach: Breach, detail: &str) {
        let Some(name) = self.ended(
            local,
            generation,
            &format!("it went over its {} limit: {detail}", breach.as_str()),
        ) else {
            return;
        };
        let turn_off = {
            let mut inner = self.inner();
            let Some(loaded) = inner.loaded.get_mut(local) else {
                return;
            };
            let now = Instant::now();
            loaded.health.breaches.push_back(now);
            while loaded
                .health
                .breaches
                .front()
                .is_some_and(|at| now.duration_since(*at) > BREACH_WINDOW)
            {
                loaded.health.breaches.pop_front();
            }
            loaded.health.breaches.len() >= BREACHES_TO_TURN_OFF
        };
        self.log(local, Level::Error, &format!("stopped: {detail}"));
        self.tell_state(local);
        if turn_off {
            self.notice(
                "error",
                format!(
                    "{name} was turned off: it went over its limits {BREACHES_TO_TURN_OFF} times \
                     in a minute ({detail}). Turn it on again with `lume plugin enable {local}`."
                ),
            );
            let store = self.store.clone();
            let local = local.to_owned();
            let this = Arc::clone(self);
            self.state.runtime().spawn(async move {
                let _ = tokio::task::spawn_blocking(move || store.set_enabled(&local, false)).await;
                this.reload().await;
            });
        } else {
            self.notice(
                "warning",
                format!("{name} was stopped: {detail}. It starts again when it is next needed."),
            );
        }
    }

    /// Writes a line to a plugin's log, keeping to its rate.
    fn log(&self, local: &str, level: Level, message: &str) {
        let line = format!(
            "{} {} {}",
            lume_plugin::broker::utc_now(),
            level.as_str(),
            message.trim_end()
        );
        let mut write = Vec::new();
        {
            let mut inner = self.inner();
            let Some(loaded) = inner.loaded.get_mut(local) else {
                return;
            };
            let rate = &mut loaded.rate;
            if rate.second.elapsed() >= Duration::from_secs(1) {
                if rate.left_out > 0 {
                    write.push(format!(
                        "{} warn {} lines were left out: the plugin wrote more than {LOG_LINES_PER_SECOND} a second",
                        lume_plugin::broker::utc_now(),
                        rate.left_out
                    ));
                }
                rate.second = Instant::now();
                rate.lines = 0;
                rate.left_out = 0;
            }
            rate.lines += 1;
            if rate.lines > LOG_LINES_PER_SECOND {
                rate.left_out += 1;
            } else {
                write.push(line);
            }
            for line in &write {
                loaded.log.push_back(line.clone());
                while loaded.log.len() > LOG_LINES_KEPT {
                    loaded.log.pop_front();
                }
            }
        }
        if level == Level::Error {
            tracing::debug!(plugin = %local, "a plugin logged an error");
        }
        let file = self.store.log_file(local);
        for line in write {
            append(&file, &line, LOG_FILE_MAX);
        }
    }

    /// Checks a call against the plugin's grants, and carries it out.
    fn call(self: &Arc<Self>, local: &str, plugin: Arc<Plugin>, id: u64, name: &str, args: Value) {
        let call = match Call::decode(name, args) {
            Ok(call) => call,
            Err(error) => {
                plugin.answer(id, Err(api_error(ErrorCode::Invalid, error.to_string())));
                return;
            }
        };
        if let Some(permission) = call.permission() {
            let granted = self
                .inner()
                .loaded
                .get(local)
                .map(|loaded| loaded.grants.clone())
                .unwrap_or_default();
            let checked = match Kind::parse(permission) {
                Some(kind) if kind.scope() == ScopeKind::None => self
                    .broker
                    .check(local, &granted, kind, None)
                    .map_err(|denied| denied.message),
                // A kind with a scope is matched where the call is carried out, which
                // knows what the call reaches; here, only that some of it is held.
                Some(kind) if holds(&granted, kind) => Ok(()),
                Some(kind) => {
                    self.broker.record(local, kind, None, false);
                    Err(format!("{local} has no permission for {kind}"))
                }
                None => Err(format!("{permission} is not a permission")),
            };
            if let Err(message) = checked {
                plugin.answer(id, Err(api_error(ErrorCode::Denied, message)));
                return;
            }
        }
        let this = Arc::clone(self);
        let local = local.to_owned();
        self.state.runtime().spawn(async move {
            let result = this.serve(&local, call).await;
            plugin.answer(id, result);
        });
    }

    async fn serve(self: &Arc<Self>, local: &str, call: Call) -> Result<Value, ApiError> {
        match call {
            Call::StatusSet(set) => self.status_set(local, &set.id, set.pane, Some(set.value)),
            Call::StatusClear(clear) => self.status_set(local, &clear.id, clear.pane, None),
            Call::CommandsExecute(execute) => self.execute(local, &execute.id, execute.args).await,
            Call::KvGet(get) => {
                self.kv(local, move |map| {
                    Ok(map.get(&get.key).cloned().unwrap_or(Value::Null))
                })
                .await
            }
            Call::KvSet(set) => {
                if set.key.is_empty() || set.key.len() > KV_KEY_MAX {
                    return Err(api_error(
                        ErrorCode::Invalid,
                        format!("a key has 1 to {KV_KEY_MAX} characters"),
                    ));
                }
                self.kv(local, move |map| {
                    map.insert(set.key, set.value);
                    Ok(Value::Null)
                })
                .await
            }
            Call::KvDelete(delete) => {
                self.kv(local, move |map| {
                    map.remove(&delete.key);
                    Ok(Value::Null)
                })
                .await
            }
            Call::KvList(_) => {
                self.kv(local, |map| {
                    Ok(json!(map.keys().cloned().collect::<Vec<_>>()))
                })
                .await
            }
            Call::SettingsGet(get) => {
                let settings = self.settings_of(local);
                Ok(match get.key {
                    Some(key) => settings.get(&key).cloned().unwrap_or(Value::Null),
                    None => settings,
                })
            }
            Call::ContextSet(set) => {
                check_name(&set.key).map_err(|problem| api_error(ErrorCode::Invalid, problem))?;
                let mut inner = self.inner();
                if let Some(loaded) = inner.loaded.get_mut(local) {
                    loaded.contexts.insert(set.key.clone(), set.value);
                }
                drop(inner);
                self.state.broadcast(&json!({
                    "type": "plugin_context",
                    "plugin": local,
                    "key": set.key,
                    "value": set.value,
                }));
                Ok(Value::Null)
            }
            Call::EventsSubscribe(subscribe) => {
                let mut inner = self.inner();
                if let Some(loaded) = inner.loaded.get_mut(local) {
                    loaded.topics.insert(subscribe.topic);
                }
                Ok(Value::Null)
            }
            Call::PanelsSet(set) => self.panel_set(local, &set.id, Some(&set.rows)),
            Call::PanelsClear(clear) => self.panel_set(local, &clear.id, None),
            Call::MetricsSnapshot(_) => {
                let latest = self
                    .inner()
                    .metrics
                    .sampler
                    .as_ref()
                    .and_then(Sampler::latest);
                Ok(latest.map_or(Value::Null, |sample| {
                    serde_json::to_value(sample).unwrap_or(Value::Null)
                }))
            }
            Call::MetricsSubscribe(subscribe) => {
                let id = self.next();
                {
                    let mut inner = self.inner();
                    let Some(loaded) = inner.loaded.get(local) else {
                        return Err(api_error(ErrorCode::Stopped, "the plugin is not installed"));
                    };
                    let declared: BTreeSet<&str> = loaded
                        .manifest()
                        .map(|manifest| {
                            manifest
                                .contributes
                                .status
                                .iter()
                                .map(|item| item.id.as_str())
                                .collect()
                        })
                        .unwrap_or_default();
                    if let Some(unknown) = subscribe
                        .visible
                        .iter()
                        .find(|item| !declared.contains(item.as_str()))
                    {
                        return Err(api_error(
                            ErrorCode::NotFound,
                            format!("the plugin's manifest has no status item {unknown}"),
                        ));
                    }
                    let generation = loaded
                        .running
                        .as_ref()
                        .map_or(0, |running| running.generation);
                    let visible = subscribe
                        .visible
                        .iter()
                        .map(|item| full_id(local, item))
                        .collect();
                    inner.metrics.subscriptions.insert(
                        id,
                        Subscription {
                            local: local.to_owned(),
                            generation,
                            visible,
                        },
                    );
                }
                self.update_metrics();
                Ok(json!(id))
            }
            Call::MetricsUnsubscribe(unsubscribe) => {
                {
                    let mut inner = self.inner();
                    let owned = inner
                        .metrics
                        .subscriptions
                        .get(&unsubscribe.subscription)
                        .is_some_and(|subscription| subscription.local == local);
                    if owned {
                        inner
                            .metrics
                            .subscriptions
                            .remove(&unsubscribe.subscription);
                    }
                }
                self.update_metrics();
                Ok(Value::Null)
            }
            // What reaches past the plugin's own things.
            other => self.serve_more(local, other).await,
        }
    }

    /// Shows rows in one of a plugin's list panels, or takes them away.
    fn panel_set(
        self: &Arc<Self>,
        local: &str,
        panel: &str,
        rows: Option<&[PanelRow]>,
    ) -> Result<Value, ApiError> {
        let rows = {
            let inner = self.inner();
            let loaded = inner
                .loaded
                .get(local)
                .ok_or_else(|| api_error(ErrorCode::Stopped, "the plugin is not installed"))?;
            let manifest = loaded
                .manifest()
                .ok_or_else(|| api_error(ErrorCode::Stopped, "the plugin is not installed"))?;
            if !manifest
                .contributes
                .panels
                .iter()
                .any(|known| known.id == panel)
            {
                return Err(api_error(
                    ErrorCode::NotFound,
                    format!("the plugin's manifest has no panel {panel}"),
                ));
            }
            let commands: BTreeSet<&str> = manifest
                .contributes
                .commands
                .iter()
                .map(|command| command.id.as_str())
                .collect();
            match rows {
                None => None,
                Some(rows) => {
                    let mut count = 0;
                    check_rows(rows, 1, &mut count, &commands)
                        .map_err(|problem| api_error(ErrorCode::Invalid, problem))?;
                    let mut value = serde_json::to_value(rows).unwrap_or(Value::Null);
                    full_commands(&mut value, local);
                    Some(value)
                }
            }
        };
        {
            let mut inner = self.inner();
            if let Some(loaded) = inner.loaded.get_mut(local) {
                match rows {
                    Some(rows) => {
                        loaded.panels.insert(panel.to_owned(), rows);
                    }
                    None => {
                        loaded.panels.remove(panel);
                    }
                }
            }
            inner
                .pending
                .panels
                .insert((local.to_owned(), panel.to_owned()));
        }
        self.tell_soon();
        Ok(Value::Null)
    }

    /// Tells pages, soon, what waits to be told: at most once every [`SHOW_EVERY`].
    fn tell_soon(self: &Arc<Self>) {
        {
            let mut inner = self.inner();
            if inner.pending.telling {
                return;
            }
            inner.pending.telling = true;
        }
        let this = Arc::clone(self);
        self.state.runtime().spawn(async move {
            loop {
                tokio::time::sleep(SHOW_EVERY).await;
                let (status, panels) = {
                    let mut inner = this.inner();
                    let status = std::mem::take(&mut inner.pending.status);
                    let panels = std::mem::take(&mut inner.pending.panels);
                    if status.is_empty() && panels.is_empty() {
                        inner.pending.telling = false;
                        return;
                    }
                    let status: Vec<Value> = status
                        .into_iter()
                        .map(|(local, item, pane)| {
                            let value = inner
                                .loaded
                                .get(&local)
                                .and_then(|loaded| loaded.status.get(&(item.clone(), pane.clone())))
                                .cloned()
                                .unwrap_or(Value::Null);
                            json!({
                                "type": "plugin_status",
                                "plugin": local,
                                "item": full_id(&local, &item),
                                "pane": pane,
                                "value": value,
                            })
                        })
                        .collect();
                    let panels: Vec<Value> = panels
                        .into_iter()
                        .map(|(local, panel)| {
                            let rows = inner
                                .loaded
                                .get(&local)
                                .and_then(|loaded| loaded.panels.get(&panel))
                                .cloned()
                                .unwrap_or(Value::Null);
                            json!({
                                "type": "plugin_panel",
                                "plugin": local,
                                "panel": full_id(&local, &panel),
                                "rows": rows,
                            })
                        })
                        .collect();
                    (status, panels)
                };
                for notice in status.iter().chain(&panels) {
                    this.state.broadcast(notice);
                }
            }
        });
    }

    /// Shows a value in one of a plugin's status items, or takes it back.
    fn status_set(
        self: &Arc<Self>,
        local: &str,
        item: &str,
        pane: Option<String>,
        value: Option<StatusValue>,
    ) -> Result<Value, ApiError> {
        {
            let mut inner = self.inner();
            let loaded = inner
                .loaded
                .get_mut(local)
                .ok_or_else(|| api_error(ErrorCode::Stopped, "the plugin is not installed"))?;
            let declared = loaded.manifest().is_some_and(|manifest| {
                manifest
                    .contributes
                    .status
                    .iter()
                    .any(|status| status.id == item)
            });
            if !declared {
                return Err(api_error(
                    ErrorCode::NotFound,
                    format!("the plugin's manifest has no status item {item}"),
                ));
            }
            if let Some(color) = value.as_ref().and_then(|value| value.color.as_deref()) {
                let named = lume_config::tokens::token_def(color)
                    .is_some_and(|token| token.kind == lume_config::TokenKind::Color);
                if !named {
                    return Err(api_error(
                        ErrorCode::Invalid,
                        format!("`{color}` is not one of the theme's colours, as color.tab.purple"),
                    ));
                }
            }
            let key = (item.to_owned(), pane.clone());
            match value {
                None => {
                    loaded.status.remove(&key);
                }
                Some(value) => {
                    let fresh = serde_json::to_value(value).unwrap_or(Value::Null);
                    let entry = loaded.status.entry(key).or_insert_with(|| json!({}));
                    if let (Some(old), Some(new)) = (entry.as_object_mut(), fresh.as_object()) {
                        for (field, value) in new {
                            old.insert(field.clone(), value.clone());
                        }
                    }
                }
            }
            inner
                .pending
                .status
                .insert((local.to_owned(), item.to_owned(), pane));
        }
        self.tell_soon();
        Ok(Value::Null)
    }

    /// Runs a command for a plugin: one of its own, or one of the application's that its
    /// permissions cover.
    async fn execute(
        self: &Arc<Self>,
        local: &str,
        id: &str,
        args: Option<Value>,
    ) -> Result<Value, ApiError> {
        let command = if id.starts_with("lume.") {
            let allowed = match plugin_access(id) {
                PluginAccess::Open => Ok(()),
                PluginAccess::UserOnly => {
                    Err(format!("{id} is run by the user only, not by plugins"))
                }
                PluginAccess::Needs(permission) => {
                    let granted = self
                        .inner()
                        .loaded
                        .get(local)
                        .map(|loaded| loaded.grants.clone())
                        .unwrap_or_default();
                    match Kind::parse(permission) {
                        Some(kind) => self
                            .broker
                            .check(local, &granted, kind, None)
                            .map_err(|denied| denied.message),
                        None => Err(format!("{permission} is not a permission")),
                    }
                }
            };
            allowed.map_err(|message| api_error(ErrorCode::Denied, message))?;
            id.to_owned()
        } else if id.contains(':') {
            return Err(api_error(
                ErrorCode::Denied,
                "a plugin runs its own commands and the application's; another plugin's are reached through its services",
            ));
        } else {
            full_id(local, id)
        };
        let services = self
            .services()
            .ok_or_else(|| api_error(ErrorCode::Failed, "the application is closing"))?;
        let window = services
            .panes
            .focused_window()
            .or_else(|| {
                services
                    .panes
                    .workspace()
                    .windows
                    .first()
                    .map(|window| window.id)
            })
            .ok_or_else(|| api_error(ErrorCode::Failed, "there is no window to run it in"))?;
        services
            .run_command(
                window,
                &command,
                &args.unwrap_or(Value::Null),
                Asker::Outside,
            )
            .await
            .map(|()| Value::Null)
            .map_err(|error| api_error(ErrorCode::Failed, error.message))
    }

    /// Reads and changes a plugin's stored values, in one step.
    async fn kv(
        &self,
        local: &str,
        change: impl FnOnce(&mut Map<String, Value>) -> Result<Value, ApiError> + Send + 'static,
    ) -> Result<Value, ApiError> {
        let file = self.store.data_dir(local).join(KV_FILE);
        tokio::task::spawn_blocking(move || {
            let mut map: Map<String, Value> = match std::fs::read_to_string(&file) {
                Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
                Err(_) => Map::new(),
            };
            let before = Value::Object(map.clone());
            let answer = change(&mut map)?;
            let after = Value::Object(map);
            if after != before {
                let text = after.to_string();
                if text.len() > KV_MAX_BYTES {
                    return Err(api_error(
                        ErrorCode::Failed,
                        format!(
                            "what a plugin stores may be {} KiB at most",
                            KV_MAX_BYTES / 1024
                        ),
                    ));
                }
                lume_config::write::write_atomic(&file, &text)
                    .map_err(|error| api_error(ErrorCode::Failed, error.to_string()))?;
            }
            Ok(answer)
        })
        .await
        .unwrap_or_else(|_| {
            Err(api_error(
                ErrorCode::Failed,
                "the stored values could not be read",
            ))
        })
    }

    /// A plugin's settings: what the user set, where the plugin takes it, over the
    /// defaults its manifest gives.
    fn settings_of(&self, local: &str) -> Value {
        let settings = self.config.plugin_settings(local);
        if settings.as_object().is_some_and(|set| !set.is_empty()) {
            return settings;
        }
        // Before the configuration has the plugin's schema: its manifest's defaults.
        let defaults: Map<String, Value> = self
            .inner()
            .loaded
            .get(local)
            .and_then(Loaded::manifest)
            .map(|manifest| {
                manifest
                    .contributes
                    .settings
                    .iter()
                    .filter_map(|setting| {
                        serde_json::to_value(&setting.default)
                            .ok()
                            .map(|value| (setting.key.clone(), value))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Value::Object(defaults)
    }

    /// Tells running plugins whose settings changed.
    pub fn config_changed(&self) {
        let running: Vec<(String, Arc<Plugin>)> = self
            .inner()
            .loaded
            .iter()
            .filter_map(|(local, loaded)| {
                loaded
                    .running
                    .as_ref()
                    .map(|running| (local.clone(), Arc::clone(&running.plugin)))
            })
            .collect();
        for (local, plugin) in running {
            let settings = self.settings_of(&local);
            let mut inner = self.inner();
            let Some(loaded) = inner.loaded.get_mut(&local) else {
                continue;
            };
            if loaded.told_settings.as_ref() == Some(&settings) {
                continue;
            }
            let first = loaded.told_settings.is_none();
            loaded.told_settings = Some(settings.clone());
            drop(inner);
            if !first {
                plugin.event("settings", settings);
            }
        }
        self.tell_workspace();
    }

    /// Tells plugins that listen for it when the workspace in use, or its root folder,
    /// changed.
    pub fn tell_workspace(&self) {
        let Ok(info) = self.workspace_info() else {
            return;
        };
        let Ok(now) = serde_json::to_value(&info) else {
            return;
        };
        let changed = {
            let mut told = self
                .told_workspace
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            // Only what plugins act on: which workspace, and its root.
            let key = |value: &Value| (value["name"].clone(), value["root"].clone());
            let changed = told.as_ref().is_some_and(|before| key(before) != key(&now));
            *told = Some(now.clone());
            changed
        };
        if !changed {
            return;
        }
        let listening: Vec<Arc<Plugin>> = self
            .inner()
            .loaded
            .values()
            .filter(|loaded| loaded.topics.contains(&Topic::Workspace))
            .filter(|loaded| allows(&loaded.grants, Kind::WorkspaceRead, None))
            .filter_map(|loaded| {
                loaded
                    .running
                    .as_ref()
                    .map(|running| Arc::clone(&running.plugin))
            })
            .collect();
        for plugin in listening {
            plugin.event("workspace", now.clone());
        }
    }

    /// Runs one of a plugin's commands: starts its code if need be, gives it the command
    /// and waits for it to be carried out.
    pub async fn run_command(self: &Arc<Self>, id: &str, args: &Value) -> Result<(), CommandError> {
        let (local, name) = {
            let inner = self.inner();
            inner
                .loaded
                .keys()
                .find_map(|local| {
                    id.strip_prefix(local.as_str())
                        .and_then(|rest| rest.strip_prefix('.'))
                        .map(|name| (local.clone(), name.to_owned()))
                })
                .ok_or_else(|| {
                    CommandError::not_found(format!("no plugin adds the command {id}"))
                })?
        };
        self.activate(&local).await.map_err(|problem| {
            CommandError::new("failed", format!("{id} could not run: {problem}"))
        })?;
        let (plugin, generation) = {
            let inner = self.inner();
            let running = inner
                .loaded
                .get(&local)
                .and_then(|loaded| loaded.running.as_ref())
                .ok_or_else(|| {
                    CommandError::new("failed", format!("{local} stopped before {id} could run"))
                })?;
            (Arc::clone(&running.plugin), running.generation)
        };
        let invocation = self.next();
        let (sender, receiver) = oneshot::channel();
        self.inner()
            .invocations
            .insert(invocation, (local.clone(), generation, sender));
        plugin.command(invocation, &name, args.clone());
        match tokio::time::timeout(COMMAND_WAIT, receiver).await {
            Ok(Ok(Ok(_))) => Ok(()),
            Ok(Ok(Err(error))) => Err(CommandError::new(
                "failed",
                format!("{id}: {}", error.message),
            )),
            Ok(Err(_)) => Err(CommandError::new(
                "failed",
                format!("{local} stopped while it ran {id}"),
            )),
            Err(_) => {
                self.inner().invocations.remove(&invocation);
                Err(CommandError::new(
                    lume_proto::app::codes::TIMEOUT,
                    format!("{id} did not finish in {} seconds", COMMAND_WAIT.as_secs()),
                ))
            }
        }
    }

    async fn follow_sessions(self: Arc<Self>) {
        let reports = Arc::clone(self.state.reports());
        let (_, mut news) = reports.follow();
        loop {
            match news.recv().await {
                Ok(news) => self.session_news(news).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::debug!(missed, "session reports were missed by plugins");
                    (_, news) = reports.follow();
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    /// Passes a session's report to the plugins that hear such reports, starting those
    /// whose activation events name it.
    async fn session_news(self: &Arc<Self>, news: News) {
        let (kind, details) = match &news.what {
            SessionNews::Event(event) => {
                let mut value = serde_json::to_value(event).unwrap_or(Value::Null);
                let kind = value
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("event")
                    .to_owned();
                if let Some(object) = value.as_object_mut() {
                    object.remove("type");
                }
                (kind, value)
            }
            SessionNews::Exit(status) => (
                "exit".to_owned(),
                serde_json::to_value(status).unwrap_or(Value::Null),
            ),
        };
        let pane = self
            .services()
            .and_then(|services| services.panes.pane_of(&news.key))
            .map(|pane| pane.to_string());
        let report = json!({
            "session": news.key.session.to_string(),
            "pane": pane,
            "kind": kind,
            "details": details,
        });
        let (deliver, start): (Vec<Arc<Plugin>>, Vec<String>) = {
            let inner = self.inner();
            let mut deliver = Vec::new();
            let mut start = Vec::new();
            for (local, loaded) in &inner.loaded {
                if !loaded.is_ready() || !allows(&loaded.grants, Kind::WorkspaceRead, None) {
                    continue;
                }
                let listens = loaded.topics.contains(&Topic::Session);
                match &loaded.running {
                    Some(running) if running.started && listens => {
                        deliver.push(Arc::clone(&running.plugin))
                    }
                    None => {
                        let named = loaded.manifest().is_some_and(|manifest| {
                            manifest
                                .events
                                .contains(&Activation::SessionEvent(kind.clone()))
                        });
                        if named {
                            start.push(local.clone());
                        }
                    }
                    _ => {}
                }
            }
            (deliver, start)
        };
        for plugin in deliver {
            plugin.event("session", report.clone());
        }
        for local in start {
            let this = Arc::clone(self);
            let report = report.clone();
            tokio::spawn(async move {
                if this.activate(&local).await.is_ok() {
                    // Its code listens now, if it means to.
                    let plugin = this
                        .inner()
                        .loaded
                        .get(&local)
                        .and_then(|loaded| loaded.running.as_ref())
                        .map(|running| Arc::clone(&running.plugin));
                    if let Some(plugin) = plugin {
                        plugin.event("session", report);
                    }
                }
            });
        }
    }

    /// Stops the code of plugins that have done nothing for a while and have nothing
    /// waiting, which their activation events would start again.
    async fn look_for_idle(self: Arc<Self>) {
        loop {
            tokio::time::sleep(IDLE_LOOK).await;
            let mut inner = self.inner();
            let busy: HashSet<String> = inner
                .invocations
                .values()
                .map(|(local, ..)| local.clone())
                .collect();
            let mut stopped = Vec::new();
            for (local, loaded) in &mut inner.loaded {
                let Some(running) = &loaded.running else {
                    continue;
                };
                let Some(manifest) = loaded.installed.manifest.as_ref().ok() else {
                    continue;
                };
                let stats = running.plugin.stats();
                let comes_back = !manifest.events.contains(&Activation::Startup)
                    && (loaded.topics.is_empty()
                        || manifest
                            .events
                            .iter()
                            .any(|event| matches!(event, Activation::SessionEvent(_))));
                // What it has open on the machine (a watch, a stream, a program) is
                // something it is doing.
                let idle = running.started
                    && stats.idle_for >= IDLE_UNLOAD
                    && stats.timers == 0
                    && stats.calls_in_flight == 0
                    && !busy.contains(local)
                    && !self.holds_open(local);
                if idle && comes_back {
                    tracing::debug!(plugin = %local, "a plugin's code was stopped for doing nothing");
                    running.plugin.stop();
                    loaded.running = None;
                    loaded.topics.clear();
                    stopped.push(local.clone());
                }
            }
            drop(inner);
            for local in stopped {
                self.release(&local);
            }
        }
    }

    /// Watches the lock file, so that what `lume plugin` changes is followed.
    fn watch_lock(self: &Arc<Self>) {
        let root = self.store.root().to_owned();
        let _ = std::fs::create_dir_all(&root);
        let lock = self.store.lock_file();
        let (changed, mut changes) = mpsc::unbounded_channel::<()>();
        let made = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if let Ok(event) = event
                && event.paths.iter().any(|path| same_file(path, &lock))
            {
                let _ = changed.send(());
            }
        });
        let mut watcher = match made {
            Ok(watcher) => watcher,
            Err(error) => {
                tracing::warn!(%error, "the plugins' lock file is not watched for changes");
                return;
            }
        };
        if let Err(error) = watcher.watch(&root, RecursiveMode::NonRecursive) {
            tracing::warn!(%error, "the plugins' folder is not watched for changes");
            return;
        }
        *self.watcher.lock().unwrap_or_else(PoisonError::into_inner) = Some(watcher);
        let this = Arc::clone(self);
        self.state.runtime().spawn(async move {
            while changes.recv().await.is_some() {
                tokio::time::sleep(LOCK_SETTLE).await;
                while changes.try_recv().is_ok() {}
                this.reload().await;
            }
        });
    }

    /// Activates a plugin for `lume ctl plugin activate`.
    pub async fn activate_for_control(self: &Arc<Self>, local: &str) -> Result<(), CommandError> {
        self.activate(local)
            .await
            .map_err(|problem| CommandError::new("failed", problem))
    }

    /// The plugins, as `lume ctl plugins` lists them.
    pub fn describe(&self) -> Value {
        let inner = self.inner();
        let visible = seen(&inner);
        let held = inner.metrics.hold.is_some();
        let reading: BTreeSet<&str> = if held {
            inner
                .metrics
                .subscriptions
                .values()
                .filter(|one| {
                    one.visible.is_empty() || one.visible.iter().any(|id| visible.contains(id))
                })
                .map(|one| one.local.as_str())
                .collect()
        } else {
            BTreeSet::new()
        };
        let plugins: Vec<Value> = inner
            .resolution
            .order
            .iter()
            .filter_map(|local| inner.loaded.get(local).map(|loaded| (local, loaded)))
            .map(|(local, loaded)| {
                let (state, reason) = match &loaded.state {
                    State::Ready => ("ready", None),
                    State::Inactive(why) => ("inactive", Some(why.clone())),
                };
                let running = loaded.running.as_ref().map(|running| {
                    let stats = running.plugin.stats();
                    json!({
                        "started": running.started,
                        "memory": stats.memory_used,
                        "timers": stats.timers,
                        "calls": stats.calls_in_flight,
                    })
                });
                json!({
                    "id": local,
                    "name": loaded.installed.name(),
                    "version": loaded.installed.entry.version,
                    "state": state,
                    "reason": reason,
                    "running": running,
                    "starts": loaded.health.starts,
                    "breaches": loaded.health.breaches.len(),
                    "problem": loaded.health.last_problem,
                    "status": loaded.status.iter().map(|((item, pane), value)| json!({
                        "item": item, "pane": pane, "value": value,
                    })).collect::<Vec<_>>(),
                    "contexts": loaded.contexts,
                    "warnings": loaded.health.warnings,
                    "panels": loaded.panels.keys().collect::<Vec<_>>(),
                    "visible": visible.iter().filter(|id| {
                        id.strip_prefix(local.as_str()).is_some_and(|rest| rest.starts_with('.'))
                    }).collect::<Vec<_>>(),
                    "metrics": {
                        "subscriptions": inner.metrics.subscriptions.values().filter(|one| one.local == *local).count(),
                        "reading": reading.contains(local.as_str()),
                    },
                    "log": loaded.log.iter().rev().take(20).rev().cloned().collect::<Vec<_>>(),
                })
            })
            .collect();
        json!(plugins)
    }

    /// What the plugins add to the windows, with what their code shows in it now: what
    /// a page fetches when it starts and whenever it is told this changed.
    pub fn snapshot(&self) -> Value {
        let inner = self.inner();
        let mut snapshot = inner.shown.contributions.to_json();
        let plugins: Vec<Value> = inner
            .resolution
            .order
            .iter()
            .filter_map(|local| inner.loaded.get(local).map(|loaded| (local, loaded)))
            .map(|(local, loaded)| {
                json!({
                    "id": local,
                    "name": loaded.installed.name(),
                    "version": loaded.installed.entry.version,
                    "ready": loaded.is_ready(),
                    "problem": loaded.health.last_problem,
                })
            })
            .collect();
        let mut values = Vec::new();
        let mut panels = Map::new();
        let mut contexts = Map::new();
        for (local, loaded) in &inner.loaded {
            for ((item, pane), value) in &loaded.status {
                values.push(json!({
                    "plugin": local,
                    "item": full_id(local, item),
                    "pane": pane,
                    "value": value,
                }));
            }
            for (panel, rows) in &loaded.panels {
                panels.insert(full_id(local, panel), rows.clone());
            }
            for (key, value) in &loaded.contexts {
                contexts.insert(format!("plugin:{local}.{key}"), json!(value));
            }
        }
        if let Some(object) = snapshot.as_object_mut() {
            object.insert("revision".to_owned(), json!(inner.shown.revision));
            object.insert("plugins".to_owned(), Value::Array(plugins));
            object.insert("values".to_owned(), Value::Array(values));
            object.insert("rows".to_owned(), Value::Object(panels));
            object.insert("contexts".to_owned(), Value::Object(contexts));
        }
        snapshot
    }

    /// The list panel of a plugin, by its full id.
    pub fn panel(&self, id: &str) -> Option<PanelDef> {
        self.inner()
            .shown
            .contributions
            .panels
            .iter()
            .find(|panel| panel.id == id)
            .cloned()
    }

    /// The plugins' things a window has on screen now. Things newly on screen start the
    /// plugins that wait for them, and the readings of the machine follow what is seen.
    pub fn visible(self: &Arc<Self>, window: &str, items: Vec<String>) {
        self.recount(window, items);
    }

    fn recount(self: &Arc<Self>, window: &str, items: Vec<String>) {
        let appeared: Vec<String> = {
            let mut inner = self.inner();
            let before = seen(&inner);
            if items.is_empty() {
                inner.visible.remove(window);
            } else {
                inner
                    .visible
                    .insert(window.to_owned(), items.into_iter().collect());
            }
            let after = seen(&inner);
            after.difference(&before).cloned().collect()
        };
        for id in appeared {
            let wanted = {
                let inner = self.inner();
                inner.loaded.iter().find_map(|(local, loaded)| {
                    let name = id.strip_prefix(local.as_str())?.strip_prefix('.')?;
                    let waits = loaded.is_ready()
                        && loaded.running.is_none()
                        && loaded.manifest().is_some_and(|manifest| {
                            manifest.events.contains(&Activation::View(name.to_owned()))
                        });
                    waits.then(|| local.clone())
                })
            };
            if let Some(local) = wanted {
                let this = Arc::clone(self);
                self.state.runtime().spawn(async move {
                    if let Err(problem) = this.activate(&local).await {
                        tracing::debug!(plugin = %local, %problem, "a plugin whose item came on screen did not start");
                    }
                });
            }
        }
        self.update_metrics();
    }

    /// A window closed: what it had on screen is not seen any more.
    pub fn window_gone(self: &Arc<Self>, window: &str) {
        self.inner().minimised.remove(window);
        self.visible(window, Vec::new());
    }

    /// A window was minimised, or brought back: while it is minimised, nothing it has
    /// on screen is seen. A page cannot always tell itself.
    pub fn window_minimised(self: &Arc<Self>, window: &str, minimised: bool) {
        let changed = {
            let mut inner = self.inner();
            if minimised {
                inner.minimised.insert(window.to_owned())
            } else {
                inner.minimised.remove(window)
            }
        };
        if changed {
            let items: Vec<String> = self
                .inner()
                .visible
                .get(window)
                .map(|items| items.iter().cloned().collect())
                .unwrap_or_default();
            // Told again what it has, the window's things count or not as it now is.
            self.recount(window, items);
        }
    }

    /// Reads the machine while a plugin that asked for readings has one of the items
    /// they are for on screen, and stops reading when none has.
    fn update_metrics(&self) {
        let mut inner = self.inner();
        let visible = seen(&inner);
        let running: BTreeMap<String, u64> = inner
            .loaded
            .iter()
            .filter_map(|(local, loaded)| {
                let running = loaded.running.as_ref()?;
                Some((local.clone(), running.generation))
            })
            .collect();
        let wanted = inner.metrics.subscriptions.values().any(|one| {
            running.get(&one.local) == Some(&one.generation)
                && (one.visible.is_empty() || one.visible.iter().any(|id| visible.contains(id)))
        });
        let metrics = &mut inner.metrics;
        if !wanted {
            metrics.listening = None;
            metrics.hold = None;
            return;
        }
        if metrics.hold.is_some() {
            return;
        }
        let sampler = metrics.sampler.get_or_insert_with(Sampler::start);
        let samples = self.samples.clone();
        metrics.listening = Some(sampler.listen(move |sample| {
            let _ = samples.send(serde_json::to_value(sample).unwrap_or(Value::Null));
        }));
        metrics.hold = Some(sampler.hold());
    }

    /// Gives each reading of the machine to the plugins whose wishes for it hold now.
    async fn follow_samples(self: Arc<Self>, mut sampled: mpsc::UnboundedReceiver<Value>) {
        while let Some(sample) = sampled.recv().await {
            let deliver: Vec<(Arc<Plugin>, Vec<u64>)> = {
                let inner = self.inner();
                let visible = seen(&inner);
                let mut by_plugin: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
                for (id, one) in &inner.metrics.subscriptions {
                    if one.visible.is_empty()
                        || one.visible.iter().any(|item| visible.contains(item))
                    {
                        by_plugin.entry(one.local.as_str()).or_default().push(*id);
                    }
                }
                by_plugin
                    .into_iter()
                    .filter_map(|(local, ids)| {
                        let running = inner.loaded.get(local)?.running.as_ref()?;
                        running.started.then(|| (Arc::clone(&running.plugin), ids))
                    })
                    .collect()
            };
            for (plugin, ids) in deliver {
                plugin.event("metrics", json!({ "to": ids, "sample": sample }));
            }
        }
    }

    /// Stops every plugin's code, as the application closes.
    pub fn shutdown(&self) {
        let mut inner = self.inner();
        let mut running = Vec::new();
        let mut released = Vec::new();
        for (local, loaded) in &mut inner.loaded {
            if let Some(run) = loaded.running.take() {
                run.plugin.stop();
                running.push(run.plugin);
                released.push(local.clone());
            }
        }
        inner.metrics = Metrics::default();
        drop(inner);
        // The programs plugins started end with the application.
        for local in released {
            self.release(&local);
        }
        // A moment for their code to hear it is stopping.
        let deadline = Instant::now() + Duration::from_millis(250);
        while Instant::now() < deadline && running.iter().any(|plugin| !plugin.stats().finished) {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// Everything the windows that are not minimised have on screen.
fn seen(inner: &Inner) -> BTreeSet<String> {
    inner
        .visible
        .iter()
        .filter(|(window, _)| !inner.minimised.contains(*window))
        .flat_map(|(_, items)| items.iter().cloned())
        .collect()
}

/// Checks the rows a plugin gives a list panel: how many, how deep, how long their
/// words, and that their actions run the plugin's own commands.
fn check_rows(
    rows: &[PanelRow],
    depth: usize,
    count: &mut usize,
    commands: &BTreeSet<&str>,
) -> Result<(), String> {
    if depth > PANEL_DEPTH_MAX {
        return Err(format!("a panel's rows go {PANEL_DEPTH_MAX} deep at most"));
    }
    let mut ids = BTreeSet::new();
    for row in rows {
        *count += 1;
        if *count > PANEL_ROWS_MAX {
            return Err(format!("a panel holds {PANEL_ROWS_MAX} rows at most"));
        }
        if !ids.insert(row.id.as_str()) {
            return Err(format!("two rows beside each other are called {}", row.id));
        }
        for text in [
            Some(&row.id),
            Some(&row.label),
            row.detail.as_ref(),
            row.badge.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if text.chars().count() > PANEL_TEXT_MAX || text.chars().any(char::is_control) {
                return Err(format!(
                    "a row's words are one line of {PANEL_TEXT_MAX} characters at most"
                ));
            }
        }
        for action in row.actions.iter().flatten() {
            if !commands.contains(action.command.as_str()) {
                return Err(format!(
                    "an action runs {}, which is not one of the plugin's commands",
                    action.command
                ));
            }
        }
        if let Some(children) = &row.children {
            check_rows(children, depth + 1, count, commands)?;
        }
    }
    Ok(())
}

/// Gives the actions of rows their commands' full ids.
fn full_commands(rows: &mut Value, local: &str) {
    let Some(rows) = rows.as_array_mut() else {
        return;
    };
    for row in rows {
        if let Some(actions) = row.get_mut("actions").and_then(Value::as_array_mut) {
            for action in actions {
                if let Some(command) = action.get("command").and_then(Value::as_str) {
                    let full = full_id(local, command);
                    action["command"] = json!(full);
                }
            }
        }
        if let Some(children) = row.get_mut("children") {
            full_commands(children, local);
        }
    }
}

/// Whether two paths name the same file, as a watcher may report one written another
/// way.
fn same_file(one: &Path, other: &Path) -> bool {
    one == other
        || one.file_name() == other.file_name()
            && one.parent().map(Path::file_name) == other.parent().map(Path::file_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(value: Value) -> Vec<PanelRow> {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_panels_rows_are_held_to_their_limits_and_run_the_plugins_own_commands() {
        let commands: BTreeSet<&str> = ["open"].into_iter().collect();
        let good = rows(json!([
            { "id": "a", "label": "A", "children": [
                { "id": "a1", "label": "A1", "actions": [{ "command": "open", "title": "Open" }] }
            ] },
            { "id": "b", "label": "B" },
        ]));
        let mut count = 0;
        assert_eq!(check_rows(&good, 1, &mut count, &commands), Ok(()));
        assert_eq!(count, 3);

        let foreign = rows(json!([
            { "id": "a", "label": "A", "actions": [{ "command": "lume.app.quit", "title": "Quit" }] }
        ]));
        assert!(
            check_rows(&foreign, 1, &mut 0, &commands)
                .unwrap_err()
                .contains("lume.app.quit")
        );
        let twice = rows(json!([{ "id": "a", "label": "A" }, { "id": "a", "label": "B" }]));
        assert!(
            check_rows(&twice, 1, &mut 0, &commands)
                .unwrap_err()
                .contains("called a")
        );
        let long = rows(json!([{ "id": "a", "label": "x".repeat(PANEL_TEXT_MAX + 1) }]));
        assert!(check_rows(&long, 1, &mut 0, &commands).is_err());
        let mut deep = json!({ "id": "leaf", "label": "leaf" });
        for level in 0..PANEL_DEPTH_MAX {
            deep = json!({ "id": format!("l{level}"), "label": "x", "children": [deep] });
        }
        assert!(
            check_rows(&rows(json!([deep])), 1, &mut 0, &commands)
                .unwrap_err()
                .contains("deep")
        );
        let many: Vec<Value> = (0..=PANEL_ROWS_MAX)
            .map(|at| json!({ "id": format!("r{at}"), "label": "r" }))
            .collect();
        assert!(
            check_rows(&rows(Value::Array(many)), 1, &mut 0, &commands)
                .unwrap_err()
                .contains("rows at most")
        );
    }

    #[test]
    fn a_rows_actions_name_the_plugins_commands_in_full() {
        let mut value = json!([
            { "id": "a", "label": "A", "actions": [{ "command": "open", "title": "Open" }],
              "children": [{ "id": "b", "label": "B", "actions": [{ "command": "stop", "title": "Stop" }] }] }
        ]);
        full_commands(&mut value, "acme.tools");
        assert_eq!(value[0]["actions"][0]["command"], "acme.tools.open");
        assert_eq!(
            value[0]["children"][0]["actions"][0]["command"],
            "acme.tools.stop"
        );
    }
}
