use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::{Breach, Host, Level, Limits, Outbound, Plugin, Spec};

/// A host that keeps every message.
#[derive(Default)]
struct Recorder {
    messages: Mutex<Vec<Outbound>>,
    changed: Condvar,
}

struct Shared(std::sync::Arc<Recorder>);

impl Host for Shared {
    fn send(&self, message: Outbound) {
        self.0.messages.lock().unwrap().push(message);
        self.0.changed.notify_all();
    }
}

impl Recorder {
    /// Waits for a message `pick` takes, and returns what it took.
    fn wait<T>(&self, within: Duration, pick: impl Fn(&Outbound) -> Option<T>) -> T {
        let deadline = Instant::now() + within;
        let mut messages = self.messages.lock().unwrap();
        loop {
            if let Some(found) = messages.iter().find_map(&pick) {
                return found;
            }
            let now = Instant::now();
            assert!(now < deadline, "waited too long; messages: {messages:#?}");
            messages = self
                .changed
                .wait_timeout(messages, deadline - now)
                .unwrap()
                .0;
        }
    }

    fn logs(&self) -> Vec<String> {
        self.messages
            .lock()
            .unwrap()
            .iter()
            .filter_map(|message| match message {
                Outbound::Log { message, .. } => Some(message.clone()),
                _ => None,
            })
            .collect()
    }
}

const WAIT: Duration = Duration::from_secs(10);

fn spec(code: &str) -> Spec {
    Spec {
        id: "acme.test".to_owned(),
        manifest_id: "acme.test".to_owned(),
        name: "Test".to_owned(),
        version: "1.0.0".to_owned(),
        code: code.to_owned(),
        file: "dist/host.js".to_owned(),
        source_map: None,
        limits: Limits::with_memory_mib(32),
    }
}

fn start(spec: Spec) -> (Plugin, std::sync::Arc<Recorder>) {
    let recorder = std::sync::Arc::new(Recorder::default());
    let plugin = Plugin::start(spec, Box::new(Shared(std::sync::Arc::clone(&recorder)))).unwrap();
    (plugin, recorder)
}

fn started(recorder: &Recorder) {
    recorder.wait(WAIT, |message| match message {
        Outbound::Started => Some(()),
        Outbound::Failed { message } => panic!("failed: {message}"),
        _ => None,
    });
}

fn command_result(recorder: &Recorder, invocation: u64) -> Result<Value, crate::ApiError> {
    recorder.wait(WAIT, |message| match message {
        Outbound::CommandDone {
            invocation: done,
            result,
        } if *done == invocation => Some(result.clone()),
        _ => None,
    })
}

fn breach(recorder: &Recorder) -> Breach {
    recorder.wait(WAIT, |message| match message {
        Outbound::Breached { breach, .. } => Some(*breach),
        _ => None,
    })
}

fn stopped(recorder: &Recorder) {
    recorder.wait(WAIT, |message| {
        matches!(message, Outbound::Stopped).then_some(())
    });
}

#[test]
fn a_command_is_carried_out_by_its_handler() {
    let (plugin, recorder) = start(spec(
        r#"
        lume.commands.register("double", (args) => args.n * 2);
        lume.commands.register("async", async () => { await null; return "later"; });
        lume.commands.register("throws", () => { throw new Error("no"); });
        "#,
    ));
    started(&recorder);
    plugin.command(1, "double", json!({ "n": 21 }));
    assert_eq!(command_result(&recorder, 1), Ok(json!(42)));
    plugin.command(2, "async", Value::Null);
    assert_eq!(command_result(&recorder, 2), Ok(json!("later")));
    plugin.command(3, "throws", Value::Null);
    assert!(
        command_result(&recorder, 3)
            .unwrap_err()
            .message
            .starts_with("no")
    );
    plugin.command(4, "missing", Value::Null);
    assert_eq!(
        command_result(&recorder, 4).unwrap_err().code,
        crate::ErrorCode::Unhandled
    );
}

