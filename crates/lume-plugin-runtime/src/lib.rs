//! Where a plugin's code runs.
//!
//! Each plugin has a QuickJS engine of its own, on a thread of its own, sharing nothing
//! with any other. The engine has the ECMAScript library and the `lume` object the
//! prelude builds, and nothing else: no files, no network, no processes, no modules.
//! Everything the code does beyond its own memory is a call it hands to the host, the
//! application, which decides whether the plugin may.
//!
//! The code is held to limits, so a plugin cannot stall the terminal or take the
//! machine's memory:
//!
//! - **Memory.** The engine has a cap; at nine tenths of it, or on running out, the
//!   plugin is stopped.
//! - **Time.** Each piece of work the code is given (running its top level, a timer, a
//!   command, an event, each promise job after) has a budget. The engine is interrupted
//!   when it runs over. One piece of work with the jobs it leads to may not take more
//!   than four budgets, or more than ten thousand jobs, so a chain of promises that
//!   never ends is stopped too.
//! - **Work the interrupt cannot reach**, inside one native operation, is watched from
//!   outside: a plugin still busy at five budgets is given up on, and its thread is
//!   left to finish by itself.
//!
//! Going over a limit is a breach. The engine is thrown away (whatever the code was in
//! the middle of is not to be trusted), and the host decides what follows: the host
//! starts the plugin again when it is next wanted, and turns it off after repeated
//! breaches.

mod engine;
pub mod sourcemap;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

pub use lume_plugin_api::{ApiError, ErrorCode};
use serde_json::Value;

/// The prelude: what the plugin's code finds before it runs.
pub const PRELUDE: &str = include_str!("prelude.generated.js");

/// The time a piece of work may take when nothing else is said.
pub const JOB_BUDGET: Duration = Duration::from_millis(100);

/// How many budgets one piece of work with its promise jobs may take, and how many jobs.
pub const TURN_BUDGETS: u32 = 4;
pub const TURN_JOBS_MAX: usize = 10_000;

/// How many budgets a plugin may be busy before the watchdog gives up on it.
pub const HUNG_BUDGETS: u32 = 5;

/// The stack the engine may use, and the thread's own, which must be larger.
pub const STACK_LIMIT: usize = 1024 * 1024;
const THREAD_STACK: usize = 8 * 1024 * 1024;

/// The most calls a plugin may have waiting for an answer, events waiting for it, and
/// timers set.
pub const CALLS_IN_FLIGHT_MAX: usize = 256;
pub const EVENTS_QUEUED_MAX: usize = 1024;
pub const TIMERS_MAX: usize = 1000;

/// The shortest a timer may be.
pub const TIMER_MIN: Duration = Duration::from_millis(4);

/// The most threads that may be left behind by plugins that hung, all together. Past
/// it, no plugin that hangs is started again.
pub const HUNG_THREADS_MAX: usize = 4;

/// The longest message a plugin may write to its log at once.
pub const LOG_MESSAGE_MAX: usize = 8 * 1024;

/// The limits a plugin's code is held to.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub memory: usize,
    pub stack: usize,
    pub job: Duration,
}

impl Limits {
    /// The limits of a plugin whose manifest asks for `mib` of memory.
    pub fn with_memory_mib(mib: u32) -> Self {
        Self {
            memory: usize::try_from(mib).unwrap_or(64) * 1024 * 1024,
            stack: STACK_LIMIT,
            job: JOB_BUDGET,
        }
    }
}

/// What a plugin is and the code to run.
#[derive(Clone, Debug)]
pub struct Spec {
    /// The id it is installed under.
    pub id: String,
    pub manifest_id: String,
    pub name: String,
    pub version: String,
    /// The bundle's text.
    pub code: String,
    /// The bundle's path in the package, as stacks name it.
    pub file: String,
    /// The bundle's source map, if the package has one.
    pub source_map: Option<String>,
    pub limits: Limits,
}

