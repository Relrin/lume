//! The application driven from outside, through `lume ctl`, with real shells in it.
//!
//! Each test starts `lume-gui` with an endpoint of its own, drives it with the `lume`
//! built beside it, and closes it by asking it to quit. Nothing here reaches into the
//! application: what a test can see is what a script could.
//!
//! The tests need a display and the application built with its page inside it, so they
//! are ignored by default. `cargo xtask e2e` builds what they need and runs them, one at
//! a time.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

/// How long the application may take to start, and to close.
const START: Duration = Duration::from_secs(60);

/// How long `lume ctl` waits for text or an exit, in seconds, as it takes it.
const WAIT: &str = "60";

/// What the shells are told through their environment, so that what a command prints
/// differs from the command as typed: the screen then shows the output, not the echo.
const MARK: &str = "k7q";

const APPLICATION: &str = if cfg!(windows) {
    "lume-gui.exe"
} else {
    "lume-gui"
};

static NEXT: AtomicU32 = AtomicU32::new(1);

/// The answer of one `lume ctl` command.
struct Answer {
    status: i32,
    json: Value,
}

impl Answer {
    fn ok(self, what: &str) -> Value {
        assert_eq!(self.status, 0, "{what} failed: {}", self.json);
        self.json
    }
}

/// A running application with an endpoint of its own.
struct App {
    child: Option<Child>,
    endpoint: String,
    log: PathBuf,
}

fn lume() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_lume"))
}

fn application() -> PathBuf {
    let beside = lume().with_file_name(APPLICATION);
    assert!(
        beside.is_file(),
        "{} is not there. Run `cargo xtask build` first, or run these tests with \
         `cargo xtask e2e`.",
        beside.display()
    );
    beside
}