#[test]
fn a_call_goes_to_the_host_and_its_answer_comes_back() {
    let (plugin, recorder) = start(spec(
        r#"
        lume.commands.register("read", async () => {
            const value = await lume.kv.get("count");
            try { await lume.kv.get("denied"); } catch (error) { return [value, error.code, error instanceof lume.LumeError]; }
        });
        "#,
    ));
    started(&recorder);
    plugin.command(1, "read", Value::Null);
    let first = recorder.wait(WAIT, |message| match message {
        Outbound::Call { id, name, args } if args["key"] == "count" => Some((*id, name.clone())),
        _ => None,
    });
    assert_eq!(first.1, "kv.get");
    plugin.answer(first.0, Ok(json!(7)));
    let second = recorder.wait(WAIT, |message| match message {
        Outbound::Call { id, args, .. } if args["key"] == "denied" => Some(*id),
        _ => None,
    });
    plugin.answer(second, Err(crate::ApiError::denied("no")));
    assert_eq!(command_result(&recorder, 1), Ok(json!([7, "denied", true])));
    // An answer to a call nobody waits for is passed over.
    plugin.answer(999, Ok(json!(1)));
    assert!(plugin.stats().calls_in_flight == 0);
}

#[test]
fn timers_fire_and_can_be_cleared() {
    let (plugin, recorder) = start(spec(
        r#"
        let ticks = 0;
        const every = lume.timers.setInterval(() => { ticks += 1; if (ticks === 3) { lume.timers.clearInterval(every); lume.log.info("ticked", ticks); } }, 5);
        const never = lume.timers.setTimeout(() => lume.log.error("should not fire"), 5);
        lume.timers.clearTimeout(never);
        lume.timers.setTimeout((word) => lume.log.info("once", word), 1, "done");
        "#,
    ));
    started(&recorder);
    recorder.wait(WAIT, |message| match message {
        Outbound::Log { message, .. } if message == "ticked 3" => Some(()),
        _ => None,
    });
    recorder.wait(WAIT, |message| match message {
        Outbound::Log { message, .. } if message == "once done" => Some(()),
        _ => None,
    });
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        !recorder
            .logs()
            .iter()
            .any(|line| line.contains("should not fire"))
    );
    assert_eq!(plugin.stats().timers, 0);
}

#[test]
fn a_loop_that_never_ends_is_stopped_within_its_budget() {
    let mut spec = spec("lume.commands.register('spin', () => { for (;;) {} });");
    spec.limits.job = Duration::from_millis(50);
    let (plugin, recorder) = start(spec);
    started(&recorder);
    let sent = Instant::now();
    plugin.command(1, "spin", Value::Null);
    assert_eq!(breach(&recorder), Breach::Time);
    let took = sent.elapsed();
    assert!(took < Duration::from_millis(500), "stopped after {took:?}");
    stopped(&recorder);
}

#[test]
fn a_loop_in_the_top_level_is_stopped_too() {
    let mut spec = spec("while (true) {}");
    spec.limits.job = Duration::from_millis(30);
    let (_plugin, recorder) = start(spec);
    assert_eq!(breach(&recorder), Breach::Time);
    stopped(&recorder);
}

#[test]
fn memory_without_bound_is_stopped_and_the_host_goes_on() {
    let (plugin, recorder) = start(spec(
        "lume.commands.register('grow', () => { const all = []; for (;;) all.push(new Array(10000).fill(1)); });",
    ));
    started(&recorder);
    plugin.command(1, "grow", Value::Null);
    assert_eq!(breach(&recorder), Breach::Memory);
    stopped(&recorder);
    // Another plugin starts and works after it.
    let (other, others) = start(spec("lume.commands.register('ok', () => 'ok');"));
    started(&others);
    other.command(1, "ok", Value::Null);
    assert_eq!(command_result(&others, 1), Ok(json!("ok")));
}

#[test]
fn promises_that_never_settle_down_are_stopped() {
    let (plugin, recorder) = start(spec(
        "lume.commands.register('flood', () => { const again = () => Promise.resolve().then(again); again(); });",
    ));
    started(&recorder);
    plugin.command(1, "flood", Value::Null);
    assert_eq!(breach(&recorder), Breach::Turn);
    stopped(&recorder);
}

