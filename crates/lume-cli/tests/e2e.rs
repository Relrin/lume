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
        Self::start_on_with(home, keep_home, endpoint, &[])
    }

    /// The same, with these variables added to the application's environment.
    fn start_on_with(
        home: PathBuf,
        keep_home: bool,
        endpoint: String,
        environment: &[(&str, String)],
    ) -> Self {
        let log = std::env::temp_dir().join(format!("lume-{endpoint}.log"));
        let _ = std::fs::remove_file(&log);
        let child = Command::new(application())
            .args(["--endpoint", &endpoint])
            .env("LUME_LOG", "trace")
            .env("LUME_LOG_FILE", &log)
            .env("LUME_CONFIG_DIR", home.join("config"))
            .env("LUME_STATE_DIR", home.join("state"))
            .envs(environment.iter().map(|(name, value)| (*name, value)))
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
    // The text is on the screen before the editor has taken the Escape after it: its
    // last line says it is still inserting until then, and would not say so afterwards.
    let deadline = Instant::now() + Duration::from_secs(15);
    let before = loop {
        let captured = app.ctl(&["capture", "--pane", &pane]).ok("capture");
        let inserting = captured["lines"].as_array().is_some_and(|lines| {
            lines
                .iter()
                .any(|line| line.to_string().contains("-- INSERT"))
        });
        if !inserting {
            break captured;
        }
        assert!(
            Instant::now() < deadline,
            "the editor never left its insert mode: {captured}"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
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
fn a_panels_rows_come_when_it_is_shown_and_a_rows_own_when_it_is_opened() {
    let home = home_with_plugins(&["tree"]);
    let app = App::start_in(home, false);
    app.status_with_session();
    let plugin = app.until_plugin("test.tree", "the plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });
    // Its panel is not on show, so its code has not started.
    assert!(plugin["running"].is_null(), "{plugin}");

    // Shown, the panel starts the plugin, which gives it its first rows: two folders
    // and a file, and nothing of what is in the folders.
    app.ctl(&[
        "command",
        "lume.panel.show",
        r#"{"panel":"test.tree.files"}"#,
    ])
    .ok("show the panel");
    let plugin = app.until_plugin("test.tree", "the panel's rows", |plugin| {
        plugin["rows"]["files"] == 3
    });
    assert!(
        plugin["visible"]
            .as_array()
            .unwrap()
            .contains(&Value::from("test.tree.files")),
        "{plugin}"
    );

    // A folder opened is asked for, and its rows join the panel's: a folder and two
    // files. So with the folder inside it.
    let expand = |path: &str| {
        let args = format!(r#"{{"panel":"test.tree.files","path":{path}}}"#);
        app.ctl(&["command", "lume.panel.expand", &args])
            .ok("open a row");
    };
    expand(r#"["src"]"#);
    app.until_plugin("test.tree", "the rows of the folder", |plugin| {
        plugin["rows"]["files"] == 6
    });
    expand(r#"["src","nested"]"#);
    let plugin = app.until_plugin("test.tree", "the rows of the folder inside it", |plugin| {
        plugin["rows"]["files"] == 7
    });
    assert!(logged(&plugin, "asked for src/nested"), "{plugin}");

    // A row that is not there is refused, and nothing changes.
    let args = r#"{"panel":"test.tree.none","path":["src"]}"#;
    assert_ne!(
        app.ctl(&["command", "lume.panel.expand", args]).status,
        0,
        "a panel no plugin adds has no rows to open"
    );
    assert_eq!(app.plugins()[0]["rows"]["files"], 7);
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

impl App {
    /// Runs `lume ctl` with these words on a thread of its own, for a request that waits
    /// for something the test does meanwhile.
    fn ctl_in_background(&self, args: &[&str]) -> std::thread::JoinHandle<Answer> {
        let endpoint = self.endpoint.clone();
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        std::thread::spawn(move || {
            let words: Vec<&str> = args.iter().map(String::as_str).collect();
            ctl_at(&endpoint, &words)
        })
    }

    /// Answers the question the window has on show as the user's press would: going
    /// ahead, or not. Waits for the question to be put.
    fn answer_prompt(&self, proceed: bool) {
        let args = serde_json::json!({ "proceed": proceed }).to_string();
        let deadline = Instant::now() + START;
        loop {
            let answer = self.ctl(&["command", "lume.window.answer", &args]);
            if answer.status == 0 {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "no question was put to the user: {}",
                answer.json
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Whether a plugin, as `ctl plugins` lists it, wrote a line holding `text` to its log.
fn logged(plugin: &Value, text: &str) -> bool {
    plugin["log"].as_array().is_some_and(|lines| {
        lines
            .iter()
            .any(|line| line.as_str().is_some_and(|line| line.contains(text)))
    })
}

/// Makes `link` a link to the folder `target`: a junction on Windows, which needs no
/// privilege, a symbolic link elsewhere.
fn link_folder(link: &Path, target: &Path) {
    #[cfg(windows)]
    {
        let made = Command::new("cmd")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .expect("cmd runs");
        assert!(
            made.status.success(),
            "the junction is made: {}{}",
            String::from_utf8_lossy(&made.stdout),
            String::from_utf8_lossy(&made.stderr)
        );
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).expect("the link is made");
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_plugin_is_refused_everything_it_was_not_given() {
    let home = home_with_plugins(&["denied"]);
    let app = App::start_in(home, false);
    app.status_with_session();
    app.until_plugin("test.denied", "the plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });
    // Its command tries every call that needs a permission, and fails if one is not
    // refused as denied.
    app.ctl(&["command", "test.denied.probe"]).ok("the probe");
    app.until_plugin("test.denied", "its report", |plugin| {
        logged(plugin, "refused all")
    });
    // Each refusal is in the audit log, which holds what was asked and nothing of the
    // data.
    let audit = std::fs::read_to_string(app.home.join("state/plugins/audit.log")).unwrap();
    let refused: Vec<Value> = audit
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry["plugin"] == "test.denied" && entry["outcome"] == "denied")
        .collect();
    assert!(refused.len() >= 20, "{audit}");
    for entry in &refused {
        let fields: Vec<&String> = entry.as_object().unwrap().keys().collect();
        assert_eq!(
            fields,
            ["capability", "outcome", "plugin", "scope", "time"],
            "{entry}"
        );
    }
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_plugin_opens_a_terminal_only_with_leave_to_start_sessions_and_run_the_program() {
    if !on_path("git") {
        println!("skipped: there is no git on this machine");
        return;
    }
    let home = home_with_plugins(&["splitter", "opener"]);
    let app = App::start_in(home, false);
    app.status_with_session();
    for id in ["test.splitter", "test.opener"] {
        app.until_plugin(id, "the plugin to be read", |plugin| {
            plugin["state"] == "ready"
        });
    }
    let panes = |app: &App| {
        app.ctl(&["list"]).ok("list")["sessions"]
            .as_array()
            .map_or(0, Vec::len)
    };
    let at_first = panes(&app);
    // Leave to change the layout is not leave to start what a terminal runs.
    app.ctl(&["command", "test.splitter.probe"]).ok("the probe");
    app.until_plugin("test.splitter", "its report", |plugin| {
        logged(plugin, "refused all")
    });
    assert_eq!(panes(&app), at_first, "a refused terminal was opened");
    // With both, the program the plugin may run is started, and no other.
    app.ctl(&["command", "test.opener.probe"]).ok("the probe");
    app.until_plugin("test.opener", "its report", |plugin| {
        logged(plugin, "opened one and refused the rest")
    });
    let audit = std::fs::read_to_string(app.home.join("state/plugins/audit.log")).unwrap();
    let entries: Vec<Value> = audit
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect();
    assert!(
        entries
            .iter()
            .any(|entry| entry["plugin"] == "test.splitter"
                && entry["capability"] == "sessions.spawn"
                && entry["outcome"] == "denied"),
        "{audit}"
    );
    assert!(
        entries.iter().any(|entry| entry["plugin"] == "test.opener"
            && entry["capability"] == "process.run"
            && entry["scope"] == "ssh"
            && entry["outcome"] == "denied"),
        "{audit}"
    );
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_plugin_gives_a_pane_its_own_picture_or_a_tile_and_no_other_plugins() {
    let home = home_with_plugins(&["marker"]);
    let app = App::start_in(home, false);
    app.status_with_session();
    app.until_plugin("test.marker", "the plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });
    // The window's one tab holds its one pane.
    let icon = |app: &App| {
        app.layout()["workspace"]["windows"][0]["centre"]["tabs"][0]["root"]["icon"].clone()
    };
    assert_eq!(icon(&app), Value::Null);

    // Its own picture is kept under the plugin's name, whatever else it asked for.
    app.ctl(&["command", "test.marker.own"]).ok("its own");
    app.until_plugin("test.marker", "its report", |plugin| {
        logged(plugin, "gave its own and was refused another's")
    });
    let own = icon(&app);
    let own = own.as_str().expect("the pane has a picture");
    assert!(own.starts_with("image:") && own.ends_with(":mark"), "{own}");

    app.ctl(&["command", "test.marker.tile"]).ok("a tile");
    app.until_plugin("test.marker", "its report", |plugin| {
        logged(plugin, "gave a tile")
    });
    assert_eq!(icon(&app), "agent");

    app.ctl(&["command", "test.marker.clear"]).ok("none");
    app.until_plugin("test.marker", "its report", |plugin| {
        logged(plugin, "took it away")
    });
    assert_eq!(icon(&app), Value::Null);
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_plugin_that_waits_for_sessions_hears_of_those_that_were_there_before_it() {
    let home = home_with_plugins(&["listener"]);
    let app = App::start_in(home, false);
    // The first terminal runs, and has reported, before the plugins are read: nothing
    // it says afterwards would start a plugin that waits for it.
    app.status_with_session();
    app.until_plugin(
        "test.listener",
        "the plugin to hear of the first terminal",
        |plugin| logged(plugin, "heard the state of p"),
    );
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_plugin_cannot_reach_past_what_it_was_given() {
    let home = home_with_plugins(&["escapes"]);
    // A link in the plugin's own data folder to the plugins folder, which holds the
    // lock file: following it leaves what the plugin was given.
    let plugins = home.join("state").join("plugins");
    let data = plugins.join("data").join("test.escapes");
    std::fs::create_dir_all(&data).unwrap();
    link_folder(&data.join("link"), &plugins);
    let app = App::start_in(home, false);
    app.status_with_session();
    app.until_plugin("test.escapes", "the plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });
    app.ctl(&["command", "test.escapes.probe"]).ok("the probe");
    app.until_plugin("test.escapes", "its report", |plugin| {
        logged(plugin, "refused all 8")
    });
    assert!(
        !std::env::home_dir()
            .unwrap()
            .join("lume-escape.txt")
            .exists()
    );
    app.quit();
}

/// Installs a test plugin under another id, beside any install of its own id.
fn install_plugin_as(home: &Path, package: &Path, id: &str) {
    let installed = Command::new(lume())
        .args(["plugin", "install"])
        .arg(package)
        .args(["--as", id, "--yes"])
        .env("LUME_CONFIG_DIR", home.join("config"))
        .env("LUME_STATE_DIR", home.join("state"))
        .output()
        .expect("lume runs");
    assert!(
        installed.status.success(),
        "the plugin is installed as {id}: {}{}",
        String::from_utf8_lossy(&installed.stdout),
        String::from_utf8_lossy(&installed.stderr)
    );
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn twenty_installed_plugins_that_do_not_run_are_not_read_before_the_first_output() {
    let home = home_with_plugins(&[]);
    for number in 0..20 {
        install_plugin_as(&home, &fixture("hello"), &format!("test.many{number:02}"));
    }
    let app = App::start_in(home, false);
    let status = app.status_with_session();
    assert_eq!(status["marks"]["plugin_files_before_output"], 0, "{status}");
    // All of them are read afterwards, and none of them runs: each waits for its
    // command.
    app.until_plugin("test.many19", "the last plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });
    let plugins = app.plugins();
    assert_eq!(plugins.len(), 20);
    for plugin in &plugins {
        assert_eq!(plugin["state"], "ready", "{plugin}");
        assert!(plugin["running"].is_null(), "{plugin}");
        assert_eq!(plugin["starts"], 0, "{plugin}");
    }
    // Still nothing of them counted against the start, now that they are read.
    let status = app.ctl(&["status"]).ok("status");
    assert_eq!(status["marks"]["plugin_files_before_output"], 0, "{status}");
    assert!(status["marks"]["plugins_ms"].is_number(), "{status}");

    // One of them still does its work when asked, under its own name.
    app.ctl(&["command", "test.many07.greet"]).ok("greet");
    let plugin = app.until_plugin("test.many07", "the count to be shown", |plugin| {
        plugin["status"][0]["value"]["text"] == "1"
    });
    assert_eq!(plugin["starts"], 1, "{plugin}");
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_plugin_uses_a_service_a_watch_a_running_program_and_a_panes_figures() {
    let home = home_with_plugins(&["provider", "worker"]);
    let app = App::start_in(home, false);
    let status = app.status_with_session();
    let pane = status["focused"]["pane"].as_str().unwrap().to_owned();
    app.until_plugin("test.worker", "the plugins to be read", |plugin| {
        plugin["state"] == "ready"
    });
    let provider = app.until_plugin("test.provider", "the provider to be read", |plugin| {
        plugin["state"] == "ready"
    });
    assert!(provider["running"].is_null(), "{provider}");

    let args = serde_json::json!({ "pane": pane }).to_string();
    app.ctl(&["command", "test.worker.work", &args])
        .ok("the work");
    let worker = app.until_plugin("test.worker", "its report", |plugin| {
        logged(plugin, "service 42 from test.provider;")
    });
    assert!(logged(&worker, "watched true"), "{worker}");
    assert!(logged(&worker, "sorted a,b exit 0"), "{worker}");
    assert!(logged(&worker, "processes true memory true"), "{worker}");
    // The provider was started to answer.
    let provider = app.until_plugin("test.provider", "the provider to run", |plugin| {
        plugin["running"]["started"] == true
    });
    assert_eq!(provider["starts"], 1, "{provider}");

    // Bound to another install of the provider, the worker's next call is put to the
    // user first, and then answered by that install.
    install_plugin_as(&app.home, &fixture("provider"), "test.provider2");
    let bound = Command::new(lume())
        .args([
            "plugin",
            "bind",
            "test.worker",
            "test.provider=test.provider2",
        ])
        .env("LUME_CONFIG_DIR", app.home.join("config"))
        .env("LUME_STATE_DIR", app.home.join("state"))
        .output()
        .expect("lume runs");
    assert!(
        bound.status.success(),
        "{}{}",
        String::from_utf8_lossy(&bound.stdout),
        String::from_utf8_lossy(&bound.stderr)
    );
    app.until_plugin(
        "test.provider2",
        "the other provider to be read",
        |plugin| plugin["state"] == "ready",
    );
    let again = app.ctl_in_background(&["command", "test.worker.work", &args]);
    app.answer_prompt(true);
    again.join().unwrap().ok("the work, bound anew");
    app.until_plugin("test.worker", "its report", |plugin| {
        logged(plugin, "service 42 from test.provider2;")
    });
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_plugin_asks_while_it_runs_and_the_user_answers_in_the_window() {
    let home = home_with_plugins(&["asker"]);
    let app = App::start_in(home, false);
    let status = app.status_with_session();
    let pane = status["focused"]["pane"].as_str().unwrap().to_owned();
    app.until_plugin("test.asker", "the plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });

    // A permission it may ask for later: the window puts the question, the user allows
    // it, and it is kept.
    let asked = app.ctl_in_background(&["command", "test.asker.request"]);
    app.answer_prompt(true);
    asked.join().unwrap().ok("the request");
    app.until_plugin("test.asker", "what it was let see", |plugin| {
        logged(plugin, "granted true") && logged(plugin, "workspace ")
    });
    let lock = std::fs::read_to_string(app.home.join("state/plugins/lock.toml")).unwrap();
    assert!(lock.contains("workspace.read"), "{lock}");

    // Typing, which the user approves each time: once allowed, and once not.
    let text = serde_json::json!({ "pane": pane, "text": "echo lume-asked\r" }).to_string();
    let typed = app.ctl_in_background(&["command", "test.asker.type", &text]);
    app.answer_prompt(true);
    typed.join().unwrap().ok("typing");
    app.wait_text_in(&pane, "lume-asked");
    let refused = app.ctl_in_background(&["command", "test.asker.type", &text]);
    app.answer_prompt(false);
    let refused = refused.join().unwrap();
    assert_ne!(refused.status, 0, "{}", refused.json);
    assert!(
        refused.json.to_string().contains("did not allow"),
        "{}",
        refused.json
    );
    app.quit();
}

/// Keeps the first line of every request made to a listener on this machine, from a
/// thread of its own.
fn counting_listener() -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use std::io::Read;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a listener");
    let port = listener.local_addr().expect("its address").port();
    let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let hearing = std::sync::Arc::clone(&heard);
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
            let mut first = [0u8; 256];
            let read = stream.read(&mut first).unwrap_or(0);
            let line = String::from_utf8_lossy(&first[..read])
                .lines()
                .next()
                .unwrap_or("(nothing read)")
                .to_owned();
            hearing.lock().unwrap().push(line);
        }
    });
    (port, heard)
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_hostile_view_reaches_nothing_but_its_own_host_half() {
    let (port, connections) = counting_listener();
    let home = home_with_plugins(&["hostile"]);
    let config = home.join("config").join("config.toml");
    let mut written = std::fs::read_to_string(&config).unwrap();
    written.push_str(&format!("\n[plugins.\"test.hostile\"]\nport = {port}\n"));
    std::fs::write(&config, written).unwrap();
    let app = App::start_in(home, false);
    app.status_with_session();
    app.until_plugin("test.hostile", "the plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });
    let sessions_before = app.ctl(&["list"]).ok("list")["sessions"]
        .as_array()
        .map_or(0, Vec::len);
    app.ctl(&[
        "command",
        "lume.panel.show",
        r#"{"panel":"test.hostile.probe"}"#,
    ])
    .ok("show the probe");
    // The page's reports go to its plugin's log.
    let log = app.home.join("state/plugins/logs/test.hostile.log");
    let deadline = Instant::now() + START;
    let report = loop {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        if text.contains("probe done") {
            break text;
        }
        assert!(Instant::now() < deadline, "the probe's report: {text}");
        std::thread::sleep(Duration::from_millis(200));
    };
    let lines: Vec<&str> = report
        .lines()
        .filter(|line| line.contains("probe A"))
        .collect();
    assert!(lines.len() >= 40, "every try is reported:\n{report}");
    let reached: Vec<&&str> = lines
        .iter()
        .filter(|line| line.contains(" reached:"))
        .collect();
    assert!(
        reached.is_empty(),
        "these got through:\n{reached:#?}\n\n{report}"
    );
    assert!(
        report.contains("view opened probe in a panel"),
        "the plugin heard its view open:\n{report}"
    );
    // The application's commands it tried had no effect: no tab was opened and no
    // setting written.
    let sessions_after = app.ctl(&["list"]).ok("list")["sessions"]
        .as_array()
        .map_or(0, Vec::len);
    assert_eq!(sessions_before, sessions_after, "the view opened no tab");
    let config = std::fs::read_to_string(app.home.join("config").join("config.toml")).unwrap();
    assert!(
        !config.contains("font-size = 31"),
        "the view wrote no setting:
{config}"
    );
    // The page then tried to send its own frame away; neither that nor anything before
    // it reached the listener.
    std::thread::sleep(Duration::from_secs(2));
    let heard = connections.lock().unwrap().clone();
    assert!(
        heard.is_empty(),
        "nothing from the view reaches the network: {heard:#?}"
    );
    let log_file = app.log.clone();
    app.quit();
    // Whether the system's channel carried the view's requests as far as the
    // application, where the page's token turned them away, is said for the record.
    let log = std::fs::read_to_string(&log_file).unwrap_or_default();
    let refused = log
        .lines()
        .filter(|line| line.contains("a command without its page's token was refused"))
        .count();
    eprintln!("requests turned away for want of the page's token: {refused}");
}

/// What a test plugin has written to its log once `mark` is in it after `from` bytes.
fn plugin_log_until(home: &Path, id: &str, from: usize, mark: &str) -> String {
    let log = home.join(format!("state/plugins/logs/{id}.log"));
    let deadline = Instant::now() + START;
    loop {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        let fresh = text.get(from..).unwrap_or_default();
        if fresh.contains(mark) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "{id} never logged {mark:?} (from byte {from}):\n{text}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// How much a test plugin has written to its log so far.
fn plugin_log_len(home: &Path, id: &str) -> usize {
    std::fs::read_to_string(home.join(format!("state/plugins/logs/{id}.log")))
        .unwrap_or_default()
        .len()
}

/// The panes of the layout that show a plugin's view, as (pane, view, what it kept).
fn view_panes(layout: &Value) -> Vec<(String, String, Value)> {
    fn walk(node: &Value, found: &mut Vec<(String, String, Value)>) {
        if node["type"] == "pane" {
            if node["content"]["type"] == "plugin_view" {
                found.push((
                    node["id"].as_str().unwrap_or_default().to_owned(),
                    node["content"]["view"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    node["content"]["state"].clone(),
                ));
            }
            return;
        }
        for child in node["children"].as_array().into_iter().flatten() {
            walk(child, found);
        }
    }
    let mut found = Vec::new();
    for window in layout["workspace"]["windows"]
        .as_array()
        .into_iter()
        .flatten()
    {
        for tab in window["centre"]["tabs"].as_array().into_iter().flatten() {
            walk(&tab["root"], &mut found);
        }
        for floating in window["floating"].as_array().into_iter().flatten() {
            let mut pane = floating["pane"].clone();
            pane["type"] = Value::from("pane");
            walk(&pane, &mut found);
        }
    }
    found
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_plugins_view_keeps_its_state_and_uses_its_plugins_streams() {
    let (port, _connections) = counting_listener();
    let home = home_with_plugins(&["echo-view", "hostile"]);
    let config = home.join("config").join("config.toml");
    let mut written = std::fs::read_to_string(&config).unwrap();
    written.push_str(&format!("\n[plugins.\"test.hostile\"]\nport = {port}\n"));
    std::fs::write(&config, &written).unwrap();
    let endpoint = format!("e2e-{}-views", std::process::id());
    let app = App::start_on(home.clone(), true, endpoint.clone());
    app.status_with_session();
    for id in ["test.echo-view", "test.hostile"] {
        app.until_plugin(id, "the plugin to be read", |plugin| {
            plugin["state"] == "ready"
        });
    }

    // A second view of the same plugin, in a panel, for the first to be told apart from.
    app.ctl(&[
        "command",
        "lume.panel.show",
        r#"{"panel":"test.echo-view.side"}"#,
    ])
    .ok("show the side panel");
    plugin_log_until(&home, "test.echo-view", 0, "echo started panel count 0");

    // The view in a pane, opened by the application's command, with what it starts with.
    app.ctl(&[
        "command",
        "lume.view.open",
        r#"{"plugin":"test.echo-view","view":"echo","state":{"task":"streams","count":4}}"#,
    ])
    .ok("open the view");
    let log = plugin_log_until(&home, "test.echo-view", 0, "echo done");
    for said in [
        "view opened echo in a pane",
        "echo started pane count 4",
        // A kind of failure the application knows nothing of, as the code named it.
        "echo own echo.own-kind {\"count\":7} a failure of its own kind",
        // A program's input and output, used by the view itself.
        "echo sorted a,b,c ended true",
        "echo ways output true/false input false/true",
        // A grant is opened once, and only by the view it was made for.
        "echo reopen refused not_found",
        "echo foreign refused not_found",
        "echo guessed refused not_found",
        // One that is taken back ends its stream.
        "echo revoked ended false",
    ] {
        assert!(log.contains(said), "{said:?} is not in:\n{log}");
    }
    assert!(!log.contains("echo failed"), "{log}");

    // Another plugin's view is handed a real grant of this one's, and gets nowhere.
    let spare = log
        .lines()
        .find_map(|line| line.split("spare grant ").nth(1))
        .expect("the view left a grant unopened")
        .trim()
        .to_owned();
    written.push_str(&format!("grant = \"{spare}\"\n"));
    std::fs::write(&config, &written).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let sessions_before = app.ctl(&["list"]).ok("list")["sessions"]
        .as_array()
        .map_or(0, Vec::len);
    app.ctl(&[
        "command",
        "lume.panel.show",
        r#"{"panel":"test.hostile.probe"}"#,
    ])
    .ok("show the probe");
    let probe = plugin_log_until(&home, "test.hostile", 0, "probe done");
    assert!(
        probe.contains("probe A14.foreign-grant blocked"),
        "another plugin's stream stays out of reach:\n{probe}"
    );
    assert!(!probe.contains(" reached:"), "{probe}");
    // Nor did the keys it made up open, close or start anything.
    assert!(probe.contains("probe A13.forged-keys noted"), "{probe}");
    let sessions_after = app.ctl(&["list"]).ok("list")["sessions"]
        .as_array()
        .map_or(0, Vec::len);
    assert_eq!(sessions_before, sessions_after, "forged keys ran nothing");

    // What the view asked to have kept is in the layout, with its pane.
    let deadline = Instant::now() + START;
    let pane = loop {
        let views = view_panes(&app.layout());
        if let Some((pane, _, state)) = views.iter().find(|(_, view, _)| view == "echo")
            && state["count"] == 5
        {
            break pane.clone();
        }
        assert!(Instant::now() < deadline, "the view's state: {views:?}");
        std::thread::sleep(Duration::from_millis(200));
    };

    // Moved to float over the window, the pane's frame starts again from what it kept.
    let at = plugin_log_len(&home, "test.echo-view");
    let float = serde_json::json!({ "type": "float", "pane": pane }).to_string();
    app.ctl(&["layout", "apply", &float]).ok("float the view");
    plugin_log_until(&home, "test.echo-view", at, "echo started pane count 5");

    // An address of the scheme the view offers to open goes to it; nothing else would
    // open it, so nobody is asked.
    let at = plugin_log_len(&home, "test.echo-view");
    app.ctl(&[
        "command",
        "lume.app.open",
        r#"{"uri":"test-echo://thing/1"}"#,
    ])
    .ok("open an address");
    plugin_log_until(
        &home,
        "test.echo-view",
        at,
        "echo started pane count 0 open test-echo://thing/1",
    );
    // A file it offers to open could also be shown in the file manager: the user is
    // asked, and chooses the view.
    let file = home.join("notes.echo");
    std::fs::write(&file, "x").unwrap();
    let asked = {
        let args = serde_json::json!({ "path": file }).to_string();
        let (lume, endpoint) = (lume(), app.endpoint.clone());
        std::thread::spawn(move || {
            Command::new(lume)
                .args([
                    "ctl",
                    "--endpoint",
                    &endpoint,
                    "command",
                    "lume.app.open",
                    &args,
                ])
                .output()
                .expect("lume runs")
        })
    };
    let choice = serde_json::json!({ "proceed": true, "item": "test.echo-view.echo" }).to_string();
    let deadline = Instant::now() + START;
    while app.ctl(&["command", "lume.window.answer", &choice]).status != 0 {
        assert!(
            Instant::now() < deadline,
            "nobody was asked what opens the file"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(asked.join().unwrap().status.success(), "the file is opened");
    let log = plugin_log_until(&home, "test.echo-view", at, "notes.echo");
    assert!(log.contains("count 0 open "), "{log}");
    // A plugin opens its own view with a call of its own, and is shown the one that is
    // open already when it asks for that.
    app.ctl(&[
        "command",
        "test.echo-view.open",
        r#"{"reuse":true,"state":{"count":50}}"#,
    ])
    .ok("the plugin opens its view");
    let log = plugin_log_until(&home, "test.echo-view", at, "opened pane");
    assert!(log[at..].contains("reused true"), "{}", &log[at..]);

    // After the application was closed and started again, each view finds what it kept.
    let kept: Vec<(String, String, Value)> = view_panes(&app.layout());
    assert_eq!(kept.len(), 3, "{kept:?}");
    app.ctl(&["command", "lume.workspace.save"]).ok("save");
    app.quit();
    let at = plugin_log_len(&home, "test.echo-view");
    let app = App::start_on(home.clone(), false, endpoint);
    // The pane with the keyboard shows a view now, so there is no session to wait for.
    app.until_plugin("test.echo-view", "the plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });
    let counts = |views: &[(String, String, Value)]| -> Vec<(String, Value)> {
        views
            .iter()
            .map(|(_, view, state)| (view.clone(), state["count"].clone()))
            .collect()
    };
    assert_eq!(
        counts(&view_panes(&app.layout())),
        counts(&kept),
        "the layout came back with what the views kept"
    );
    // The tab on show holds the view opened last: it starts from the count it kept.
    plugin_log_until(
        &home,
        "test.echo-view",
        at,
        "echo started pane count 1 open ",
    );
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_view_that_gets_in_the_way_is_stopped_or_left_to_itself() {
    let home = home_with_plugins(&["hostile"]);
    let config = home.join("config").join("config.toml");
    let base = std::fs::read_to_string(&config).unwrap();
    let app = App::start_in(home.clone(), false);
    app.status_with_session();
    app.until_plugin("test.hostile", "the plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });
    for (turn, mode) in ["flood", "large", "spin"].into_iter().enumerate() {
        std::fs::write(
            &config,
            format!("{base}\n[plugins.\"test.hostile\"]\nabuse = \"{mode}\"\n"),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        let at = plugin_log_len(&home, "test.hostile");
        app.ctl(&[
            "command",
            "lume.panel.show",
            r#"{"panel":"test.hostile.abuse"}"#,
        ])
        .ok("show the view");
        plugin_log_until(&home, "test.hostile", at, &format!("abuse {mode} started"));
        if mode == "spin" {
            // A view that only keeps its own processor busy is left to it.
            std::thread::sleep(Duration::from_secs(1));
        } else {
            // One that sends more than it may is taken down, and its plugin told.
            plugin_log_until(&home, "test.hostile", at, "view closed abuse");
        }
        // Either way the terminal beside it goes on taking keys and showing output.
        let began = Instant::now();
        let mark = format!("still-here-{turn}");
        app.type_line(&format!("echo {mark}"));
        app.wait_text(&mark);
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "the terminal answered {:?} after a view began to {mode}",
            began.elapsed()
        );
        app.ctl(&[
            "command",
            "lume.panel.close",
            r#"{"panel":"test.hostile.abuse"}"#,
        ])
        .ok("close the view");
        if mode == "spin" {
            plugin_log_until(&home, "test.hostile", at, "view closed abuse");
        }
    }
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_view_out_of_sight_is_kept_for_a_while_and_then_started_again_from_what_it_kept() {
    let home = home_with_plugins(&["echo-view"]);
    let app = App::start_in(home.clone(), false);
    app.status_with_session();
    app.until_plugin("test.echo-view", "the plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });
    app.ctl(&[
        "command",
        "lume.view.open",
        r#"{"plugin":"test.echo-view","view":"echo","state":{"count":7}}"#,
    ])
    .ok("open the view");
    plugin_log_until(&home, "test.echo-view", 0, "echo started pane count 7");
    let window = app.layout()["workspace"]["windows"][0]["id"]
        .as_str()
        .expect("the window has an id")
        .to_owned();
    let show = |index: u32| {
        let intent = serde_json::json!({
            "type": "tab_activate",
            "target": { "type": "index", "window": window, "index": index },
        })
        .to_string();
        app.ctl(&["layout", "apply", &intent]).ok("show a tab");
    };

    // Away to the terminal's tab and back at once: the view was not started again,
    // and its plugin never heard it close.
    let at = plugin_log_len(&home, "test.echo-view");
    show(0);
    std::thread::sleep(Duration::from_secs(3));
    show(1);
    std::thread::sleep(Duration::from_secs(3));
    let log = std::fs::read_to_string(home.join("state/plugins/logs/test.echo-view.log"))
        .unwrap_or_default();
    assert!(
        !log[at..].contains("started") && !log[at..].contains("view closed"),
        "a view out of sight for a moment is kept as it is:\n{}",
        &log[at..]
    );

    // Away for longer than a view out of sight is kept: it is let go of, which its
    // plugin hears, and started again from what it kept when it is next shown.
    show(0);
    plugin_log_until(&home, "test.echo-view", at, "view closed echo");
    let log = std::fs::read_to_string(home.join("state/plugins/logs/test.echo-view.log"))
        .unwrap_or_default();
    assert!(!log[at..].contains("started"), "{}", &log[at..]);
    show(1);
    plugin_log_until(&home, "test.echo-view", at, "echo started pane count 8");
    app.quit();
}

/// How long a linked plugin may take to be loaded again after one of its files changed.
const RELOAD_LIMIT: Duration = Duration::from_secs(1);

/// Writes the files of a plugin that is being worked on into `folder`: code that says
/// `word` when it starts, and a panel whose page says `word` and how often it started.
fn write_work_plugin(folder: &Path, version: &str, host_word: &str, view_word: &str) {
    std::fs::create_dir_all(folder.join("ui")).expect("the plugin's folders");
    let write = |name: &str, text: String| {
        std::fs::write(folder.join(name), text).expect("the plugin's file is written");
    };
    write(
        "plugin.toml",
        format!(
            "schema = 1\nid = \"test.work\"\nname = \"Work\"\nversion = \"{version}\"\n\
             api = \"1\"\nmain = \"host.js\"\nactivation = [\"onStartup\", \"onView:side\"]\n\n\
             [[contributes.panels]]\nid = \"side\"\ntitle = \"Work\"\nkind = \"frame\"\n\
             area = \"right\"\nentry = \"ui/index.html\"\n"
        ),
    );
    write(
        "host.js",
        format!(
            "lume.log.info(\"host {host_word}\");\n\
             lume.views.handle(\"said\", (args) => {{\n  lume.log.info(`view ${{args.line}}`);\n  \
             return null;\n}});\n"
        ),
    );
    write(
        "ui/index.html",
        "<!doctype html>\n<html lang=\"en\">\n  <head>\n    <meta charset=\"utf-8\" />\n    \
         <title>Work</title>\n    <script src=\"view.js\" defer></script>\n  </head>\n  \
         <body></body>\n</html>\n"
            .to_owned(),
    );
    write(
        "ui/view.js",
        format!(
            "\"use strict\";\nwindow.addEventListener(\"message\", (event) => {{\n  \
             if (!event.data || event.data.lume !== \"connect\" || event.ports.length !== 1) return;\n  \
             const port = event.ports[0];\n  port.onmessage = () => undefined;\n  \
             const count = event.data.state?.count ?? 0;\n  port.postMessage({{ t: \"ready\" }});\n  \
             port.postMessage({{ t: \"state\", state: {{ count: count + 1 }} }});\n  \
             port.postMessage({{\n    t: \"call\",\n    id: 1,\n    name: \"said\",\n    \
             args: {{ line: `{view_word} count ${{count}}` }},\n  }});\n}});\n"
        ),
    );
}

/// Waits until a plugin's log says `mark` after byte `from`, looking often; says how
/// long that took.
fn plugin_log_within(home: &Path, id: &str, from: usize, mark: &str) -> Duration {
    let log = home.join(format!("state/plugins/logs/{id}.log"));
    let began = Instant::now();
    loop {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        if text.get(from..).unwrap_or_default().contains(mark) {
            return began.elapsed();
        }
        assert!(
            began.elapsed() < START,
            "{id} never logged {mark:?} (from byte {from}):\n{text}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_linked_plugin_is_loaded_again_within_a_second_of_a_change() {
    let home = home_with_plugins(&[]);
    let work = home.join("work");
    write_work_plugin(&work, "0.1.0", "one", "one");
    // What a build installs is not looked at, however much of it there is.
    std::fs::create_dir_all(work.join("node_modules/dep")).unwrap();
    std::fs::write(work.join("node_modules/dep/index.js"), "// a dependency").unwrap();
    let linked = Command::new(lume())
        .args(["plugin", "link"])
        .arg(&work)
        .env("LUME_CONFIG_DIR", home.join("config"))
        .env("LUME_STATE_DIR", home.join("state"))
        .output()
        .expect("lume runs");
    assert!(
        linked.status.success(),
        "the folder is linked: {}{}",
        String::from_utf8_lossy(&linked.stdout),
        String::from_utf8_lossy(&linked.stderr)
    );

    // One file of the folder written again with some of its text changed.
    let change = |name: &str, from: &str, to: &str| {
        let file = work.join(name);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains(from), "{name} holds {from:?}");
        std::fs::write(&file, text.replace(from, to)).unwrap();
    };

    let endpoint = format!("e2e-{}-link", std::process::id());
    let app = App::start_on(home.clone(), true, endpoint);
    app.status_with_session();
    let plugin = app.until_plugin("test.work", "the linked plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });
    assert_eq!(plugin["linked"], true, "{plugin}");
    plugin_log_until(&home, "test.work", 0, "host one");
    app.ctl(&[
        "command",
        "lume.panel.show",
        r#"{"panel":"test.work.side"}"#,
    ])
    .ok("show the panel");
    plugin_log_until(&home, "test.work", 0, "view one count 0");

    // Its code changes: the code starts again, and the view that is open stays.
    let at = plugin_log_len(&home, "test.work");
    change("host.js", "host one", "host two");
    let code = plugin_log_within(&home, "test.work", at, "host two");
    assert!(
        code < RELOAD_LIMIT,
        "the changed code started after {code:?}"
    );
    std::thread::sleep(Duration::from_millis(500));
    let log = std::fs::read_to_string(home.join("state/plugins/logs/test.work.log")).unwrap();
    assert!(
        !log[at..].contains("view "),
        "changed code leaves the view as it is:\n{}",
        &log[at..]
    );

    // Its page changes: the view loads again, from what it kept.
    std::thread::sleep(Duration::from_millis(500));
    let at = plugin_log_len(&home, "test.work");
    change("ui/view.js", "`one count", "`two count");
    let page = plugin_log_within(&home, "test.work", at, "view two count 1");
    assert!(
        page < RELOAD_LIMIT,
        "the changed page loaded after {page:?}"
    );
    let log = std::fs::read_to_string(home.join("state/plugins/logs/test.work.log")).unwrap();
    assert!(
        !log[at..].contains("host two"),
        "a changed page leaves the code running:\n{}",
        &log[at..]
    );

    // Both at once, as a build writes them.
    std::thread::sleep(Duration::from_millis(500));
    let at = plugin_log_len(&home, "test.work");
    change("host.js", "host two", "host three");
    change("ui/view.js", "`two count", "`three count");
    let both = plugin_log_within(&home, "test.work", at, "view three count 2");
    assert!(
        both < RELOAD_LIMIT,
        "both halves were loaded after {both:?}"
    );
    plugin_log_until(&home, "test.work", at, "host three");

    // Its manifest changes: what is installed follows it, and its code starts again.
    std::thread::sleep(Duration::from_millis(500));
    let at = plugin_log_len(&home, "test.work");
    change("plugin.toml", "0.1.0", "0.2.0");
    let manifest = plugin_log_within(&home, "test.work", at, "host three");
    let plugin = app.until_plugin("test.work", "the new version to be read", |plugin| {
        plugin["version"] == "0.2.0"
    });
    assert!(
        plugin["reloaded"]["times"].as_u64().unwrap_or(0) >= 4,
        "{plugin}"
    );
    println!(
        "linked plugin loaded again: code {} ms, page {} ms, both {} ms, manifest {} ms",
        code.as_millis(),
        page.as_millis(),
        both.as_millis(),
        manifest.as_millis()
    );

    // A file that is nothing to the running plugin starts nothing again.
    std::thread::sleep(Duration::from_millis(500));
    let at = plugin_log_len(&home, "test.work");
    std::fs::write(work.join("node_modules/dep/index.js"), "// changed").unwrap();
    std::fs::create_dir_all(work.join("src")).unwrap();
    std::fs::write(work.join("src/view.ts"), "// a source file").unwrap();
    std::thread::sleep(Duration::from_secs(1));
    let log = std::fs::read_to_string(home.join("state/plugins/logs/test.work.log")).unwrap();
    assert!(
        !log[at..].contains("host ") && !log[at..].contains("view "),
        "{}",
        &log[at..]
    );
    app.quit();
    // The folder is the author's: nothing of it was touched.
    assert!(work.join("plugin.toml").is_file());
}

/// A plugin's log from byte `from` on.
fn plugin_log_from(home: &Path, id: &str, from: usize) -> String {
    let text = std::fs::read_to_string(home.join(format!("state/plugins/logs/{id}.log")))
        .unwrap_or_default();
    text.get(from..).unwrap_or_default().to_owned()
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_panel_and_a_view_take_the_windows_new_look_and_go_on_as_they_were() {
    let home = home_with_plugins(&["echo-view"]);
    let config = home.join("config").join("config.toml");
    let base = std::fs::read_to_string(&config).unwrap();
    let look = |theme: &str| {
        std::fs::write(
            &config,
            format!("{base}\n[appearance]\ntheme = \"{theme}\"\n"),
        )
        .unwrap();
    };
    look("lume-dark");
    let endpoint = format!("e2e-{}-look", std::process::id());
    let app = App::start_on(home.clone(), true, endpoint.clone());
    app.status_with_session();
    app.until_plugin("test.echo-view", "the plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });
    app.ctl(&[
        "command",
        "lume.panel.show",
        r#"{"panel":"test.echo-view.side"}"#,
    ])
    .ok("show the side panel");
    plugin_log_until(&home, "test.echo-view", 0, "echo started panel count 0");
    app.ctl(&[
        "command",
        "lume.view.open",
        r#"{"plugin":"test.echo-view","view":"echo","state":{"count":3}}"#,
    ])
    .ok("open the view");
    plugin_log_until(&home, "test.echo-view", 0, "echo started pane count 3");

    // The settings name another theme: both views are given the new look, and neither
    // is started again for it.
    let colour_of = |log: &str, mark: &str| -> String {
        log.lines()
            .rev()
            .find_map(|line| line.split(mark).nth(1))
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    let at = plugin_log_len(&home, "test.echo-view");
    look("lume-light");
    plugin_log_until(&home, "test.echo-view", at, "echo look pane light ");
    let log = plugin_log_until(&home, "test.echo-view", at, "echo look panel light ");
    let light = colour_of(&log[at..], "echo look pane light ");
    assert!(light.starts_with('#') || light.contains('('), "{log}");
    assert_eq!(
        light,
        colour_of(&log[at..], "echo look panel light "),
        "both views were given the same colours"
    );

    // And back again.
    let back = plugin_log_len(&home, "test.echo-view");
    look("lume-dark");
    plugin_log_until(&home, "test.echo-view", back, "echo look panel dark ");
    let log = plugin_log_until(&home, "test.echo-view", back, "echo look pane dark ");
    assert_ne!(colour_of(&log[back..], "echo look pane dark "), light);
    let since = plugin_log_from(&home, "test.echo-view", at);
    assert!(
        !since.contains("started") && !since.contains("view closed"),
        "a new look leaves the views running:\n{since}"
    );

    // A neighbour is split off beside the view, and the window is given the light look
    // once more: the view is still the one that was started first.
    let view = view_panes(&app.layout())
        .into_iter()
        .find(|(_, view, _)| view == "echo")
        .expect("the view has a pane")
        .0;
    let split = serde_json::json!({ "pane": view }).to_string();
    app.ctl(&["command", "lume.pane.split-right", &split])
        .ok("split a terminal off beside the view");
    let again = plugin_log_len(&home, "test.echo-view");
    look("lume-light");
    plugin_log_until(&home, "test.echo-view", again, "echo look pane light ");
    let since = plugin_log_from(&home, "test.echo-view", at);
    assert!(!since.contains("started"), "{since}");

    // Closed and started again in the light look, the views come back with what they
    // kept, and a change of look reaches them as before.
    app.ctl(&["command", "lume.workspace.save"]).ok("save");
    app.quit();
    let at = plugin_log_len(&home, "test.echo-view");
    let app = App::start_on(home.clone(), false, endpoint);
    app.until_plugin("test.echo-view", "the plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });
    plugin_log_until(&home, "test.echo-view", at, "echo started pane count 4");
    plugin_log_until(&home, "test.echo-view", at, "echo started panel count 1");
    let at = plugin_log_len(&home, "test.echo-view");
    look("lume-dark");
    plugin_log_until(&home, "test.echo-view", at, "echo look pane dark ");
    plugin_log_until(&home, "test.echo-view", at, "echo look panel dark ");
    app.quit();
}

/// Runs git in `folder` as somebody with a name, and gives back what it printed.
fn git_in(folder: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(folder)
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.org"])
        .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Runs `lume plugin` with these words on the folders of `home`, with nobody to answer
/// a question. Gives back whether it succeeded and what it printed.
fn plugin_command(home: &Path, args: &[&str]) -> (bool, String) {
    let output = Command::new(lume())
        .arg("plugin")
        .args(args)
        .env("LUME_CONFIG_DIR", home.join("config"))
        .env("LUME_STATE_DIR", home.join("state"))
        .stdin(Stdio::null())
        .output()
        .expect("lume runs");
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_plugin_from_a_repository_is_pinned_and_an_update_that_asks_for_more_waits_for_a_yes() {
    if !on_path("git") {
        println!("skipped: there is no git on this machine");
        return;
    }
    let home = home_with_plugins(&[]);
    let origin = home.join("origin");
    std::fs::create_dir_all(&origin).unwrap();
    let write = |version: &str, permissions: &str, word: &str| {
        std::fs::write(
            origin.join("plugin.toml"),
            format!(
                "schema = 1\nid = \"test.fetched\"\nname = \"Fetched\"\nversion = \"{version}\"\n\
                 api = \"1\"\nmain = \"host.js\"\nactivation = [\"onStartup\"]\n\
                 [permissions]\n{permissions}"
            ),
        )
        .unwrap();
        std::fs::write(
            origin.join("host.js"),
            format!("lume.log.info(\"host {word}\");\n"),
        )
        .unwrap();
    };
    write("1.0.0", "\"system.metrics\" = true\n", "one");
    git_in(&origin, &["init", "--quiet"]);
    git_in(&origin, &["add", "-A"]);
    git_in(&origin, &["commit", "--quiet", "-m", "one"]);
    let first = git_in(&origin, &["rev-parse", "HEAD"]);
    let address = format!(
        "file:///{}",
        origin
            .display()
            .to_string()
            .replace('\\', "/")
            .trim_start_matches('/')
    );

    // What is installed says which commit it is and what its files hashed to.
    let (done, said) = plugin_command(&home, &["install", &address, "--yes"]);
    assert!(done, "{said}");
    let installed = || -> Value {
        let (done, said) = plugin_command(&home, &["list", "--json"]);
        assert!(done, "{said}");
        let listed: Value = serde_json::from_str(said.trim()).expect("the list is JSON");
        listed["plugins"][0].clone()
    };
    let lock_file = home.join("state/plugins/lock.toml");
    let lock = std::fs::read_to_string(&lock_file).unwrap();
    assert_eq!(installed()["kind"], "git");
    assert_eq!(installed()["commit"], first.as_str());
    assert!(lock.contains(&first), "the commit is in the lock:\n{lock}");
    assert!(lock.contains("content-hash = \"sha256:"), "{lock}");

    let app = App::start_in(home.clone(), false);
    app.status_with_session();
    app.until_plugin("test.fetched", "the plugin to be read", |plugin| {
        plugin["state"] == "ready" && plugin["version"] == "1.0.0"
    });
    plugin_log_until(&home, "test.fetched", 0, "host one");

    // Nothing new in the repository: nothing changes.
    let (done, said) = plugin_command(&home, &["update", "--yes"]);
    assert!(done, "{said}");
    assert_eq!(std::fs::read_to_string(&lock_file).unwrap(), lock);

    // A newer commit that asks for nothing more goes in on a yes said ahead, and the
    // running application takes it up.
    write("1.1.0", "\"system.metrics\" = true\n", "two");
    git_in(&origin, &["commit", "--quiet", "-am", "two"]);
    let second = git_in(&origin, &["rev-parse", "HEAD"]);
    let at = plugin_log_len(&home, "test.fetched");
    let (done, said) = plugin_command(&home, &["update", "--yes"]);
    assert!(done, "{said}");
    assert_eq!(installed()["commit"], second.as_str());
    app.until_plugin("test.fetched", "the update to be read", |plugin| {
        plugin["version"] == "1.1.0"
    });
    plugin_log_until(&home, "test.fetched", at, "host two");

    // One that asks for more is not installed on a yes said ahead: with nobody there
    // to ask, it is left alone, and the application goes on with what it had.
    write(
        "1.2.0",
        "\"system.metrics\" = true\n\"clipboard.write\" = true\n",
        "three",
    );
    git_in(&origin, &["commit", "--quiet", "-am", "three"]);
    let at = plugin_log_len(&home, "test.fetched");
    let before = std::fs::read_to_string(&lock_file).unwrap();
    let (done, said) = plugin_command(&home, &["update", "--yes"]);
    assert!(!done, "{said}");
    assert!(said.contains("more than you agreed to"), "{said}");
    assert_eq!(std::fs::read_to_string(&lock_file).unwrap(), before);
    assert_eq!(installed()["commit"], second.as_str());
    assert!(!before.contains("clipboard.write"), "{before}");
    std::thread::sleep(Duration::from_secs(1));
    let plugin = app.until_plugin("test.fetched", "the plugin as it was", |_| true);
    assert_eq!(plugin["version"], "1.1.0", "{plugin}");
    assert!(
        !plugin_log_from(&home, "test.fetched", at).contains("host three"),
        "the wider version did not run"
    );
    app.quit();
}

/// Where the add-ons repository is on this machine: where `LUME_ADDONS` says, or beside
/// this repository.
fn addons() -> Option<PathBuf> {
    let folder = std::env::var_os("LUME_ADDONS").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../lume-addons"),
        PathBuf::from,
    );
    let folder = folder.canonicalize().ok()?;
    // A canonical path on Windows begins with `\\?\`, which git and the command line
    // are better off without.
    let plain = folder.display().to_string();
    Some(PathBuf::from(
        plain.strip_prefix(r"\\?\").unwrap_or(&plain).to_owned(),
    ))
}

/// A port on this machine that nothing listens on now.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("a port on this machine")
        .port()
}

/// The pages of an application that was started with a debugging port, looked into
/// from outside. Only WebView2 has such a port.
struct Pages {
    port: u16,
}

/// All the text of a page, the text inside its components' own trees included, with
/// nothing put between the pieces.
const PAGE_TEXT: &str = "(() => { const out = []; const walk = (node) => { \
    if (node.nodeName === 'STYLE' || node.nodeName === 'SCRIPT') return;     if (node.nodeType === 3) out.push(node.nodeValue); \
    if (node.shadowRoot) walk(node.shadowRoot); \
    for (const child of node.childNodes) walk(child); }; \
    walk(document.documentElement); return out.join(''); })()";

impl Pages {
    /// What `expression` gives in the page whose address holds `part`; nothing while
    /// there is no such page or it does not answer.
    fn evaluate(&self, part: &str, expression: &str) -> Option<Value> {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/frame.mjs");
        let output = Command::new("node")
            .arg(script)
            .args([&self.port.to_string(), part, expression])
            .stdin(Stdio::null())
            .output()
            .expect("node runs: the tests that look into a view need it");
        if !output.status.success() {
            return None;
        }
        serde_json::from_slice(&output.stdout).ok()
    }

    /// Waits until the text of the page whose address holds `part` holds every one of
    /// `wanted`, and gives the text back.
    fn until_text(&self, part: &str, wanted: &[&str]) -> String {
        let deadline = Instant::now() + START;
        loop {
            let text = self
                .evaluate(part, PAGE_TEXT)
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_default();
            if wanted.iter().all(|one| text.contains(one)) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "the page {part} never showed {wanted:?}:\n{text}"
            );
            std::thread::sleep(Duration::from_millis(300));
        }
    }
}

/// The words of the status item `item` of a plugin as `ctl plugins` lists it.
fn status_text(plugin: &Value, item: &str) -> Option<String> {
    plugin["status"]
        .as_array()?
        .iter()
        .find(|one| one["item"] == item)?["value"]["text"]
        .as_str()
        .map(str::to_owned)
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn the_git_review_plugin_shows_the_changes_of_a_real_repository() {
    const ID: &str = "addons.git-review";
    let Some(plugin) = addons()
        .map(|folder| folder.join("plugins").join("git-review"))
        .filter(|plugin| plugin.join("dist").join("host.js").is_file())
    else {
        println!(
            "skipped: the add-ons repository is not beside this one, and LUME_ADDONS names \
             none with plugins/git-review built in it"
        );
        return;
    };
    if !on_path("git") {
        println!("skipped: there is no git on this machine");
        return;
    }

    // A repository with one commit, one file changed since and one that is new.
    let home = home_with_plugins(&[]);
    let repository = home.join("repository");
    std::fs::create_dir_all(&repository).unwrap();
    std::fs::write(
        repository.join("notes.txt"),
        "first line\nlumeOldWord stays here\nthird line\n",
    )
    .unwrap();
    git_in(&repository, &["init", "--quiet"]);
    git_in(&repository, &["add", "-A"]);
    git_in(&repository, &["commit", "--quiet", "-m", "one"]);
    std::fs::write(
        repository.join("notes.txt"),
        "first line\nlumeNewWord came instead\nthird line\n",
    )
    .unwrap();
    std::fs::write(repository.join("fresh.txt"), "lumeFreshWord\n").unwrap();

    // The plugin as it is in its repository, built: nothing is made for the test.
    let (done, said) = plugin_command(&home, &["link", &plugin.display().to_string()]);
    assert!(done, "{said}");

    // On Windows the application's pages can be looked into from outside, with a
    // browser of its own so that the port is this application's.
    let pages = cfg!(windows).then(|| Pages { port: free_port() });
    let environment: Vec<(&str, String)> = pages
        .iter()
        .flat_map(|pages| {
            [
                (
                    "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
                    format!("--remote-debugging-port={}", pages.port),
                ),
                (
                    "WEBVIEW2_USER_DATA_FOLDER",
                    home.join("webview").display().to_string(),
                ),
            ]
        })
        .collect();
    let endpoint = format!("e2e-{}-review", std::process::id());
    let app = App::start_on_with(home.clone(), true, endpoint.clone(), &environment);
    app.status_with_session();
    app.until_plugin(ID, "the plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });
    // The tests' shell does not say which folder it is in, so the workspace does.
    let root = serde_json::json!({ "root": repository }).to_string();
    app.ctl(&["command", "lume.workspace.set-root", &root])
        .ok("set the workspace's folder");

    // The button's command opens the review in a pane; the changes are a panel.
    app.ctl(&["command", "addons.git-review.open"])
        .ok("open the review");
    app.ctl(&[
        "command",
        "lume.panel.show",
        r#"{"panel":"addons.git-review.changes"}"#,
    ])
    .ok("show the changes");
    let deadline = Instant::now() + START;
    while !view_panes(&app.layout())
        .iter()
        .any(|(_, view, _)| view == "review")
    {
        assert!(Instant::now() < deadline, "the review has no pane");
        std::thread::sleep(Duration::from_millis(200));
    }
    // The button counts the files that changed.
    app.until_plugin(ID, "two changed files to be counted", |plugin| {
        status_text(plugin, "git").as_deref() == Some("2")
    });

    if let Some(pages) = &pages {
        // The panel lists both files. Pressing one shows it in the review.
        pages.until_text("ui/changes.html", &["notes.txt", "fresh.txt"]);
        let press = |name: &str| {
            let pressed = pages.evaluate(
                "ui/changes.html",
                &format!(
                    "(() => {{ const row = [...document.querySelectorAll('.rv-trow')]\
                     .find((one) => one.textContent.includes('{name}')); \
                     const button = row?.querySelector('button.rv-tpick'); \
                     if (!button) return false; button.click(); return true; }})()"
                ),
            );
            assert_eq!(
                pressed,
                Some(Value::Bool(true)),
                "{name} has a row to press"
            );
        };
        // The file that git does not know yet is shown as all new.
        press("fresh.txt");
        let review = pages.until_text("ui/review.html", &["fresh.txt", "lumeFreshWord"]);
        assert!(!review.contains("lumeNewWord"), "{review}");
        // The changed one, with the line that went, the line that came and a line
        // that stayed.
        press("notes.txt");
        let review = pages.until_text(
            "ui/review.html",
            &["notes.txt", "lumeOldWord", "lumeNewWord", "third line"],
        );
        assert!(!review.contains("lumeFreshWord"), "{review}");
        // The diff is drawn by the library, in a tree of its own inside the page.
        let drawn = pages.evaluate(
            "ui/review.html",
            "[...document.querySelectorAll('.rv-hunk *')].filter((one) => one.shadowRoot).length",
        );
        assert!(
            drawn.as_ref().and_then(Value::as_u64).unwrap_or(0) >= 1,
            "a hunk is drawn: {drawn:?}"
        );
    } else {
        println!("not looked at here: what the views draw (only WebView2 has a port for it)");
    }

    // A commit made outside, as a terminal would make it, is followed.
    git_in(&repository, &["add", "-A"]);
    git_in(&repository, &["commit", "--quiet", "-m", "two"]);
    // With nothing changed the button counts nothing: it draws no mark at all.
    app.until_plugin(ID, "the commit to be seen", |plugin| {
        status_text(plugin, "git").as_deref() == Some("")
    });
    if let Some(pages) = &pages {
        pages.until_text("ui/review.html", &["Everything is committed."]);
    }

    // After the application was closed and started again the review is where it was,
    // and reads the repository as it is by then.
    std::fs::write(repository.join("later.txt"), "lumeLaterWord\n").unwrap();
    app.ctl(&["command", "lume.workspace.save"]).ok("save");
    app.quit();
    let app = App::start_on_with(home.clone(), false, endpoint, &environment);
    app.until_plugin(ID, "the plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });
    assert!(
        view_panes(&app.layout())
            .iter()
            .any(|(_, view, _)| view == "review"),
        "the review came back with the layout"
    );
    app.until_plugin(ID, "the new file to be counted", |plugin| {
        status_text(plugin, "git").as_deref() == Some("1")
    });
    if let Some(pages) = &pages {
        pages.until_text("ui/changes.html", &["later.txt"]);
    }
    app.quit();
    // The plugin's folder is its author's: nothing was written into it.
    assert!(plugin.join("plugin.toml").is_file());
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn the_time_a_question_is_on_show_is_not_taken_from_what_a_command_may_take() {
    let home = home_with_plugins(&["asker"]);
    let app = App::start_in(home, false);
    let status = app.status_with_session();
    let pane = status["focused"]["pane"].as_str().unwrap().to_owned();
    app.until_plugin("test.asker", "the plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });

    // The command types into a pane, which the user approves first; the user takes
    // longer over it than a command has. (An answer is taken once the question is up,
    // and waited for until then, so what is slept here is all time before the answer.)
    let text = serde_json::json!({ "pane": pane, "text": "echo lume-waited\r" }).to_string();
    let began = Instant::now();
    let typed = app.ctl_in_background(&["command", "test.asker.type", &text]);
    std::thread::sleep(Duration::from_secs(33));
    app.answer_prompt(true);
    let answer = typed.join().unwrap();
    assert!(
        began.elapsed() > Duration::from_secs(30),
        "{:?}",
        began.elapsed()
    );
    answer.ok("the command, approved late");
    app.wait_text_in(&pane, "lume-waited");
    app.quit();
}

/// Runs `lume <words>` against an application, with `piped` on its standard input if
/// there is any, and returns its status, what it printed and what it said beside.
fn verb_at(endpoint: &str, words: &[&str], piped: Option<&str>) -> (i32, String, String) {
    use std::io::Write as _;
    let mut child = Command::new(lume())
        .args(["--endpoint", endpoint])
        .args(words)
        .stdin(if piped.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("lume runs");
    if let Some(text) = piped {
        let mut input = child.stdin.take().expect("its input is a pipe");
        input
            .write_all(text.as_bytes())
            .expect("the input is written");
    }
    let output = child.wait_with_output().expect("lume ends");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_plugins_verb_prints_what_its_command_gives_and_ends_with_its_status() {
    let home = home_with_plugins(&["verbs"]);
    let config = home.join("config").join("config.toml");
    let base = std::fs::read_to_string(&config).unwrap();
    let profiles = "\n[plugins.\"test.verbs\".profiles.fast]\nagent = \"one\"\n";
    std::fs::write(&config, format!("{base}{profiles}")).unwrap();
    std::fs::create_dir_all(home.join("config").join("workspaces")).unwrap();
    let app = App::start_in(home, false);
    app.until_plugin("test.verbs", "the plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });

    // The verbs are listed with the plugin each comes from.
    let listed = app.ctl(&["verbs"]).ok("verbs");
    let verbs = listed["verbs"].as_array().unwrap();
    assert_eq!(verbs.len(), 6, "{verbs:?}");
    assert_eq!(verbs[0]["verb"], "verbs echo");
    assert_eq!(verbs[0]["plugin"], "test.verbs");
    assert_eq!(verbs[0]["command"], "test.verbs.echo");
    assert_eq!(verbs[0]["usage"], "lume verbs echo [<word>...]");

    // Two steps, in order; the arguments are the plugin's own to read, and what is
    // piped in reaches it.
    let (status, out, err) = verb_at(
        &app.endpoint,
        &["verbs", "echo", "a", "--help", "c d"],
        Some("piped\n"),
    );
    assert_eq!(status, 0, "{err}");
    assert_eq!(out, "argv [\"a\",\"--help\",\"c d\"]\nstdin \"piped\\n\"\n");
    assert_eq!(err, "cwd true\nechoed\n");
    let (status, out, _) = verb_at(&app.endpoint, &["verbs", "echo"], None);
    assert_eq!((status, out.as_str()), (0, "argv []\nstdin null\n"));

    // The status is the command's, and what it says beside goes to standard error.
    let (status, out, err) = verb_at(&app.endpoint, &["verbs", "fail"], None);
    assert_eq!((status, out.as_str(), err.as_str()), (7, "", "it failed\n"));

    // A command that gives what is not a step, or throws, fails the verb and says so.
    let (status, out, err) = verb_at(&app.endpoint, &["verbs", "broken"], None);
    assert_eq!((status, out.as_str()), (1, ""), "{err}");
    assert!(err.contains("test.verbs.broken gave back text"), "{err}");
    let (status, out, err) = verb_at(&app.endpoint, &["verbs", "throw"], None);
    assert_eq!((status, out.as_str()), (1, "first\n"), "{err}");
    assert!(err.contains("the second step went wrong"), "{err}");

    // A setting that is a table is read as it is written in the file.
    let (status, out, err) = verb_at(&app.endpoint, &["verbs", "table"], None);
    assert_eq!(status, 0, "{err}");
    assert_eq!(out, "{\"fast\":{\"agent\":\"one\"}}\n");

    // A change in the file reaches the plugin's code, which is running by now.
    let from = plugin_log_len(&app.home, "test.verbs");
    let more = "\n[plugins.\"test.verbs\".profiles.deep]\nagent = \"two\"\n";
    std::fs::write(&config, format!("{base}{profiles}{more}")).unwrap();
    let log = plugin_log_until(&app.home, "test.verbs", from, "profiles now");
    assert!(log.contains("\"deep\":{\"agent\":\"two\"}"), "{log}");
    let (_, out, _) = verb_at(&app.endpoint, &["verbs", "table"], None);
    assert_eq!(
        out,
        "{\"deep\":{\"agent\":\"two\"},\"fast\":{\"agent\":\"one\"}}\n"
    );

    // Words no plugin adds a verb for are not understood, and the verbs are named.
    let (status, out, err) = verb_at(&app.endpoint, &["verbs", "nope"], None);
    assert_eq!((status, out.as_str()), (2, ""), "{err}");
    assert!(err.contains("verbs is not something lume knows"), "{err}");
    assert!(err.contains("lume verbs echo"), "{err}");
    assert!(err.contains("(test.verbs)"), "{err}");
    let (status, _, err) = verb_at(&app.endpoint, &["frobnicate"], None);
    assert_eq!(status, 2, "{err}");
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_verb_goes_on_past_a_calls_limit_and_stops_when_its_client_goes_away() {
    use std::io::{BufRead as _, BufReader};
    let home = home_with_plugins(&["verbs"]);
    let app = App::start_in(home, false);
    app.until_plugin("test.verbs", "the plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });

    let mut child = Command::new(lume())
        .args(["--endpoint", &app.endpoint, "verbs", "hang"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("lume runs");
    let mut printed = BufReader::new(child.stdout.take().expect("its output is a pipe"));
    let mut line = String::new();
    printed.read_line(&mut line).expect("the first step prints");
    assert_eq!(line, "started\n");

    // Longer than one call of a command may take: the run is many short calls, and
    // none of them counts against the plugin.
    std::thread::sleep(Duration::from_secs(35));
    assert!(
        child.try_wait().expect("lume can be waited for").is_none(),
        "the verb ended by itself"
    );
    let from = plugin_log_len(&app.home, "test.verbs");
    child.kill().expect("lume is stopped");
    child.wait().expect("lume has ended");
    let log = plugin_log_until(&app.home, "test.verbs", from, "hang cancelled after");
    assert!(log.contains("hang started"), "{log}");

    // The plugin is as it was: on, running, and its verbs answer.
    let plugin = app.until_plugin("test.verbs", "the plugin to be listed", |_| true);
    assert_eq!(plugin["state"], "ready", "{plugin}");
    assert_eq!(plugin["breaches"], 0, "{plugin}");
    let (status, _, err) = verb_at(&app.endpoint, &["verbs", "fail"], None);
    assert_eq!(status, 7, "{err}");
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_verb_two_plugins_add_belongs_to_the_first_and_the_other_is_told() {
    let home = home_with_plugins(&["verbs"]);
    install_plugin_as(&home, &fixture("verbs"), "test.verbs2");
    let app = App::start_in(home, false);
    let second = app.until_plugin("test.verbs2", "the second plugin to be read", |plugin| {
        plugin["state"] == "ready"
    });
    let told = second.to_string();
    assert!(
        told.contains("the verb `verbs echo` is not added: test.verbs adds it already"),
        "{told}"
    );
    let listed = app.ctl(&["verbs"]).ok("verbs");
    let verbs = listed["verbs"].as_array().unwrap();
    assert_eq!(verbs.len(), 6, "{verbs:?}");
    assert!(verbs.iter().all(|verb| verb["plugin"] == "test.verbs"));
    let (status, _, err) = verb_at(&app.endpoint, &["verbs", "fail"], None);
    assert_eq!(status, 7, "{err}");
    app.quit();
}

#[test]
fn a_verb_needs_the_application_and_says_so() {
    let endpoint = format!("e2e-{}-nothing-runs-here", std::process::id());
    let (status, out, err) = verb_at(&endpoint, &["agent", "run", "--profile", "x"], None);
    assert_eq!((status, out.as_str()), (4, ""), "{err}");
    assert!(
        err.contains("agent is not one of lume's own commands"),
        "{err}"
    );
    assert!(err.contains("the application is not running"), "{err}");
    // What cannot begin a verb is refused before anything is asked of anyone.
    let (status, _, err) = verb_at(&endpoint, &["--frobnicate"], None);
    assert_eq!(status, 2, "{err}");
    assert!(
        err.contains("--frobnicate is not something lume knows"),
        "{err}"
    );
}

#[test]
fn checking_the_configuration_holds_a_plugins_table_setting_to_a_table() {
    let home = home_with_plugins(&["verbs"]);
    let config = home.join("config").join("config.toml");
    let base = std::fs::read_to_string(&config).unwrap();
    let check = || {
        let output = Command::new(lume())
            .args(["config", "check"])
            .env("LUME_CONFIG_DIR", home.join("config"))
            .env("LUME_STATE_DIR", home.join("state"))
            .output()
            .expect("lume runs");
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        )
    };
    std::fs::write(
        &config,
        format!("{base}\n[plugins.\"test.verbs\".profiles.fast]\nagent = \"one\"\n"),
    )
    .unwrap();
    assert_eq!(check(), (0, "no problems\n".to_owned()));

    std::fs::write(
        &config,
        format!("{base}\n[plugins.\"test.verbs\"]\nprofiles = \"fast\"\n"),
    )
    .unwrap();
    let (status, out) = check();
    assert_eq!(status, 1, "{out}");
    assert!(
        out.contains("`plugins.\"test.verbs\".profiles`: expected a table"),
        "{out}"
    );
    assert!(out.ends_with("1 problem\n"), "{out}");
    let _ = std::fs::remove_dir_all(&home);
}
