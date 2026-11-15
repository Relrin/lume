//! `cargo xtask smoke`: start the application as a user would find it, type into it
//! through the window, and see what was typed arrive in the terminal.
//!
//! The typing goes through WebDriver, so it travels the whole way a key travels: the
//! window, the webview's own event handling, the page, the session. What appears is read
//! through `lume ctl`, since a terminal is drawn on a canvas that WebDriver cannot read.
//!
//! WebDriver for the application is provided by `tauri-driver`, which must be on the
//! search path together with the webview's own driver. It exists for Windows and Linux.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use crate::suites::{self, Profile};

/// Where `tauri-driver` listens.
const DRIVER: &str = "http://127.0.0.1:4444";

/// How long the driver and the application may each take to come up.
const START_TIMEOUT: Duration = Duration::from_secs(60);

/// What is typed. It is the echo of the typing itself that is looked for.
const TYPED: &str = "echo lume-smoke-ok";

/// The WebDriver code point of the Enter key.
const ENTER: &str = "\u{e007}";

/// The driver's process, ended when this is dropped.
struct Driver(Child);

impl Drop for Driver {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_driver() -> Result<Driver> {
    let mut command = Command::new("tauri-driver");
    // On Windows the webview's driver is found through the variable the runner images
    // set, when it is not on the search path.
    if let Some(dir) = std::env::var_os("EdgeWebDriver") {
        command
            .arg("--native-driver")
            .arg(Path::new(&dir).join("msedgedriver.exe"));
    }
    let child = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("starting tauri-driver (install it with `cargo install tauri-driver`)")?;
    let driver = Driver(child);

    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if ureq::get(&format!("{DRIVER}/status")).call().is_ok() {
            return Ok(driver);
        }
        if Instant::now() > deadline {
            bail!("tauri-driver did not start listening");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// One WebDriver command with a JSON body, and the `value` of its answer. A command the
/// driver refuses fails with the driver's own words, which say what went wrong.
fn post(path: &str, body: Value) -> Result<Value> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let mut response = agent
        .post(&format!("{DRIVER}{path}"))
        .header("Content-Type", "application/json")
        .send(body.to_string())
        .with_context(|| format!("sending {path} to WebDriver"))?;
    let status = response.status();
    let text = response.body_mut().read_to_string()?;
    let answer: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
    if !status.is_success() {
        let said = answer["value"]["message"]
            .as_str()
            .map_or_else(|| answer.to_string(), str::to_owned);
        bail!("WebDriver refused {path} ({status}): {said}");
    }
    Ok(answer["value"].clone())
}

/// The key presses and releases that type `text`.
fn typing(text: &str) -> Value {
    let keys: Vec<Value> = text
        .chars()
        .flat_map(|key| {
            let key = key.to_string();
            [
                json!({ "type": "keyDown", "value": key }),
                json!({ "type": "keyUp", "value": key }),
            ]
        })
        .collect();
    json!({ "actions": [{ "type": "key", "id": "keyboard", "actions": keys }] })
}

fn ctl(lume: &Path, endpoint: &str, args: &[&str]) -> Result<Value> {
    let output = Command::new(lume)
        .args(["ctl", "--endpoint", endpoint, "--timeout", "30"])
        .args(args)
        .output()
        .context("running lume")?;
    let answer: Value = serde_json::from_slice(&output.stdout)
        .with_context(|| format!("lume ctl {} gave no answer", args.join(" ")))?;
    if !output.status.success() {
        bail!("lume ctl {} failed: {answer}", args.join(" "));
    }
    Ok(answer)
}

/// Starts the application under WebDriver, types a command and waits to see it.
pub fn run() -> Result<()> {
    let app = suites::app(Profile::Debug)?;
    let lume = suites::lume(Profile::Debug)?;
    let endpoint = format!("smoke-{}", std::process::id());
    let _driver = start_driver()?;

    let session = post(
        "/session",
        json!({
            "capabilities": {
                "alwaysMatch": {
                    "browserName": "wry",
                    "tauri:options": {
                        "application": app,
                        // One argument: the webview's driver passes each on as a
                        // switch of its own, so a separate value would become one.
                        "args": [format!("--endpoint={endpoint}")],
                    },
                },
            },
        }),
    )?;
    let id = session["sessionId"]
        .as_str()
        .ok_or_else(|| anyhow!("WebDriver started no session: {session}"))?
        .to_owned();

    let typed = (|| -> Result<()> {
        // A shell of the test's own, started without the history a user's shell keeps of
        // what is typed into it. It takes the window's pane, as any new session does.
        let shell: &[&str] = if cfg!(windows) {
            &["spawn", "--", "cmd.exe", "/d"]
        } else {
            &["spawn", "--env", "HISTFILE=/dev/null", "--", "sh"]
        };
        ctl(&lume, &endpoint, shell)?;
        // The shell is up when its first output is on the screen.
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            let screen = ctl(&lume, &endpoint, &["capture"])?;
            let shown = screen["lines"]
                .as_array()
                .is_some_and(|lines| lines.iter().any(|line| line != ""));
            if shown {
                break;
            }
            if Instant::now() > deadline {
                bail!("the shell printed nothing");
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        post(
            &format!("/session/{id}/actions"),
            typing(&format!("{TYPED}{ENTER}")),
        )?;
        let found = ctl(&lume, &endpoint, &["wait", "--text", TYPED])?;
        println!(
            "smoke: typed through the window, and the terminal shows {:?}",
            found["line"].as_str().unwrap_or_default()
        );
        Ok(())
    })();

    // Closed through its own endpoint first, so that its shell is ended with it; ending
    // the WebDriver session would end the process and leave the shell behind.
    let _ = ctl(&lume, &endpoint, &["quit"]);
    let _ = ureq::delete(&format!("{DRIVER}/session/{id}")).call();
    typed
}
