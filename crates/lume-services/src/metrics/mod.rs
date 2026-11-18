//! How busy the machine is: the processor, memory, graphics card and network.
//!
//! The sampler reads the system once a second, but only while someone is looking: a
//! reading costs a little each time, and a meter nobody can see is not worth it. Each
//! consumer holds the sampler while what it shows is on screen ([`Sampler::hold`]), and
//! with no one holding it the sampler reads nothing at all.
//!
//! A processor's busyness and a network's speed are differences between two readings,
//! so when the sampler starts it reads twice, a quarter of a second apart, and the first
//! sample is out after that. A graphics card can only be read where the system offers a
//! way to; where it does not, or the way cannot be opened, the sample says the graphics
//! card is unavailable and the sampler tries again every ten seconds.

use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;

#[cfg(any(test, target_os = "linux"))]
mod proc;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// How often the sampler reads while it is held.
pub const PERIOD: Duration = Duration::from_secs(1);

/// How long after its first reading the sampler reads again, for the first sample.
pub const PRIMING: Duration = Duration::from_millis(250);

/// How long the sampler waits before trying again to read a graphics card it could not.
pub const GPU_RETRY: Duration = Duration::from_secs(10);

/// An amount of something used, of the whole there is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Usage {
    pub used: u64,
    pub total: u64,
}

/// How much of a graphics card's own memory is used. The whole is not known for one
/// that shares the system's memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct GpuMemory {
    pub used: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

/// What the graphics card is doing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Gpu {
    /// There is no way to read it on this system, or it could not be opened.
    Unavailable,
    Read {
        /// How busy its busiest engine is, from 0 to 1.
        #[serde(skip_serializing_if = "Option::is_none")]
        busy: Option<f32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        memory: Option<GpuMemory>,
    },
}

/// How fast the network is going, in bytes a second.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Rates {
    pub received: f64,
    pub sent: f64,
}

/// One sample.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Sample {
    /// How busy the processor is, from 0 to 1, over all its cores.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<Usage>,
    pub gpu: Gpu,
    /// Over the network interfaces that are hardware, not the loopback or virtual ones.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<Rates>,
}

/// The processor's time since the system started: all of it and the part spent idle,
/// in any one unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTimes {
    pub idle: u64,
    pub total: u64,
}

/// Bytes received and sent since some point, counting up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub received: u64,
    pub sent: u64,
}

/// What a system offers to read.
pub trait System: Send {
    fn cpu(&mut self) -> std::io::Result<CpuTimes>;
    fn memory(&mut self) -> std::io::Result<Usage>;
    fn network(&mut self) -> std::io::Result<Counters>;
}

/// A graphics card that can be read. It keeps what it needs between readings, so its
/// busyness is over the time since the one before.
pub trait GpuSource: Send {
    fn read(&mut self) -> std::io::Result<(Option<f32>, Option<GpuMemory>)>;
}

/// Opens a way to read the graphics card, if there is one.
pub type GpuOpener = Box<dyn FnMut() -> Option<Box<dyn GpuSource>> + Send>;

/// How busy the processor was between two readings, from 0 to 1.
pub fn cpu_busy(before: CpuTimes, now: CpuTimes) -> Option<f32> {
    let total = now.total.checked_sub(before.total)?;
    let idle = now.idle.checked_sub(before.idle)?;
    if total == 0 {
        return None;
    }
    #[allow(clippy::cast_precision_loss, reason = "a share, not an exact count")]
    let busy = 1.0 - idle.min(total) as f64 / total as f64;
    #[allow(clippy::cast_possible_truncation, reason = "a share between 0 and 1")]
    Some(busy as f32)
}

/// How fast the network went between two readings `seconds` apart. A counter that went
/// down was started again, and says nothing.
pub fn network_rates(before: Counters, now: Counters, seconds: f64) -> Option<Rates> {
    if seconds <= 0.0 {
        return None;
    }
    let received = now.received.checked_sub(before.received)?;
    let sent = now.sent.checked_sub(before.sent)?;
    #[allow(clippy::cast_precision_loss, reason = "a rate, not an exact count")]
    Some(Rates {
        received: received as f64 / seconds,
        sent: sent as f64 / seconds,
    })
}

