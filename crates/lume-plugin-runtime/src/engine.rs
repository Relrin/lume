//! The plugin's thread: its engine, and the loop that gives the code its work.
//!
//! The code is given one piece of work at a time: an answer to a call, a command, an
//! event, a timer. Each runs with the promise jobs it leads to, which together are a
//! turn. Calls the code makes during a turn are held and handed on when the turn is
//! over, so nothing of the host's runs in the middle of the code's work.

use std::cell::{Cell, RefCell};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use rquickjs::allocator::{Allocator, RustAllocator};
use rquickjs::context::EvalOptions;
use rquickjs::{
    CatchResultExt, CaughtError, Coerced, Context, Ctx, FromJs, Function, Object, Persistent,
    Runtime,
};
use serde_json::Value;

use crate::sourcemap::SourceMap;
use crate::{
    ApiError, Breach, CALLS_IN_FLIGHT_MAX, Inbound, LOG_MESSAGE_MAX, Level, Limits, Outbound,
    PRELUDE, Shared, Spec, TIMER_MIN, TIMERS_MAX, TURN_BUDGETS, TURN_JOBS_MAX,
};

/// A deadline that is never reached.
const NEVER: u64 = u64::MAX;

/// The share of the memory cap at which the plugin is stopped, in tenths.
const MEMORY_STOP_TENTHS: usize = 9;

/// The allocator the engine uses: Rust's, held to the plugin's cap. It counts what it
/// gives out, refuses what would go over, and remembers that it refused, so running out
/// of memory stops the plugin even when its code catches the error.
struct Capped {
    inner: RustAllocator,
    used: usize,
    limit: usize,
    refused: Arc<AtomicBool>,
    /// `used` as the engine's thread reads it after a turn: counting here is far cheaper
    /// than asking the engine, which walks its whole heap to answer.
    total: Arc<AtomicUsize>,
}

impl Capped {
    fn fits(&mut self, more: usize) -> bool {
        let fits = self
            .used
            .checked_add(more)
            .is_some_and(|total| total <= self.limit);
        if !fits {
            self.refused.store(true, Ordering::Relaxed);
        }
        fits
    }
}

// SAFETY: every call is passed to Rust's allocator, which keeps the trait's promises; the
// counting only refuses calls, and a refusal is a null pointer, which the trait allows.
unsafe impl Allocator for Capped {
    fn alloc(&mut self, size: usize) -> *mut u8 {
        if !self.fits(size) {
            return std::ptr::null_mut();
        }
        let pointer = self.inner.alloc(size);
        if !pointer.is_null() {
            // SAFETY: just allocated by the same allocator.
            self.used += unsafe { RustAllocator::usable_size(pointer) };
            self.total.store(self.used, Ordering::Relaxed);
        }
        pointer
    }

