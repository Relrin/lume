//! The application driven from outside, through `lume ctl`, with real shells in it.
//!
//! Each test starts `lume-gui` with an endpoint of its own, drives it with the `lume`
//! built beside it, and closes it by asking it to quit. Nothing here reaches into the
//! application: what a test can see is what a script could.
//!
//! The application is given folders of its own for its configuration and its saved
//! workspaces, so a test reads and writes nothing of the user's, and a shell that keeps
//! no history to start in its first pane.
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

/// How long the settings window is given to open before the test goes on. Nothing a
/// script can ask says whether it is there.
const SETTINGS_WINDOW_UP: Duration = Duration::from_secs(2);

/// How many numbered lines are printed for a search to look through: several screens.
const SEARCHED_LINES: u32 = 300;

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

/// Where the tests' applications run their sessions unless a test says otherwise:
/// inside the application, so that a test's shells end when its application closes.
const IN_PROCESS_HOST: &str = "\n[host]\nmode = \"in-process\"\n";

/// A running application with an endpoint of its own.
struct App {
    child: Option<Child>,
    endpoint: String,
    log: PathBuf,
    /// Where the application keeps its configuration and its saved workspaces.
    home: PathBuf,
    /// Whether the folder is removed when the application has closed.
    keep_home: bool,
}