/// How long the sampler waits, so the tests can make it quick.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub period: Duration,
    pub priming: Duration,
    pub gpu_retry: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            period: PERIOD,
            priming: PRIMING,
            gpu_retry: GPU_RETRY,
        }
    }
}

type Listener = Arc<dyn Fn(&Sample) + Send + Sync>;

#[derive(Default)]
struct State {
    holders: usize,
    stopped: bool,
    latest: Option<Sample>,
    listeners: Vec<(u64, Listener)>,
    next_listener: u64,
    /// How many readings have been taken, for the tests.
    readings: u64,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The sampler. It reads on a thread of its own, while it is held.
pub struct Sampler {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

/// The sampler held by one consumer; it lets go when this is dropped.
pub struct Hold {
    shared: Arc<Shared>,
}

impl Drop for Hold {
    fn drop(&mut self) {
        let mut state = self.shared.state();
        state.holders = state.holders.saturating_sub(1);
        drop(state);
        self.shared.changed.notify_all();
    }
}

/// A listener; it stops listening when this is dropped.
pub struct Listening {
    shared: Arc<Shared>,
    id: u64,
}

impl Drop for Listening {
    fn drop(&mut self) {
        self.shared
            .state()
            .listeners
            .retain(|(id, _)| *id != self.id);
    }
}

impl Sampler {
    /// The sampler of this system.
    pub fn start() -> Self {
        Self::with(platform_system(), Box::new(platform_gpu), Timing::default())
    }