/// How bad a line of a plugin's log is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "debug" => Self::Debug,
            "warn" => Self::Warn,
            "error" => Self::Error,
            _ => Self::Info,
        }
    }
}

/// Which limit was gone over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Breach {
    /// A piece of work ran over its budget.
    Time,
    /// A piece of work and its promise jobs ran over theirs.
    Turn,
    /// The memory cap was reached, or nearly.
    Memory,
}

impl Breach {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Time => "time",
            Self::Turn => "turn",
            Self::Memory => "memory",
        }
    }
}

/// What a running plugin tells its host.
#[derive(Clone, Debug, PartialEq)]
pub enum Outbound {
    /// A call, to be answered with [`Plugin::answer`] by its id.
    Call {
        id: u64,
        name: String,
        args: Value,
    },
    /// The calls of one turn of the code have all been handed on: what they show
    /// belongs together, and the host shows it together. Sent after a turn that made
    /// calls, and not after one that made none.
    TurnDone,
    /// A command the host gave the plugin was carried out, or failed.
    CommandDone {
        invocation: u64,
        result: Result<Value, ApiError>,
    },
    Log {
        level: Level,
        message: String,
    },
    /// The plugin's top level ran: its handlers are registered.
    Started,
    /// Its top level failed; it stops.
    Failed {
        message: String,
    },
    /// It went over a limit; it stops.
    Breached {
        breach: Breach,
        detail: String,
    },
    /// It was busy too long for the interrupt to stop it; it is given up on.
    Hung,
    /// Its engine is gone. Always the last message.
    Stopped,
}

/// Where a running plugin's messages go. Called from the plugin's thread, and from the
/// watchdog's, so it must not block.
pub trait Host: Send + Sync + 'static {
    fn send(&self, message: Outbound);
}

/// What the host gives a running plugin.
#[derive(Debug)]
enum Inbound {
    Answer {
        id: u64,
        result: Result<Value, ApiError>,
    },
    Command {
        invocation: u64,
        name: String,
        args: Value,
    },
    Service {
        invocation: u64,
        name: String,
        args: Value,
    },
    Event {
        topic: String,
        payload: Value,
    },
    /// The newest thing on a topic waits in [`Shared::latest`].
    Latest {
        topic: String,
    },
    Stop,
}

/// What both the plugin's thread and the host see of it.
struct Shared {
    host: Box<dyn Host>,
    started: Instant,
    /// When the current piece of work began, in nanoseconds from `started`; 0 when idle.
    busy_since: AtomicU64,
    job_ns: u64,
    hung: AtomicBool,
    finished: AtomicBool,
    events_queued: AtomicUsize,
    /// How many events it missed because too many waited already.
    events_missed: AtomicU64,
    calls_in_flight: AtomicUsize,
    timers: AtomicUsize,
    memory_used: AtomicU64,
    /// When the plugin last did anything, in nanoseconds from `started`.
    last_activity: AtomicU64,
    /// The newest thing on each topic only the newest counts on, until the code takes
    /// it.
    latest: Mutex<HashMap<String, Value>>,
}

impl Shared {
    fn now_ns(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

/// What a running plugin is doing, as the host can see.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    pub calls_in_flight: usize,
    pub timers: usize,
    pub memory_used: u64,
    /// How many events it missed because too many waited for it already.
    pub events_missed: u64,
    /// How long since it last did anything.
    pub idle_for: Duration,
    pub hung: bool,
    pub finished: bool,
}

/// A running plugin. Dropping it stops it.
pub struct Plugin {
    inbound: Sender<Inbound>,
    shared: Arc<Shared>,
}

/// Why a plugin could not be started.
#[derive(Debug)]
pub enum StartError {
    /// Too many threads are already left behind by plugins that hung.
    TooManyHung,
    /// The thread could not be made.
    Thread(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyHung => write!(
                f,
                "{HUNG_THREADS_MAX} plugins hung already and their threads are still busy"
            ),
            Self::Thread(error) => write!(f, "the plugin's thread could not be started: {error}"),
        }
    }
}

