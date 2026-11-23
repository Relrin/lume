//! What the tests that drive the application from outside share: starting it with folders
//! of its own, asking it things through `lume ctl`, and looking into its pages.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

/// How long the application may take to start, and to close.
pub const START: Duration = Duration::from_secs(60);

/// How long `lume ctl` waits for text or an exit, in seconds, as it takes it.
pub const WAIT: &str = "60";

pub const APPLICATION: &str = if cfg!(windows) {
    "lume-gui.exe"
} else {
    "lume-gui"
};

pub static NEXT: AtomicU32 = AtomicU32::new(1);

/// The answer of one `lume ctl` command.
pub struct Answer {
    pub status: i32,
    pub json: Value,
}

impl Answer {
    pub fn ok(self, what: &str) -> Value {
        assert_eq!(self.status, 0, "{what} failed: {}", self.json);
        self.json
    }
}

/// Where the tests' applications run their sessions unless a test says otherwise:
/// inside the application, so that a test's shells end when its application closes.
pub const IN_PROCESS_HOST: &str = "\n[host]\nmode = \"in-process\"\n";

/// A running application with an endpoint of its own.
pub struct App {
    pub child: Option<Child>,
    pub endpoint: String,
    pub log: PathBuf,
    /// Where the application keeps its configuration and its saved workspaces.
    pub home: PathBuf,
    /// Whether the folder is removed when the application has closed.
    pub keep_home: bool,
}

/// The configuration the tests' applications run with: a shell for the first pane that
/// starts fast and keeps no history of what is typed.
pub fn configuration() -> String {
    let (shell, args) = if cfg!(windows) {
        ("cmd.exe", "[\"/d\"]")
    } else {
        ("/bin/sh", "[]")
    };
    // A Rex server of the user's own is none of the tests' business.
    format!("[terminal]\nshell = \"{shell}\"\nshell-args = {args}\n\n[rex]\nlocal = false\n")
}

pub fn lume() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_lume"))
}

pub fn application() -> PathBuf {
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
    pub fn start() -> Self {
        Self::start_with("")
    }

    /// Starts an application that has never run before, with `settings` added to the
    /// configuration the tests' applications run with.
    pub fn start_with(settings: &str) -> Self {
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
    pub fn start_in(home: PathBuf, keep_home: bool) -> Self {
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
    pub fn start_on(home: PathBuf, keep_home: bool, endpoint: String) -> Self {
        Self::start_on_with(home, keep_home, endpoint, &[])
    }

    /// The same, with these variables added to the application's environment.
    pub fn start_on_with(
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

    pub fn ctl(&self, args: &[&str]) -> Answer {
        ctl_at(&self.endpoint, args)
    }

    /// The application's status once the pane that has the keyboard has a session. The
    /// endpoint answers before the page has had the session started, so a status read
    /// straight after the start may name none yet.
    pub fn status_with_session(&self) -> Value {
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
    pub fn type_line(&self, line: &str) {
        self.ctl(&["send", "--enter", "--", line]).ok("send");
    }

    /// Waits until `text` is on the screen of the pane that has the keyboard.
    pub fn wait_text(&self, text: &str) -> Value {
        self.ctl(&["wait", "--text", text])
            .ok(&format!("waiting for {text:?}"))
    }

    /// Types a line into a pane, whether or not its tab is on show.
    pub fn type_line_in(&self, pane: &str, line: &str) {
        self.ctl(&["send", "--pane", pane, "--enter", "--", line])
            .ok("send");
    }

    /// Waits until `text` is on the screen of a pane.
    pub fn wait_text_in(&self, pane: &str, text: &str) -> Value {
        self.ctl(&["wait", "--pane", pane, "--text", text])
            .ok(&format!("waiting for {text:?} in {pane}"))
    }

    /// The layout: its windows, their tabs, and the panes in them.
    pub fn layout(&self) -> Value {
        self.ctl(&["layout", "view"]).ok("layout view")
    }

    /// The layout once it says where its first window is on the desktop. A window
    /// tells where it is a moment after it was last moved, so this also waits for the
    /// place to have stopped changing.
    pub fn layout_with_bounds(&self) -> Value {
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
    pub fn workspaces(&self) -> Value {
        self.ctl(&["workspaces"]).ok("workspaces")
    }

    /// Waits until there are `count` sessions.
    pub fn until_sessions(&self, count: usize, what: &str) {
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
    pub fn pid_of(&self, pane: &str) -> u32 {
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
    pub fn session_pids(&self) -> Vec<u32> {
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
    pub fn until_closed(mut self, what: &str) {
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
    pub fn screen(&self) -> Vec<String> {
        let captured = self.ctl(&["capture"]).ok("capture");
        captured["lines"]
            .as_array()
            .expect("the lines of a screen")
            .iter()
            .map(|line| line.as_str().unwrap_or_default().to_owned())
            .collect()
    }

    /// Ends the application's process at once, as a crash would.
    pub fn kill(mut self) {
        let mut child = self.child.take().expect("the application is running");
        child.kill().expect("the application can be ended");
        let _ = child.wait();
    }

    /// Asks the application to close and waits until its process has ended.
    pub fn quit(self) -> PathBuf {
        self.quit_with(&["quit"])
    }

    /// Asks the application to close with `request`, and waits until its process has
    /// ended.
    pub fn quit_with(mut self, request: &[&str]) -> PathBuf {
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
pub fn ctl_at(endpoint: &str, args: &[&str]) -> Answer {
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

/// The ids of the panes of a tab's tree, in the order they are laid out.
pub fn panes_of(node: &Value) -> Vec<String> {
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
pub fn tabs_of(layout: &Value) -> Vec<Vec<String>> {
    layout["workspace"]["windows"][0]["centre"]["tabs"]
        .as_array()
        .expect("the window's tabs")
        .iter()
        .map(|tab| panes_of(&tab["root"]))
        .collect()
}

/// What a pane of a layout shows.
pub fn content_of(layout: &Value, pane: &str) -> Value {
    pub fn find(node: &Value, pane: &str) -> Option<Value> {
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

/// Whether a program of this name is on the search path.
pub fn on_path(program: &str) -> bool {
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

/// A port on this machine that nothing listens on now.
pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("a port on this machine")
        .port()
}

/// The pages of an application that was started with a debugging port, looked into
/// from outside. Only WebView2 has such a port.
pub struct Pages {
    pub port: u16,
}

/// All the text of a page, the text inside its components' own trees included, with
/// nothing put between the pieces.
pub const PAGE_TEXT: &str = "(() => { const out = []; const walk = (node) => { \
    if (node.nodeName === 'STYLE' || node.nodeName === 'SCRIPT') return;     if (node.nodeType === 3) out.push(node.nodeValue); \
    if (node.shadowRoot) walk(node.shadowRoot); \
    for (const child of node.childNodes) walk(child); }; \
    walk(document.documentElement); return out.join(''); })()";

impl Pages {
    /// What `expression` gives in the page whose address holds `part`; nothing while
    /// there is no such page or it does not answer.
    pub fn evaluate(&self, part: &str, expression: &str) -> Option<Value> {
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
    pub fn until_text(&self, part: &str, wanted: &[&str]) -> String {
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
