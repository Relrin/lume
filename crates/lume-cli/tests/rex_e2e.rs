//! The application as a client of a real Rex server.
//!
//! Each test starts a server of its own, with its own pipe or socket, its own folder of
//! state and an empty configuration, and starts the application with that server in its
//! configuration. The user's own server is not looked for, reached or started. What the
//! application did is asked of the server with the server's own command, `rex`, which is
//! another client of it.
//!
//! Rex is not something a build machine has, so these tests are a target of their own,
//! ignored by default, and run on a machine that has it with `cargo xtask rex-e2e`.
//!
//! What is typed is typed into `cmd.exe /d` or `/bin/sh`, started for the test. The
//! server's own choice of a shell, which a new session or a split gets, may keep a
//! history of the user's: it is started and never typed into.

mod support;

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::*;

/// The label the server has in the application's configuration.
const HOST: &str = "Test server";

/// How long something may take to come about, on either side.
const SOON: Duration = Duration::from_secs(20);

/// A Rex server started for one test.
struct Server {
    child: Option<Child>,
    program: PathBuf,
    address: String,
    home: PathBuf,
}

fn program() -> PathBuf {
    let program = std::env::var_os("LUME_REX_BIN").map(PathBuf::from);
    program.expect("the Rex program is named by `cargo xtask rex-e2e`, which runs these tests")
}

/// A shell that keeps no history, as a program and its arguments.
fn plain_shell() -> &'static [&'static str] {
    if cfg!(windows) {
        &["cmd.exe", "/d"]
    } else {
        &["/bin/sh"]
    }
}

impl Server {
    fn start() -> Self {
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!("lume-e2e-{}-{number}", std::process::id());
        let home = std::env::temp_dir().join(format!("{name}-rex"));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("init.lua"), "").unwrap();
        let address = if cfg!(windows) {
            format!("npipe:////./pipe/{name}")
        } else {
            format!("unix://{}", home.join("s.sock").display())
        };
        let mut server = Self {
            child: None,
            program: program(),
            address,
            home,
        };
        server.run();
        server
    }

    /// Starts the server and waits until it answers.
    fn run(&mut self) {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.home.join("server.log"))
            .unwrap();
        let mut command = self.command();
        command
            .args([
                "server",
                "run",
                "--log-level",
                "warn",
                "--listen",
                &self.address,
            ])
            .arg("--state-dir")
            .arg(self.home.join("state"))
            .arg("--config")
            .arg(self.home.join("init.lua"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log);
        self.child = Some(command.spawn().expect("the Rex server starts"));
        until("the Rex server to answer", || {
            let (done, said) = self.try_rex(&["server", "status", "--json"]);
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                panic!("the Rex server ended with {status}: {}", self.log());
            }
            done.then_some(said)
        });
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.home.join("server.log")).unwrap_or_default()
    }

    /// The program, told to start nothing, with none of the user's choices for it.
    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("REX_") {
                command.env_remove(name);
            }
        }
        command.env("REX_AUTOSTART", "0").stdin(Stdio::null());
        command
    }

    /// Runs `rex` as another client of this server.
    fn try_rex(&self, args: &[&str]) -> (bool, String) {
        let output = self
            .command()
            .args(["--autostart=false", "-S", &self.address])
            .args(args)
            .output()
            .expect("rex runs");
        let text = if output.status.success() {
            &output.stdout
        } else {
            &output.stderr
        };
        (
            output.status.success(),
            String::from_utf8_lossy(text).into_owned(),
        )
    }

    fn rex(&self, args: &[&str]) -> String {
        let (done, said) = self.try_rex(args);
        assert!(done, "rex {args:?}: {said}");
        said
    }

    fn rex_json(&self, args: &[&str]) -> Value {
        let said = self.rex(args);
        serde_json::from_str(&said).unwrap_or_else(|error| panic!("rex {args:?}: {error}: {said}"))
    }

    /// Makes a session that runs a plain shell, and gives its block.
    fn session(&self, label: &str) -> String {
        let mut args = vec!["new", label, "--json", "--shell=none", "--"];
        args.extend(plain_shell());
        let made = self.rex_json(&args);
        made["initial_windows"][0]["block_ids"][0]
            .as_str()
            .expect("the session's block")
            .to_owned()
    }

    fn view(&self, session: &str) -> Value {
        self.rex_json(&["api", "call", "-s", session, "session.view"])
    }

    /// The blocks of a session, in the order of its windows.
    fn blocks(&self, session: &str) -> Vec<String> {
        let mut found = Vec::new();
        for window in self.view(session)["windows"].as_array().unwrap() {
            for layer in window["layers"].as_array().unwrap() {
                for block in layer["blocks"].as_array().unwrap() {
                    found.push(block["block_id"].as_str().unwrap().to_owned());
                }
            }
        }
        found
    }

    /// The lines on a block's screen and in its scrollback.
    fn captured(&self, block: &str) -> Vec<String> {
        self.rex(&["capture", "-b", block, "--trim"])
            .lines()
            .map(|line| line.trim_end().to_owned())
            .collect()
    }

    fn size(&self, session: &str, block: &str) -> (u64, u64) {
        let asked = json!({ "block_id": block }).to_string();
        let size = self.rex_json(&[
            "api",
            "call",
            "-s",
            session,
            "com.superlogical.terminal.size",
            &asked,
        ]);
        (
            size["columns"].as_u64().unwrap(),
            size["rows"].as_u64().unwrap(),
        )
    }

    fn stop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let _ = self.try_rex(&["server", "stop"]);
        let deadline = Instant::now() + SOON;
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

