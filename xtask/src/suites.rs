//! Running the application's test suites: the ones that run inside its webview, and the
//! ones that drive it from outside.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::{paths, run};

/// Which build of the application to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Debug,
    Release,
}

impl Profile {
    fn dir(self) -> &'static str {
        match self {
            Profile::Debug => "debug",
            Profile::Release => "release",
        }
    }
}

fn binary(profile: Profile, name: &str) -> Result<PathBuf> {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    let path = paths::build_dir().join(profile.dir()).join(file);
    if !path.is_file() {
        bail!("{} has not been built", path.display());
    }
    Ok(path)
}

/// The application.
pub fn app(profile: Profile) -> Result<PathBuf> {
    binary(profile, "lume-gui")
}

/// The console command that drives it.
pub fn lume(profile: Profile) -> Result<PathBuf> {
    binary(profile, "lume")
}

/// Where the suites leave their reports and the files they produce.
pub fn out_dir() -> PathBuf {
    paths::root().join("target").join("harness")
}

/// What a suite reported, and whether the application said it passed.
pub struct Report {
    pub passed: bool,
    pub json: Value,
}

/// Runs one suite inside the application's webview and reads its report.
///
/// The report is read from a file: a windowed program has no standard output on every
/// platform. `extra` is passed on to the application after the suite's name.
pub fn run_in_webview(profile: Profile, suite: &str, extra: &[&str]) -> Result<Report> {
    let dir = out_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let report = dir.join(format!("{suite}.json"));
    // A report left by an earlier run must not be taken for this one's.
    let _ = std::fs::remove_file(&report);

    let status = Command::new(app(profile)?)
        .args(["--harness", suite, "--report"])
        .arg(&report)
        .args(extra)
        .current_dir(paths::root())
        .status()
        .with_context(|| format!("starting the {suite} suite"))?;
    let text = std::fs::read_to_string(&report).with_context(|| {
        format!("the {suite} suite left no report (the application ended with {status})")
    })?;
    let json = serde_json::from_str(&text)
        .with_context(|| format!("{} is not a report", report.display()))?;
    Ok(Report {
        passed: status.success(),
        json,
    })
}

/// The lines of a report's `error` and `problems`, for showing when a suite fails.
fn complaints(report: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(error) = report["error"].as_str() {
        lines.push(error.to_owned());
    }
    for problem in report["problems"].as_array().into_iter().flatten() {
        lines.push(problem.as_str().unwrap_or_default().to_owned());
    }
    lines
}

fn corpus(profile: Profile) -> Result<()> {
    let report = run_in_webview(profile, "corpus", &[])?;
    let empty = Vec::new();
    let recordings = report.json["recordings"].as_array().unwrap_or(&empty);
    let failed: Vec<String> = recordings
        .iter()
        .filter(|recording| recording["ok"] != true)
        .map(|recording| {
            format!(
                "  {}: {}",
                recording["name"].as_str().unwrap_or("?"),
                recording["problem"].as_str().unwrap_or("differs")
            )
        })
        .collect();
    if !report.passed || !failed.is_empty() {
        bail!(
            "the corpus differs inside the webview:\n{}\n{}",
            failed.join("\n"),
            complaints(&report.json).join("\n")
        );
    }
    println!(
        "corpus: {} recordings replay inside the webview as they do natively",
        recordings.len()
    );
    Ok(())
}

fn snapshots(profile: Profile) -> Result<()> {
    let out = out_dir().join("snapshots");
    let out = out.to_string_lossy();
    let report = run_in_webview(
        profile,
        "snapshots",
        &["--references", "tests/snapshots", "--out", &out],
    )?;
    let empty = Vec::new();
    let frames = report.json["frames"].as_array().unwrap_or(&empty);
    let failed: Vec<String> = frames
        .iter()
        .flat_map(|frame| {
            let name = frame["name"].as_str().unwrap_or("?").to_owned();
            frame["problems"]
                .as_array()
                .into_iter()
                .flatten()
                .map(move |problem| format!("  {name}: {}", problem.as_str().unwrap_or("?")))
        })
        .collect();
    if !report.passed || !failed.is_empty() {
        bail!(
            "the renderer's pictures differ (what was drawn is in {out}/failed):\n{}\n{}",
            failed.join("\n"),
            complaints(&report.json).join("\n")
        );
    }
    let tiers: Vec<&str> = report.json["tiers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    println!(
        "snapshots: {} frames agree on {}",
        frames.len(),
        tiers.join(", ")
    );
    let unreferenced = report.json["unreferenced"].as_u64().unwrap_or(0);
    if unreferenced > 0 {
        println!(
            "snapshots: {unreferenced} frames have no reference for this webview engine; \
             what was drawn is in {out}/references"
        );
    }
    Ok(())
}

/// Runs the suite of the window's own page: what it must hold to, and its scenes
/// compared with what they were made of when they were last found right.
pub fn run_chrome(profile: Profile) -> Result<Report> {
    let out = out_dir().join("chrome");
    let out = out.to_string_lossy();
    run_in_webview(
        profile,
        "chrome",
        &["--references", "tests/snapshots", "--out", &out],
    )
}

/// Runs the suite that times the window's own page under load.
pub fn run_measure(profile: Profile) -> Result<Report> {
    run_in_webview(profile, "measure", &[])
}

fn chrome(profile: Profile) -> Result<()> {
    let report = run_chrome(profile)?;
    if !report.passed {
        bail!(
            "the window's page does not hold to what it must:
{}",
            complaints(&report.json).join(
                "
"
            )
        );
    }
    println!(
        "chrome: {} renders for {} KiB of output in three panes; a tab off the screen \
         keeps {} of {} renderers",
        report.json["outputCommits"],
        report.json["outputBytes"].as_u64().unwrap_or(0) / 1024,
        report.json["renderers"]["otherTab"],
        report.json["renderers"]["firstTab"],
    );
    let scenes = report.json["scenes"].as_array().map_or(0, Vec::len);
    println!("chrome: {scenes} scenes are made of what they were, in both looks");
    Ok(())
}

/// Runs everything that needs the application's window: the suites inside its
/// webview, then the tests that drive it from outside with real shells.
pub fn end_to_end() -> Result<()> {
    corpus(Profile::Debug)?;
    snapshots(Profile::Debug)?;
    chrome(Profile::Debug)?;
    // Stand in for a Rex server, which no build machine has, and for the SSH client.
    run::cargo(&[
        "build",
        "-p",
        "lume-rex",
        "--example",
        "pretend_server",
        "--example",
        "stand_in_ssh",
    ])?;
    run::cargo(&[
        "test",
        "-p",
        "lume-cli",
        "--test",
        "e2e",
        "--test",
        "hostile_e2e",
        // The second is run whatever came of the first.
        "--no-fail-fast",
        "--",
        "--ignored",
        "--test-threads=1",
    ])
}