#[test]
fn deep_recursion_is_an_error_the_code_can_catch() {
    let (plugin, recorder) = start(spec(
        r#"
        const down = (n) => down(n + 1) + 1;
        lume.commands.register("deep", () => { try { down(0); } catch (error) { return error.name; } });
        "#,
    ));
    started(&recorder);
    plugin.command(1, "deep", Value::Null);
    let result = command_result(&recorder, 1).unwrap();
    assert!(
        result == json!("RangeError") || result == json!("InternalError"),
        "{result}"
    );
}

#[test]
fn a_top_level_that_throws_fails_the_start_with_its_stack_in_the_log() {
    let (_plugin, recorder) = start(spec(
        "function boom() { throw new Error('broken'); }\nboom();",
    ));
    let message = recorder.wait(WAIT, |message| match message {
        Outbound::Failed { message } => Some(message.clone()),
        _ => None,
    });
    assert!(message.contains("failed when it started"), "{message}");
    let logs = recorder.logs();
    assert!(
        logs.iter()
            .any(|line| line.contains("broken") && line.contains("dist/host.js:1")),
        "{logs:#?}"
    );
    stopped(&recorder);
}

#[test]
fn a_stack_names_the_source_when_the_package_has_a_map() {
    let mut spec = spec("lume.commands.register('fail', () => { throw new Error('mapped'); });");
    // Every column of line 1 comes from src/main.ts line 4.
    spec.source_map =
        Some(r#"{"version":3,"sources":["../src/main.ts"],"mappings":"AAGA"}"#.to_owned());
    let (plugin, recorder) = start(spec);
    started(&recorder);
    plugin.command(1, "fail", Value::Null);
    command_result(&recorder, 1).unwrap_err();
    let logs = recorder.logs();
    assert!(
        logs.iter().any(|line| line.contains("src/main.ts:4")),
        "{logs:#?}"
    );
}

#[test]
fn the_code_has_nothing_but_the_language_and_lume() {
    let (plugin, recorder) = start(spec(
        r#"
        lume.commands.register("look", () => ({
            require: typeof require,
            process: typeof process,
            fetch: typeof fetch,
            setTimeout: typeof setTimeout,
            os: typeof os,
            std: typeof std,
            importing: (() => { try { eval("import('x')"); return "allowed"; } catch (error) { return "refused"; } })(),
            frozen: Object.isFrozen(lume) && Object.isFrozen(lume.kv),
            self: lume.self.id,
        }));
        "#,
    ));
    started(&recorder);
    plugin.command(1, "look", Value::Null);
    let found = command_result(&recorder, 1).unwrap();
    for name in ["require", "process", "fetch", "setTimeout", "os", "std"] {
        assert_eq!(found[name], "undefined", "{name}");
    }
    assert_eq!(found["frozen"], true);
    assert_eq!(found["self"], "acme.test");
}

#[test]
fn the_lume_object_has_every_call_the_api_defines() {
    let (plugin, recorder) = start(spec(
        r#"
        lume.commands.register("shape", () => {
            const names = [];
            const walk = (value, path) => {
                for (const key of Object.keys(value)) {
                    const next = value[key];
                    if (typeof next === "function") names.push(path.concat(key).join("."));
                    else if (next && typeof next === "object") walk(next, path.concat(key));
                }
            };
            walk(lume, []);
            return names;
        });
        "#,
    ));
    started(&recorder);
    plugin.command(1, "shape", Value::Null);
    let found = command_result(&recorder, 1).unwrap();
    let names: Vec<&str> = found
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for method in lume_plugin_api::METHODS
        .iter()
        .filter(|method| !method.internal)
    {
        assert!(
            names.contains(&method.name),
            "lume.{} is missing; there are {names:?}",
            method.name
        );
    }
}

#[test]
fn stopping_with_work_still_waiting_takes_the_engine_down_cleanly() {
    let (plugin, recorder) = start(spec(
        r#"
        lume.timers.setInterval(() => {}, 10);
        lume.kv.get("never answered").then(() => {});
        new Promise(() => {});
        lume.sessions.onEvent(() => {});
        lume.onDeactivate(() => lume.log.info("bye"));
        "#,
    ));
    started(&recorder);
    plugin.stop();
    stopped(&recorder);
    assert!(recorder.logs().iter().any(|line| line == "bye"));
    // The subscription was asked for, as a call needing a permission.
    recorder.wait(WAIT, |message| match message {
        Outbound::Call { name, .. } if name == "events.subscribe" => Some(()),
        _ => None,
    });
}

#[test]
fn events_reach_the_listeners() {
    let (plugin, recorder) = start(spec(
        r#"lume.sessions.onEvent((report) => lume.log.info("heard", report.kind));"#,
    ));
    started(&recorder);
    assert!(plugin.event(
        "session",
        json!({ "session": "s1", "kind": "bell", "details": {} })
    ));
    recorder.wait(WAIT, |message| match message {
        Outbound::Log {
            message,
            level: Level::Info,
        } if message == "heard bell" => Some(()),
        _ => None,
    });
}

#[test]
fn a_plugin_busy_where_the_interrupt_cannot_reach_is_given_up_on() {
    let mut spec = spec(
        r#"lume.commands.register("parse", () => JSON.parse("[" + "1,".repeat(2000000) + "1]").length);"#,
    );
    spec.limits = Limits::with_memory_mib(256);
    spec.limits.job = Duration::from_millis(2);
    let (plugin, recorder) = start(spec);
    started(&recorder);
    plugin.command(1, "parse", Value::Null);
    // Either the watchdog saw it busy past five budgets, or the interrupt stopped it
    // once the native work returned; it never runs on.
    let outcome = recorder.wait(WAIT, |message| match message {
        Outbound::Hung => Some("hung"),
        Outbound::Breached { .. } => Some("breached"),
        _ => None,
    });
    assert!(outcome == "hung" || outcome == "breached");
    stopped(&recorder);
}

#[test]
fn the_calls_of_a_turn_are_followed_by_its_end() {
    let (plugin, recorder) = start(spec(
        r#"
        lume.commands.register("show", () => {
            lume.kv.set("a", 1);
            lume.kv.set("b", 2);
        });
        "#,
    ));
    started(&recorder);
    plugin.command(1, "show", Value::Null);
    command_result(&recorder, 1).unwrap();
    recorder.wait(WAIT, |message| {
        matches!(message, Outbound::TurnDone).then_some(())
    });
    let messages = recorder.messages.lock().unwrap();
    let kinds: Vec<&str> = messages
        .iter()
        .filter_map(|message| match message {
            Outbound::Call { .. } => Some("call"),
            Outbound::TurnDone => Some("end"),
            _ => None,
        })
        .collect();
    // Both calls, then one end: what a turn shows is told as one.
    assert_eq!(kinds, ["call", "call", "end"]);
}

#[test]
fn only_the_newest_of_what_waits_on_a_topic_is_heard() {
    let (plugin, recorder) = start(spec(
        r#"
        lume.commands.register("busy", () => {
            const until = Date.now() + 40;
            while (Date.now() < until) {}
        });
        lume.settings.onChange((value) => lume.log.info("heard", value.n));
        "#,
    ));
    started(&recorder);
    // While the code is busy, three arrive: the first waits, the others replace it.
    plugin.command(1, "busy", Value::Null);
    assert!(!plugin.latest("settings", json!({ "n": 1 })));
    assert!(plugin.latest("settings", json!({ "n": 2 })));
    assert!(plugin.latest("settings", json!({ "n": 3 })));
    recorder.wait(WAIT, |message| match message {
        Outbound::Log { message, .. } if message == "heard 3" => Some(()),
        _ => None,
    });
    // One taken, the next waits again.
    assert!(!plugin.latest("settings", json!({ "n": 4 })));
    recorder.wait(WAIT, |message| match message {
        Outbound::Log { message, .. } if message == "heard 4" => Some(()),
        _ => None,
    });
    let heard: Vec<String> = recorder
        .logs()
        .into_iter()
        .filter(|line| line.starts_with("heard"))
        .collect();
    assert_eq!(heard, ["heard 3", "heard 4"]);
}
