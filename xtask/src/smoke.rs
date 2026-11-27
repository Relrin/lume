//! `cargo xtask smoke`: start the application as a user would find it, type into it
//! through the window, and see what was typed arrive in the terminal.
//!
//! The typing goes through WebDriver, so it travels the whole way a key travels: the
//! window, the webview's own event handling, the page, the session. What appears is read
//! through `lume ctl`, since a terminal is drawn on a canvas that WebDriver cannot read.
//!
//! WebDriver for the application is provided by `tauri-driver`, which must be on the
//! search path together with the webview's own driver. It exists for Windows and Linux.
//!
//! While the window is there it is also photographed, with two panes, once in the dark
//! look and once in the light. The pictures are kept for people to look at; nothing
//! compares them, since two machines never draw a window quite alike.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use crate::paths::{self, PrivateDirs};
use crate::suites::{self, Profile};

/// Where `tauri-driver` listens.
const DRIVER: &str = "http://127.0.0.1:4444";

/// How long the driver and the application may each take to come up.
const START_TIMEOUT: Duration = Duration::from_secs(60);

/// What is typed. It is the echo of the typing itself that is looked for.
const TYPED: &str = "echo lume-smoke-ok";

/// The WebDriver code point of the Enter key.
const ENTER: &str = "\u{e007}";

/// The looks the window is photographed in: what each is called, and its theme.
const LOOKS: [(&str, &str); 2] = [("dark", "lume-dark"), ("light", "lume-light")];

/// How long the window may take to change its look once the settings say so.
const LOOK_TIMEOUT: Duration = Duration::from_secs(15);

/// How long the window is left to draw its terminals in a new look before the picture.
const LOOK_SETTLE: Duration = Duration::from_millis(500);

/// The driver's process, ended when this is dropped.
struct Driver(Child);

impl Drop for Driver {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Where what the smoke test leaves is kept: the pictures, and what the driver and the
/// application said, which are all there is to go by when a session does not start.
fn out_dir() -> PathBuf {
    paths::root().join("target").join("smoke")
}

/// What the driver writes, and what the application it starts logs.
fn driver_log() -> PathBuf {
    out_dir().join("tauri-driver.log")
}

fn app_log() -> PathBuf {
    out_dir().join("lume-gui.log")
}

/// The last lines of a file, or a word that there is none.
fn tail(path: &Path) -> String {
    const LINES: usize = 30;
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let lines: Vec<&str> = text.lines().collect();
            lines[lines.len().saturating_sub(LINES)..].join("\n")
        }
        Err(_) => format!("({} is not there)", path.display()),
    }
}

