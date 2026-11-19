//! Figures of the engine: what a plugin costs at rest, how fast it starts, and how
//! quickly each limit stops it. Ignored by default; run in release for figures worth
//! reading:
//!
//! ```text
//! cargo test --release -p lume-plugin-runtime --test figures -- --ignored --nocapture --test-threads 1
//! ```
//!
//! Each prints a line of the table and checks the figure against what the limits
//! promise.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use lume_plugin_runtime::{Breach, Host, Limits, Outbound, Plugin, Spec};
use serde_json::Value;

#[derive(Default)]
struct Recorder {
    messages: Mutex<Vec<(Instant, Outbound)>>,
    changed: Condvar,
}

struct Sink(Arc<Recorder>);

impl Host for Sink {
    fn send(&self, message: Outbound) {
        self.0
            .messages
            .lock()
            .unwrap()
            .push((Instant::now(), message));
        self.0.changed.notify_all();
    }
}

impl Recorder {
    fn wait<T>(&self, pick: impl Fn(&Outbound) -> Option<T>) -> (Instant, T) {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut messages = self.messages.lock().unwrap();
        loop {
            if let Some(found) = messages
                .iter()
                .find_map(|(at, message)| pick(message).map(|found| (*at, found)))
            {
                return found;
            }
            assert!(Instant::now() < deadline, "waited too long: {messages:?}");
            messages = self
                .changed
                .wait_timeout(messages, Duration::from_secs(1))
                .unwrap()
                .0;
        }
    }
}

fn spec(id: &str, code: String, limits: Limits) -> Spec {
    Spec {
        id: id.to_owned(),
        manifest_id: id.to_owned(),
        name: id.to_owned(),
        version: "1.0.0".to_owned(),
        code,
        file: "host.js".to_owned(),
        source_map: None,
        limits,
    }
}

fn start(spec: Spec) -> (Plugin, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    let plugin = Plugin::start(spec, Box::new(Sink(Arc::clone(&recorder)))).unwrap();
    (plugin, recorder)
}

fn started(recorder: &Recorder) -> Instant {
    recorder
        .wait(|message| match message {
            Outbound::Started => Some(()),
            Outbound::Failed { message } => panic!("{message}"),
            _ => None,
        })
        .0
}

/// A bundle of about `size` bytes: functions that are defined and not called, as a
/// bundler's output mostly is, and a command.
fn bundle(size: usize) -> String {
    let mut code = String::from("const parts = [];\n");
    let mut index = 0;
    while code.len() < size {
        code.push_str(&format!(
            "function part{index}(value) {{ const doubled = value * 2; if (doubled > {index}) {{ return doubled - {index}; }} return [value, doubled, 'part{index}'].join(':'); }}\nparts.push(part{index});\n"
        ));
        index += 1;
    }
    code.push_str("lume.commands.register('ping', () => parts.length);\n");
    code
}

#[test]
#[ignore = "a measurement; run with --ignored in release"]
fn figures_at_rest_and_at_start() {
    let limits = Limits::with_memory_mib(64);
    let (plugin, recorder) = start(spec("rest", String::new(), limits));
    started(&recorder);
    println!(
        "| engine at rest | {} KiB |",
        plugin.stats().memory_used / 1024
    );

    let code = bundle(200 * 1024);
    for count in [1usize, 10, 50] {
        let began = Instant::now();
        let mut running = Vec::new();
        for index in 0..count {
            running.push(start(spec(&format!("p{index}"), code.clone(), limits)));
        }
        for (_, recorder) in &running {
            started(recorder);
        }
        let took = began.elapsed();
        let memory: u64 = running
            .iter()
            .map(|(plugin, _)| plugin.stats().memory_used)
            .sum();
        println!(
            "| {count} plugins of 200 KiB started | {:.1} ms all, {:.2} ms each | {} MiB engine memory |",
            took.as_secs_f64() * 1000.0,
            took.as_secs_f64() * 1000.0 / count as f64,
            memory / 1024 / 1024
        );
        if count == 1 {
            assert!(took < Duration::from_millis(50), "one plugin took {took:?}");
        }
        if count == 50 {
            assert!(memory < 150 * 1024 * 1024, "50 plugins hold {memory} bytes");
        }
    }
}

#[test]
#[ignore = "a measurement; run with --ignored in release"]
fn figures_of_the_limits() {
    let budget = Duration::from_millis(100);
    let cases = [
        ("an endless loop", "for (;;) {}", Breach::Time),
        (
            "a backtracking expression",
            "/(a+)+$/.test('a'.repeat(40) + '!')",
            Breach::Time,
        ),
        (
            "endless promises",
            "const again = () => Promise.resolve().then(again); again();",
            Breach::Turn,
        ),
    ];
    for (what, body, expected) in cases {
        let mut limits = Limits::with_memory_mib(64);
        limits.job = budget;
        let code = format!("lume.commands.register('go', () => {{ {body} }});");
        let (plugin, recorder) = start(spec("limit", code, limits));
        started(&recorder);
        let sent = Instant::now();
        plugin.command(1, "go", Value::Null);
        let (at, breach) = recorder.wait(|message| match message {
            Outbound::Breached { breach, .. } => Some(*breach),
            _ => None,
        });
        let took = at.duration_since(sent);
        println!("| {what} | {} ms to stop | {breach:?} |", took.as_millis());
        assert_eq!(breach, expected, "{what}");
        assert!(took < budget * 5, "{what} took {took:?}");
    }

    for mib in [16u32, 64, 256] {
        let limits = Limits::with_memory_mib(mib);
        let code = "lume.commands.register('go', () => { const all = []; for (;;) all.push(new Array(10000).fill(1)); });".to_owned();
        let (plugin, recorder) = start(spec("memory", code, limits));
        started(&recorder);
        let sent = Instant::now();
        plugin.command(1, "go", Value::Null);
        let (at, breach) = recorder.wait(|message| match message {
            Outbound::Breached { breach, .. } => Some(*breach),
            _ => None,
        });
        println!(
            "| memory without end, {mib} MiB cap | {} ms to stop | {breach:?} |",
            at.duration_since(sent).as_millis()
        );
        assert_eq!(breach, Breach::Memory);
        recorder.wait(|message| matches!(message, Outbound::Stopped).then_some(()));
    }

    let (plugin, recorder) = start(spec(
        "recursion",
        "const down = (n) => down(n + 1); lume.commands.register('go', () => { try { down(0); } catch (e) { return e.name; } });".to_owned(),
        Limits::with_memory_mib(64),
    ));
    started(&recorder);
    plugin.command(1, "go", Value::Null);
    let (_, result) = recorder.wait(|message| match message {
        Outbound::CommandDone { result, .. } => Some(result.clone()),
        _ => None,
    });
    println!("| deep recursion | caught as {} |", result.unwrap());
}