    fn calloc(&mut self, count: usize, size: usize) -> *mut u8 {
        let Some(total) = count.checked_mul(size) else {
            self.refused.store(true, Ordering::Relaxed);
            return std::ptr::null_mut();
        };
        if !self.fits(total) {
            return std::ptr::null_mut();
        }
        let pointer = self.inner.calloc(count, size);
        if !pointer.is_null() {
            // SAFETY: just allocated by the same allocator.
            self.used += unsafe { RustAllocator::usable_size(pointer) };
            self.total.store(self.used, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&mut self, pointer: *mut u8) {
        if pointer.is_null() {
            return;
        }
        // SAFETY: the engine frees only what this allocator gave it.
        unsafe {
            self.used = self
                .used
                .saturating_sub(RustAllocator::usable_size(pointer));
            self.total.store(self.used, Ordering::Relaxed);
            self.inner.dealloc(pointer);
        }
    }

    unsafe fn realloc(&mut self, pointer: *mut u8, new_size: usize) -> *mut u8 {
        if pointer.is_null() {
            return self.alloc(new_size);
        }
        // SAFETY: the engine resizes only what this allocator gave it.
        let old = unsafe { RustAllocator::usable_size(pointer) };
        if new_size > old && !self.fits(new_size - old) {
            return std::ptr::null_mut();
        }
        // SAFETY: as above.
        let moved = unsafe { self.inner.realloc(pointer, new_size) };
        if !moved.is_null() {
            // SAFETY: just allocated by the same allocator.
            let new = unsafe { RustAllocator::usable_size(moved) };
            self.used = self.used.saturating_sub(old) + new;
            self.total.store(self.used, Ordering::Relaxed);
        }
        moved
    }

    unsafe fn usable_size(pointer: *mut u8) -> usize {
        // SAFETY: the caller's promise is passed on.
        unsafe { RustAllocator::usable_size(pointer) }
    }
}

/// A call the code made, held until its turn is over.
struct Made {
    id: u64,
    name: String,
    json: String,
}

/// Why the engine is to stop.
enum Stop {
    Breach(Breach, String),
    Failed(String),
    Hung,
}

/// The timers the code has set.
#[derive(Default)]
struct Timers {
    queue: BinaryHeap<Reverse<(Instant, u64, u64)>>,
    /// Each timer's interval, whether it repeats, and the number of its current place
    /// in the queue (older places are passed over).
    live: HashMap<u64, (Duration, bool, u64)>,
    next: u64,
}

impl Timers {
    fn set(&mut self, id: u64, delay: Duration, repeat: bool) -> bool {
        if self.live.len() >= TIMERS_MAX && !self.live.contains_key(&id) {
            return false;
        }
        let delay = delay.max(TIMER_MIN);
        self.next += 1;
        self.live.insert(id, (delay, repeat, self.next));
        self.queue
            .push(Reverse((Instant::now() + delay, self.next, id)));
        true
    }

    fn clear(&mut self, id: u64) {
        self.live.remove(&id);
    }

    fn is_current(&self, place: u64, id: u64) -> bool {
        self.live
            .get(&id)
            .is_some_and(|(_, _, current)| *current == place)
    }

    fn next_due(&mut self) -> Option<Instant> {
        while let Some(Reverse((at, place, id))) = self.queue.peek().copied() {
            if self.is_current(place, id) {
                return Some(at);
            }
            self.queue.pop();
        }
        None
    }

    /// The timers due now, in the order they fall due; those that repeat are set again.
    fn due(&mut self, now: Instant) -> Vec<u64> {
        let mut due = Vec::new();
        while let Some(Reverse((at, place, id))) = self.queue.peek().copied() {
            if at > now {
                break;
            }
            self.queue.pop();
            if !self.is_current(place, id) {
                continue;
            }
            due.push(id);
            match self.live.get(&id).copied() {
                Some((interval, true, _)) => {
                    self.next += 1;
                    self.live.insert(id, (interval, true, self.next));
                    self.queue.push(Reverse((now + interval, self.next, id)));
                }
                _ => {
                    self.live.remove(&id);
                }
            }
        }
        due
    }
}

struct Engine {
    runtime: Runtime,
    context: Context,
    dispatch: Option<Persistent<Function<'static>>>,
    made: Rc<RefCell<Vec<Made>>>,
    rejections: Rc<RefCell<Vec<String>>>,
    deadline: Arc<AtomicU64>,
    fired: Arc<AtomicBool>,
    /// Whether the allocator refused anything since the last look.
    refused: Arc<AtomicBool>,
    /// What the engine holds of the allocator, as the allocator counts it.
    allocated: Arc<AtomicUsize>,
    shared: Arc<Shared>,
    limits: Limits,
    file: String,
    /// The package's map of its code to its source, as text, and what it says once a
    /// stack has needed it: most code never throws, and reading a map takes longer
    /// than starting the code does.
    map_text: Option<String>,
    map: std::cell::OnceCell<Option<SourceMap>>,
    timers: RefCell<Timers>,
    waiting: RefCell<HashSet<u64>>,
    /// When the current turn began.
    turn_started: Cell<Option<Instant>>,
}

pub(crate) fn run(spec: Spec, inbound: Receiver<Inbound>, shared: &Arc<Shared>) {
    let shared = Arc::clone(shared);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let engine = match Engine::new(&spec, &shared) {
            Ok(engine) => engine,
            Err(message) => {
                shared.host.send(Outbound::Failed { message });
                return;
            }
        };
        if let Some(stop) = engine
            .start(&spec.code)
            .err()
            .or_else(|| engine.serve(&inbound).err())
        {
            engine.report(stop);
        }
        engine.close();
    }));
    if outcome.is_err() {
        shared.host.send(Outbound::Failed {
            message: "the plugin's engine stopped with an internal error".to_owned(),
        });
    }
}

