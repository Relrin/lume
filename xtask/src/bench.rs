//! `cargo xtask bench`: measure the application and hold the numbers to their budgets.
//!
//! Most of the measuring is done inside the application's webview, by three suites:
//! one times a terminal against a real session, one times the window's own page under
//! load, and the suite that checks the page counts the renders terminal output causes
//! there, which must be none. How long the application takes to start is measured here,
//! from outside, by starting it and asking it how far it has come. The numbers are put
//! in one report and compared with the limits in `tests/bench/budgets.toml`, and a
//! number outside its limit fails the run.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::paths;
use crate::suites::{self, Profile};

/// How many times the application is started to time its start, after one start that
/// is not counted: the first start after a build reads everything from disk.
const STARTS: usize = 5;

/// How many plugins are installed for the starts that show what installed plugins
/// cost a start, and how many pairs of starts, one without them and one with, are
/// timed. None of the plugins runs: each waits for a command nobody gives.
const PLUGINS_INSTALLED: usize = 20;
const PLUGIN_PAIRS: usize = 21;

/// How many times the pairs' differences are drawn again to say how sure their middle
/// one is.
const RESAMPLES: usize = 2000;

/// How long one start may take before the measurement gives up.
const START_TIMEOUT: Duration = Duration::from_secs(60);

/// How often the application is asked how far it has come.
const POLL: Duration = Duration::from_millis(20);

/// How long the screen must stay as it is, with text at the cursor, to count as a prompt.
const PROMPT_SETTLED: Duration = Duration::from_millis(200);

/// What the suite that times the window's page reports, each under the name it has
/// in the one report.
const PAGE_NUMBERS: [&str; 5] = ["panes16", "drag", "tabSwitch", "search", "cursorJumps"];

/// What is taken from the report of the suite that checks the window's page.
const PAGE_CHECKS: [&str; 3] = ["outputCommits", "outputBytes", "switchCommits"];

/// The limits of one profile, by the path of the number in the report.
type Limits = BTreeMap<String, Limit>;

/// What one number is held to.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Limit {
    /// The number must not be above this.
    max: Option<f64>,
    /// The number must not be below this.
    min: Option<f64>,
    /// What the number is counted in, for the table.
    unit: String,
    /// Whether a report without the number passes. A webview that lacks a renderer tier
    /// has no number for it, and that is not the benchmark's to judge.
    #[serde(default)]
    optional: bool,
}

/// The budgets file: a table of limits for each profile.
fn load_limits(profile: &str) -> Result<Limits> {
    let path = paths::root()
        .join("tests")
        .join("bench")
        .join("budgets.toml");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut profiles: BTreeMap<String, Limits> =
        toml::from_str(&text).with_context(|| format!("{} is not valid", path.display()))?;
    let known: Vec<String> = profiles.keys().cloned().collect();
    profiles.remove(profile).with_context(|| {
        format!(
            "{} has no profile called {profile} (it has {})",
            path.display(),
            known.join(", ")
        )
    })
}

/// The number at a dotted path in the report, such as `redraw.webgpu.finish.p95`.
fn number_at(report: &Value, path: &str) -> Option<f64> {
    path.split('.')
        .try_fold(report, |value, key| value.get(key))?
        .as_f64()
}

/// One row of the result: a number against its limit.
struct Row {
    path: String,
    measured: Option<f64>,
    limit: String,
    unit: String,
    verdict: Verdict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    Within,
    Outside,
    /// The report has no such number, and need not have.
    NotMeasured,
    /// The report has no such number, and should have.
    Missing,
}

fn judge(report: &Value, limits: &Limits) -> Vec<Row> {
    limits
        .iter()
        .map(|(path, limit)| {
            let measured = number_at(report, path);
            let verdict = match measured {
                None if limit.optional => Verdict::NotMeasured,
                None => Verdict::Missing,
                Some(value) => {
                    let above = limit.max.is_some_and(|max| value > max);
                    let below = limit.min.is_some_and(|min| value < min);
                    if above || below {
                        Verdict::Outside
                    } else {
                        Verdict::Within
                    }
                }
            };
            let bound = match (limit.min, limit.max) {
                (Some(min), Some(max)) => format!("{min} to {max}"),
                (Some(min), None) => format!("at least {min}"),
                (None, Some(max)) => format!("at most {max}"),
                (None, None) => "recorded only".to_owned(),
            };
            Row {
                path: path.clone(),
                measured,
                limit: bound,
                unit: limit.unit.clone(),
                verdict,
            }
        })
        .collect()
}