impl App {
    fn start() -> Self {
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let endpoint = format!("e2e-{}-{number}", std::process::id());
        let log = std::env::temp_dir().join(format!("lume-{endpoint}.log"));
        let _ = std::fs::remove_file(&log);
        let child = Command::new(application())
            .args(["--endpoint", &endpoint])
            .env("LUME_LOG", "trace")
            .env("LUME_LOG_FILE", &log)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the application starts");
        let app = Self {
            child: Some(child),
            endpoint,
            log,
        };

        // The endpoint is served before the window is up; the first answer is the sign
        // that the application is running.
        let deadline = Instant::now() + START;
        loop {
            let status = app.ctl(&["status"]);
            if status.status == 0 {
                assert_eq!(
                    status.json["bundled_ui"], true,
                    "the application was built without its page. Run `cargo xtask build`."
                );
                return app;
            }
            assert!(
                Instant::now() < deadline,
                "the application did not start: {}",
                status.json
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn ctl(&self, args: &[&str]) -> Answer {
        let output = Command::new(lume())
            .args(["ctl", "--endpoint", &self.endpoint, "--timeout", WAIT])
            .args(args)
            .output()
            .expect("lume runs");
        let text = String::from_utf8_lossy(&output.stdout);
        Answer {
            status: output.status.code().unwrap_or(-1),
            json: serde_json::from_str(text.trim()).unwrap_or(Value::String(text.into_owned())),
        }
    }

    /// Types a line into the session shown.
    fn type_line(&self, line: &str) {
        self.ctl(&["send", "--enter", "--", line]).ok("send");
    }

    /// Waits until `text` is on the screen of the session shown.
    fn wait_text(&self, text: &str) -> Value {
        self.ctl(&["wait", "--text", text])
            .ok(&format!("waiting for {text:?}"))
    }

    /// The ids of the operating-system processes behind the sessions.
    fn session_pids(&self) -> Vec<u32> {
        let listed = self.ctl(&["list"]).ok("list");
        listed["sessions"]
            .as_array()
            .expect("a list of sessions")
            .iter()
            .filter_map(|session| session["info"]["pid"].as_u64())
            .map(|pid| pid as u32)
            .collect()
    }

    /// Asks the application to close and waits until its process has ended.
    fn quit(mut self) -> PathBuf {
        self.ctl(&["quit"]).ok("quit");
        let mut child = self.child.take().expect("the application is running");
        let deadline = Instant::now() + START;
        loop {
            if let Some(status) = child.try_wait().expect("the application can be waited for") {
                assert!(status.success(), "the application ended with {status}");
                break;
            }
            assert!(Instant::now() < deadline, "the application did not close");
            std::thread::sleep(Duration::from_millis(50));
        }
        self.log.clone()
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // Reached with the application still running only when a test failed. It is asked
        // to close, so that its shells are ended with it, and killed only if it will not.
        let Some(mut child) = self.child.take() else {
            return;
        };
        let _ = self.ctl(&["quit"]);
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Whether a process with this id is running.
fn process_alive(pid: u32) -> bool {
    if cfg!(windows) {
        let listed = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .expect("tasklist runs");
        String::from_utf8_lossy(&listed.stdout).contains(&format!("\"{pid}\""))
    } else {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

/// Whether a program of this name is on the search path.
fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{program}.exe"), program.to_owned()]
    } else {
        vec![program.to_owned()]
    };
    std::env::split_paths(&path).any(|dir| names.iter().any(|name| dir.join(name).is_file()))
}

/// A shell and how to say a few things in it.
struct Shell {
    program: &'static str,
    /// Arguments that start it without the user's own configuration, and without the
    /// history it would keep of what these tests type.
    args: &'static [&'static str],
    /// Variables it is started with, for the same reason.
    env: &'static [&'static str],
    /// Prints `lume-k7q-<word>`, with the middle taken from the environment.
    say: fn(&str) -> String,
    /// Prints the terminal's size, and what the screen shows for 90 columns by 20 rows.
    size: (&'static str, &'static str),
    /// Prints `line-1` to `line-3000`.
    many_lines: &'static str,
}

fn shells() -> Vec<Shell> {
    let candidates: Vec<Shell> = if cfg!(windows) {
        // What PowerShell says of the window it runs in. Asked of PowerShell from the
        // command prompt too, whose own way of saying it is worded in the user's language.
        const SIZE: &str =
            "\"size-$($Host.UI.RawUI.WindowSize.Width)x$($Host.UI.RawUI.WindowSize.Height)\"";
        let powershell = |program| Shell {
            program,
            // Without the line editor, which keeps a history file of what is typed.
            args: &[
                "-NoLogo",
                "-NoProfile",
                "-NoExit",
                "-Command",
                "Remove-Module PSReadLine",
            ],
            env: &[],
            say: |word| format!("echo \"lume-${{env:LUME_MARK}}-{word}\""),
            size: (SIZE, "size-90x20"),
            many_lines: "1..3000 | ForEach-Object { \"line-$_\" }",
        };
        vec![
            Shell {
                program: "cmd.exe",
                args: &["/d"],
                env: &[],
                say: |word| format!("echo lume-%LUME_MARK%-{word}"),
                size: (
                    "powershell -NoProfile -Command \"'size-' + $Host.UI.RawUI.WindowSize.Width + 'x' + $Host.UI.RawUI.WindowSize.Height\"",
                    "size-90x20",
                ),
                many_lines: "for /l %i in (1,1,3000) do @echo line-%i",
            },
            powershell("powershell.exe"),
            powershell("pwsh.exe"),
        ]
    } else {
        let posix = |program, args| Shell {
            program,
            args,
            env: &["HISTFILE=/dev/null"],
            say: |word| format!("echo \"lume-${{LUME_MARK}}-{word}\""),
            size: ("echo \"size-$(stty size)\"", "size-20 90"),
            many_lines: "i=1; while [ $i -le 3000 ]; do echo line-$i; i=$((i+1)); done",
        };
        vec![
            posix("sh", &[]),
            posix("bash", &["--norc", "--noprofile"]),
            posix("zsh", &["-f"]),
            Shell {
                program: "fish",
                args: &["--no-config", "--private"],
                env: &[],
                say: |word| format!("echo \"lume-$LUME_MARK-{word}\""),
                size: ("echo \"size-\"(stty size)", "size-20 90"),
                many_lines: "for i in (seq 3000); echo line-$i; end",
            },
        ]
    };
    candidates
        .into_iter()
        .filter(|shell| on_path(shell.program.trim_end_matches(".exe")))
        .collect()
}

/// Starts `shell` in the window, with `mark` in its environment, and returns the
/// session's id.
fn start(app: &App, shell: &Shell, mark: &str) -> String {
    let mark = format!("LUME_MARK={mark}");
    let mut spawn = vec!["spawn", "--size", "100x30", "--env", &mark];
    for variable in shell.env {
        spawn.extend_from_slice(&["--env", variable]);
    }
    spawn.extend_from_slice(&["--", shell.program]);
    spawn.extend_from_slice(shell.args);
    let spawned = app.ctl(&spawn).ok(&format!("starting {}", shell.program));
    spawned["session"]
        .as_str()
        .expect("a session id")
        .to_owned()
}

/// Starts `shell` in the window and runs it through everything a session does.
fn exercise(app: &App, shell: &Shell) {
    let program = shell.program;
    let session = start(app, shell, MARK);

    // Typing reaches the shell and its output reaches the screen.
    app.type_line(&(shell.say)("ready"));
    app.wait_text(&format!("lume-{MARK}-ready"));

    let screen = app.ctl(&["capture"]).ok("capture");
    assert_eq!(screen["session"], session.as_str(), "{program}");
    assert_eq!(screen["size"]["cols"], 100, "{program}");
    assert_eq!(screen["size"]["rows"], 30, "{program}");
    let lines = screen["lines"].as_array().expect("lines");
    assert_eq!(lines.len(), 30, "{program}");
    assert!(
        lines.iter().any(|line| line
            .as_str()
            .is_some_and(|line| line.contains("lume-k7q-ready"))),
        "{program}: {screen}"
    );

    // A resize reaches the program: it is asked what size it sees.
    let resized = app.ctl(&["resize", "90x20"]).ok("resize");
    assert_eq!(resized["size"]["cols"], 90, "{program}");
    app.type_line(shell.size.0);
    app.wait_text(shell.size.1);

    // Text outside ASCII goes in and comes back as it was.
    app.type_line(&(shell.say)("héllo-日本語"));
    app.wait_text(&format!("lume-{MARK}-héllo-日本語"));

    // A long run of output arrives whole and in order: its last line is on the screen,
    // with the line before it above.
    app.type_line(shell.many_lines);
    app.wait_text("line-3000");
    // Read from one picture of the screen: the prompt that follows moves the lines up.
    let screen = app.ctl(&["capture"]).ok("capture");
    let lines: Vec<&str> = screen["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let last = lines
        .iter()
        .position(|line| line.contains("line-3000"))
        .unwrap_or_else(|| panic!("{program}: the last line left the screen: {screen}"));
    assert!(
        last > 0 && lines[last - 1].contains("line-2999"),
        "{program}: {screen}"
    );

    // The exit status is the shell's own.
    app.type_line("exit 7");
    let exited = app
        .ctl(&["wait", "--exit", "--session", &session])
        .ok("exit");
    assert_eq!(exited["status"]["code"], 7, "{program}");
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn every_shell_on_this_machine_runs_in_the_window() {
    let shells = shells();
    assert!(!shells.is_empty(), "no shell was found to test with");
    let app = App::start();
    for shell in &shells {
        println!("exercising {}", shell.program);
        exercise(&app, shell);
    }
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_wait_that_is_not_met_times_out_and_says_so() {
    let app = App::start();
    let waited = Command::new(lume())
        .args(["ctl", "--endpoint", &app.endpoint, "--timeout", "0.5"])
        .args(["wait", "--text", "this text never appears on the screen"])
        .output()
        .expect("lume runs");
    assert_eq!(waited.status.code(), Some(3));
    let answer: Value = serde_json::from_slice(&waited.stdout).expect("an answer");
    assert_eq!(answer["ok"], false);
    assert_eq!(answer["code"], "timeout");

    // A session nobody has heard of is refused, not waited for.
    let unknown = app.ctl(&["kill", "00000000000000aa"]);
    assert_eq!(unknown.status, 1);
    assert_eq!(unknown.json["code"], "not_found");
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_second_start_shows_the_first_and_exits() {
    let app = App::start();
    let before = app.ctl(&["status"]).ok("status");

    // The application itself, started again on the same endpoint.
    let second = Command::new(application())
        .args(["--endpoint", &app.endpoint])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("the application starts");
    assert!(second.success(), "the second copy ended with {second}");

    // The console command, which does the same and starts nothing.
    let launcher = Command::new(lume())
        .args(["--endpoint", &app.endpoint])
        .status()
        .expect("lume runs");
    assert!(launcher.success());

    // The first is still the one answering, with the session it had.
    let after = app.ctl(&["status"]).ok("status");
    assert_eq!(after["pid"], before["pid"]);
    assert_eq!(after["shown"], before["shown"]);
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn closing_ends_every_shell_and_the_log_holds_nothing_that_was_typed() {
    let shell = shells().into_iter().next().expect("a shell to test with");
    let app = App::start();
    // A second session, so that one is shown and one is in the background. The shell
    // is told a secret through its environment and prints it, so the screen holds what
    // was never typed; what is typed holds a word of its own.
    let secret = format!("s3cr3t{}", std::process::id());
    let typed = format!("typ3d{}", std::process::id());
    start(&app, &shell, &secret);
    app.type_line(&(shell.say)(&typed));
    app.wait_text(&format!("lume-{secret}-{typed}"));

    let pids = app.session_pids();
    assert_eq!(pids.len(), 2, "two sessions are running");
    assert!(pids.iter().all(|pid| process_alive(*pid)));

    let log = app.quit();

    // Closing the application ends what ran in it.
    let deadline = Instant::now() + Duration::from_secs(15);
    while pids.iter().any(|pid| process_alive(*pid)) {
        assert!(
            Instant::now() < deadline,
            "a shell outlived the application: {pids:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    // Written at the most talkative level, the log still says nothing of the terminal's
    // contents: neither what was printed nor what was typed.
    let written = std::fs::read_to_string(&log).expect("the application wrote its log");
    assert!(
        written.contains("session started"),
        "the log is at its most talkative"
    );
    for held in [&secret, &typed] {
        assert!(!written.contains(held.as_str()), "the log holds {held:?}");
    }
    let _ = std::fs::remove_file(log);
}