/// The configuration the tests' applications run with: a shell for the first pane that
/// starts fast and keeps no history of what is typed.
fn configuration() -> String {
    let (shell, args) = if cfg!(windows) {
        ("cmd.exe", "[\"/d\"]")
    } else {
        ("/bin/sh", "[]")
    };
    format!("[terminal]\nshell = \"{shell}\"\nshell-args = {args}\n")
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
    /// Starts an application that has never run before.
    fn start() -> Self {
        Self::start_with("")
    }

    /// Starts an application that has never run before, with `settings` added to the
    /// configuration the tests' applications run with.
    fn start_with(settings: &str) -> Self {
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let home = std::env::temp_dir().join(format!("lume-e2e-{}-{number}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("config")).expect("a folder for the configuration");
        let host = if settings.contains("[host]") {
            ""
        } else {
            IN_PROCESS_HOST
        };
        std::fs::write(
            home.join("config").join("config.toml"),
            configuration() + host + settings,
        )
        .expect("the configuration is written");
        Self::start_in(home, false)
    }

    /// Starts an application whose configuration and saved workspaces are in `home`.
    fn start_in(home: PathBuf, keep_home: bool) -> Self {
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        Self::start_on(
            home,
            keep_home,
            format!("e2e-{}-{number}", std::process::id()),
        )
    }

    /// Starts an application on `endpoint`, whose configuration and saved workspaces
    /// are in `home`. An application on the endpoint of one before it uses the session
    /// host that one used.
    fn start_on(home: PathBuf, keep_home: bool, endpoint: String) -> Self {
        let log = std::env::temp_dir().join(format!("lume-{endpoint}.log"));
        let _ = std::fs::remove_file(&log);
        let child = Command::new(application())
            .args(["--endpoint", &endpoint])
            .env("LUME_LOG", "trace")
            .env("LUME_LOG_FILE", &log)
            .env("LUME_CONFIG_DIR", home.join("config"))
            .env("LUME_STATE_DIR", home.join("state"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the application starts");
        let app = Self {
            child: Some(child),
            endpoint,
            log,
            home,
            keep_home,
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
        ctl_at(&self.endpoint, args)
    }

    /// The application's status once the pane that has the keyboard has a session. The
    /// endpoint answers before the page has had the session started, so a status read
    /// straight after the start may name none yet.
    fn status_with_session(&self) -> Value {
        let deadline = Instant::now() + START;
        loop {
            let status = self.ctl(&["status"]).ok("status");
            if status["focused"]["session"].is_string() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "the focused pane never had a session: {status}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Types a line into the pane that has the keyboard.
    fn type_line(&self, line: &str) {
        self.ctl(&["send", "--enter", "--", line]).ok("send");
    }

    /// Waits until `text` is on the screen of the pane that has the keyboard.
    fn wait_text(&self, text: &str) -> Value {
        self.ctl(&["wait", "--text", text])
            .ok(&format!("waiting for {text:?}"))
    }

    /// Types a line into a pane, whether or not its tab is on show.
    fn type_line_in(&self, pane: &str, line: &str) {
        self.ctl(&["send", "--pane", pane, "--enter", "--", line])
            .ok("send");
    }

    /// Waits until `text` is on the screen of a pane.
    fn wait_text_in(&self, pane: &str, text: &str) -> Value {
        self.ctl(&["wait", "--pane", pane, "--text", text])
            .ok(&format!("waiting for {text:?} in {pane}"))
    }

    /// The layout: its windows, their tabs, and the panes in them.
    fn layout(&self) -> Value {
        self.ctl(&["layout", "view"]).ok("layout view")
    }

    /// The layout once it says where its first window is on the desktop. A window
    /// tells where it is a moment after it was last moved, so this also waits for the
    /// place to have stopped changing.
    fn layout_with_bounds(&self) -> Value {
        let deadline = Instant::now() + START;
        let mut last = Value::Null;
        loop {
            let layout = self.layout();
            let bounds = layout["workspace"]["windows"][0]["bounds"].clone();
            if bounds.is_object() && bounds == last {
                return layout;
            }
            assert!(
                Instant::now() < deadline,
                "the window never said where it is: {layout}"
            );
            last = bounds;
            std::thread::sleep(Duration::from_millis(1500));
        }
    }

    /// The workspaces there are, and which is on show.
    fn workspaces(&self) -> Value {
        self.ctl(&["workspaces"]).ok("workspaces")
    }

    /// Waits until there are `count` sessions.
    fn until_sessions(&self, count: usize, what: &str) {
        let deadline = Instant::now() + START;
        loop {
            let listed = self.ctl(&["list"]).ok("list");
            if listed["sessions"].as_array().map_or(0, Vec::len) == count {
                return;
            }
            assert!(Instant::now() < deadline, "{what}: {listed}");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// The process behind the session a pane shows.
    fn pid_of(&self, pane: &str) -> u32 {
        let listed = self.ctl(&["list"]).ok("list");
        listed["sessions"]
            .as_array()
            .expect("a list of sessions")
            .iter()
            .find(|session| session["pane"] == pane)
            .and_then(|session| session["info"]["pid"].as_u64())
            .unwrap_or_else(|| panic!("pane {pane} shows no session: {listed}")) as u32
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

    /// Waits until the application's process has ended of its own accord, and checks
    /// that it ended well.
    fn until_closed(mut self, what: &str) {
        let mut child = self.child.take().expect("the application is running");
        let deadline = Instant::now() + START;
        loop {
            if let Some(status) = child.try_wait().expect("the application can be waited for") {
                assert!(status.success(), "the application ended with {status}");
                break;
            }
            if Instant::now() >= deadline {
                // Not left running: the test has failed, and its shells go with it.
                self.child = Some(child);
                panic!("{what}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if !self.keep_home {
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }

    /// The lines on the screen of the pane that has the keyboard.
    fn screen(&self) -> Vec<String> {
        let captured = self.ctl(&["capture"]).ok("capture");
        captured["lines"]
            .as_array()
            .expect("the lines of a screen")
            .iter()
            .map(|line| line.as_str().unwrap_or_default().to_owned())
            .collect()
    }

    /// Ends the application's process at once, as a crash would.
    fn kill(mut self) {
        let mut child = self.child.take().expect("the application is running");
        child.kill().expect("the application can be ended");
        let _ = child.wait();
    }

    /// Asks the application to close and waits until its process has ended.
    fn quit(self) -> PathBuf {
        self.quit_with(&["quit"])
    }

    /// Asks the application to close with `request`, and waits until its process has
    /// ended.
    fn quit_with(mut self, request: &[&str]) -> PathBuf {
        self.ctl(request).ok("quit");
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
        if !self.keep_home {
            let _ = std::fs::remove_dir_all(&self.home);
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

/// The answer of `lume ctl` for the application on `endpoint`, running or not.
fn ctl_at(endpoint: &str, args: &[&str]) -> Answer {
    let output = Command::new(lume())
        .args(["ctl", "--endpoint", endpoint, "--timeout", WAIT])
        .args(args)
        .output()
        .expect("lume runs");
    let text = String::from_utf8_lossy(&output.stdout);
    Answer {
        status: output.status.code().unwrap_or(-1),
        json: serde_json::from_str(text.trim()).unwrap_or(Value::String(text.into_owned())),
    }
}

/// What `lume host` says about the session host of the application on `endpoint`.
fn host(endpoint: &str, command: &[&str]) -> Answer {
    let output = Command::new(lume())
        .args(["host", "--endpoint", endpoint])
        .args(command)
        .output()
        .expect("lume runs");
    let text = String::from_utf8_lossy(&output.stdout);
    Answer {
        status: output.status.code().unwrap_or(-1),
        json: serde_json::from_str(text.trim()).unwrap_or(Value::String(text.into_owned())),
    }
}

/// Stops the session host of a test whatever becomes of the test, so that a test that
/// fails leaves no shell running in it.
struct HostGuard(String);

impl Drop for HostGuard {
    fn drop(&mut self) {
        let _ = host(&self.0, &["stop", "--end-sessions"]);
    }
}

/// The ids of the panes of a tab's tree, in the order they are laid out.
fn panes_of(node: &Value) -> Vec<String> {
    match node["type"].as_str() {
        Some("pane") => vec![node["id"].as_str().expect("a pane id").to_owned()],
        _ => node["children"]
            .as_array()
            .expect("a split's children")
            .iter()
            .flat_map(panes_of)
            .collect(),
    }
}

/// The panes of each tab of a layout's first window.
fn tabs_of(layout: &Value) -> Vec<Vec<String>> {
    layout["workspace"]["windows"][0]["centre"]["tabs"]
        .as_array()
        .expect("the window's tabs")
        .iter()
        .map(|tab| panes_of(&tab["root"]))
        .collect()
}

/// What a pane of a layout shows.
fn content_of(layout: &Value, pane: &str) -> Value {
    fn find(node: &Value, pane: &str) -> Option<Value> {
        match node["type"].as_str() {
            Some("pane") => (node["id"] == pane).then(|| node["content"].clone()),
            _ => node["children"]
                .as_array()?
                .iter()
                .find_map(|child| find(child, pane)),
        }
    }
    layout["workspace"]["windows"][0]["centre"]["tabs"]
        .as_array()
        .expect("the window's tabs")
        .iter()
        .find_map(|tab| find(&tab["root"], pane))
        .unwrap_or_else(|| panic!("the layout has no pane {pane}: {layout}"))
}

/// Waits until a process is gone, and fails if it outlives `limit`.
fn until_gone(pid: u32, limit: Duration, what: &str) {
    let deadline = Instant::now() + limit;
    while process_alive(pid) {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(100));
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
    /// Prints forty thousand long lines, some megabytes, as fast as it can.
    flood: &'static str,
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
            flood: "1..40000 | ForEach-Object { \"line-$_-0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz\" }",
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
                flood: "for /l %i in (1,1,40000) do @echo line-%i-0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz",
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
            flood: "i=1; while [ $i -le 40000 ]; do echo line-$i-0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz; i=$((i+1)); done",
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
                flood: "for i in (seq 40000); echo line-$i-0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz; end",
            },
        ]
    };
    candidates
        .into_iter()
        .filter(|shell| on_path(shell.program.trim_end_matches(".exe")))
        .collect()
}

/// Starts `shell` with `mark` in its environment, in a new tab or wherever `place`
/// says, and returns what the application answered: the session and its pane.
fn start_at(app: &App, shell: &Shell, mark: &str, place: &[&str]) -> Value {
    let mark = format!("LUME_MARK={mark}");
    let mut spawn = vec!["spawn"];
    spawn.extend_from_slice(place);
    spawn.extend_from_slice(&["--env", &mark]);
    for variable in shell.env {
        spawn.extend_from_slice(&["--env", variable]);
    }
    spawn.extend_from_slice(&["--", shell.program]);
    spawn.extend_from_slice(shell.args);
    app.ctl(&spawn).ok(&format!("starting {}", shell.program))
}

/// Starts `shell` in a new tab, 100 columns by 30 rows, with `mark` in its
/// environment, and returns the session's id.
fn start(app: &App, shell: &Shell, mark: &str) -> String {
    let spawned = start_at(app, shell, mark, &["--size", "100x30"]);
    spawned["session"]
        .as_str()
        .expect("a session id")
        .to_owned()
}

/// Starts `shell` in a new tab and runs it through everything a session does.
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
    // Read once the page has its session: read earlier, the status would name none, and
    // the one read below would differ from it only because the page had started since.
    let before = app.status_with_session();

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
    assert_eq!(
        after["pid"], before["pid"],
        "before: {before}, after: {after}"
    );
    assert_eq!(
        after["focused"], before["focused"],
        "before: {before}, after: {after}"
    );
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn panes_and_tabs_are_made_driven_and_closed_by_id() {
    let shell = shells().into_iter().next().expect("a shell to test with");
    let app = App::start();
    let first = app.status_with_session()["focused"]["pane"]
        .as_str()
        .expect("a pane has the keyboard")
        .to_owned();

    // A pane beside the first, then a tab, which takes the first tab off the screen.
    let beside = start_at(&app, &shell, MARK, &["--split", "right"]);
    let beside = beside["pane"].as_str().expect("a pane id").to_owned();
    assert!(start_at(&app, &shell, MARK, &["--tab"])["tab"].is_string());
    let layout = app.layout();
    let tabs = tabs_of(&layout);
    assert_eq!(tabs.len(), 2, "{layout}");
    assert_eq!(tabs[0], [first.clone(), beside.clone()], "{layout}");
    assert_eq!(tabs[1].len(), 1, "{layout}");
    let third = tabs[1][0].clone();
    assert_eq!(app.status_with_session()["focused"]["pane"], third.as_str());

    // A pane in a tab that is not on show is typed into and read all the same.
    app.type_line_in(&beside, &(shell.say)("hidden"));
    let found = app.wait_text_in(&beside, &format!("lume-{MARK}-hidden"));
    assert_eq!(found["pane"], beside.as_str());
    let screen = app.ctl(&["capture", "--pane", &beside]).ok("capture");
    assert_eq!(screen["pane"], beside.as_str());
    assert!(
        screen["lines"]
            .as_array()
            .expect("lines")
            .iter()
            .any(|line| line
                .as_str()
                .is_some_and(|line| line.contains("lume-k7q-hidden"))),
        "{screen}"
    );
    // What was typed there did not reach the pane on show.
    let shown = app.ctl(&["capture"]).ok("capture");
    assert_eq!(shown["pane"], third.as_str());
    assert!(!shown.to_string().contains("lume-k7q-hidden"), "{shown}");

    // A command switches tabs, and the keyboard goes with it.
    app.ctl(&["command", "lume.tab.activate", r#"{"index":1}"#])
        .ok("activating the first tab");
    assert_eq!(
        app.ctl(&["status"]).ok("status")["focused"]["pane"],
        beside.as_str()
    );

    // Closing a pane ends the shell in it and nothing else.
    let closing = app.pid_of(&beside);
    let staying = app.pid_of(&first);
    let close = format!(r#"{{"type":"close","pane":"{beside}"}}"#);
    app.ctl(&["layout", "apply", &close]).ok("closing a pane");
    until_gone(
        closing,
        Duration::from_secs(15),
        "the shell of a closed pane was not ended",
    );
    assert!(process_alive(staying));
    assert_eq!(tabs_of(&app.layout())[0], std::slice::from_ref(&first));

    // A shell that ends without an error takes its pane with it.
    app.type_line_in(&first, "exit");
    let deadline = Instant::now() + Duration::from_secs(15);
    while tabs_of(&app.layout()).len() != 1 {
        assert!(
            Instant::now() < deadline,
            "the pane of a shell that ended stayed"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(tabs_of(&app.layout()), [[third]]);

    // An intent that cannot be applied is refused with its reason.
    let refused = app.ctl(&["layout", "apply", &close]);
    assert_eq!(refused.status, 1);
    assert_eq!(refused.json["code"], "not_found");
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_restart_brings_the_tabs_and_their_directories_back() {
    let shell = shells().into_iter().next().expect("a shell to test with");
    let app = App::start();
    app.status_with_session();
    // A folder with a name nothing else has, for a pane to be started in.
    let folder = app.home.join(format!("lume-here-{}", std::process::id()));
    std::fs::create_dir_all(&folder).expect("a folder to start a shell in");
    let folder_text = folder.to_string_lossy().into_owned();

    let tabbed = start_at(&app, &shell, MARK, &["--tab", "--cwd", &folder_text]);
    let tabbed = tabbed["pane"].as_str().expect("a pane id").to_owned();
    let beside = start_at(&app, &shell, MARK, &["--split", "down"]);
    let beside = beside["pane"].as_str().expect("a pane id").to_owned();
    let before = app.layout();
    let old_pid = app.pid_of(&tabbed);

    let home = app.home.clone();
    let mut app = app;
    app.keep_home = true;
    app.quit();
    until_gone(
        old_pid,
        Duration::from_secs(15),
        "a shell outlived the application",
    );

    // The application again, with what the first one left.
    let app = App::start_in(home, false);
    app.status_with_session();
    let after = app.layout();
    assert_eq!(tabs_of(&after), tabs_of(&before), "{after}");
    assert_eq!(
        after["workspace"]["windows"][0]["focus"],
        before["workspace"]["windows"][0]["focus"]
    );
    assert_eq!(content_of(&after, &tabbed)["cwd"], folder_text.as_str());
    assert_eq!(
        content_of(&after, &tabbed)["launcher"],
        content_of(&before, &tabbed)["launcher"]
    );

    // Every pane has a shell again, and the one that was started in the folder is
    // started there.
    let panes: Vec<String> = tabs_of(&after).into_iter().flatten().collect();
    assert_eq!(panes.len(), 3);
    assert!(panes.contains(&beside));
    let print_directory = if cfg!(windows) { "cd" } else { "pwd" };
    let named = folder
        .file_name()
        .expect("the folder has a name")
        .to_string_lossy()
        .into_owned();
    // The restored panes run the shell they ran before, started afresh.
    let deadline = Instant::now() + START;
    while app.ctl(&["list"]).ok("list")["sessions"]
        .as_array()
        .map_or(0, Vec::len)
        != 3
    {
        assert!(
            Instant::now() < deadline,
            "the panes' shells were not started again"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_ne!(app.pid_of(&tabbed), old_pid);
    app.type_line_in(&tabbed, print_directory);
    app.wait_text_in(&tabbed, &named);
    app.quit();
}

/// The id of a pane an answer to `spawn` names.
fn pane_of(spawned: &Value) -> String {
    spawned["pane"].as_str().expect("a pane id").to_owned()
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_profile_starts_its_terminal_and_a_split_runs_the_same_profile() {
    let (program, args, show) = if cfg!(windows) {
        ("cmd.exe", "[\"/d\"]", "echo lume-%LUME_E2E%-seen")
    } else {
        ("/bin/sh", "[]", "echo \"lume-${LUME_E2E}-seen\"")
    };
    // The settings close a pane whose program ended well; this profile keeps it.
    let app = App::start_with(&format!(
        "\n[[profile]]\nid = \"mine\"\nname = \"Mine\"\nlauncher = \"shell\"\n\
         program = \"{program}\"\nargs = {args}\nenv = {{ LUME_E2E = \"k7q\" }}\n\
         close-on-exit = \"never\"\n"
    ));
    app.status_with_session();

    // The user's profile is listed at once; the ones made for what is installed
    // follow when the machine has been looked at.
    let deadline = Instant::now() + START;
    let listed = loop {
        let listed = app.ctl(&["profiles"]).ok("profiles")["profiles"].clone();
        if listed["looked"] == true {
            break listed;
        }
        assert!(
            Instant::now() < deadline,
            "the machine was never looked at: {listed}"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    let profiles = listed["profiles"].as_array().expect("profiles");
    let mine = &profiles[0];
    assert_eq!(mine["id"], "mine", "{listed}");
    assert_eq!(mine["origin"], "user");
    assert_eq!(mine["launcher"]["type"], "shell");
    let generated: Vec<&str> = profiles
        .iter()
        .filter(|profile| profile["origin"] == "generated")
        .filter_map(|profile| profile["id"].as_str())
        .collect();
    assert!(
        generated.iter().any(|id| id.starts_with("auto.")),
        "no shell was found on this machine: {listed}"
    );

    // A terminal started from the profile runs its program with its variables.
    let spawned = app.ctl(&["spawn", "--profile", "mine"]).ok("spawn");
    let pane = pane_of(&spawned);
    app.type_line_in(&pane, show);
    app.wait_text_in(&pane, "lume-k7q-seen");
    assert_eq!(content_of(&app.layout(), &pane)["profile"], "mine");

    // Splitting it starts the same profile beside it.
    let split = format!(r#"{{"pane":"{pane}"}}"#);
    app.ctl(&["command", "lume.pane.split-right", &split])
        .ok("splitting the profile's pane");
    let layout = app.layout();
    let tab = tabs_of(&layout)
        .into_iter()
        .find(|tab| tab.contains(&pane))
        .expect("the tab of the profile's pane");
    assert_eq!(tab.len(), 2, "{layout}");
    let beside = tab
        .iter()
        .find(|other| **other != pane)
        .expect("the new pane")
        .clone();
    assert_eq!(content_of(&layout, &beside)["profile"], "mine");
    app.type_line_in(&beside, show);
    app.wait_text_in(&beside, "lume-k7q-seen");

    // A profile that is not there is refused, not replaced by another.
    let refused = app.ctl(&["spawn", "--profile", "no-such-profile"]);
    assert_eq!(refused.status, 1, "{}", refused.json);
    assert_eq!(refused.json["code"], "not_found");

    // The profile's own rule for a program that ended holds over the settings'.
    app.type_line_in(&beside, "exit");
    app.ctl(&["wait", "--exit", "--pane", &beside])
        .ok("waiting for the shell to end");
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        tabs_of(&app.layout())
            .iter()
            .any(|tab| tab.contains(&beside)),
        "the pane of a profile that keeps its panes was closed"
    );

    // A Linux distribution, where there is one. Nothing is typed into it: its shell
    // is the user's own, with the user's history.
    match generated.iter().find(|id| id.starts_with("auto.wsl.")) {
        Some(distribution) => {
            let spawned = app
                .ctl(&["spawn", "--profile", distribution])
                .ok("starting a distribution");
            let pane = pane_of(&spawned);
            let session = spawned["session"].as_str().expect("a session id");
            std::thread::sleep(Duration::from_secs(2));
            let sessions = app.ctl(&["list"]).ok("list");
            let running = sessions["sessions"]
                .as_array()
                .expect("sessions")
                .iter()
                .find(|listed| listed["info"]["id"] == session)
                .expect("the distribution's session");
            assert!(
                running["info"]["program"]
                    .as_str()
                    .is_some_and(|program| program.to_lowercase().ends_with("wsl.exe")),
                "{running}"
            );
            assert!(running["info"]["exit"].is_null(), "{running}");
            assert_eq!(content_of(&app.layout(), &pane)["profile"], *distribution);
        }
        None => eprintln!("skipped the distribution: this machine lists none"),
    }
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn docks_floating_panes_and_where_the_window_was_come_back_after_a_restart() {
    let shell = shells().into_iter().next().expect("a shell to test with");
    // The list of tabs is a panel at the left, so there is something docked.
    let app = App::start_with("[layout]\ntab-placement = \"left\"\n");
    app.status_with_session();

    // Three tabs, two of them divided, one pane taken out to float and one put away.
    let beside = pane_of(&start_at(&app, &shell, MARK, &["--split", "right"]));
    app.ctl(&[
        "command",
        "lume.pane.minimise",
        &format!(r#"{{"pane":"{beside}"}}"#),
    ])
    .ok("putting a pane away");
    start_at(&app, &shell, MARK, &["--tab"]);
    let below = pane_of(&start_at(&app, &shell, MARK, &["--split", "down"]));
    start_at(&app, &shell, MARK, &["--tab"]);
    app.ctl(&[
        "command",
        "lume.pane.float",
        &format!(r#"{{"pane":"{below}"}}"#),
    ])
    .ok("floating a pane");
    let place = r#"{"x":100000,"y":150000,"w":400000,"h":500000}"#;
    let moved = format!(r#"{{"type":"float_move","pane":"{below}","rect":{place}}}"#);
    app.ctl(&["layout", "apply", &moved])
        .ok("moving the floating pane");
    let window = app.layout()["window"]
        .as_str()
        .expect("a window id")
        .to_owned();
    let wider =
        format!(r#"{{"type":"panel_resize","window":"{window}","area":"left","size":300}}"#);
    app.ctl(&["layout", "apply", &wider])
        .ok("resizing the side panel");

    let before = app.layout_with_bounds();
    let shown = &before["workspace"]["windows"][0];
    assert_eq!(tabs_of(&before).len(), 3, "{before}");
    assert_eq!(shown["dock"]["left"]["panels"][0]["id"], "lume.tabs");
    assert_eq!(shown["dock"]["left"]["size"], 300);
    // The pane that was put away is first: it left its tab before the other floated.
    assert_eq!(shown["floating"][0]["pane"]["id"], beside.as_str());
    assert_eq!(shown["floating"][0]["minimised"], "tab");
    assert_eq!(shown["floating"][1]["pane"]["id"], below.as_str());
    assert_eq!(shown["floating"][1]["minimised"], Value::Null);
    assert_eq!(
        shown["floating"][1]["rect"],
        serde_json::from_str::<Value>(place).unwrap()
    );
    let before = before.clone();

    let home = app.home.clone();
    let mut app = app;
    app.keep_home = true;
    app.quit();

    let app = App::start_in(home, false);
    app.status_with_session();
    let after = app.layout_with_bounds();
    let (was, is) = (
        &before["workspace"]["windows"][0],
        &after["workspace"]["windows"][0],
    );
    assert_eq!(tabs_of(&after), tabs_of(&before), "{after}");
    assert_eq!(is["dock"], was["dock"], "{after}");
    assert_eq!(is["floating"][0]["pane"]["id"], beside.as_str(), "{after}");
    assert_eq!(is["floating"][0]["minimised"], "tab", "{after}");
    assert_eq!(is["floating"][1]["pane"]["id"], below.as_str(), "{after}");
    assert_eq!(is["floating"][1]["rect"], was["floating"][1]["rect"]);
    assert_eq!(is["bounds"], was["bounds"], "{after}");
    // Every pane has a shell again, the floating one and the one put away too.
    app.until_sessions(5, "the panes' shells were not started again");

    // Brought back, the pane that was put away is among the tabs again.
    let back = format!(r#"{{"type":"minimise","pane":"{beside}","on":false}}"#);
    app.ctl(&["layout", "apply", &back])
        .ok("bringing a pane back");
    let restored = app.layout();
    assert!(
        tabs_of(&restored).iter().any(|tab| tab.contains(&beside)),
        "{restored}"
    );
    assert_eq!(
        restored["workspace"]["windows"][0]["floating"]
            .as_array()
            .expect("the floating panes")
            .len(),
        1,
        "{restored}"
    );
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_second_start_runs_a_program_in_a_tab_and_shows_a_workspace() {
    let shell = shells().into_iter().next().expect("a shell to test with");
    let app = App::start();
    let pid = app.status_with_session()["pid"].clone();
    let folder = app.home.join(format!("lume-there-{}", std::process::id()));
    std::fs::create_dir_all(&folder).expect("a folder to start a program in");

    // The console command with a program to run, typed in that folder.
    let ran = Command::new(lume())
        .args(["--endpoint", &app.endpoint, "-e", shell.program])
        .args(shell.args)
        .current_dir(&folder)
        .status()
        .expect("lume runs");
    assert!(ran.success(), "lume ended with {ran}");

    // The first application has a second tab, whose program was started there.
    let layout = app.layout();
    let tabs = tabs_of(&layout);
    assert_eq!(tabs.len(), 2, "{layout}");
    let content = content_of(&layout, &tabs[1][0]);
    assert_eq!(content["launcher"]["program"], shell.program);
    let named = folder
        .file_name()
        .expect("the folder has a name")
        .to_string_lossy()
        .into_owned();
    assert!(
        content["cwd"]
            .as_str()
            .is_some_and(|cwd| cwd.ends_with(&named)),
        "{content}"
    );
    app.until_sessions(2, "the program was not started");
    assert_eq!(app.ctl(&["status"]).ok("status")["pid"], pid);

    // Started again with a workspace's name, it shows that workspace, begun on the
    // spot, and what ran in the first one is ended.
    let shown = Command::new(lume())
        .args(["--endpoint", &app.endpoint, "--workspace", "other place"])
        .status()
        .expect("lume runs");
    assert!(shown.success(), "lume ended with {shown}");
    let workspaces = app.workspaces();
    assert_eq!(workspaces["active"], "other place");
    assert_eq!(
        workspaces["workspaces"]
            .as_array()
            .expect("a list of workspaces")
            .len(),
        2,
        "{workspaces}"
    );
    assert_eq!(tabs_of(&app.layout()).len(), 1);
    app.until_sessions(1, "the first workspace's sessions were not ended");

    // The first workspace is as it was left.
    app.ctl(&["command", "lume.workspace.switch", r#"{"name":"default"}"#])
        .ok("showing the first workspace");
    assert_eq!(app.workspaces()["active"], "default");
    assert_eq!(tabs_of(&app.layout()), tabs);
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn two_windows_show_one_workspace_and_closing_one_ends_what_ran_in_it() {
    let shell = shells().into_iter().next().expect("a shell to test with");
    let app = App::start();
    let first = app.status_with_session()["focused"]["pane"]
        .as_str()
        .expect("a pane has the keyboard")
        .to_owned();

    let far = pane_of(&start_at(&app, &shell, MARK, &["--window"]));
    let layout = app.layout();
    let windows = layout["workspace"]["windows"]
        .as_array()
        .expect("the windows");
    assert_eq!(windows.len(), 2, "{layout}");
    let second = windows[1]["id"].as_str().expect("a window id").to_owned();
    assert_eq!(layout["workspace"]["focused_window"], second.as_str());
    assert_eq!(windows[1]["focus"], far.as_str());

    // Each window's page shows its own panes: what is typed in one is on its screen
    // and not on the other's.
    app.type_line_in(&far, &(shell.say)("far"));
    app.wait_text_in(&far, &format!("lume-{MARK}-far"));
    let near = app.ctl(&["capture", "--pane", &first]).ok("capture");
    assert!(!near.to_string().contains("lume-k7q-far"), "{near}");

    // A change made through one window's panes reaches the layout both follow.
    let beside = pane_of(&start_at(&app, &shell, MARK, &["--split", "right"]));
    let layout = app.layout();
    assert_eq!(
        panes_of(&layout["workspace"]["windows"][1]["centre"]["tabs"][0]["root"]),
        [far.clone(), beside.clone()],
        "{layout}"
    );

    // Closing the second window ends what ran in it, and the first carries on.
    let (ending, staying) = (app.pid_of(&far), app.pid_of(&first));
    let close = format!(r#"{{"type":"window_close","window":"{second}"}}"#);
    app.ctl(&["layout", "apply", &close])
        .ok("closing the second window");
    until_gone(
        ending,
        Duration::from_secs(15),
        "the shell of a closed window was not ended",
    );
    assert!(process_alive(staying));
    let layout = app.layout();
    assert_eq!(
        layout["workspace"]["windows"]
            .as_array()
            .expect("the windows")
            .len(),
        1,
        "{layout}"
    );
    app.type_line_in(&first, "echo still-here");
    app.wait_text_in(&first, "still-here");
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_pane_is_searched_by_command_and_goes_on_taking_what_is_typed() {
    let app = App::start();
    app.status_with_session();
    // More numbered lines than a screen holds, so that there is scrollback to search.
    let print = if cfg!(windows) {
        format!("for /L %i in (1,1,{SEARCHED_LINES}) do @echo lume-line-%i-end")
    } else {
        format!(
            "i=1; while [ $i -le {SEARCHED_LINES} ]; do echo lume-line-$i-end; i=$((i+1)); done"
        )
    };
    app.type_line(&print);
    let last = format!("lume-line-{SEARCHED_LINES}-end");
    app.wait_text(&last);

    // The search is the page's own, and is asked for through the application. What it
    // scrolls into view is not what a program sees as its screen, which stays as it is.
    app.ctl(&[
        "command",
        "lume.search.open",
        r#"{"query":"lume-line-7-end"}"#,
    ])
    .ok("opening the search");
    app.ctl(&["command", "lume.search.next"])
        .ok("the next match");
    app.ctl(&["command", "lume.search.previous"])
        .ok("the match before");
    assert!(
        app.screen().iter().any(|line| line.contains(&last)),
        "searching changed the screen a program sees: {:?}",
        app.screen()
    );

    // A pane there is none of cannot be searched, and saying so breaks nothing.
    let refused = app.ctl(&["command", "lume.search.open", r#"{"pane":"p999"}"#]);
    assert_ne!(refused.status, 0, "{}", refused.json);

    app.ctl(&["command", "lume.search.close"])
        .ok("closing the search");
    app.type_line("echo after-the-search");
    app.wait_text("after-the-search");
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn the_application_ends_with_its_last_terminal_though_the_settings_are_open() {
    let app = App::start();
    app.status_with_session();
    app.ctl(&[
        "command",
        "lume.settings.open",
        r#"{"section":"appearance"}"#,
    ])
    .ok("opening the settings");
    std::thread::sleep(SETTINGS_WINDOW_UP);
    // Asked for again, the window that is open is the one shown.
    app.ctl(&["command", "lume.settings.open"])
        .ok("showing the settings again");
    // The settings are no window of the workspace: it has the one it had.
    let layout = app.layout();
    assert_eq!(
        layout["workspace"]["windows"]
            .as_array()
            .expect("the windows")
            .len(),
        1,
        "{layout}"
    );
    // A section that could not be one is refused nothing and opens nothing odd.
    app.ctl(&["command", "lume.settings.open", r#"{"section":"../x#y"}"#])
        .ok("a section that is none");

    // The shell of the only pane ends, its pane, tab and window close, and nothing is
    // left to keep the application running: not the settings window either.
    app.type_line("exit");
    app.until_closed("the application went on with only its settings to show");
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn closing_ends_every_shell_and_the_log_holds_nothing_that_was_typed() {
    let shell = shells().into_iter().next().expect("a shell to test with");
    let app = App::start();
    // A second session, in a tab of its own, so that one is on show and one is not. The
    // shell is told a secret through its environment and prints it, so the screen holds
    // what was never typed; what is typed holds a word of its own.
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

/// What `lume ctl events` prints for one pane, read as it comes.
struct Follower {
    child: Child,
    reports: std::sync::mpsc::Receiver<Value>,
    /// Every report read so far, so that one that came early can still be asked for.
    seen: std::cell::RefCell<Vec<Value>>,
}

impl Follower {
    /// Starts following `pane`. Returns once the application has said it is.
    fn of(app: &App, pane: &str) -> Self {
        use std::io::BufRead;

        let mut child = Command::new(lume())
            .args(["ctl", "--endpoint", &app.endpoint, "events", "--pane", pane])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("lume runs");
        let printed = child.stdout.take().expect("what lume prints");
        let (found, reports) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(printed).lines() {
                let Ok(line) = line else { return };
                let report = serde_json::from_str(&line).unwrap_or(Value::String(line));
                if found.send(report).is_err() {
                    return;
                }
            }
        });
        let follower = Self {
            child,
            reports,
            seen: std::cell::RefCell::new(Vec::new()),
        };
        let answer = match follower.reports.recv_timeout(START) {
            Ok(answer) => answer,
            Err(_) => panic!("the application never said it was following the pane"),
        };
        assert_eq!(answer["type"], "following", "{answer}");
        follower
    }

    /// What `wanted` gives for the first report it gives anything for, among those
    /// that have come and those still to come.
    fn until<T>(&self, what: &str, wanted: impl Fn(&Value) -> Option<T>) -> T {
        if let Some(found) = self.seen.borrow().iter().find_map(&wanted) {
            return found;
        }
        let deadline = Instant::now() + START;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let Ok(report) = self.reports.recv_timeout(left) else {
                panic!(
                    "{what} was never reported; what was: {:?}",
                    self.seen.borrow()
                );
            };
            let found = wanted(&report);
            self.seen.borrow_mut().push(report);
            if let Some(found) = found {
                return found;
            }
        }
    }

    /// The first directory reported that ends as `ending` does, however it is told:
    /// as a report of its own, or as part of how the session stands.
    fn directory_ending(&self, ending: &str) -> String {
        self.until(&format!("a directory ending in {ending}"), |report| {
            let event = &report["event"];
            let url = match event["type"].as_str()? {
                "cwd" => event["url"].as_str()?,
                "state" => event["state"]["cwd"].as_str()?,
                _ => return None,
            };
            url.ends_with(ending).then(|| url.to_owned())
        })
    }

    /// The first program reported to have the terminal, told either way.
    fn foreground(&self) -> String {
        self.until("the program that has the terminal", |report| {
            let event = &report["event"];
            let name = match event["type"].as_str()? {
                "foreground" => event["name"].as_str()?,
                "state" => event["state"]["foreground"]["name"].as_str()?,
                _ => return None,
            };
            Some(name.to_owned())
        })
    }
}

impl Drop for Follower {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A folder for a shell to go to, whose name needs escaping in a URL, and how that
/// name reads once it is escaped.
fn awkward_folder(app: &App) -> (PathBuf, &'static str) {
    let folder = app.home.join("lume e2e \u{e9}");
    std::fs::create_dir_all(&folder).expect("a folder to work in");
    (folder, "lume%20e2e%20%C3%A9")
}

/// A path as it is typed into a shell: with forward slashes, which every shell takes.
fn typed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// The bash to run here: the one on the path, or on Windows the one Git brings.
fn bash() -> Option<String> {
    if cfg!(windows) {
        let program_files = std::env::var_os("ProgramFiles")?;
        let git = Path::new(&program_files)
            .join("Git")
            .join("bin")
            .join("bash.exe");
        git.is_file().then(|| git.to_string_lossy().into_owned())
    } else {
        on_path("bash").then(|| "bash".to_owned())
    }
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_shell_reports_where_it_is_and_what_it_runs_and_keeps_its_own_prompt() {
    let app = App::start();
    app.status_with_session();
    let Some(bash) = bash() else {
        eprintln!("skipped: there is no bash on this machine");
        app.quit();
        return;
    };
    let (folder, escaped) = awkward_folder(&app);

    // A start-up file of the user's own, with a prompt and an alias in it. It is read
    // as it would be anywhere, and nothing is written to it.
    let rc = app.home.join("bashrc");
    std::fs::write(
        &rc,
        "PS1='lume-prompt> '\nalias greet='echo \"lume-${LUME_MARK}-hello\"'\n",
    )
    .expect("a start-up file");
    let before = std::fs::read(&rc).expect("the start-up file");
    let mark = format!("LUME_MARK={MARK}");
    let spawned = app
        .ctl(&[
            "spawn",
            "--env",
            &mark,
            "--env",
            "HISTFILE=/dev/null",
            "--",
            &bash,
            "--noprofile",
            "--rcfile",
            &typed(&rc),
        ])
        .ok("starting bash");
    let pane = pane_of(&spawned);
    let follower = Follower::of(&app, &pane);

    // The shell's own prompt and alias are as its start-up file made them.
    app.wait_text_in(&pane, "lume-prompt>");
    app.type_line_in(&pane, "greet");
    app.wait_text_in(&pane, &format!("lume-{MARK}-hello"));

    // Where it goes is reported, in a form that survives a blank and an accent.
    app.type_line_in(&pane, &format!("cd '{}'", typed(&folder)));
    let url = follower.directory_ending(escaped);
    assert!(url.starts_with("file://"), "{url}");

    // A command that fails is reported with its line and its exit code.
    app.type_line_in(&pane, "(exit 3)");
    follower.until("the command starting", |report| {
        let event = &report["event"];
        (event["type"] == "command_started" && event["command"] == "(exit 3)").then_some(())
    });
    follower.until("the command ending with its exit code", |report| {
        let event = &report["event"];
        (event["type"] == "command_finished" && event["exit"] == 3).then_some(())
    });

    // The program that has the terminal is the shell.
    assert_eq!(follower.foreground(), "bash");

    // A pane opened from it starts where it is.
    let split = format!(r#"{{"pane":"{pane}"}}"#);
    app.ctl(&["command", "lume.pane.split-right", &split])
        .ok("splitting the pane");
    let layout = app.layout();
    let beside = tabs_of(&layout)
        .into_iter()
        .find(|tab| tab.contains(&pane))
        .expect("the tab of the pane")
        .into_iter()
        .find(|other| *other != pane)
        .expect("the new pane");
    let started_in = content_of(&layout, &beside)["cwd"]
        .as_str()
        .expect("where the new pane started")
        .to_owned();
    assert_eq!(Path::new(&started_in), folder, "{layout}");

    // And the start-up file is as it was.
    assert_eq!(std::fs::read(&rc).expect("the start-up file"), before);

    // PowerShell, where there is one. It is started as a user starts it, so its line
    // editor keeps a history: nothing is typed into it. What it says by itself, at
    // its first prompt, is where it is.
    if cfg!(windows) && on_path("powershell") {
        let spawned = app
            .ctl(&[
                "spawn",
                "--cwd",
                &folder.to_string_lossy(),
                "--",
                "powershell.exe",
                "-NoLogo",
                "-NoProfile",
            ])
            .ok("starting PowerShell");
        let follower = Follower::of(&app, &pane_of(&spawned));
        let url = follower.directory_ending(escaped);
        assert!(url.starts_with("file://"), "{url}");
        assert_eq!(follower.foreground(), "powershell");
    } else {
        eprintln!("skipped PowerShell: this machine has none");
    }
    app.quit();
}

impl App {
    /// Whether something runs in a pane that closing it would end, as the application
    /// tells it.
    fn busy(&self, pane: &str) -> bool {
        let listed = self.ctl(&["list"]).ok("list");
        listed["sessions"]
            .as_array()
            .expect("a list of sessions")
            .iter()
            .find(|session| session["pane"] == pane)
            .and_then(|session| session["busy"].as_bool())
            .unwrap_or_else(|| panic!("pane {pane} shows no session: {listed}"))
    }

    /// Waits until a pane is said to run something, or not.
    fn until_busy(&self, pane: &str, busy: bool, what: &str) {
        let deadline = Instant::now() + START;
        while self.busy(pane) != busy {
            if Instant::now() >= deadline {
                let listed = self.ctl(&["list"]).ok("list");
                panic!("{what} never happened: {listed}");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_shell_at_its_prompt_runs_nothing_and_a_program_it_runs_is_something() {
    let app = App::start();
    app.status_with_session();
    let Some(bash) = bash() else {
        eprintln!("skipped: there is no bash on this machine");
        app.quit();
        return;
    };
    let spawned = app
        .ctl(&[
            "spawn",
            "--env",
            "HISTFILE=/dev/null",
            "--env",
            "PS1=lume-idle> ",
            "--",
            &bash,
            "--noprofile",
            "--norc",
        ])
        .ok("starting bash");
    let pane = pane_of(&spawned);
    let follower = Follower::of(&app, &pane);
    app.wait_text_in(&pane, "lume-idle>");
    // At its prompt the shell runs nothing that closing its pane would end.
    app.until_busy(&pane, false, "the shell to be seen at its prompt");

    // A program it runs is something, until it ends.
    app.type_line_in(&pane, "sleep 4");
    app.until_busy(&pane, true, "the program to be seen");
    follower.until("the command ending", |report| {
        (report["event"]["type"] == "command_finished").then_some(())
    });
    app.until_busy(&pane, false, "the program to end");
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn the_shells_outlive_the_application_and_come_back_with_it() {
    let shell = shells().into_iter().next().expect("a shell to test with");
    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    let endpoint = format!("e2e-{}-{number}", std::process::id());
    let _stops = HostGuard(endpoint.clone());
    let home = std::env::temp_dir().join(format!("lume-e2e-{}-{number}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("config")).expect("a folder for the configuration");
    std::fs::write(
        home.join("config").join("config.toml"),
        configuration() + "\n[host]\nmode = \"daemon\"\nidle-exit = 2\n",
    )
    .expect("the configuration is written");

    let app = App::start_on(home.clone(), true, endpoint.clone());
    app.status_with_session();
    let spawned = start_at(&app, &shell, MARK, &["--tab"]);
    let pane = pane_of(&spawned);
    let typed = format!("b3fore{}", std::process::id());
    app.type_line_in(&pane, &(shell.say)(&typed));
    app.wait_text_in(&pane, &format!("lume-{MARK}-{typed}"));
    let pid = app.pid_of(&pane);
    let pids = app.session_pids();
    assert_eq!(pids.len(), 2, "two sessions are running");
    let listed = app.ctl(&["list"]).ok("list");
    let session = listed["sessions"]
        .as_array()
        .expect("a list of sessions")
        .iter()
        .find(|listed| listed["pane"] == pane.as_str())
        .and_then(|listed| listed["info"]["id"].as_str())
        .expect("the pane's session")
        .to_owned();
    let status = host(&endpoint, &["status"]).ok("host status");
    assert_eq!(status["sessions"], 2, "{status}");
    let host_pid = status["pid"].clone();

    // The application is ended as a crash would end it. The shells carry on.
    app.kill();
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        pids.iter().all(|pid| process_alive(*pid)),
        "a shell ended with the application"
    );

    // The next start shows the same shells, with what they showed.
    let app = App::start_on(home.clone(), true, endpoint.clone());
    app.status_with_session();
    assert_eq!(app.pid_of(&pane), pid, "the pane started a new shell");
    let screen = app.ctl(&["capture", "--pane", &pane]).ok("capture");
    let shown = screen["lines"].to_string();
    assert!(shown.contains(&format!("lume-{MARK}-{typed}")), "{shown}");
    let more = format!("aft3r{}", std::process::id());
    app.type_line_in(&pane, &(shell.say)(&more));
    app.wait_text_in(&pane, &format!("lume-{MARK}-{more}"));

    // Closing leaves them running too, in the same host.
    app.quit();
    assert!(
        pids.iter().all(|pid| process_alive(*pid)),
        "a shell ended with the application"
    );
    assert_eq!(
        host(&endpoint, &["status"]).ok("host status")["pid"],
        host_pid
    );

    // With no application, the sessions are still driven, through the host.
    let listed = ctl_at(&endpoint, &["list"]).ok("list with no application");
    assert_eq!(
        listed["sessions"].as_array().map(Vec::len),
        Some(2),
        "{listed}"
    );
    let alone = format!("n0app{}", std::process::id());
    ctl_at(
        &endpoint,
        &[
            "send",
            "--session",
            &session,
            "--enter",
            "--",
            &(shell.say)(&alone),
        ],
    )
    .ok("send with no application");
    ctl_at(
        &endpoint,
        &[
            "wait",
            "--session",
            &session,
            "--text",
            &format!("lume-{MARK}-{alone}"),
        ],
    )
    .ok("wait with no application");
    let screen =
        ctl_at(&endpoint, &["capture", "--session", &session]).ok("capture with no application");
    assert!(screen["lines"].to_string().contains(&alone), "{screen}");
    // What needs the application says so.
    for needs_app in [
        &["status"][..],
        &["capture", "--pane", &pane],
        &["layout", "view"],
    ] {
        let refused = ctl_at(&endpoint, needs_app);
        assert_eq!(refused.status, 4, "{needs_app:?}: {}", refused.json);
        assert_eq!(refused.json["code"], "not_running", "{}", refused.json);
    }
    // A program started with no application runs in no pane.
    let mark = format!("LUME_MARK={MARK}");
    let mut spawn = vec!["spawn", "--env", mark.as_str()];
    for variable in shell.env {
        spawn.extend_from_slice(&["--env", variable]);
    }
    spawn.extend_from_slice(&["--", shell.program]);
    spawn.extend_from_slice(shell.args);
    let unshown = ctl_at(&endpoint, &spawn).ok("spawn with no application")["session"]
        .as_str()
        .expect("a session id")
        .to_owned();

    // Quitting and ending the sessions ends them, and the host follows.
    let app = App::start_on(home, false, endpoint.clone());
    app.status_with_session();
    assert_eq!(app.pid_of(&pane), pid);
    // The session no window shows is driven through the application by its id.
    app.ctl(&[
        "send",
        "--session",
        &unshown,
        "--enter",
        "--",
        &(shell.say)("unsh0wn"),
    ])
    .ok("send to a session no window shows");
    app.ctl(&[
        "wait",
        "--session",
        &unshown,
        "--text",
        &format!("lume-{MARK}-unsh0wn"),
    ])
    .ok("wait in a session no window shows");
    let log = app.quit_with(&["command", "lume.app.quit-end-sessions"]);
    let deadline = Instant::now() + Duration::from_secs(15);
    while pids.iter().any(|pid| process_alive(*pid)) {
        assert!(
            Instant::now() < deadline,
            "a shell outlived quitting with its session ended"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let status = host(&endpoint, &["status"]);
        if status.status == 4 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the host stayed with nothing to do: {}",
            status.json
        );
        std::thread::sleep(Duration::from_millis(200));
    }

    // The host's log says what it did and nothing that passed through a terminal.
    let host_log = log.with_extension("host.log");
    let written = std::fs::read_to_string(&host_log).expect("the host wrote its log");
    assert!(written.contains("the session host is serving"), "{written}");
    for held in [&typed, &more] {
        assert!(
            !written.contains(held.as_str()),
            "the host's log holds {held}"
        );
    }
    let _ = std::fs::remove_file(&host_log);
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn one_session_shows_in_two_windows_and_ends_with_the_last_of_them() {
    let shell = shells().into_iter().next().expect("a shell to test with");
    let app = App::start();
    app.status_with_session();
    let first = pane_of(&start_at(&app, &shell, MARK, &["--tab"]));
    let pid = app.pid_of(&first);

    let args = format!(r#"{{"pane":"{first}"}}"#);
    app.ctl(&["command", "lume.pane.show-in-new-window", &args])
        .ok("showing the pane's session in a new window");
    let layout = app.layout();
    let windows = layout["workspace"]["windows"]
        .as_array()
        .expect("the windows");
    assert_eq!(windows.len(), 2, "{layout}");
    let second = windows[1]["focus"]
        .as_str()
        .expect("the new window's pane")
        .to_owned();
    let window = windows[1]["id"].as_str().expect("a window id").to_owned();
    assert_ne!(second, first);
    // One session, shown twice: no shell was started for the new pane.
    assert_eq!(
        app.session_pids().len(),
        2,
        "a shell was started for the new pane"
    );

    // What is typed in either pane reaches the one shell.
    app.type_line_in(&second, &(shell.say)("tw0"));
    app.wait_text_in(&first, &format!("lume-{MARK}-tw0"));
    app.type_line_in(&first, &(shell.say)("tw1"));
    app.wait_text_in(&second, &format!("lume-{MARK}-tw1"));

    // Closing the window of one of them leaves the shell to the other.
    let close = format!(r#"{{"type":"window_close","window":"{window}"}}"#);
    app.ctl(&["layout", "apply", &close])
        .ok("closing the second window");
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        process_alive(pid),
        "the shell ended with one of its two panes"
    );
    app.type_line_in(&first, &(shell.say)("tw2"));
    app.wait_text_in(&first, &format!("lume-{MARK}-tw2"));

    // Closing the last one ends it.
    app.ctl(&["command", "lume.pane.close", &args])
        .ok("closing the last pane");
    until_gone(
        pid,
        Duration::from_secs(15),
        "the shell outlived the last pane that showed it",
    );
    app.quit();
}

/// A full-screen editor to run, and its arguments: one that reads no configuration and
/// keeps nothing of what is typed. On Windows, the one Git brings.
fn full_screen_editor() -> Option<(String, &'static [&'static str])> {
    const ARGS: &[&str] = &["-u", "NONE", "-i", "NONE", "-N"];
    if cfg!(windows) {
        let program_files = std::env::var_os("ProgramFiles")?;
        let vim = Path::new(&program_files)
            .join("Git")
            .join("usr")
            .join("bin")
            .join("vim.exe");
        vim.is_file()
            .then(|| (vim.to_string_lossy().into_owned(), ARGS))
    } else {
        ["vim", "vi"]
            .into_iter()
            .find(|program| on_path(program))
            .map(|program| (program.to_owned(), ARGS))
    }
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_full_screen_program_comes_back_after_a_crash_and_takes_input() {
    let Some((editor, args)) = full_screen_editor() else {
        eprintln!("skipped: there is no vim on this machine");
        return;
    };
    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    let endpoint = format!("e2e-{}-{number}", std::process::id());
    let _stops = HostGuard(endpoint.clone());
    let home = std::env::temp_dir().join(format!("lume-e2e-{}-{number}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("config")).expect("a folder for the configuration");
    std::fs::write(
        home.join("config").join("config.toml"),
        configuration() + "\n[host]\nmode = \"daemon\"\nidle-exit = 2\n",
    )
    .expect("the configuration is written");

    let app = App::start_on(home.clone(), true, endpoint.clone());
    app.status_with_session();
    let mut spawn = vec!["spawn", "--tab", "--", editor.as_str()];
    spawn.extend_from_slice(args);
    let pane = pane_of(&app.ctl(&spawn).ok("starting the editor"));
    app.ctl(&["send", "--pane", &pane, "--escapes", "ifu11-k7q-before\\e"])
        .ok("typing into the editor");
    app.wait_text_in(&pane, "fu11-k7q-before");
    let before = app.ctl(&["capture", "--pane", &pane]).ok("capture");
    assert_eq!(
        before["alternate"], true,
        "the editor is not full screen: {before}"
    );
    let pid = app.pid_of(&pane);

    // The application is ended as a crash would end it, with the editor on its screen.
    app.kill();
    std::thread::sleep(Duration::from_secs(1));
    assert!(process_alive(pid), "the editor ended with the application");

    // The next start shows the editor as it was, and the editor takes what is typed.
    let app = App::start_on(home, false, endpoint);
    app.status_with_session();
    assert_eq!(app.pid_of(&pane), pid, "the pane started another program");
    let after = app.ctl(&["capture", "--pane", &pane]).ok("capture");
    assert_eq!(after["alternate"], true, "{after}");
    assert_eq!(
        after["lines"], before["lines"],
        "the screen came back otherwise"
    );
    app.ctl(&["send", "--pane", &pane, "--escapes", "ofu11-k7q-after\\e"])
        .ok("typing into the editor again");
    app.wait_text_in(&pane, "fu11-k7q-after");
    app.ctl(&["send", "--pane", &pane, "--escapes", ":q!\\r"])
        .ok("leaving the editor");
    until_gone(pid, Duration::from_secs(15), "the editor did not end");
    app.quit_with(&["command", "lume.app.quit-end-sessions"]);
}

/// A client of the application's control endpoint that is shown a session, grants the
/// output a window would grant at first, and then reads nothing more: a window that
/// hangs. It lasts as long as what is returned is kept.
fn hanging_view(endpoint: &str, session: &str) -> (tokio::runtime::Runtime, lume_ipc::Stream) {
    use lume_proto::app::{APP_PROTO_VERSION, AppMessage, AppRequest, AppRequestBody};
    use lume_proto::{Body, ClientKind, ControlDecoder, Message, TermSize, encode};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let stream = runtime.block_on(async {
        let endpoint = lume_ipc::Endpoint::named(endpoint).expect("an endpoint name");
        let mut io = lume_ipc::connect(&endpoint)
            .await
            .expect("the endpoint is served");
        let mut decoder = ControlDecoder::new();
        let requests = [
            AppRequestBody::Hello {
                proto: APP_PROTO_VERSION,
                proto_min: APP_PROTO_VERSION,
                client_kind: ClientKind::Test,
            },
            AppRequestBody::Attach {
                pane: None,
                session: Some(session.parse().expect("a session id")),
                size: TermSize::new(80, 24).expect("a size"),
            },
        ];
        for (id, body) in (1..).zip(requests) {
            let mut out = Vec::new();
            AppMessage::Request(AppRequest { id, body })
                .encode(&mut out)
                .expect("the request encodes");
            io.write_all(&out).await.expect("the request is sent");
            let mut chunk = vec![0; 16 * 1024];
            let json = loop {
                if let Some(json) = decoder.next().expect("the answer decodes") {
                    break json;
                }
                let read = io.read(&mut chunk).await.expect("the answer is read");
                assert_ne!(read, 0, "the application closed the connection");
                decoder.push(&chunk[..read]);
            };
            match AppMessage::decode(&json).expect("an answer") {
                AppMessage::Response(response) => {
                    assert!(response.result.is_ok(), "{:?}", response.result);
                }
                other => panic!("not an answer: {other:?}"),
            }
        }
        let mut out = Vec::new();
        encode(
            &Message::new(1, Body::Credit(lume_proto::tuning::CREDIT_WINDOW)),
            &mut out,
        )
        .expect("the credit encodes");
        io.write_all(&out).await.expect("the credit is sent");
        io
    });
    (runtime, stream)
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_window_goes_on_while_another_view_of_its_session_hangs() {
    let shell = shells().into_iter().next().expect("a shell to test with");
    let app = App::start();
    app.status_with_session();
    let spawned = start_at(&app, &shell, MARK, &["--tab"]);
    let pane = pane_of(&spawned);
    let session = spawned["session"]
        .as_str()
        .expect("a session id")
        .to_owned();
    app.type_line_in(&pane, &(shell.say)("ready"));
    app.wait_text_in(&pane, &format!("lume-{MARK}-ready"));

    // Another view of the session takes some output and then hangs, while the session
    // prints megabytes.
    let hanging = hanging_view(&app.endpoint, &session);
    app.type_line_in(&pane, shell.flood);
    app.type_line_in(&pane, &(shell.say)("f1ooded"));
    app.wait_text_in(&pane, &format!("lume-{MARK}-f1ooded"));
    app.type_line_in(&pane, &(shell.say)("after"));
    app.wait_text_in(&pane, &format!("lume-{MARK}-after"));
    drop(hanging);

    // The view that hung was left behind rather than holding the session up.
    let log = app.quit();
    let written = std::fs::read_to_string(&log).expect("the application wrote its log");
    assert!(
        written.contains("a client fell behind"),
        "the hanging view was never left behind"
    );
}

/// A test plugin's package, from `tests/plugins`.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/plugins")
        .join(name)
}

/// Installs a test plugin into the folders of `home`, as `lume plugin install` does.
fn install_plugin(home: &Path, package: &Path) {
    let installed = Command::new(lume())
        .args(["plugin", "install"])
        .arg(package)
        .arg("--yes")
        .env("LUME_CONFIG_DIR", home.join("config"))
        .env("LUME_STATE_DIR", home.join("state"))
        .output()
        .expect("lume runs");
    assert!(
        installed.status.success(),
        "the plugin is installed: {}{}",
        String::from_utf8_lossy(&installed.stdout),
        String::from_utf8_lossy(&installed.stderr)
    );
}

/// A home with the tests' configuration and these test plugins installed in it.
fn home_with_plugins(names: &[&str]) -> PathBuf {
    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    let home = std::env::temp_dir().join(format!("lume-e2e-{}-{number}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("config")).expect("a folder for the configuration");
    std::fs::write(
        home.join("config").join("config.toml"),
        configuration() + IN_PROCESS_HOST,
    )
    .expect("the configuration is written");
    for name in names {
        install_plugin(&home, &fixture(name));
    }
    home
}

impl App {
    /// The plugins as the application lists them, once it has read them.
    fn plugins(&self) -> Vec<Value> {
        let answer = self.ctl(&["plugins"]).ok("plugins");
        answer["plugins"].as_array().cloned().unwrap_or_default()
    }

    /// Waits until the application lists the plugin `id` and `check` holds for it.
    fn until_plugin(&self, id: &str, what: &str, check: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + START;
        loop {
            let found = self.plugins().into_iter().find(|plugin| plugin["id"] == id);
            if let Some(plugin) = found
                && check(&plugin)
            {
                return plugin;
            }
            assert!(Instant::now() < deadline, "{what}: {:?}", self.plugins());
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_plugins_command_runs_its_code_and_what_it_shows_is_kept() {
    let home = home_with_plugins(&["hello"]);
    let app = App::start_in(home, false);
    app.until_plugin("test.hello", "the plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });
    // Nothing of the plugins was read before the terminal's first output.
    let status = app.status_with_session();
    assert_eq!(status["marks"]["plugin_files_before_output"], 0, "{status}");

    // Its code starts with its command, which it carries out.
    assert!(app.plugins()[0]["running"].is_null());
    app.ctl(&["command", "test.hello.greet"]).ok("greet");
    app.ctl(&["command", "test.hello.greet"]).ok("greet again");
    let plugin = app.until_plugin("test.hello", "the count to be shown", |plugin| {
        plugin["status"][0]["value"]["text"] == "2"
    });
    assert_eq!(plugin["running"]["started"], true, "{plugin}");
    assert!(
        plugin["log"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line.as_str().unwrap().ends_with("info greeted 2")),
        "{plugin}"
    );
    let kv = app.home.join("state/plugins/data/test.hello/kv.json");
    assert_eq!(std::fs::read_to_string(kv).unwrap(), r#"{"count":2}"#);

    // Its command is in the registry, named under its id.
    let commands = app.ctl(&["commands"]).ok("commands");
    assert!(
        commands["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command["id"] == "test.hello.greet" && command["runs"] == "plugin")
    );
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_plugins_meters_are_read_only_while_they_are_on_screen() {
    let home = home_with_plugins(&["meters"]);
    let app = App::start_in(home, false);
    app.status_with_session();
    // On screen in the status bar, where their setting puts them, the meters start the
    // plugin, which asks for readings and shows them.
    let plugin = app.until_plugin("test.meters", "the meters to be read and shown", |plugin| {
        plugin["metrics"]["reading"] == true
            && plugin["status"].as_array().is_some_and(|status| {
                status
                    .iter()
                    .any(|one| one["item"] == "cpu" && one["value"]["text"].is_string())
            })
    });
    let visible = plugin["visible"].as_array().cloned().unwrap_or_default();
    for item in ["cpu", "memory", "gpu", "network"] {
        assert!(
            visible.contains(&Value::from(format!("test.meters.{item}"))),
            "{plugin}"
        );
    }
    assert_eq!(plugin["warnings"], Value::Array(Vec::new()), "{plugin}");

    // Turned off in the settings, they leave the screen without the plugin's code, and
    // the machine is no longer read.
    let file = app.home.join("config").join("config.toml");
    let mut text = std::fs::read_to_string(&file).unwrap();
    text.push_str(
        "
[plugins.\"test.meters\"]
cpu = false
memory = false
gpu = false
network = false
",
    );
    std::fs::write(&file, text).unwrap();
    app.until_plugin("test.meters", "the reading to stop", |plugin| {
        plugin["metrics"]["reading"] == false
            && plugin["visible"].as_array().is_some_and(Vec::is_empty)
    });
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_plugin_that_runs_away_is_stopped_while_the_terminal_keeps_up() {
    let app = App::start();
    app.status_with_session();
    // Installed while the application runs: it follows the lock file.
    install_plugin(&app.home, &fixture("runaway-loop"));
    install_plugin(&app.home, &fixture("runaway-alloc"));
    app.until_plugin(
        "test.runaway-loop",
        "the plugin installed while running to be read",
        |plugin| plugin["state"] == "ready",
    );

    // Memory without end stops the plugin; the terminal answers all along.
    let grow = app.ctl(&["command", "test.runaway-alloc.grow"]);
    assert_eq!(grow.status, 1, "{}", grow.json);
    app.until_plugin("test.runaway-alloc", "the breach to be counted", |plugin| {
        plugin["breaches"] == 1
    });

    for round in 1..=3 {
        let sent = Instant::now();
        let spin = app.ctl(&["command", "test.runaway-loop.spin"]);
        assert_eq!(spin.status, 1, "{}", spin.json);
        // The loop was stopped within its budget, well before the command's own wait.
        assert!(
            sent.elapsed() < Duration::from_secs(5),
            "round {round}: {:?}",
            sent.elapsed()
        );
        let line = format!("echo {MARK}{round}");
        let typed = Instant::now();
        app.type_line(&line);
        app.wait_text(&format!("{MARK}{round}"));
        assert!(
            typed.elapsed() < Duration::from_secs(3),
            "round {round}: the terminal was slow"
        );
    }
    // Three times in a minute and it is turned off.
    app.until_plugin(
        "test.runaway-loop",
        "the plugin to be turned off",
        |plugin| {
            plugin["state"] == "inactive"
                && plugin["reason"]
                    .as_str()
                    .unwrap_or("")
                    .contains("turned off")
        },
    );
    let lock = std::fs::read_to_string(app.home.join("state/plugins/lock.toml")).unwrap();
    assert!(lock.contains("enabled = false"), "{lock}");
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_package_changed_after_it_was_installed_is_not_used() {
    let home = home_with_plugins(&["hello"]);
    std::fs::write(
        home.join("state/plugins/packages/test.hello/1.0.0/host.js"),
        "lume.commands.register('greet', () => 'something else');",
    )
    .unwrap();
    let app = App::start_in(home, false);
    let plugin = app.until_plugin("test.hello", "the changed plugin to be read", |plugin| {
        plugin["state"] == "inactive"
    });
    assert!(
        plugin["reason"].as_str().unwrap().contains("changed"),
        "{plugin}"
    );
    // Its command is not there to run.
    let greet = app.ctl(&["command", "test.hello.greet"]);
    assert_eq!(greet.status, 1);
    assert_eq!(greet.json["code"], "not_found");
    app.quit();
}