fn print_table(rows: &[Row]) {
    let width = rows.iter().map(|row| row.path.len()).max().unwrap_or(0);
    for row in rows {
        let measured = match row.measured {
            Some(value) => format!("{value} {}", row.unit),
            None => "not measured".to_owned(),
        };
        let verdict = match row.verdict {
            Verdict::Within => "ok",
            Verdict::Outside => "OUTSIDE ITS LIMIT",
            Verdict::NotMeasured => "",
            Verdict::Missing => "MISSING",
        };
        println!(
            "{:width$}  {:>16}  {:<16}  {verdict}",
            row.path, measured, row.limit
        );
    }
}

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples.get(samples.len() / 2).copied().unwrap_or(0.0)
}

/// One request to a running application through the console command.
fn ask(lume: &Path, endpoint: &str, args: &[&str]) -> Option<Value> {
    let output = Command::new(lume)
        .args(["ctl", "--endpoint", endpoint])
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

/// The points of a start the application reports, in the order they are reached, with
/// the name each has in the report.
const MARKS: [(&str, &str); 18] = [
    ("config_ms", "configMs"),
    ("host_ms", "hostMs"),
    ("panes_ms", "panesMs"),
    ("setup_ms", "setupMs"),
    ("window_ms", "windowMs"),
    ("spawned_ms", "spawnedMs"),
    ("page_ms", "pageMs"),
    ("parsed_ms", "parsedMs"),
    ("carrier_ms", "carrierMs"),
    ("vt_ms", "vtMs"),
    ("gpu_ms", "gpuMs"),
    ("fonts_ms", "fontsMs"),
    ("layout_ms", "layoutMs"),
    ("frame_ms", "frameMs"),
    ("attached_ms", "attachedMs"),
    ("output_ms", "outputMs"),
    ("page_output_ms", "pageOutputMs"),
    ("drawn_ms", "drawnMs"),
];

/// How far one start had come when its shell's first output reached the page, and when
/// the screen showed a prompt. All in milliseconds from the start of the process.
struct Start {
    /// The application's own marks, as its status reports them.
    marks: Value,
    output_ms: f64,
    prompt_ms: f64,
    /// When the installed plugins had been read, which is after the first output.
    plugins_ms: Option<f64>,
    /// How many of the plugins' files were read before the first output.
    plugin_files_before_output: Option<f64>,
}

impl Start {
    /// One mark of the start, if the application reported it.
    fn mark(&self, name: &str) -> Option<f64> {
        self.marks[name].as_f64()
    }

    /// The start on one line: every mark, then the prompt.
    fn line(&self) -> String {
        let mut line = String::new();
        for (name, _) in MARKS {
            let label = name.trim_end_matches("_ms");
            match self.mark(name) {
                Some(ms) => line.push_str(&format!("{label} {ms:.0}, ")),
                None => line.push_str(&format!("{label} not reached, ")),
            }
        }
        line.push_str(&format!("prompt {:.0} ms", self.prompt_ms));
        line
    }
}

/// Starts the application once and times its start: until the first output of its
/// shell was handed to the page, by the application's own account, and until the
/// screen showed a prompt, by the clock here.
fn time_one_start(app: &Path, lume: &Path, endpoint: &str, plugins: usize) -> Result<Start> {
    // Each start is a first start: nothing of the user's is read, and no workspace is
    // there to be brought back.
    let dirs = paths::PrivateDirs::fresh(endpoint)?;
    install_plugins(lume, &dirs, plugins)?;
    let mut command = Command::new(app);
    dirs.apply(&mut command);
    let started = Instant::now();
    let mut child = command
        .args(["--endpoint", endpoint])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("starting the application")?;

    // The application's marks, once the last of them is set.
    let mut reached: Option<(Value, f64)> = None;
    // The marks as last reported, to say how far a start that never finished had come.
    let mut last_marks = Value::Null;
    // The screen as last seen with text at the cursor, and when it was first seen so.
    let mut settling: Option<(Value, Instant)> = None;
    let result = loop {
        if started.elapsed() > START_TIMEOUT {
            break Err(anyhow::anyhow!(
                "the application showed no prompt in time; its start had come to {last_marks}"
            ));
        }
        std::thread::sleep(POLL);
        let Some((marks, output_ms)) = &reached else {
            if let Some(status) = ask(lume, endpoint, &["status"]) {
                last_marks = status["marks"].clone();
                if let Some(output_ms) = last_marks["output_ms"].as_f64() {
                    reached = Some((last_marks.clone(), output_ms));
                }
            }
            continue;
        };
        let Some(screen) = ask(lume, endpoint, &["capture"]) else {
            continue;
        };
        let seen = Instant::now();
        let row = screen["cursor"]["row"].as_u64().unwrap_or(0) as usize;
        let at_cursor = screen["lines"][row].as_str().unwrap_or("");
        if at_cursor.trim().is_empty() {
            settling = None;
            continue;
        }
        let lines = screen["lines"].clone();
        match &settling {
            Some((before, since)) if *before == lines => {
                if seen.duration_since(*since) >= PROMPT_SETTLED {
                    // The plugins are read after the first output: asked for now,
                    // and waited for a little where there are some to read.
                    let mut late = Value::Null;
                    for _ in 0..if plugins > 0 { 150 } else { 1 } {
                        if let Some(status) = ask(lume, endpoint, &["status"]) {
                            late = status["marks"].clone();
                        }
                        if late["plugins_ms"].is_number() {
                            break;
                        }
                        std::thread::sleep(POLL);
                    }
                    // The page tells of its drawing a moment after the output reached
                    // it, so the marks read last hold every one.
                    let marks = if late.is_object() { &late } else { marks };
                    break Ok(Start {
                        marks: marks.clone(),
                        output_ms: *output_ms,
                        prompt_ms: since.duration_since(started).as_secs_f64() * 1000.0,
                        plugins_ms: late["plugins_ms"].as_f64(),
                        plugin_files_before_output: late["plugin_files_before_output"].as_f64(),
                    });
                }
            }
            _ => settling = Some((lines, seen)),
        }
    };

    // Closed, not killed, and with its sessions ended, whichever host runs them.
    if ask(lume, endpoint, &["command", "lume.app.quit-end-sessions"]).is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
    result
}

/// Installs `count` plugins into the folders of a start: the same small plugin under
/// as many names, which waits for a command and so never runs.
fn install_plugins(lume: &Path, dirs: &paths::PrivateDirs, count: usize) -> Result<()> {
    let package = paths::root().join("tests").join("plugins").join("hello");
    for number in 0..count {
        let mut command = Command::new(lume);
        dirs.apply(&mut command);
        let output = command
            .args(["plugin", "install"])
            .arg(&package)
            .args(["--as", &format!("bench.p{number:02}"), "--yes"])
            .stdin(Stdio::null())
            .output()
            .context("installing a plugin for the measurement")?;
        if !output.status.success() {
            bail!(
                "a plugin could not be installed for the measurement: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    Ok(())
}

/// A number from 0 up to `below`, the next of a fixed row of them: the same row every
/// run, so that a report can be made again.
fn next_below(state: &mut u64, below: usize) -> usize {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    usize::try_from(*state % below.max(1) as u64).unwrap_or(0)
}

/// The middle of `samples`, and the span the middle of 95 in a hundred redrawings of
/// them lies in: how sure the middle is.
fn middle_and_span(samples: &[f64]) -> (f64, f64, f64) {
    let middle = median(samples.to_vec());
    if samples.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut middles: Vec<f64> = (0..RESAMPLES)
        .map(|_| {
            let drawn: Vec<f64> = (0..samples.len())
                .map(|_| samples[next_below(&mut state, samples.len())])
                .collect();
            median(drawn)
        })
        .collect();
    middles.sort_by(f64::total_cmp);
    let at = |share: f64| {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "a place in a short list"
        )]
        let index = ((middles.len() - 1) as f64 * share).round() as usize;
        middles[index]
    };
    (middle, at(0.025), at(0.975))
}

/// What installed plugins that do not run cost a start: pairs of starts, one with none
/// installed and one with [`PLUGINS_INSTALLED`], taken in turn and in changing order so
/// that whatever else the machine does falls on both alike. The difference of each pair
/// is kept, and what is reported is their middle with the span it is sure within.
fn time_plugin_starts(profile: Profile) -> Result<Value> {
    let app = suites::app(profile)?;
    let lume = suites::lume(profile)?;
    let mut differences = Vec::new();
    let mut read = Vec::new();
    let mut before_output: f64 = 0.0;
    for pair in 0..PLUGIN_PAIRS {
        let mut output = [0.0, 0.0];
        // Which of the two goes first changes with every pair.
        let order = if pair % 2 == 0 { [0, 1] } else { [1, 0] };
        for which in order {
            let plugins = if which == 0 { 0 } else { PLUGINS_INSTALLED };
            let endpoint = format!("bench-{}-p{pair}-{which}", std::process::id());
            let start = time_one_start(&app, &lume, &endpoint, plugins)?;
            output[which] = start.output_ms;
            if which == 1 {
                read.extend(start.plugins_ms.map(|at| at - start.output_ms));
                before_output = before_output.max(start.plugin_files_before_output.unwrap_or(0.0));
            }
        }
        differences.push(output[1] - output[0]);
    }
    let (middle, low, high) = middle_and_span(&differences);
    println!(
        "bench:   with {PLUGINS_INSTALLED} plugins installed the first output comes {middle:+.0} ms \
         later (between {low:+.0} and {high:+.0}, {PLUGIN_PAIRS} pairs)"
    );
    Ok(json!({
        "installed": PLUGINS_INSTALLED,
        "pairs": PLUGIN_PAIRS,
        "differenceMs": { "p50": middle, "low": low, "high": high, "all": differences },
        // From the first output until the plugins had been read.
        "readAfterOutputMs": { "p50": median(read.clone()), "all": read },
        "filesBeforeOutput": before_output,
    }))
}

/// Installs one of the test plugins into the folders of a run.
fn install_fixture(lume: &Path, dirs: &paths::PrivateDirs, name: &str) -> Result<()> {
    let package = paths::root().join("tests").join("plugins").join(name);
    let mut command = Command::new(lume);
    dirs.apply(&mut command);
    let output = command
        .args(["plugin", "install"])
        .arg(&package)
        .arg("--yes")
        .stdin(Stdio::null())
        .output()
        .context("installing a plugin for the measurement")?;
    if !output.status.success() {
        bail!(
            "the plugin {name} could not be installed for the measurement: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

/// Waits until a test plugin's log has a line with `mark` after its first `from` bytes,
/// and gives what follows the mark on that line.
fn logged(log: &Path, from: usize, mark: &str, limit: Duration) -> Result<String> {
    let began = Instant::now();
    loop {
        let text = std::fs::read_to_string(log).unwrap_or_default();
        let fresh = text.get(from..).unwrap_or_default();
        if let Some(rest) = fresh
            .lines()
            .find_map(|line| line.split_once(mark).map(|(_, rest)| rest))
        {
            return Ok(rest.trim().to_owned());
        }
        if began.elapsed() > limit {
            bail!("{} never said {mark:?}: {fresh}", log.display());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn log_len(log: &Path) -> usize {
    std::fs::read_to_string(log).unwrap_or_default().len()
}

/// How many bytes the view reads to time a stream, and how often the window's page is
/// asked for an answer while a view is in its way.
const VIEW_STREAM_BYTES: u64 = 16 * 1024 * 1024;
const PAGE_ANSWERS: u64 = 1500;

/// What the plugins' views cost: how fast a view reads one of its plugin's streams
/// directly, beside the same bytes passed on by the plugin's code; how long a small
/// job takes through a program's input and output; and how long the window's page
/// takes to answer while a view floods its channel, sends as much as it may, or never
/// gives up its processor.
fn time_views(profile: Profile) -> Result<Value> {
    let app = suites::app(profile)?;
    let lume = suites::lume(profile)?;
    let endpoint = format!("bench-{}-views", std::process::id());
    let dirs = paths::PrivateDirs::fresh(&endpoint)?;
    for name in ["echo-view", "hostile"] {
        install_fixture(&lume, &dirs, name)?;
    }
    let mut command = Command::new(&app);
    dirs.apply(&mut command);
    let mut child = command
        .args(["--endpoint", &endpoint])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("starting the application")?;
    let logs = dirs.state_dir().join("plugins").join("logs");
    let echo = logs.join("test.echo-view.log");
    let hostile = logs.join("test.hostile.log");
    let run = |args: &[&str]| -> Result<()> {
        ask(&lume, &endpoint, args)
            .map(drop)
            .with_context(|| format!("the application refused {args:?}"))
    };

    let measured = (|| -> Result<Value> {
        // The plugins are read once the first output is out.
        let began = Instant::now();
        loop {
            let ready = ask(&lume, &endpoint, &["plugins"]).is_some_and(|answer| {
                let plugins = answer["plugins"].as_array().cloned().unwrap_or_default();
                plugins.len() == 2 && plugins.iter().all(|plugin| plugin["state"] == "ready")
            });
            if ready {
                break;
            }
            if began.elapsed() > START_TIMEOUT {
                bail!("the plugins for the measurement were not read in time");
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        let open = json!({
            "plugin": "test.echo-view",
            "view": "echo",
            "state": { "task": "bench", "bytes": VIEW_STREAM_BYTES },
        })
        .to_string();
        run(&["command", "lume.view.open", &open])?;
        let figures = logged(&echo, 0, "echo bench ", Duration::from_secs(180))?;
        let stream: Value = serde_json::from_str(&figures).context("reading the view's figures")?;

        // The window's answers with nothing in their way, and then with each kind of
        // view that gets in it, begun while the answers are being timed.
        let mut answers = serde_json::Map::new();
        for mode in ["quiet", "stream", "flood", "large", "spin"] {
            let at = log_len(&echo);
            let asking = {
                let args = json!({ "label": mode, "count": PAGE_ANSWERS }).to_string();
                let (lume, endpoint) = (lume.clone(), endpoint.clone());
                std::thread::spawn(move || {
                    ask(
                        &lume,
                        &endpoint,
                        &["command", "test.echo-view.page-answers", &args],
                    )
                })
            };
            if mode == "stream" {
                // A view reading a long stream as fast as it comes.
                let flow = json!({
                    "plugin": "test.echo-view",
                    "view": "echo",
                    "state": { "task": "flow", "bytes": 4 * VIEW_STREAM_BYTES },
                })
                .to_string();
                run(&["command", "lume.view.open", &flow])?;
            } else if mode != "quiet" {
                std::fs::write(
                    dirs.config_file(),
                    format!("[plugins.\"test.hostile\"]\nabuse = \"{mode}\"\n"),
                )?;
                std::thread::sleep(Duration::from_millis(600));
                let from = log_len(&hostile);
                run(&[
                    "command",
                    "lume.panel.show",
                    r#"{"panel":"test.hostile.abuse"}"#,
                ])?;
                logged(
                    &hostile,
                    from,
                    &format!("abuse {mode} started"),
                    Duration::from_secs(30),
                )?;
            }
            let _ = asking.join();
            let samples = logged(
                &echo,
                at,
                &format!("page-answers {mode} "),
                Duration::from_secs(120),
            )?;
            let figures: Value =
                serde_json::from_str(&samples).context("reading the window's answers")?;
            if mode == "stream" {
                logged(&echo, at, "echo flow ", Duration::from_secs(120))?;
            } else if mode != "quiet" {
                run(&[
                    "command",
                    "lume.panel.close",
                    r#"{"panel":"test.hostile.abuse"}"#,
                ])?;
            }
            println!(
                "bench:   the window answers in {} ms (p95 {}, at most {}) beside a view: {mode}",
                figures["p50"], figures["p95"], figures["max"],
            );
            answers.insert(mode.to_owned(), json!({ "pageAnswerMs": figures }));
        }
        println!(
            "bench:   a view reads a stream at {:.0} MiB/s itself, {:.0} MiB/s through its plugin's code",
            stream["streamMibPerSecond"].as_f64().unwrap_or(0.0),
            stream["relayMibPerSecond"].as_f64().unwrap_or(0.0),
        );
        let mut report = Value::Object(answers);
        for (key, value) in stream.as_object().into_iter().flatten() {
            report["stream"][key] = value.clone();
        }
        Ok(report)
    })();

    // Closed, not killed, and with its sessions ended, whichever host runs them.
    if ask(&lume, &endpoint, &["command", "lume.app.quit-end-sessions"]).is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
    measured
}

fn time_starts(profile: Profile) -> Result<Value> {
    let app = suites::app(profile)?;
    let lume = suites::lume(profile)?;
    let mut starts = Vec::new();
    for run in 0..=STARTS {
        let endpoint = format!("bench-{}-{run}", std::process::id());
        let start = time_one_start(&app, &lume, &endpoint, 0)?;
        // Every start is shown, so that a slow one can be seen to be slow in the window,
        // the page or the shell. The first is the cold one and is not counted.
        let counted = if run > 0 { "" } else { " (cold, not counted)" };
        println!("bench:   {}{counted}", start.line());
        if run > 0 {
            starts.push(start);
        }
    }
    let output: Vec<f64> = starts.iter().map(|start| start.output_ms).collect();
    let prompt: Vec<f64> = starts.iter().map(|start| start.prompt_ms.round()).collect();
    let mut report = json!({
        "starts": STARTS,
        "outputMs": { "p50": median(output.clone()), "all": output },
        "promptMs": { "p50": median(prompt.clone()), "all": prompt },
    });
    // The marks on the way to the first output, recorded beside it.
    for (name, key) in MARKS {
        if report.get(key).is_some() {
            continue;
        }
        let all: Vec<f64> = starts.iter().filter_map(|start| start.mark(name)).collect();
        if all.len() == starts.len() {
            report[key] = json!({ "p50": median(all.clone()), "all": all });
        }
    }
    Ok(report)
}

/// Puts what the two suites of the window's page reported into the one report: the
/// numbers of the one that times it, and the counts of renders of the one that checks
/// it under `chrome`. What either could not do joins the report's problems.
fn merge_page(report: &mut Value, measure: &Value, chrome: &Value) {
    for name in PAGE_NUMBERS {
        if let Some(numbers) = measure.get(name) {
            report[name] = numbers.clone();
        }
    }
    let mut checks = serde_json::Map::new();
    for name in PAGE_CHECKS {
        if let Some(count) = chrome.get(name) {
            checks.insert(name.to_owned(), count.clone());
        }
    }
    report["chrome"] = Value::Object(checks);

    let mut problems: Vec<Value> = report["problems"].as_array().cloned().unwrap_or_default();
    for (suite, from) in [("measure", measure), ("chrome", chrome)] {
        let said = from["error"].as_str().into_iter().chain(
            from["problems"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        );
        for problem in said {
            problems.push(Value::from(format!("{suite}: {problem}")));
        }
    }
    report["problems"] = Value::Array(problems);
}

/// How long a window put away while the application keeps running is given for its fresh
/// shells to start before it is launched again.
const READY_SETTLE: Duration = Duration::from_millis(2500);

/// Milliseconds since the Unix epoch, the clock the application gives the times of a
/// launch against it on.
fn unix_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |since| since.as_secs_f64() * 1000.0)
}

/// Times launches of an application that is kept running in the background: from the
/// start of a second process, which hands the launch over to the running one, until
/// the window that was kept ready has drawn its first frame on screen again.
fn time_ready_starts(profile: Profile) -> Result<Value> {
    let app = suites::app(profile)?;
    let lume = suites::lume(profile)?;
    let endpoint = format!("bench-{}-ready", std::process::id());
    let dirs = paths::PrivateDirs::fresh(&endpoint)?;
    std::fs::write(dirs.config_file(), "[app]\nkeep-running = true\n")
        .context("writing the settings of a kept application")?;
    let mut command = Command::new(&app);
    dirs.apply(&mut command);
    let mut child = command
        .args(["--endpoint", &endpoint])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("starting the application")?;
    let started = Instant::now();
    let shown_drawn = |status: &Value| status["marks"]["shown_drawn_unix_ms"].as_f64();
    let result = (|| -> Result<Vec<f64>> {
        // The first start, and its shell, are done with first.
        loop {
            if started.elapsed() > START_TIMEOUT {
                anyhow::bail!("the application did not start in time");
            }
            std::thread::sleep(POLL);
            if ask(&lume, &endpoint, &["status"])
                .is_some_and(|status| status["marks"]["drawn_ms"].is_number())
            {
                break;
            }
        }
        std::thread::sleep(READY_SETTLE);
        let mut times = Vec::new();
        for run in 0..=STARTS {
            ask(&lume, &endpoint, &["command", "lume.window.close"])
                .context("putting the window away")?;
            std::thread::sleep(READY_SETTLE);
            let launched = unix_ms();
            let mut second = Command::new(&app);
            dirs.apply(&mut second);
            let mut second = second
                .args(["--endpoint", &endpoint])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .context("launching the application again")?;
            let waiting = Instant::now();
            let drawn = loop {
                if waiting.elapsed() > START_TIMEOUT {
                    anyhow::bail!("the window kept ready was not shown in time");
                }
                std::thread::sleep(Duration::from_millis(5));
                if let Some(at) = ask(&lume, &endpoint, &["status"])
                    .as_ref()
                    .and_then(shown_drawn)
                    .filter(|at| *at >= launched)
                {
                    break at;
                }
            };
            let _ = second.wait();
            let ms = drawn - launched;
            let counted = if run > 0 { "" } else { " (first, not counted)" };
            println!("bench:   launched while kept ready: drawn {ms:.0} ms{counted}");
            if run > 0 {
                times.push(ms);
            }
        }
        Ok(times)
    })();
    if ask(&lume, &endpoint, &["command", "lume.app.quit-end-sessions"]).is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
    let times = result?;
    Ok(json!({ "drawnMs": { "p50": median(times.clone()), "all": times } }))
}

/// Times the starts of the release build alone and prints the middle of each mark, for
/// working on the start without the rest of the suite.
pub fn startup_only() -> Result<()> {
    println!("bench: timing launches of an application kept ready");
    let ready = time_ready_starts(Profile::Release)?;
    if let Some(p50) = ready["drawnMs"]["p50"].as_f64() {
        println!("bench:   {:<12} {p50:>6.0} ms", "ready");
    }
    println!("bench: timing {STARTS} starts of the application");
    let report = time_starts(Profile::Release)?;
    for (_, key) in MARKS {
        if let Some(p50) = report[key]["p50"].as_f64() {
            println!("bench:   {key:<12} {p50:>6.0} ms");
        }
    }
    if let Some(p50) = report["promptMs"]["p50"].as_f64() {
        println!("bench:   {:<12} {p50:>6.0} ms", "promptMs");
    }
    Ok(())
}

/// Times only what the plugins' views cost, and prints it.
pub fn views_only() -> Result<()> {
    println!("bench: timing what plugins' views read and what one in the way costs the window");
    let report = time_views(Profile::Release)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

/// Measures the release build and compares the numbers with the limits of `profile`.
pub fn run(profile: &str) -> Result<()> {
    let limits = load_limits(profile)?;

    println!("bench: running the benchmark suite in the webview (about two minutes)");
    let suite = suites::run_in_webview(Profile::Release, "bench", &[])?;
    let mut report = suite.json;
    println!("bench: timing the window's page with many panes, a drag and tab switches");
    let measure = suites::run_measure(Profile::Release)?;
    println!("bench: counting the renders terminal output causes in the window's page");
    let chrome = suites::run_chrome(Profile::Release)?;
    merge_page(&mut report, &measure.json, &chrome.json);
    println!("bench: timing {STARTS} starts of the application");
    report["startup"] = time_starts(Profile::Release)?;
    println!(
        "bench: timing {PLUGIN_PAIRS} pairs of starts, without plugins and with \
         {PLUGINS_INSTALLED} installed"
    );
    report["startup"]["plugins"] = time_plugin_starts(Profile::Release)?;
    println!("bench: timing what plugins' views read and what one in the way costs the window");
    report["views"] = time_views(Profile::Release)?;
    println!("bench: timing launches of an application kept running in the background");
    report["startup"]["ready"] = time_ready_starts(Profile::Release)?;

    let path = paths::root()
        .join("target")
        .join("bench")
        .join("report.json");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(&report)?)
        .with_context(|| format!("writing {}", path.display()))?;

    println!();
    let rows = judge(&report, &limits);
    print_table(&rows);
    println!("\nbench: the full report is in {}", path.display());

    let problems: Vec<&str> = report["problems"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    for problem in &problems {
        println!("bench: not measured: {problem}");
    }
    if report["hidden"] == true {
        println!("bench: the window was hidden for part of the run, which slows frames");
    }

    let outside = rows
        .iter()
        .filter(|row| matches!(row.verdict, Verdict::Outside | Verdict::Missing))
        .count();
    if outside > 0 {
        bail!("{outside} numbers are outside the limits of the {profile} profile");
    }
    if !suite.passed {
        bail!("the benchmark suite did not finish every measurement");
    }
    if !measure.passed {
        bail!("the window's page was not timed in every way it should be");
    }
    if !chrome.passed {
        bail!("the window's page does not hold to what it must");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(text: &str) -> Limits {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn the_middle_of_differences_is_said_with_the_span_it_is_sure_within() {
        // All alike: the span is the middle itself.
        assert_eq!(middle_and_span(&[4.0; 21]), (4.0, 4.0, 4.0));
        // Scattered round nothing: the span holds nothing, and is the same every time.
        let scattered: Vec<f64> = (0..21).map(|at| f64::from(at % 7) - 3.0).collect();
        let (middle, low, high) = middle_and_span(&scattered);
        assert!(low <= middle && middle <= high, "{low} {middle} {high}");
        assert!(low <= 0.0 && high >= 0.0, "{low} {high}");
        assert!(high - low <= 4.0, "{low} {high}");
        assert_eq!(middle_and_span(&scattered), (middle, low, high));
        // A real cost is seen: every pair some 40 ms slower.
        let slower: Vec<f64> = (0..21).map(|at| 40.0 + f64::from(at % 5)).collect();
        assert!(middle_and_span(&slower).1 >= 40.0);
        assert_eq!(middle_and_span(&[]), (0.0, 0.0, 0.0));
    }

    #[test]
    fn a_number_is_found_by_its_path() {
        let report = json!({ "redraw": { "webgpu": { "finish": { "p95": 1.9 } } }, "n": 3 });
        assert_eq!(number_at(&report, "redraw.webgpu.finish.p95"), Some(1.9));
        assert_eq!(number_at(&report, "n"), Some(3.0));
        assert_eq!(number_at(&report, "redraw.webgl2.finish.p95"), None);
        assert_eq!(number_at(&report, "redraw.webgpu"), None);
    }

    #[test]
    fn numbers_are_held_to_their_limits() {
        let report = json!({ "a": 5.0, "b": 5.0, "c": 5.0, "d": 5.0 });
        let limits = limits(
            r#"
            a = { max = 5.0, unit = "ms" }
            b = { max = 4.9, unit = "ms" }
            c = { min = 5.1, unit = "MiB/s" }
            d = { min = 1.0, max = 9.0, unit = "ms" }
            "#,
        );
        let verdicts: Vec<Verdict> = judge(&report, &limits)
            .iter()
            .map(|row| row.verdict)
            .collect();
        assert_eq!(
            verdicts,
            [
                Verdict::Within,
                Verdict::Outside,
                Verdict::Outside,
                Verdict::Within
            ]
        );
    }

    #[test]
    fn a_number_that_is_missing_fails_unless_it_may_be() {
        let report = json!({});
        let limits = limits(
            r#"
            needed = { max = 1.0, unit = "ms" }
            spare = { max = 1.0, unit = "ms", optional = true }
            "#,
        );
        let verdicts: Vec<Verdict> = judge(&report, &limits)
            .iter()
            .map(|row| row.verdict)
            .collect();
        assert_eq!(verdicts, [Verdict::Missing, Verdict::NotMeasured]);
    }

    #[test]
    fn the_budgets_file_has_the_profiles_the_tool_is_run_with() {
        for profile in ["reference", "ci"] {
            let limits = load_limits(profile).unwrap();
            assert!(!limits.is_empty(), "{profile}");
            for (path, limit) in &limits {
                assert!(
                    limit.max.is_some() || limit.min.is_some(),
                    "{profile}: {path} has no limit"
                );
            }
        }
        assert!(load_limits("nonesuch").is_err());
    }

    #[test]
    fn the_numbers_of_the_windows_page_join_the_one_report() {
        let mut report = json!({ "transport": { "mibPerSecond": 60.0 }, "problems": ["old"] });
        let measure = json!({
            "ok": false,
            "panes16": { "webgl2": { "framesPerSecond": 144.0 } },
            "drag": { "framesPerSecond": 143.9 },
            "tabSwitch": {},
            "problems": ["the divider was dragged and went 0 pixels"],
        });
        let chrome = json!({
            "ok": true,
            "outputCommits": 0,
            "outputBytes": 4096,
            "switchCommits": 1,
            "renderers": { "firstTab": 2 },
            "problems": [],
        });
        merge_page(&mut report, &measure, &chrome);

        assert_eq!(
            number_at(&report, "panes16.webgl2.framesPerSecond"),
            Some(144.0)
        );
        assert_eq!(number_at(&report, "drag.framesPerSecond"), Some(143.9));
        assert_eq!(number_at(&report, "chrome.outputCommits"), Some(0.0));
        assert_eq!(number_at(&report, "transport.mibPerSecond"), Some(60.0));
        // A tier or a switch that was not timed has no number, and nothing is made up.
        assert_eq!(number_at(&report, "tabSwitch.webgl2Ms"), None);
        assert!(report.get("search").is_none());
        assert!(report["chrome"].get("renderers").is_none());
        assert_eq!(
            report["problems"],
            json!(["old", "measure: the divider was dragged and went 0 pixels"])
        );
    }

    #[test]
    fn a_suite_that_stopped_with_an_error_says_so_in_the_report() {
        let mut report = json!({});
        merge_page(&mut report, &json!({ "error": "no window" }), &json!({}));
        assert_eq!(report["problems"], json!(["measure: no window"]));
        assert_eq!(report["chrome"], json!({}));
    }

    #[test]
    fn the_median_is_the_middle_sample() {
        assert_eq!(median(vec![3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(vec![]), 0.0);
    }
}