impl Drop for Server {
    fn drop(&mut self) {
        // Its programs end with the sessions they are in.
        if let (true, listed) = self.try_rex(&["ls", "--json"])
            && let Ok(listed) = serde_json::from_str::<Value>(&listed)
        {
            for session in listed["sessions"].as_array().into_iter().flatten() {
                let _ = self.try_rex(&["kill", session["session_id"].as_str().unwrap_or("")]);
            }
        }
        self.stop();
        if std::thread::panicking() {
            eprintln!("the Rex server said:\n{}", self.log());
        } else {
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }
}

/// Waits until `found` finds something.
fn until<T>(what: &str, mut found: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + SOON;
    loop {
        if let Some(found) = found() {
            return found;
        }
        assert!(Instant::now() < deadline, "waited too long for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn settings(server: &Server) -> String {
    // A literal string: an address holds what a basic string would read as an escape.
    format!(
        "\n[[rex.host]]\nlabel = '{HOST}'\naddress = '{}'\n",
        server.address
    )
}

/// The application with the server in its configuration.
fn application_of(server: &Server) -> App {
    let app = App::start_with(&settings(server));
    app.status_with_session();
    app
}

/// Runs a command of the application.
fn command(app: &App, id: &str, args: Value) -> Answer {
    app.ctl(&["command", id, &args.to_string()])
}

/// How the one server stands, as the application says.
fn host(app: &App) -> Value {
    let listed = app.ctl(&["rex"]).ok("rex");
    assert_eq!(
        listed["rex"]["hosts"].as_array().map(Vec::len),
        Some(1),
        "{listed}"
    );
    listed["rex"]["hosts"][0].clone()
}

fn open(app: &App, session: &str) {
    command(
        app,
        "lume.rex.open",
        json!({ "host": HOST, "session": session }),
    )
    .ok("open");
    until("the session to be on show", || {
        let layout = app.layout();
        (layout["workspace"]["name"] == session).then_some(())
    });
}

/// The panes of the layout on show, tab by tab.
fn tabs(app: &App) -> Vec<Vec<String>> {
    tabs_of(&app.layout())
}

fn only_pane(app: &App) -> String {
    let tabs = tabs(app);
    assert_eq!(tabs.len(), 1, "{tabs:?}");
    assert_eq!(tabs[0].len(), 1, "{tabs:?}");
    tabs[0][0].clone()
}

/// Waits until the window shows as many panes in each tab as `wanted` says.
fn until_panes(app: &App, wanted: &[usize]) -> Vec<Vec<String>> {
    until(&format!("panes {wanted:?}"), || {
        let tabs = tabs(app);
        let counts: Vec<usize> = tabs.iter().map(Vec::len).collect();
        (counts == wanted).then_some(tabs)
    })
}

fn apply(app: &App, intent: Value) {
    app.ctl(&["layout", "apply", &intent.to_string()])
        .ok(&format!("{intent}"));
}

fn own_workspace_on_show(app: &App) -> bool {
    app.layout()["workspace"]["backend"].is_null()
}

/// What a shell prints for a line, told apart from the line as it was typed.
fn echo(app: &App, pane: &str, mark: &str) {
    let line = if cfg!(windows) {
        format!("set /a 41+1 >nul & echo {mark}-%OS%")
    } else {
        format!("echo {mark}-$((41+1))")
    };
    app.type_line_in(pane, &line);
    app.wait_text_in(pane, &printed(mark));
}

fn printed(mark: &str) -> String {
    if cfg!(windows) {
        format!("{mark}-Windows_NT")
    } else {
        format!("{mark}-42")
    }
}

/// Starts the application so that its page can be looked into, where that is possible.
fn application_with_pages(server: &Server) -> (App, Option<Pages>) {
    if !cfg!(windows) {
        return (application_of(server), None);
    }
    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    let endpoint = format!("e2e-{}-{number}", std::process::id());
    let home = std::env::temp_dir().join(format!("lume-{endpoint}"));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("config")).unwrap();
    std::fs::write(
        home.join("config").join("config.toml"),
        configuration() + IN_PROCESS_HOST + &settings(server),
    )
    .unwrap();
    let pages = Pages { port: free_port() };
    let app = App::start_on_with(
        home.clone(),
        false,
        endpoint,
        &[
            (
                "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
                format!("--remote-debugging-port={}", pages.port),
            ),
            (
                "WEBVIEW2_USER_DATA_FOLDER",
                home.join("webview").display().to_string(),
            ),
        ],
    );
    app.status_with_session();
    (app, Some(pages))
}

/// The address of a window's page, as far as it tells it from others.
const WINDOW_PAGE: &str = "tauri.localhost";

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn a_servers_sessions_are_listed_and_one_is_shown_as_a_workspace() {
    let server = Server::start();
    let block = server.session("alpha");
    server.session("beta");
    let app = application_of(&server);
    let own = app.layout()["workspace"]["name"].clone();

    // Nothing is connected to until somebody asks.
    assert_eq!(host(&app)["state"]["kind"], "idle");
    command(&app, "lume.rex.connect", json!({ "host": HOST })).ok("connect");
    let listed = host(&app);
    assert_eq!(listed["state"]["kind"], "connected", "{listed}");
    assert_eq!(listed["state"]["api_revision"], 19);
    let labels: Vec<&str> = listed["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|session| session["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["alpha", "beta"]);
    // It says what it is.
    let clients = server.rex(&["client", "ls", "--all"]);
    assert!(clients.contains("Lume"), "{clients}");

    open(&app, "alpha");
    let layout = app.layout();
    assert_eq!(layout["workspace"]["backend"], listed["id"]);
    let pane = only_pane(&app);
    let status = app.status_with_session();
    assert_eq!(status["focused"]["pane"], pane);
    echo(&app, &pane, "alpha-says");
    let wanted = printed("alpha-says");
    assert!(server.captured(&block).contains(&wanted));

    // From one session of the server to another, and back to a workspace of its own.
    open(&app, "beta");
    assert_eq!(host(&app)["sessions"].as_array().unwrap().len(), 2);
    command(&app, "lume.workspace.switch", json!({ "name": own })).ok("switch");
    assert!(own_workspace_on_show(&app));
    app.status_with_session();
    // Neither session ended by not being shown any more.
    assert_eq!(
        server.rex_json(&["ls", "--json"])["sessions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // The one last shown is shown again, with what it printed before.
    command(
        &app,
        "lume.rex.open",
        json!({ "host": HOST, "session": "alpha" }),
    )
    .ok("again");
    until("the session to be shown again", || {
        (app.layout()["workspace"]["name"] == "alpha").then_some(())
    });
    app.wait_text_in(&only_pane(&app), &wanted);
    command(&app, "lume.rex.disconnect", json!({ "host": HOST })).ok("disconnect");
    assert!(own_workspace_on_show(&app));
    assert_eq!(host(&app)["state"]["kind"], "idle");
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn the_window_shows_what_the_terminal_prints_and_types_into_it() {
    let server = Server::start();
    let block = server.session("typed");
    let (app, pages) = application_with_pages(&server);
    open(&app, "typed");
    let pane = only_pane(&app);

    // The terminal takes the size of its pane, which only the page knows.
    let sized = until("the terminal to take its pane's size", || {
        let size = server.size("typed", &block);
        (size != (80, 24)).then_some(size)
    });
    assert!(sized.0 > 20 && sized.1 > 5, "{sized:?}");
    echo(&app, &pane, "seen");
    let Some(pages) = pages else {
        println!("what the page shows is not looked at here");
        app.quit();
        return;
    };

    // The page's own terminal holds what the server's printed: it finds it.
    app.ctl(&[
        "command",
        "lume.search.open",
        &json!({ "query": printed("seen") }).to_string(),
    ])
    .ok("search");
    let count = "document.querySelector('.search-count')?.textContent ?? ''";
    until("the page to find what was printed", || {
        let found = pages.evaluate(WINDOW_PAGE, count)?;
        let found = found.as_str().unwrap_or_default().to_owned();
        (found.contains('1')).then_some(found)
    });
    app.ctl(&["command", "lume.search.close"]).ok("close");

    // Typed in the page, it reaches the program, and what that prints comes back.
    let line = if cfg!(windows) {
        "echo by-hand-%OS%"
    } else {
        "echo by-hand-42"
    };
    let typing = format!(
        "(() => {{ const input = document.querySelector('.terminal-input'); \
         input.focus(); \
         return document.execCommand('insertText', false, {}); }})()",
        json!(line)
    );
    assert_eq!(
        pages.evaluate(WINDOW_PAGE, &typing),
        Some(json!(true)),
        "the page took no typing"
    );
    app.wait_text_in(&pane, line);
    app.ctl(&["send", "--pane", &pane, "--enter", "--", ""])
        .ok("enter");
    app.wait_text_in(&pane, &printed("by-hand"));

    // The workspace switcher says whose layout it is.
    let text = pages.until_text(WINDOW_PAGE, &["typed"]);
    assert!(!text.contains("could not"), "{text}");
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn what_is_changed_here_is_changed_on_the_server() {
    let server = Server::start();
    let first = server.session("ours");
    let app = application_of(&server);
    open(&app, "ours");
    let pane = only_pane(&app);

    // A split, with a program of the server's machine in it.
    let shell = plain_shell();
    apply(
        &app,
        json!({
            "type": "split", "pane": pane, "dir": "right", "ratio": 250_000,
            "content": { "type": "terminal", "launcher": {
                "type": "shell", "program": shell[0], "args": &shell[1..], "login": false,
            }},
        }),
    );
    let panes = until_panes(&app, &[2]);
    let blocks = server.blocks("ours");
    assert_eq!(blocks.len(), 2);
    let second = blocks
        .iter()
        .find(|block| **block != first)
        .unwrap()
        .clone();
    let beside = panes[0].iter().find(|id| **id != pane).unwrap().clone();
    echo(&app, &beside, "split");
    assert!(
        server
            .captured(&second)
            .iter()
            .any(|line| *line == printed("split"))
    );

    // A name.
    apply(
        &app,
        json!({ "type": "pane_rename", "pane": beside, "title": "logs" }),
    );
    let view = server.view("ours");
    let label = view["windows"][0]["layers"][0]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|block| block["block_id"] == second.as_str())
        .map(|block| block["label"].clone());
    assert_eq!(label, Some(json!("logs")), "{view}");

    // Zoomed and back, by the command a key runs.
    app.ctl(&["command", "lume.pane.zoom"]).ok("zoom");
    until("the server to zoom", || {
        server.view("ours")["windows"][0]["zoomed_block_id"]
            .is_string()
            .then_some(())
    });
    app.ctl(&["command", "lume.pane.zoom"]).ok("unzoom");

    // A tab, and its closing.
    app.ctl(&["command", "lume.tab.new"]).ok("new tab");
    until_panes(&app, &[2, 1]);
    assert_eq!(server.view("ours")["windows"].as_array().unwrap().len(), 2);
    app.ctl(&["command", "lume.tab.close"]).ok("close tab");
    until_panes(&app, &[2]);
    assert_eq!(server.view("ours")["windows"].as_array().unwrap().len(), 1);

    // A pane, closed.
    apply(&app, json!({ "type": "close", "pane": beside }));
    until_panes(&app, &[1]);
    assert_eq!(server.blocks("ours"), [first]);
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn what_another_client_changes_is_changed_here() {
    let server = Server::start();
    let first = server.session("theirs");
    let app = application_of(&server);
    open(&app, "theirs");
    only_pane(&app);

    let shell = plain_shell();
    let mut split = vec![
        "split",
        "--split",
        "below",
        "-b",
        &first,
        "--shell=none",
        "--",
    ];
    split.extend(shell);
    server.rex(&split);
    until_panes(&app, &[2]);
    let mut run = vec!["run", "-s", "theirs", "--shell=none", "--"];
    run.extend(shell);
    server.rex(&run);
    let tabs = until_panes(&app, &[2, 1]);

    // What another client types is seen, in a tab that is not on show as well.
    let window = server.view("theirs")["windows"][1].clone();
    let block = window["layers"][0]["blocks"][0]["block_id"]
        .as_str()
        .unwrap()
        .to_owned();
    server.rex(&["send", "-b", &block, "echo from-elsewhere\r"]);
    app.wait_text_in(&tabs[1][0], "from-elsewhere");

    server.rex(&["session", "rename", "theirs", "renamed"]);
    until("the new name", || {
        (app.layout()["workspace"]["name"] == "renamed").then_some(())
    });
    server.rex(&["block", "close", &block]);
    until_panes(&app, &[2]);
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn a_full_screen_program_fills_its_pane_and_takes_the_panes_new_size() {
    let editors = [
        PathBuf::from(r"C:\Program Files\Git\usr\bin\vim.exe"),
        PathBuf::from("/usr/bin/vim"),
    ];
    let Some(vim) = editors.iter().find(|editor| editor.is_file()) else {
        println!("skipped: there is no vim on this machine");
        return;
    };
    let server = Server::start();
    let file = server.home.join("notes.txt");
    let text: String = (1..=200)
        .map(|line| format!("line number {line}\n"))
        .collect();
    std::fs::write(&file, text).unwrap();
    let made = server.rex_json(&[
        "new",
        "editor",
        "--json",
        "--shell=none",
        "--",
        &vim.display().to_string(),
        "-i",
        "NONE",
        "-u",
        "NONE",
        "-n",
        &file.display().to_string().replace('\\', "/"),
    ]);
    let block = made["initial_windows"][0]["block_ids"][0]
        .as_str()
        .unwrap()
        .to_owned();
    let app = application_of(&server);
    open(&app, "editor");
    let pane = only_pane(&app);
    app.wait_text_in(&pane, "line number 3");

    // The program's last line is on the last row of the terminal, whatever its size.
    let same = |app: &App| {
        let captured = app.ctl(&["capture", "--pane", &pane]).ok("capture");
        let ours: Vec<String> = captured["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line.as_str().unwrap().trim_end().to_owned())
            .collect();
        let theirs = server.captured(&block);
        let filled = ours
            .iter()
            .filter(|line| line.starts_with("line number"))
            .count() as u64;
        let (_, rows) = server.size("editor", &block);
        (ours.len() as u64 == rows
            && filled + 1 == rows
            && ours[..filled as usize] == theirs[..filled as usize])
            .then_some((ours, rows))
    };
    let (_, rows) = until("the editor to fill its pane", || same(&app));
    // The editor drops what is typed while it settles, so it is asked until it goes there.
    until("the editor to go to a line", || {
        app.ctl(&["send", "--pane", &pane, "--", "50Gzt"]).ok("go");
        std::thread::sleep(Duration::from_secs(1));
        let captured = app.ctl(&["capture", "--pane", &pane]).ok("capture");
        (captured["lines"][0] == "line number 50").then_some(())
    });

    // The pane is halved, and the program with it.
    let shell = plain_shell();
    apply(
        &app,
        json!({
            "type": "split", "pane": pane, "dir": "down", "ratio": 500_000,
            "content": { "type": "terminal", "launcher": {
                "type": "shell", "program": shell[0], "args": &shell[1..], "login": false,
            }},
        }),
    );
    until_panes(&app, &[2]);
    let (_, halved) = until("the editor to take its new size", || {
        // An editor that is busy can miss that its console changed, and sees it when it
        // is next asked to draw.
        app.ctl(&["send", "--pane", &pane, "--", "\u{c}"])
            .ok("redraw");
        std::thread::sleep(Duration::from_millis(300));
        same(&app).filter(|(_, now)| *now < rows)
    });
    assert!(halved > 4, "{halved} of {rows}");
    app.ctl(&["send", "--pane", &pane, "--", ":q!\r"])
        .ok("leave");
    until("the editor's pane to be gone", || {
        let shown = tabs(&app);
        (shown.len() == 1 && !shown[0].contains(&pane)).then_some(())
    });
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn a_session_is_made_here_and_one_that_ends_gives_way_to_the_workspace_before_it() {
    let server = Server::start();
    let app = application_of(&server);
    let none = command(&app, "lume.rex.open", json!({ "host": HOST }));
    assert_eq!(none.status, 1);
    assert_eq!(none.json["code"], "no_session", "{}", none.json);

    command(
        &app,
        "lume.rex.new-session",
        json!({ "host": HOST, "label": "made here" }),
    )
    .ok("new session");
    until("the new session to be on show", || {
        (app.layout()["workspace"]["name"] == "made here").then_some(())
    });
    let listed = server.rex_json(&["ls", "--json"]);
    assert_eq!(listed["sessions"][0]["label"], "made here");
    app.status_with_session();

    server.rex(&["kill", "made here"]);
    until("the own workspace to be on show again", || {
        own_workspace_on_show(&app).then_some(())
    });
    app.status_with_session();
    // The connection stays: it is the session that ended.
    let host = host(&app);
    assert_eq!(host["state"]["kind"], "connected");
    assert_eq!(host["sessions"], json!([]));
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn closing_the_last_pane_of_a_session_gives_way_to_the_workspace_before_it() {
    let server = Server::start();
    server.session("last");
    let app = application_of(&server);
    open(&app, "last");
    let pane = only_pane(&app);
    apply(&app, json!({ "type": "close", "pane": pane }));
    until("the own workspace to be on show again", || {
        own_workspace_on_show(&app).then_some(())
    });
    app.status_with_session();
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn a_server_that_is_restarted_is_connected_to_again_with_its_session_on_show() {
    let mut server = Server::start();
    server.session("kept");
    let app = application_of(&server);
    open(&app, "kept");
    server.stop();
    until("the connection to be seen as lost", || {
        (host(&app)["state"]["kind"] == "waiting").then_some(())
    });
    // What was on show stays on show, and the application's own sessions are listed.
    assert!(!own_workspace_on_show(&app));
    app.ctl(&["list"]).ok("list");

    server.run();
    until("the connection to be made again", || {
        (host(&app)["state"]["kind"] == "connected").then_some(())
    });
    // The server brings its sessions back, with programs started anew.
    let restored = server.rex_json(&["ls", "--json"]);
    assert_eq!(restored["sessions"][0]["label"], "kept", "{restored}");
    assert_eq!(app.layout()["workspace"]["name"], "kept");
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn a_session_outlives_the_application_that_showed_it() {
    let server = Server::start();
    let block = server.session("outlives");
    let app = application_of(&server);
    open(&app, "outlives");
    echo(&app, &only_pane(&app), "before");
    app.quit();

    // Nothing of it ended, and nobody says it is still looked at.
    assert_eq!(server.blocks("outlives"), [block]);
    let clients = server.rex(&["client", "ls", "--all"]);
    assert!(!clients.contains("Lume"), "{clients}");

    let app = application_of(&server);
    assert!(own_workspace_on_show(&app));
    open(&app, "outlives");
    app.wait_text_in(&only_pane(&app), &printed("before"));
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn a_server_that_does_not_run_is_said_not_to_be_there_and_is_not_started() {
    let mut server = Server::start();
    let app = application_of(&server);
    server.stop();
    let refused = command(&app, "lume.rex.connect", json!({ "host": HOST }));
    assert_eq!(refused.status, 1);
    assert_eq!(refused.json["code"], "unreachable", "{}", refused.json);
    assert_eq!(host(&app)["state"]["kind"], "failed");
    assert!(!server.try_rex(&["server", "status"]).0);

    // Tried again once it runs, it is reached.
    server.run();
    command(&app, "lume.rex.connect", json!({ "host": HOST })).ok("connect");
    assert_eq!(host(&app)["state"]["kind"], "connected");
    app.quit();
}