/// The text of what was thrown, and whether it was the engine running out of memory.
fn describe(error: &CaughtError<'_>) -> (String, bool) {
    let text = match error {
        CaughtError::Exception(exception) => {
            let message = exception.message().unwrap_or_default();
            match exception.stack() {
                Some(stack) if !stack.trim().is_empty() => {
                    format!("{message}\n{}", stack.trim_end())
                }
                _ => message,
            }
        }
        CaughtError::Value(value) => Coerced::<String>::from_js(value.ctx(), value.clone())
            .map(|text| text.0)
            .unwrap_or_else(|_| "a value that is not an error".to_owned()),
        CaughtError::Error(error) => error.to_string(),
    };
    let memory = text.contains("out of memory")
        || matches!(error, CaughtError::Error(rquickjs::Error::Allocation));
    (text, memory)
}

impl Engine {
    fn new(spec: &Spec, shared: &Arc<Shared>) -> Result<Self, String> {
        let failed =
            |error: rquickjs::Error| format!("the plugin's engine could not be made: {error}");
        let refused = Arc::new(AtomicBool::new(false));
        let allocated = Arc::new(AtomicUsize::new(0));
        let runtime = Runtime::new_with_alloc(Capped {
            inner: RustAllocator,
            used: 0,
            limit: spec.limits.memory,
            refused: Arc::clone(&refused),
            total: Arc::clone(&allocated),
        })
        .map_err(failed)?;
        runtime.set_max_stack_size(spec.limits.stack);
        let deadline = Arc::new(AtomicU64::new(NEVER));
        let fired = Arc::new(AtomicBool::new(false));
        {
            let (deadline, fired, shared) = (
                Arc::clone(&deadline),
                Arc::clone(&fired),
                Arc::clone(shared),
            );
            runtime.set_interrupt_handler(Some(Box::new(move || {
                let over = shared.now_ns() > deadline.load(Ordering::Relaxed);
                if over {
                    fired.store(true, Ordering::Relaxed);
                }
                over
            })));
        }
        let rejections = Rc::new(RefCell::new(Vec::new()));
        {
            let rejections = Rc::clone(&rejections);
            runtime.set_host_promise_rejection_tracker(Some(Box::new(
                move |ctx, _promise, reason, handled| {
                    if handled {
                        return;
                    }
                    let text = Coerced::<String>::from_js(&ctx, reason)
                        .map(|text| text.0)
                        .unwrap_or_default();
                    rejections.borrow_mut().push(text);
                },
            )));
        }
        let context = Context::full(&runtime).map_err(failed)?;
        let made = Rc::new(RefCell::new(Vec::new()));
        let dispatch = context.with(|ctx| -> rquickjs::Result<Persistent<Function<'static>>> {
            let mut options = EvalOptions::default();
            options.global = true;
            options.strict = true;
            options.filename = Some("lume:prelude".to_owned());
            let make: Function = ctx.eval_with_options(PRELUDE, options)?;
            let call = {
                let made = Rc::clone(&made);
                let shared = Arc::clone(shared);
                Function::new(
                    ctx.clone(),
                    move |id: f64, name: String, json: String| -> bool {
                        #[allow(
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss,
                            reason = "the prelude's call numbers are small whole numbers"
                        )]
                        let id = id as u64;
                        if id != 0 {
                            if shared.calls_in_flight.load(Ordering::Relaxed) >= CALLS_IN_FLIGHT_MAX
                            {
                                return false;
                            }
                            shared.calls_in_flight.fetch_add(1, Ordering::Relaxed);
                        }
                        made.borrow_mut().push(Made { id, name, json });
                        true
                    },
                )?
            };
            let info = Object::new(ctx.clone())?;
            info.set("id", spec.id.as_str())?;
            info.set("manifestId", spec.manifest_id.as_str())?;
            info.set("name", spec.name.as_str())?;
            info.set("version", spec.version.as_str())?;
            let dispatch: Function = make.call((call, info))?;
            Ok(Persistent::save(&ctx, dispatch))
        });
        let dispatch = dispatch.map_err(failed)?;
        Ok(Self {
            runtime,
            context,
            dispatch: Some(dispatch),
            made,
            rejections,
            deadline,
            fired,
            refused,
            allocated,
            shared: Arc::clone(shared),
            limits: spec.limits,
            file: spec.file.clone(),
            map_text: spec.source_map.clone(),
            map: std::cell::OnceCell::new(),
            timers: RefCell::new(Timers::default()),
            waiting: RefCell::new(HashSet::new()),
            turn_started: Cell::new(None),
        })
    }

    /// Gives the code a budget for its next piece of work.
    fn begin_job(&self) {
        let now = self.shared.now_ns();
        let budget = u64::try_from(self.limits.job.as_nanos()).unwrap_or(NEVER);
        self.fired.store(false, Ordering::Relaxed);
        self.deadline
            .store(now.saturating_add(budget), Ordering::Relaxed);
        if self.shared.busy_since.load(Ordering::Relaxed) == 0 {
            self.shared.busy_since.store(now.max(1), Ordering::Release);
            crate::turn_began();
        }
    }

    fn end_turn(&self) {
        self.deadline.store(NEVER, Ordering::Relaxed);
        if self.shared.busy_since.swap(0, Ordering::AcqRel) != 0 {
            crate::turn_ended();
        }
        self.shared
            .last_activity
            .store(self.shared.now_ns(), Ordering::Relaxed);
        self.turn_started.set(None);
    }

    /// A stack written with the source's places, where the package has a map.
    fn place(&self, text: String) -> String {
        let map = self
            .map
            .get_or_init(|| self.map_text.as_deref().and_then(SourceMap::parse));
        match map {
            Some(map) => map.rewrite(&text, &self.file),
            None => text,
        }
    }

    /// Why a failure stops the engine, if it does: going over the time or the memory.
    fn breach_of(&self, memory: bool) -> Option<Stop> {
        if self.fired.load(Ordering::Relaxed) {
            return Some(Stop::Breach(
                Breach::Time,
                format!(
                    "a piece of its work took more than {} ms",
                    self.limits.job.as_millis()
                ),
            ));
        }
        memory.then(|| {
            Stop::Breach(
                Breach::Memory,
                format!("it ran out of its {} MiB", self.limits.memory / 1024 / 1024),
            )
        })
    }

    /// Runs one piece of work and the promise jobs it leads to.
    fn turn(
        &self,
        work: impl for<'js> FnOnce(&Ctx<'js>, &Function<'js>) -> rquickjs::Result<()>,
    ) -> Result<(), Stop> {
        let started = Instant::now();
        self.turn_started.set(Some(started));
        self.begin_job();
        let first = self.context.with(|ctx| {
            let dispatch = match self.dispatch.clone().map(|saved| saved.restore(&ctx)) {
                Some(Ok(dispatch)) => dispatch,
                _ => return Err((String::from("the plugin's engine lost its prelude"), false)),
            };
            work(&ctx, &dispatch)
                .catch(&ctx)
                .map_err(|error| describe(&error))
        });
        let outcome = self.after(first, started);
        self.end_turn();
        self.hand_on();
        outcome
    }

    /// What follows a piece of work: its error, if it failed, then its jobs, then a look
    /// at the memory.
    fn after(&self, first: Result<(), (String, bool)>, started: Instant) -> Result<(), Stop> {
        if let Err((text, memory)) = first {
            if let Some(stop) = self.breach_of(memory) {
                return Err(stop);
            }
            self.log(Level::Error, self.place(text));
        }
        let mut jobs = 0usize;
        while self.runtime.is_job_pending() {
            if self.shared.hung.load(Ordering::Acquire) {
                return Err(Stop::Hung);
            }
            if jobs >= TURN_JOBS_MAX || started.elapsed() > self.limits.job * TURN_BUDGETS {
                return Err(Stop::Breach(
                    Breach::Turn,
                    format!(
                        "its work and the promises it led to ran {jobs} jobs and {} ms without stopping",
                        started.elapsed().as_millis()
                    ),
                ));
            }
            self.begin_job();
            if let Err(exception) = self.runtime.execute_pending_job() {
                let (text, memory) = exception.0.with(|ctx| {
                    let thrown = ctx.catch();
                    let error = if let Some(exception) = thrown.as_exception() {
                        CaughtError::Exception(exception.clone())
                    } else {
                        CaughtError::Value(thrown)
                    };
                    describe(&error)
                });
                if let Some(stop) = self.breach_of(memory) {
                    return Err(stop);
                }
                self.log(Level::Error, self.place(text));
            }
            jobs += 1;
        }
        if self.refused.swap(false, Ordering::Relaxed) {
            return Err(Stop::Breach(
                Breach::Memory,
                format!("it ran out of its {} MiB", self.limits.memory / 1024 / 1024),
            ));
        }
        let used = self.allocated.load(Ordering::Relaxed);
        self.shared
            .memory_used
            .store(used as u64, Ordering::Relaxed);
        if used >= self.limits.memory / 10 * MEMORY_STOP_TENTHS {
            return Err(Stop::Breach(
                Breach::Memory,
                format!(
                    "it used {} of its {} MiB",
                    used / 1024 / 1024,
                    self.limits.memory / 1024 / 1024
                ),
            ));
        }
        if self.shared.hung.load(Ordering::Acquire) {
            return Err(Stop::Hung);
        }
        Ok(())
    }

    fn log(&self, level: Level, mut message: String) {
        if message.len() > LOG_MESSAGE_MAX {
            let mut end = LOG_MESSAGE_MAX;
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            message.truncate(end);
            message.push_str(" …");
        }
        self.shared.host.send(Outbound::Log { level, message });
    }

    /// Hands on what the code asked for during the turn.
    fn hand_on(&self) {
        for text in self.rejections.borrow_mut().drain(..) {
            self.log(
                Level::Warn,
                format!(
                    "a promise was rejected and nothing handled it: {}",
                    self.place(text)
                ),
            );
        }
        let made: Vec<Made> = self.made.borrow_mut().drain(..).collect();
        let mut called = false;
        for Made { id, name, json } in made {
            let args: Value = match serde_json::from_str(&json) {
                Ok(args) => args,
                Err(_) => {
                    if id != 0 {
                        self.answer_now(id, Err(ApiError::invalid("what was sent is not JSON")));
                    }
                    continue;
                }
            };
            match name.as_str() {
                "timers.set" => {
                    let timer = args["id"].as_u64().unwrap_or(0);
                    let delay = Duration::from_millis(args["ms"].as_f64().map_or(0, |ms| {
                        #[allow(
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss,
                            reason = "a time in milliseconds"
                        )]
                        let ms = ms.clamp(0.0, 1e12) as u64;
                        ms
                    }));
                    let repeat = args["repeat"].as_bool().unwrap_or(false);
                    if !self.timers.borrow_mut().set(timer, delay, repeat) {
                        self.log(
                            Level::Warn,
                            format!("a timer was not set: a plugin may have {TIMERS_MAX} at most"),
                        );
                    }
                }
                "timers.clear" => self
                    .timers
                    .borrow_mut()
                    .clear(args["id"].as_u64().unwrap_or(0)),
                "log.write" => {
                    let level = Level::parse(args["level"].as_str().unwrap_or("info"));
                    let message = args["message"].as_str().unwrap_or_default().to_owned();
                    self.log(level, self.place(message));
                }
                "commands.done" => {
                    let invocation = args["invocation"].as_u64().unwrap_or(0);
                    let result = match args.get("error") {
                        Some(error) => Err(serde_json::from_value(error.clone())
                            .unwrap_or_else(|_| ApiError::failed(error.to_string()))),
                        None => Ok(args.get("value").cloned().unwrap_or(Value::Null)),
                    };
                    self.shared
                        .host
                        .send(Outbound::CommandDone { invocation, result });
                }
                // Only what changes what the plugin shows is sent without an answer.
                told if id == 0 && !lume_plugin_api::TOLD.contains(&told) => {}
                _ => {
                    if id != 0 {
                        self.waiting.borrow_mut().insert(id);
                    }
                    self.shared.host.send(Outbound::Call { id, name, args });
                    called = true;
                }
            }
        }
        if called {
            self.shared.host.send(Outbound::TurnDone);
        }
        self.shared
            .timers
            .store(self.timers.borrow().live.len(), Ordering::Relaxed);
    }

    /// Answers a call that is refused before it reaches the host.
    fn answer_now(&self, id: u64, result: Result<Value, ApiError>) {
        self.shared.calls_in_flight.fetch_sub(1, Ordering::Relaxed);
        let (kind, json) = match result {
            Ok(value) => ("result", value.to_string()),
            Err(error) => ("error", serde_json::to_string(&error).unwrap_or_default()),
        };
        let _ = self.turn(|_, dispatch| dispatch.call::<_, ()>((kind, id as f64, json)));
    }

    /// Runs the plugin's top level.
    fn start(&self, code: &str) -> Result<(), Stop> {
        let file = self.file.clone();
        let mut failure = None;
        let outcome = self.turn(|ctx, _| {
            let mut options = EvalOptions::default();
            options.global = true;
            options.filename = Some(file);
            let ran = ctx.eval_with_options::<(), _>(code, options);
            if let Err(error) = &ran {
                failure = Some(error.to_string());
            }
            ran
        });
        outcome?;
        if failure.is_some() {
            // The error itself was logged with its stack.
            return Err(Stop::Failed(
                "its code failed when it started; see its log".to_owned(),
            ));
        }
        self.shared.host.send(Outbound::Started);
        Ok(())
    }

    /// Gives the code its work until it is told to stop or goes over a limit.
    fn serve(&self, inbound: &Receiver<Inbound>) -> Result<(), Stop> {
        loop {
            // Timers that are due go first, so that a stream of messages does not keep
            // them waiting.
            let due = self.timers.borrow_mut().due(Instant::now());
            for timer in due {
                self.turn(|_, dispatch| {
                    dispatch.call::<_, ()>(("timer", timer as f64, String::new()))
                })?;
            }
            let next = self.timers.borrow_mut().next_due();
            let message = match next {
                Some(at) => inbound.recv_timeout(at.saturating_duration_since(Instant::now())),
                None => inbound.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match message {
                Ok(Inbound::Answer { id, result }) => {
                    if !self.waiting.borrow_mut().remove(&id) {
                        continue;
                    }
                    self.shared.calls_in_flight.fetch_sub(1, Ordering::Relaxed);
                    let (kind, json) = match result {
                        Ok(value) => ("result", value.to_string()),
                        Err(error) => ("error", serde_json::to_string(&error).unwrap_or_default()),
                    };
                    self.turn(|_, dispatch| dispatch.call::<_, ()>((kind, id as f64, json)))?;
                }
                Ok(Inbound::Command {
                    invocation,
                    name,
                    args,
                }) => {
                    let json = named(&name, &args);
                    self.turn(|_, dispatch| {
                        dispatch.call::<_, ()>(("command", invocation as f64, json))
                    })?;
                }
                Ok(Inbound::Service {
                    invocation,
                    name,
                    args,
                }) => {
                    let json = named(&name, &args);
                    self.turn(|_, dispatch| {
                        dispatch.call::<_, ()>(("service", invocation as f64, json))
                    })?;
                }
                Ok(Inbound::View {
                    invocation,
                    name,
                    args,
                    from,
                }) => {
                    #[derive(serde::Serialize)]
                    struct Called<'a> {
                        name: &'a str,
                        args: &'a Value,
                        from: &'a Value,
                    }
                    let json = serde_json::to_string(&Called {
                        name: &name,
                        args: &args,
                        from: &from,
                    })
                    .unwrap_or_default();
                    self.turn(|_, dispatch| {
                        dispatch.call::<_, ()>(("view", invocation as f64, json))
                    })?;
                }
                Ok(Inbound::Event { topic, payload }) => {
                    self.shared.events_queued.fetch_sub(1, Ordering::AcqRel);
                    let json = delivery(&topic, &payload);
                    self.turn(|_, dispatch| dispatch.call::<_, ()>(("event", 0.0, json)))?;
                }
                Ok(Inbound::Latest { topic }) => {
                    let payload = self
                        .shared
                        .latest
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .remove(&topic);
                    let Some(payload) = payload else {
                        continue;
                    };
                    let json = delivery(&topic, &payload);
                    self.turn(|_, dispatch| dispatch.call::<_, ()>(("event", 0.0, json)))?;
                }
                Ok(Inbound::Stop) | Err(RecvTimeoutError::Disconnected) => {
                    // The code is told it is stopping; going over a limit now changes
                    // nothing.
                    let _ = self
                        .turn(|_, dispatch| dispatch.call::<_, ()>(("leave", 0.0, String::new())));
                    return Ok(());
                }
                // The timers that fell due are fired at the top of the loop.
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    fn report(&self, stop: Stop) {
        let message = match stop {
            Stop::Breach(breach, detail) => Outbound::Breached { breach, detail },
            Stop::Failed(message) => Outbound::Failed { message },
            // The watchdog has said so already.
            Stop::Hung => return,
        };
        self.shared.host.send(message);
    }

    /// Throws the engine away, in the order its parts need: what the code held first,
    /// then a last collection, then the context and the runtime.
    fn close(mut self) {
        self.made.borrow_mut().clear();
        self.dispatch.take();
        self.runtime.run_gc();
        let Self {
            runtime, context, ..
        } = self;
        drop(context);
        drop(runtime);
    }
}

/// A command or a service call as the code is handed it: `{"name", "args"}`, written
/// from what is held without copying it first.
fn named<A: serde::Serialize>(name: &str, args: &A) -> String {
    #[derive(serde::Serialize)]
    struct Named<'a, A> {
        name: &'a str,
        args: &'a A,
    }
    serde_json::to_string(&Named { name, args }).unwrap_or_default()
}

/// An event as the code is handed it: `{"topic", "payload"}`, written from what is held
/// without copying it first.
fn delivery<T: serde::Serialize + ?Sized, P: serde::Serialize>(topic: &T, payload: &P) -> String {
    #[derive(serde::Serialize)]
    struct Delivery<'a, T: ?Sized, P> {
        topic: &'a T,
        payload: &'a P,
    }
    serde_json::to_string(&Delivery { topic, payload }).unwrap_or_default()
}
#[cfg(test)]
mod tests;