fn start_driver() -> Result<(Driver, PrivateDirs)> {
    let mut command = Command::new("tauri-driver");
    // The application is started by the driver and takes the driver's environment: it
    // is given folders of its own, so the test touches nothing of the user's, and a log.
    let dirs = PrivateDirs::fresh("smoke")?;
    dirs.apply(&mut command);
    std::fs::create_dir_all(out_dir())
        .with_context(|| format!("creating {}", out_dir().display()))?;
    let _ = std::fs::remove_file(app_log());
    command
        .env("LUME_LOG", "debug")
        .env("LUME_LOG_FILE", app_log());
    // On Windows the webview's driver is found through the variable the runner images
    // set, when it is not on the search path.
    if let Some(dir) = std::env::var_os("EdgeWebDriver") {
        command
            .arg("--native-driver")
            .arg(Path::new(&dir).join("msedgedriver.exe"));
    }
    let said = std::fs::File::create(driver_log())
        .with_context(|| format!("creating {}", driver_log().display()))?;
    let child = command
        .stdout(said.try_clone()?)
        .stderr(said)
        .spawn()
        .context("starting tauri-driver (install it with `cargo install tauri-driver`)")?;
    let driver = Driver(child);

    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if ureq::get(&format!("{DRIVER}/status")).call().is_ok() {
            return Ok((driver, dirs));
        }
        if Instant::now() > deadline {
            bail!("tauri-driver did not start listening");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// One WebDriver command, and the `value` of its answer: with a JSON body it is
/// posted, without one it is asked for. A command the driver refuses fails with the
/// driver's own words, which say what went wrong.
fn command(path: &str, body: Option<Value>) -> Result<Value> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let address = format!("{DRIVER}{path}");
    let sent = match body {
        Some(body) => agent
            .post(&address)
            .header("Content-Type", "application/json")
            .send(body.to_string()),
        None => agent.get(&address).call(),
    };
    let mut response = sent.with_context(|| format!("sending {path} to WebDriver"))?;
    let status = response.status();
    // A picture of a window is larger than the answers a body is read up to by default.
    let text = response
        .body_mut()
        .with_config()
        .limit(ANSWER_MAX_BYTES)
        .read_to_string()?;
    let answer: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
    if !status.is_success() {
        let said = answer["value"]["message"]
            .as_str()
            .map_or_else(|| answer.to_string(), str::to_owned);
        bail!("WebDriver refused {path} ({status}): {said}");
    }
    Ok(answer["value"].clone())
}

fn post(path: &str, body: Value) -> Result<Value> {
    command(path, Some(body))
}

/// The largest answer read from the driver: a picture of a large window, as text.
const ANSWER_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// The bytes that text in base64 stands for, or nothing if it is not base64.
fn base64(text: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3);
    let mut held = 0u32;
    let mut bits = 0;
    for symbol in text.bytes() {
        let value = match symbol {
            b'A'..=b'Z' => symbol - b'A',
            b'a'..=b'z' => symbol - b'a' + 26,
            b'0'..=b'9' => symbol - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            // What fills the last group out, and the line ends some encoders add.
            b'=' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        held = (held << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push(u8::try_from((held >> bits) & 0xff).ok()?);
        }
    }
    Some(bytes)
}

/// A picture of the window as the driver takes it: the bytes of a PNG file.
fn screenshot(session: &str) -> Result<Vec<u8>> {
    let taken = command(&format!("/session/{session}/screenshot"), None)?;
    let png = taken
        .as_str()
        .and_then(base64)
        .ok_or_else(|| anyhow!("WebDriver's picture of the window is not a picture"))?;
    if !png.starts_with(b"\x89PNG") {
        bail!("WebDriver's picture of the window is not a PNG file");
    }
    Ok(png)
}

/// Waits until the page says it is drawn in a look. A driver that will not ask the
/// page is not held against the run: the look is given a moment instead.
fn wait_for_look(session: &str, look: &str) {
    let deadline = Instant::now() + LOOK_TIMEOUT;
    loop {
        let asked = post(
            &format!("/session/{session}/execute/sync"),
            json!({ "script": "return document.documentElement.dataset.scheme", "args": [] }),
        );
        match asked {
            Ok(shown) if shown == look => break,
            Ok(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            _ => break,
        }
    }
    std::thread::sleep(LOOK_SETTLE);
}

/// Photographs the window with two panes in each look, and says where the pictures are.
fn photograph(
    lume: &Path,
    endpoint: &str,
    session: &str,
    dirs: &PrivateDirs,
    shell: &[&str],
) -> Result<Vec<PathBuf>> {
    // A second pane beside the first, so the picture shows what is between two.
    let mut beside = vec!["spawn", "--split", "right"];
    beside.extend(shell.iter().skip(1));
    ctl(lume, endpoint, &beside)?;

    let out = out_dir();
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    let mut pictures = Vec::new();
    for (look, theme) in LOOKS {
        // The settings file is the test's own, and the window follows it as it changes.
        let settings = dirs.config_file();
        std::fs::write(&settings, format!("[appearance]\ntheme = \"{theme}\"\n"))
            .with_context(|| format!("writing {}", settings.display()))?;
        wait_for_look(session, look);
        let picture = out.join(format!("window-{look}.png"));
        std::fs::write(&picture, screenshot(session)?)
            .with_context(|| format!("writing {}", picture.display()))?;
        pictures.push(picture);
    }
    Ok(pictures)
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
    let (_driver, dirs) = start_driver()?;

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
    )
    .with_context(|| {
        format!(
            "--- what the driver said\n{}\n--- what the application logged\n{}",
            tail(&driver_log()),
            tail(&app_log())
        )
    })?;
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
        // The pictures are for people. A driver that takes none is said, and the test
        // of typing stands on its own.
        match photograph(&lume, &endpoint, &id, &dirs, shell) {
            Ok(pictures) => {
                for picture in pictures {
                    println!("smoke: a picture of the window is in {}", picture.display());
                }
            }
            Err(error) => println!("smoke: no picture of the window was kept: {error:#}"),
        }
        Ok(())
    })();

    // Closed through its own endpoint first, with its sessions ended: with a session
    // host of its own, closing alone would leave its shells running for the next start,
    // and ending the WebDriver session would end the process and leave them behind.
    let _ = ctl(&lume, &endpoint, &["command", "lume.app.quit-end-sessions"]);
    let _ = ureq::delete(&format!("{DRIVER}/session/{id}")).call();
    typed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_is_read_with_or_without_what_fills_it_out() {
        assert_eq!(base64("").unwrap(), b"");
        assert_eq!(base64("bHVtZQ==").unwrap(), b"lume");
        assert_eq!(base64("bHVtZQ").unwrap(), b"lume");
        assert_eq!(base64("bHVtZSE=").unwrap(), b"lume!");
        assert_eq!(base64("iVBORw0KGgo=").unwrap(), b"\x89PNG\r\n\x1a\n");
        assert_eq!(base64("//79").unwrap(), [0xff, 0xfe, 0xfd]);
        // Broken into lines, as some encoders leave it.
        assert_eq!(base64("bHVt\r\nZQ==").unwrap(), b"lume");
    }

    #[test]
    fn what_is_not_base64_is_nothing() {
        assert_eq!(base64("bHVt*Q=="), None);
        assert_eq!(base64("not base64"), None);
    }

    #[test]
    fn typing_is_a_press_and_a_release_for_each_key() {
        let actions = typing("ab");
        let keys = actions["actions"][0]["actions"].as_array().unwrap();
        assert_eq!(keys.len(), 4);
        assert_eq!(keys[0], json!({ "type": "keyDown", "value": "a" }));
        assert_eq!(keys[3], json!({ "type": "keyUp", "value": "b" }));
    }
}
