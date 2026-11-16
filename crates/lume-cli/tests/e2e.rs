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
        std::fs::write(
            home.join("config").join("config.toml"),
            configuration() + settings,
        )
        .expect("the configuration is written");
        Self::start_in(home, false)
    }

    /// Starts an application whose configuration and saved workspaces are in `home`.
    fn start_in(home: PathBuf, keep_home: bool) -> Self {
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let endpoint = format!("e2e-{}-{number}", std::process::id());
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