impl std::error::Error for StartError {}

/// The plugins that hung and whose threads have not finished.
static HUNG_THREADS: AtomicUsize = AtomicUsize::new(0);

impl Plugin {
    /// Starts a plugin's code on a thread of its own. Its messages go to `host`.
    pub fn start(spec: Spec, host: Box<dyn Host>) -> Result<Self, StartError> {
        if HUNG_THREADS.load(Ordering::Relaxed) >= HUNG_THREADS_MAX {
            return Err(StartError::TooManyHung);
        }
        let shared = Arc::new(Shared {
            host,
            started: Instant::now(),
            busy_since: AtomicU64::new(0),
            job_ns: u64::try_from(spec.limits.job.as_nanos()).unwrap_or(u64::MAX),
            hung: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            events_queued: AtomicUsize::new(0),
            events_missed: AtomicU64::new(0),
            calls_in_flight: AtomicUsize::new(0),
            timers: AtomicUsize::new(0),
            memory_used: AtomicU64::new(0),
            last_activity: AtomicU64::new(0),
            latest: Mutex::new(HashMap::new()),
        });
        let (inbound, receiver) = channel();
        let thread_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name(format!("plugin {}", spec.id))
            .stack_size(THREAD_STACK)
            .spawn(move || {
                engine::run(spec, receiver, &thread_shared);
                thread_shared.finished.store(true, Ordering::Release);
                if thread_shared.hung.load(Ordering::Acquire) {
                    HUNG_THREADS.fetch_sub(1, Ordering::Relaxed);
                }
                thread_shared.host.send(Outbound::Stopped);
            })
            .map_err(|error| StartError::Thread(error.to_string()))?;
        watch(&shared);
        Ok(Self { inbound, shared })
    }

    /// Answers a call the plugin made.
    pub fn answer(&self, id: u64, result: Result<Value, ApiError>) {
        let _ = self.inbound.send(Inbound::Answer { id, result });
    }

    /// Has the plugin carry out one of its commands. The outcome comes back as
    /// [`Outbound::CommandDone`] with the same invocation.
    pub fn command(&self, invocation: u64, name: &str, args: Value) {
        let _ = self.inbound.send(Inbound::Command {
            invocation,
            name: name.to_owned(),
            args,
        });
    }

    /// Calls one of the services the plugin's code offers, by its name. The code
    /// answers with [`Outbound::CommandDone`] with the same invocation, as for a
    /// command.
    pub fn service(&self, invocation: u64, name: &str, args: Value) {
        let _ = self.inbound.send(Inbound::Service {
            invocation,
            name: name.to_owned(),
            args,
        });
    }