    /// A sampler reading `system` and the graphics card `gpu` opens.
    pub fn with(system: Box<dyn System>, gpu: GpuOpener, timing: Timing) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        });
        let thread = {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("metrics".to_owned())
                .spawn(move || run(&shared, system, gpu, timing))
                .ok()
        };
        Self { shared, thread }
    }

    /// Holds the sampler: it reads while anyone does.
    pub fn hold(&self) -> Hold {
        self.shared.state().holders += 1;
        self.shared.changed.notify_all();
        Hold {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Whether anyone holds the sampler, so that it reads.
    pub fn is_held(&self) -> bool {
        self.shared.state().holders > 0
    }

    /// The latest sample, if one has been taken.
    pub fn latest(&self) -> Option<Sample> {
        self.shared.state().latest
    }

    /// Calls `listener` with every sample from now on, on the sampler's thread.
    pub fn listen(&self, listener: impl Fn(&Sample) + Send + Sync + 'static) -> Listening {
        let mut state = self.shared.state();
        let id = state.next_listener;
        state.next_listener += 1;
        state.listeners.push((id, Arc::new(listener)));
        Listening {
            shared: Arc::clone(&self.shared),
            id,
        }
    }

    /// How many readings the sampler has taken.
    pub fn readings(&self) -> u64 {
        self.shared.state().readings
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.shared.state().stopped = true;
        self.shared.changed.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// What one reading found.
struct Reading {
    at: Instant,
    cpu: Option<CpuTimes>,
    memory: Option<Usage>,
    network: Option<Counters>,
}

/// Waits for up to `wait`, or until the sampler is let go or stopped. Says whether it is
/// still to read.
fn pause(shared: &Shared, wait: Duration) -> bool {
    let deadline = Instant::now() + wait;
    let mut state = shared.state();
    loop {
        if state.stopped || state.holders == 0 {
            return false;
        }
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        state = shared
            .changed
            .wait_timeout(state, deadline - now)
            .unwrap_or_else(PoisonError::into_inner)
            .0;
    }
}

fn run(shared: &Shared, mut system: Box<dyn System>, mut open_gpu: GpuOpener, timing: Timing) {
    let mut gpu: Option<Box<dyn GpuSource>> = None;
    let mut gpu_tried: Option<Instant> = None;
    loop {
        // Asleep until someone holds the sampler.
        {
            let mut state = shared.state();
            while state.holders == 0 && !state.stopped {
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            if state.stopped {
                return;
            }
        }
        let mut before: Option<Reading> = None;
        loop {
            let reading = Reading {
                at: Instant::now(),
                cpu: system.cpu().ok(),
                memory: system.memory().ok(),
                network: system.network().ok(),
            };
            if gpu.is_none() && gpu_tried.is_none_or(|tried| tried.elapsed() >= timing.gpu_retry) {
                gpu = open_gpu();
                gpu_tried = Some(Instant::now());
            }
            let gpu_read = match gpu.as_mut().map(|source| source.read()) {
                Some(Ok((busy, memory))) => Gpu::Read { busy, memory },
                Some(Err(_)) => {
                    // It went away; try to open it again later.
                    gpu = None;
                    gpu_tried = Some(Instant::now());
                    Gpu::Unavailable
                }
                None => Gpu::Unavailable,
            };
            let first = before.is_none();
            if let Some(before) = &before {
                let seconds = reading.at.duration_since(before.at).as_secs_f64();
                let sample = Sample {
                    cpu: before
                        .cpu
                        .zip(reading.cpu)
                        .and_then(|(old, new)| cpu_busy(old, new)),
                    memory: reading.memory,
                    gpu: gpu_read,
                    network: before
                        .network
                        .zip(reading.network)
                        .and_then(|(old, new)| network_rates(old, new, seconds)),
                };
                let listeners: Vec<Listener> = {
                    let mut state = shared.state();
                    state.latest = Some(sample);
                    state.readings += 1;
                    state.listeners.iter().map(|(_, l)| Arc::clone(l)).collect()
                };
                for listener in listeners {
                    listener(&sample);
                }
            } else {
                shared.state().readings += 1;
            }
            before = Some(reading);
            let wait = if first { timing.priming } else { timing.period };
            if !pause(shared, wait) {
                break;
            }
        }
    }
}

/// The system this program runs on.
fn platform_system() -> Box<dyn System> {
    #[cfg(windows)]
    return Box::new(windows::WindowsSystem);
    #[cfg(target_os = "linux")]
    return Box::new(linux::LinuxSystem);
    #[cfg(target_os = "macos")]
    return Box::new(macos::MacSystem::new());
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    return Box::new(Nothing);
}

/// Opens this system's graphics card, if it can be read.
fn platform_gpu() -> Option<Box<dyn GpuSource>> {
    #[cfg(windows)]
    return windows::open_gpu();
    #[cfg(target_os = "linux")]
    return linux::open_gpu();
    #[cfg(target_os = "macos")]
    return macos::open_gpu();
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    return None;
}

/// A system with nothing to read.
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
struct Nothing;

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
impl System for Nothing {
    fn cpu(&mut self) -> std::io::Result<CpuTimes> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
    fn memory(&mut self) -> std::io::Result<Usage> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
    fn network(&mut self) -> std::io::Result<Counters> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    /// A system whose counters go up by the same amount at every reading.
    struct Steady {
        step: Arc<AtomicU64>,
    }

    impl System for Steady {
        fn cpu(&mut self) -> std::io::Result<CpuTimes> {
            let step = self.step.fetch_add(1, Ordering::Relaxed);
            // A quarter of the time busy.
            Ok(CpuTimes {
                idle: step * 300,
                total: step * 400,
            })
        }
        fn memory(&mut self) -> std::io::Result<Usage> {
            Ok(Usage { used: 3, total: 8 })
        }
        fn network(&mut self) -> std::io::Result<Counters> {
            let step = self.step.load(Ordering::Relaxed);
            Ok(Counters {
                received: step * 1000,
                sent: step * 10,
            })
        }
    }

    struct Card;

    impl GpuSource for Card {
        fn read(&mut self) -> std::io::Result<(Option<f32>, Option<GpuMemory>)> {
            Ok((
                Some(0.5),
                Some(GpuMemory {
                    used: 1,
                    total: Some(4),
                }),
            ))
        }
    }

    fn quick() -> Timing {
        Timing {
            period: Duration::from_millis(20),
            priming: Duration::from_millis(5),
            gpu_retry: Duration::from_millis(30),
        }
    }

    fn wait_for(what: impl Fn() -> bool) {
        let started = Instant::now();
        while !what() {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "waited too long"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn the_processor_and_the_network_are_differences_between_readings() {
        let before = CpuTimes {
            idle: 100,
            total: 200,
        };
        let now = CpuTimes {
            idle: 130,
            total: 240,
        };
        assert_eq!(cpu_busy(before, now), Some(0.25));
        assert_eq!(cpu_busy(now, before), None);
        assert_eq!(cpu_busy(now, now), None);
        let rates = network_rates(
            Counters {
                received: 0,
                sent: 0,
            },
            Counters {
                received: 2000,
                sent: 100,
            },
            2.0,
        )
        .unwrap();
        assert_eq!(
            rates,
            Rates {
                received: 1000.0,
                sent: 50.0
            }
        );
        assert_eq!(
            network_rates(
                Counters {
                    received: 5,
                    sent: 5
                },
                Counters::default(),
                1.0
            ),
            None
        );
    }

    #[test]
    fn nothing_is_read_while_nobody_holds_the_sampler() {
        let step = Arc::new(AtomicU64::new(1));
        let opened = Arc::new(AtomicU64::new(0));
        let sampler = {
            let opened = Arc::clone(&opened);
            Sampler::with(
                Box::new(Steady {
                    step: Arc::clone(&step),
                }),
                Box::new(move || {
                    opened.fetch_add(1, Ordering::Relaxed);
                    Some(Box::new(Card) as Box<dyn GpuSource>)
                }),
                quick(),
            )
        };
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(sampler.readings(), 0);
        assert!(sampler.latest().is_none());

        let heard = Arc::new(Mutex::new(Vec::new()));
        let _listening = {
            let heard = Arc::clone(&heard);
            sampler.listen(move |sample| heard.lock().unwrap().push(*sample))
        };
        let hold = sampler.hold();
        let second = sampler.hold();
        wait_for(|| heard.lock().unwrap().len() >= 3);
        let sample = sampler.latest().unwrap();
        assert_eq!(sample.cpu, Some(0.25));
        assert_eq!(sample.memory, Some(Usage { used: 3, total: 8 }));
        assert!(matches!(sample.gpu, Gpu::Read { busy: Some(b), .. } if b == 0.5));
        assert!(sample.network.is_some_and(|rates| rates.received > 0.0));
        // The card was opened once, not at every reading.
        assert_eq!(opened.load(Ordering::Relaxed), 1);

        drop(hold);
        assert!(sampler.is_held());
        drop(second);
        assert!(!sampler.is_held());
        std::thread::sleep(Duration::from_millis(30));
        let stopped_at = sampler.readings();
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(sampler.readings(), stopped_at);
    }

    #[test]
    fn a_card_that_cannot_be_opened_is_unavailable_and_tried_again_later() {
        let tries = Arc::new(AtomicU64::new(0));
        let sampler = {
            let tries = Arc::clone(&tries);
            Sampler::with(
                Box::new(Steady {
                    step: Arc::new(AtomicU64::new(1)),
                }),
                Box::new(move || {
                    tries.fetch_add(1, Ordering::Relaxed);
                    None
                }),
                quick(),
            )
        };
        let _hold = sampler.hold();
        wait_for(|| sampler.latest().is_some());
        assert_eq!(sampler.latest().unwrap().gpu, Gpu::Unavailable);
        wait_for(|| tries.load(Ordering::Relaxed) >= 2);
        // Not at every reading: once per retry time.
        let readings = sampler.readings();
        assert!(readings > tries.load(Ordering::Relaxed), "{readings}");
    }

    #[test]
    fn a_sample_is_written_for_plugins_as_plain_data() {
        let sample = Sample {
            cpu: Some(0.5),
            memory: Some(Usage { used: 1, total: 2 }),
            gpu: Gpu::Unavailable,
            network: None,
        };
        assert_eq!(
            serde_json::to_value(sample).unwrap(),
            serde_json::json!({
                "cpu": 0.5,
                "memory": { "used": 1, "total": 2 },
                "gpu": { "state": "unavailable" }
            })
        );
    }

    /// Reads this machine for real. Run by hand or in CI on each system.
    #[test]
    #[ignore = "reads the machine it runs on; run with --ignored"]
    fn this_machine_can_be_read() {
        let sampler = Sampler::start();
        let _hold = sampler.hold();
        wait_for(|| sampler.latest().is_some());
        std::thread::sleep(PERIOD + Duration::from_millis(100));
        let sample = sampler.latest().unwrap();
        println!("{}", serde_json::to_string_pretty(&sample).unwrap());
        assert!(sample.cpu.is_some_and(|cpu| (0.0..=1.0).contains(&cpu)));
        let memory = sample.memory.unwrap();
        assert!(memory.used > 0 && memory.used <= memory.total);
        assert!(sample.network.is_some());
    }
}
