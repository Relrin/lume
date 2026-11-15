//! `cargo xtask bench`: measure the application and hold the numbers to their budgets.
//!
//! Most of the measuring is done by the benchmark suite inside the application's
//! webview. How long the application takes to start is measured here, from outside, by
//! starting it and asking it how far it has come. The numbers are then compared with
//! the limits in `tests/bench/budgets.toml`, and a number outside its limit fails the run.

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

/// How long one start may take before the measurement gives up.
const START_TIMEOUT: Duration = Duration::from_secs(60);

/// How often the application is asked how far it has come.
const POLL: Duration = Duration::from_millis(20);

/// How long the screen must stay as it is, with text at the cursor, to count as a prompt.
const PROMPT_SETTLED: Duration = Duration::from_millis(200);

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
const MARKS: [(&str, &str); 5] = [
    ("window_ms", "windowMs"),
    ("page_ms", "pageMs"),
    ("carrier_ms", "carrierMs"),
    ("attached_ms", "attachedMs"),
    ("output_ms", "outputMs"),
];

/// How far one start had come when its shell's first output reached the page, and when
/// the screen showed a prompt. All in milliseconds from the start of the process.
struct Start {
    /// The application's own marks, as its status reports them.
    marks: Value,
    output_ms: f64,
    prompt_ms: f64,
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
fn time_one_start(app: &Path, lume: &Path, endpoint: &str) -> Result<Start> {
    let started = Instant::now();
    let mut child = Command::new(app)
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
                    break Ok(Start {
                        marks: marks.clone(),
                        output_ms: *output_ms,
                        prompt_ms: since.duration_since(started).as_secs_f64() * 1000.0,
                    });
                }
            }
            _ => settling = Some((lines, seen)),
        }
    };

    // Closed, not killed: its shell is ended with it.
    if ask(lume, endpoint, &["quit"]).is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
    result
}

fn time_starts(profile: Profile) -> Result<Value> {
    let app = suites::app(profile)?;
    let lume = suites::lume(profile)?;
    let mut starts = Vec::new();
    for run in 0..=STARTS {
        let endpoint = format!("bench-{}-{run}", std::process::id());
        let start = time_one_start(&app, &lume, &endpoint)?;
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

/// Measures the release build and compares the numbers with the limits of `profile`.
pub fn run(profile: &str) -> Result<()> {
    let limits = load_limits(profile)?;

    println!("bench: running the benchmark suite in the webview (about two minutes)");
    let suite = suites::run_in_webview(Profile::Release, "bench", &[])?;
    let mut report = suite.json;
    println!("bench: timing {STARTS} starts of the application");
    report["startup"] = time_starts(Profile::Release)?;

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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(text: &str) -> Limits {
        toml::from_str(text).unwrap()
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
    fn the_median_is_the_middle_sample() {
        assert_eq!(median(vec![3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(vec![]), 0.0);
    }
}