    /// Tells the plugin of something on a topic it listens to. Says whether it was
    /// taken: a plugin that has too many waiting misses this one.
    pub fn event(&self, topic: &str, payload: Value) -> bool {
        let queued = self.shared.events_queued.fetch_add(1, Ordering::AcqRel);
        if queued >= EVENTS_QUEUED_MAX {
            self.shared.events_queued.fetch_sub(1, Ordering::AcqRel);
            self.shared.events_missed.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        self.inbound
            .send(Inbound::Event {
                topic: topic.to_owned(),
                payload,
            })
            .is_ok()
    }

    /// Tells the plugin of the newest thing on a topic where only the newest counts,
    /// as a reading of the machine. One the code has not taken yet is replaced by this
    /// one, so a plugin that is busy hears the latest when it is free and never a row
    /// of old ones. Says whether an older one was replaced.
    pub fn latest(&self, topic: &str, payload: Value) -> bool {
        let mut latest = self
            .shared
            .latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if latest.insert(topic.to_owned(), payload).is_some() {
            return true;
        }
        drop(latest);
        let _ = self.inbound.send(Inbound::Latest {
            topic: topic.to_owned(),
        });
        false
    }

    /// Asks the plugin to stop: its code is told, then its engine goes.
    pub fn stop(&self) {
        let _ = self.inbound.send(Inbound::Stop);
    }

    pub fn stats(&self) -> Stats {
        let now = self.shared.now_ns();
        let last = self.shared.last_activity.load(Ordering::Relaxed);
        Stats {
            calls_in_flight: self.shared.calls_in_flight.load(Ordering::Relaxed),
            timers: self.shared.timers.load(Ordering::Relaxed),
            memory_used: self.shared.memory_used.load(Ordering::Relaxed),
            events_missed: self.shared.events_missed.load(Ordering::Relaxed),
            idle_for: Duration::from_nanos(now.saturating_sub(last)),
            hung: self.shared.hung.load(Ordering::Relaxed),
            finished: self.shared.finished.load(Ordering::Relaxed),
        }
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        self.stop();
    }
}

/// How often the watchdog looks while some plugin's code runs.
const WATCH_EVERY: Duration = Duration::from_millis(20);

/// How many plugins' code is in a turn now. While none is, the watchdog sleeps until one
/// begins one, rather than looking every [`WATCH_EVERY`] for nothing.
static IN_TURN: AtomicUsize = AtomicUsize::new(0);

/// What wakes the watchdog when a turn begins.
fn turns() -> &'static (Mutex<()>, Condvar) {
    static TURNS: OnceLock<(Mutex<()>, Condvar)> = OnceLock::new();
    TURNS.get_or_init(|| (Mutex::new(()), Condvar::new()))
}

/// A plugin's code began a turn.
pub(crate) fn turn_began() {
    if IN_TURN.fetch_add(1, Ordering::AcqRel) == 0 {
        let (lock, woken) = turns();
        let _held = lock.lock().unwrap_or_else(PoisonError::into_inner);
        woken.notify_one();
    }
}

/// A plugin's code ended a turn it began.
pub(crate) fn turn_ended() {
    let _ = IN_TURN.try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
        count.checked_sub(1)
    });
}

fn watched() -> &'static Mutex<Vec<Weak<Shared>>> {
    static WATCHED: OnceLock<Mutex<Vec<Weak<Shared>>>> = OnceLock::new();
    WATCHED.get_or_init(|| {
        std::thread::Builder::new()
            .name("plugin watchdog".to_owned())
            .spawn(|| {
                loop {
                    {
                        let (lock, woken) = turns();
                        let held = lock.lock().unwrap_or_else(PoisonError::into_inner);
                        let _held = woken
                            .wait_while(held, |()| IN_TURN.load(Ordering::Acquire) == 0)
                            .unwrap_or_else(PoisonError::into_inner);
                    }
                    std::thread::sleep(WATCH_EVERY);
                    let list = watched()
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .clone();
                    for shared in list.iter().filter_map(Weak::upgrade) {
                        let since = shared.busy_since.load(Ordering::Acquire);
                        if since == 0 || shared.hung.load(Ordering::Acquire) {
                            continue;
                        }
                        let busy = shared.now_ns().saturating_sub(since);
                        if busy > shared.job_ns.saturating_mul(u64::from(HUNG_BUDGETS))
                            && !shared.hung.swap(true, Ordering::AcqRel)
                        {
                            HUNG_THREADS.fetch_add(1, Ordering::Relaxed);
                            shared.host.send(Outbound::Hung);
                        }
                    }
                    watched()
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .retain(|weak| weak.strong_count() > 0);
                }
            })
            .ok();
        Mutex::new(Vec::new())
    })
}

fn watch(shared: &Arc<Shared>) {
    watched()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Arc::downgrade(shared));
}

/// How many plugins hung whose threads are still busy.
pub fn hung_threads() -> usize {
    HUNG_THREADS.load(Ordering::Relaxed)
}
