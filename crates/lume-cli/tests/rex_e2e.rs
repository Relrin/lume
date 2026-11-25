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

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::drive::*;
use support::*;

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

    /// Makes a session that runs a full-screen editor on `text`, and gives its block once
    /// the editor has drawn `drawn` and waits: until then it throws away what is typed,
    /// and may not see that its terminal has another size. Without an editor on this
    /// machine, says so and gives nothing. The editor reads and writes nothing of the
    /// user's.
    fn editor(&self, session: &str, text: &str, drawn: &str) -> Option<String> {
        let editors = [
            PathBuf::from(r"C:\Program Files\Git\usr\bin\vim.exe"),
            PathBuf::from("/usr/bin/vim"),
        ];
        let Some(vim) = editors.iter().find(|editor| editor.is_file()) else {
            println!("skipped: there is no vim on this machine");
            return None;
        };
        let file = self.home.join("notes.txt");
        std::fs::write(&file, text).unwrap();
        let made = self.rex_json(&[
            "new",
            session,
            "--json",
            "--shell=none",
            "--",
            &vim.display().to_string(),
            "-N",
            "-i",
            "NONE",
            "-u",
            "NONE",
            "-n",
            "--cmd",
            "set encoding=utf-8",
            "--cmd",
            VIM_WITHOUT_FOCUS,
            &file.display().to_string().replace('\\', "/"),
        ]);
        let block = made["initial_windows"][0]["block_ids"][0]
            .as_str()
            .unwrap()
            .to_owned();
        let mut last = Vec::new();
        until("the editor to be drawn and to wait", || {
            std::thread::sleep(Duration::from_millis(700));
            let now = self.captured(&block);
            let waits = now == last && now.iter().any(|line| line.contains(drawn));
            last = now;
            waits.then_some(())
        });
        Some(block)
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

/// Starts the application so that its page can be looked into, where that is possible.
fn application_with_pages(server: &Server) -> (App, Option<Pages>) {
    if !cfg!(windows) {
        return (application_of(server), None);
    }
    let (app, pages) = application_to_drive(&settings(server));
    (app, Some(pages))
}

/// Looks at a host of the selector from the keyboard, and goes into what it holds.
fn look_at(pages: &Pages, label: &str) {
    press(pages, &["Tab"]);
    let looked = format!(
        "document.querySelector('.selector-host[data-marked] .selector-name')?.textContent === {}",
        json!(label)
    );
    until("the host to be looked at", || {
        if page(pages, &looked) == true {
            return Some(());
        }
        press(pages, &["ArrowDown"]);
        None
    });
    press(pages, &["Tab"]);
    until_page(
        pages,
        "what the host holds to have the arrows",
        "document.querySelector('.selector-host[data-marked]') === null",
    );
}

/// Marks something of the selector's right column by what it says.
fn mark(pages: &Pages, text: &str) {
    let marked = format!(
        "document.querySelector('.selector-row[data-marked] .selector-name, \
         .selector-act[data-marked] .selector-act-label')?.textContent === {}",
        json!(text)
    );
    until(&format!("{text} to be marked"), || {
        if page(pages, &marked) == true {
            return Some(());
        }
        press(pages, &["ArrowDown"]);
        None
    });
}

/// Waits until the button of the top bar names a host and what of it is on show.
fn until_button(pages: &Pages, host: &str, name: &str) {
    until_page(
        pages,
        &format!("the button to name {name} on {host}"),
        &format!(
            "document.querySelector('.switcher-button')?.ariaLabel === {}",
            json!(format!("{host}, {name}"))
        ),
    );
}

/// The labels of the server's sessions, in its order.
fn labels(server: &Server) -> Vec<String> {
    let listed = server.rex_json(&["ls", "--json"]);
    let sessions = listed["sessions"].as_array().cloned().unwrap_or_default();
    sessions
        .iter()
        .map(|session| session["label"].as_str().unwrap_or_default().to_owned())
        .collect()
}

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
    let server = Server::start();
    let text: String = (1..=200)
        .map(|line| format!("line number {line}\n"))
        .collect();
    let Some(block) = server.editor("editor", &text, "line number 3") else {
        return;
    };
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
    // An editor that is busy can miss that its console changed, and sees it when it is
    // next asked to draw.
    let redraw = || {
        app.ctl(&["send", "--pane", &pane, "--", "\u{c}"])
            .ok("redraw");
        std::thread::sleep(Duration::from_millis(300));
    };
    let (_, rows) = until("the editor to fill its pane", || {
        redraw();
        same(&app)
    });
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
        redraw();
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

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn the_connect_palette_connects_a_server_and_attaches_to_its_sessions() {
    if !cfg!(windows) {
        println!("skipped: the page is driven on Windows only");
        return;
    }
    let server = Server::start();
    server.session("alpha");
    server.session("beta");
    let (app, pages) = application_to_drive(&settings(&server));
    connect_palette(&app, &pages, HOST);
    until_texts(&pages, ".connect-row-line", &["Not connected"]);

    // It stays to list the sessions of the server it connects.
    press(&pages, &["Enter"]);
    until_texts(&pages, ".connect-session-name", &["alpha", "beta"]);
    until_texts(&pages, ".connect-row-line", &["2 sessions"]);
    until_texts(
        &pages,
        ".connect-primary .connect-act-label",
        &["Attach to alpha"],
    );
    // The server is on this machine: how near it is is not said.
    until_texts(&pages, ".connect-stats dt", &["Server", "API rev"]);
    assert_eq!(texts(&pages, ".connect-stats dd")[1], "19");
    until_texts(&pages, ".connect-row .connect-meta", &[]);
    assert!(own_workspace_on_show(&app));

    press(&pages, &["Enter"]);
    until_on_show(&app, "alpha");
    until_button(&pages, HOST, "alpha");
    // The palette is gone, and what is typed goes to the terminal.
    until_page(
        &pages,
        "the palette to be gone",
        "document.querySelector('.connect') === null",
    );
    until_page(
        &pages,
        "the terminal to have the keyboard",
        "document.activeElement?.classList.contains('terminal-input') === true",
    );

    // Tab goes into the sessions, where the arrows mark another.
    connect_palette(&app, &pages, HOST);
    until_texts(&pages, ".connect-session .connect-meta", &["Attached"]);
    press(&pages, &["Tab", "ArrowDown"]);
    until_texts(
        &pages,
        ".connect-primary .connect-act-label",
        &["Attach to beta"],
    );
    press(&pages, &["Enter"]);
    until_on_show(&app, "beta");
    until_button(&pages, HOST, "beta");
    until_texts(
        &pages,
        ".status-served-text",
        &[&format!("{HOST} · session beta · 1 pane")],
    );
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn the_connect_palette_makes_sessions_and_says_why_a_server_is_not_there() {
    if !cfg!(windows) {
        println!("skipped: the page is driven on Windows only");
        return;
    }
    let mut server = Server::start();
    server.session("alpha");
    let (app, pages) = application_to_drive(&settings(&server));
    connect_palette(&app, &pages, HOST);
    press(&pages, &["Enter"]);
    until_texts(&pages, ".connect-session-name", &["alpha"]);
    press(&pages, &["Ctrl+n"]);
    until("the session to be made", || {
        (labels(&server).len() == 2).then_some(())
    });
    let made = labels(&server)
        .into_iter()
        .find(|label| label != "alpha")
        .unwrap();
    until_on_show(&app, &made);

    // Disconnected by its button, the own workspace is back.
    connect_palette(&app, &pages, HOST);
    let button = |label: &str| {
        format!(
            "(() => {{ const found = [...document.querySelectorAll('.connect-acts button')] \
             .find((one) => one.textContent.startsWith({})); \
             found?.click(); return found !== undefined; }})()",
            json!(label)
        )
    };
    until_page(&pages, "the button that disconnects", &button("Disconnect"));
    until("the own workspace to be on show", || {
        own_workspace_on_show(&app).then_some(())
    });
    app.ctl(&["command", "lume.connect.open"]).ok("put away");

    // With the server gone, why it cannot be reached is said, and it is tried again.
    server.stop();
    connect_palette(&app, &pages, HOST);
    press(&pages, &["Enter"]);
    until_texts(&pages, ".connect-row-line", &["Unreachable"]);
    until_texts(&pages, ".connect-primary .connect-act-label", &["Retry"]);
    let why = texts(&pages, ".connect-call");
    assert!(why[0].starts_with("Unreachable. "), "{why:?}");
    server.run();
    press(&pages, &["Enter"]);
    until_page(
        &pages,
        "the sessions to be listed again",
        "document.querySelectorAll('.connect-session').length === 2",
    );
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn a_connection_without_encryption_is_made_only_by_keys_of_its_own() {
    if !cfg!(windows) {
        println!("skipped: the page is driven on Windows only");
        return;
    }
    // An address kept for documentation: nothing is there.
    let plain = "\n[[rex.host]]\nlabel = 'Plain'\naddress = 'http://192.0.2.1:7297'\n";
    let (app, pages) = application_to_drive(plain);
    connect_palette(&app, &pages, "Plain");
    press(&pages, &["Enter"]);
    until_texts(&pages, ".connect-row-line", &["Not encrypted"]);
    let why = texts(&pages, ".connect-call");
    assert!(why[0].contains("would not be encrypted"), "{why:?}");
    // Nothing else asks, and Enter does not agree.
    press(&pages, &["Enter", "Enter"]);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(host(&app)["state"]["code"], "confirmation_required");
    assert_eq!(
        page(&pages, "document.querySelector('.prompt') === null"),
        true
    );

    press(&pages, &["Ctrl+Enter"]);
    until("the connection to be tried", || {
        let state = host(&app)["state"].clone();
        (state["kind"] == "connecting" || state["code"] == "unreachable").then_some(())
    });
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn the_command_palette_finds_a_session_and_opens_it() {
    if !cfg!(windows) {
        println!("skipped: the page is driven on Windows only");
        return;
    }
    let server = Server::start();
    server.session("alpha");
    server.session("needle");
    let (app, pages) = application_to_drive(&settings(&server));
    command(&app, "lume.rex.connect", json!({ "host": HOST })).ok("connect");
    app.ctl(&["command", "lume.palette.open"]).ok("palette");
    until_page(
        &pages,
        "the palette to have the keyboard",
        "document.activeElement?.className === 'palette-input'",
    );
    press(&pages, &["text:#", "text:needle"]);
    until_texts(&pages, ".palette-row-text", &["needle"]);
    until_texts(&pages, ".palette-row-sub", &[HOST]);
    press(&pages, &["Enter"]);
    until_on_show(&app, "needle");
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn the_selector_opens_a_session_and_goes_back_to_a_workspace_of_its_name() {
    if !cfg!(windows) {
        println!("skipped: the page is driven on Windows only");
        return;
    }
    let server = Server::start();
    let (app, pages) = application_to_drive(&settings(&server));
    let own = app.layout()["workspace"]["name"]
        .as_str()
        .unwrap()
        .to_owned();
    server.session(&own);
    until_button(&pages, "This PC", &own);

    selector(&pages);
    until_texts(&pages, ".selector-host .selector-name", &["This PC", HOST]);
    until_texts(&pages, ".selector-row[data-on] .selector-name", &[&own]);
    look_at(&pages, HOST);
    mark(&pages, "Connect");
    press(&pages, &["Enter"]);
    until_texts(&pages, ".selector-row .selector-name", &[&own]);
    mark(&pages, &own);
    press(&pages, &["Enter"]);
    until_on_show(&app, &own);
    until_button(&pages, HOST, &own);

    // The workspace of the same name is not the one on show, and can be gone back to.
    selector(&pages);
    until_texts(
        &pages,
        ".selector-row[data-on] .selector-right",
        &["Attached"],
    );
    press(&pages, &["Tab", "ArrowUp"]);
    until_page(
        &pages,
        "this machine to be looked at",
        "document.querySelector('.selector-host[data-marked]')?.dataset.host === 'pc'",
    );
    press(&pages, &["Tab"]);
    until_texts(&pages, ".selector-row .selector-name", &[&own]);
    until_texts(&pages, ".selector-row[data-on] .selector-name", &[]);
    mark(&pages, &own);
    press(&pages, &["Enter"]);
    until("the own workspace to be on show", || {
        own_workspace_on_show(&app).then_some(())
    });
    until_button(&pages, "This PC", &own);
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn the_selector_makes_renames_and_ends_sessions() {
    if !cfg!(windows) {
        println!("skipped: the page is driven on Windows only");
        return;
    }
    let server = Server::start();
    server.session("alpha");
    let (app, pages) = application_to_drive(&settings(&server));
    open(&app, "alpha");
    until_button(&pages, HOST, "alpha");

    // What is typed and not there can be made.
    selector(&pages);
    press(&pages, &["text:nightly"]);
    mark(&pages, &format!("Create “nightly” on {HOST}"));
    press(&pages, &["Enter"]);
    until_on_show(&app, "nightly");
    assert_eq!(labels(&server), ["alpha", "nightly"]);

    // Renamed in place, the server has the name, and so has the button.
    selector(&pages);
    mark(&pages, "nightly");
    press(&pages, &["F2"]);
    until_page(
        &pages,
        "the name to be typed over",
        "document.activeElement?.className === 'selector-rename'",
    );
    press(&pages, &["text:weekly", "Enter"]);
    until("the server to have the name", || {
        (labels(&server) == ["alpha", "weekly"]).then_some(())
    });
    until_texts(&pages, ".selector-row .selector-name", &["alpha", "weekly"]);
    until_button(&pages, HOST, "weekly");

    // A name that is taken is not sent.
    until_page(
        &pages,
        "the keyboard to be back",
        "document.activeElement?.className === 'selector-input'",
    );
    mark(&pages, "weekly");
    press(&pages, &["F2", "text:alpha", "Enter"]);
    until_page(
        &pages,
        "the name to be refused",
        "document.querySelector('.selector-rename')?.ariaInvalid === 'true'",
    );
    press(&pages, &["Escape"]);
    assert_eq!(labels(&server), ["alpha", "weekly"]);

    // Ending it asks, and a no leaves it as it was.
    let asked = "document.querySelector('.prompt')?.ariaLabel === 'End “weekly”?'";
    until_page(
        &pages,
        "the keyboard to be back",
        "document.activeElement?.className === 'selector-input'",
    );
    mark(&pages, "weekly");
    press(&pages, &["Ctrl+Delete"]);
    until_page(&pages, "the question", asked);
    press(&pages, &["Enter"]);
    until_page(
        &pages,
        "the question to go",
        "document.querySelector('.prompt') === null",
    );
    assert_eq!(labels(&server), ["alpha", "weekly"]);
    until_on_show(&app, "weekly");

    selector(&pages);
    mark(&pages, "weekly");
    press(&pages, &["Ctrl+Delete"]);
    until_page(&pages, "the question", asked);
    page(
        &pages,
        "(() => { document.querySelector('.prompt-proceed').click(); return true; })()",
    );
    until("the session to be ended", || {
        (labels(&server) == ["alpha"]).then_some(())
    });
    until("the own workspace to be on show", || {
        own_workspace_on_show(&app).then_some(())
    });
    assert_eq!(host(&app)["state"]["kind"], "connected");
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn the_page_follows_what_another_client_does_and_a_server_that_goes_and_comes() {
    if !cfg!(windows) {
        println!("skipped: the page is driven on Windows only");
        return;
    }
    let mut server = Server::start();
    server.session("alpha");
    server.session("beta");
    let (app, pages) = application_to_drive(&settings(&server));
    open(&app, "alpha");
    until_button(&pages, HOST, "alpha");
    selector(&pages);
    let names = ".selector-row .selector-name";
    until_texts(&pages, names, &["alpha", "beta"]);

    // While the selector is open, another client renames one, makes one and ends one.
    server.rex(&["session", "rename", "alpha", "gamma"]);
    until_texts(&pages, names, &["gamma", "beta"]);
    until_button(&pages, HOST, "gamma");
    server.session("delta");
    until_texts(&pages, names, &["gamma", "beta", "delta"]);
    server.rex(&["kill", "beta"]);
    until_texts(&pages, names, &["gamma", "delta"]);
    until_texts(
        &pages,
        ".status-served-text",
        &[&format!("{HOST} · session gamma · 1 pane")],
    );
    press(&pages, &["Escape"]);
    until_page(
        &pages,
        "the selector to go",
        "document.querySelector('.selector') === null",
    );

    server.stop();
    // Shown by a ring that turns in the place of the dot, in the bar and below.
    until_page(
        &pages,
        "the connection to be shown as being made again",
        "document.querySelectorAll('.switcher-dot[data-link=busy]').length === 2",
    );
    until_texts(
        &pages,
        ".status-served-text",
        &[&format!("{HOST} · session gamma · 1 pane")],
    );
    server.run();
    until_page(
        &pages,
        "the button to say it is connected",
        "document.querySelector('.switcher-dot')?.dataset.link === 'up'",
    );
    until_button(&pages, HOST, "gamma");
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn an_empty_session_is_shown_and_a_tab_can_be_made_in_it() {
    if !cfg!(windows) {
        println!("skipped: the page is driven on Windows only");
        return;
    }
    let server = Server::start();
    let block = server.session("hollow");
    server.rex(&["block", "close", &block]);
    let (app, pages) = application_to_drive(&settings(&server));
    command(
        &app,
        "lume.rex.open",
        json!({ "host": HOST, "session": "hollow" }),
    )
    .ok("open");
    until_on_show(&app, "hollow");
    until_button(&pages, HOST, "hollow");
    until_texts(
        &pages,
        ".status-served-text",
        &[&format!("{HOST} · session hollow · 0 panes")],
    );
    until_page(
        &pages,
        "the window to offer a tab",
        "document.querySelector('.empty-action') !== null",
    );
    page(
        &pages,
        "(() => { document.querySelector('.empty-action').click(); return true; })()",
    );
    until_panes(&app, &[1]);
    assert_eq!(server.blocks("hollow").len(), 1);
    app.status_with_session();
    app.quit();
}

/// What presses the button of a page that says `label`, once it can be pressed.
fn pressing(within: &str, label: &str) -> String {
    format!(
        "(() => {{ const found = [...document.querySelectorAll({})] \
         .find((one) => one.textContent === {} && !one.disabled); \
         found?.click(); return found !== undefined; }})()",
        json!(format!("{within} button")),
        json!(label)
    )
}

/// The Rex servers the application lists, as their labels and addresses.
fn listed(app: &App) -> Vec<(String, String)> {
    let listed = app.ctl(&["rex"]).ok("rex");
    let hosts = listed["rex"]["hosts"].as_array().cloned();
    let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    hosts
        .unwrap_or_default()
        .iter()
        .map(|host| (text(&host["label"]), text(&host["address"])))
        .collect()
}

/// What stands in the application's configuration file.
fn written(app: &App) -> String {
    std::fs::read_to_string(app.home.join("config").join("config.toml")).unwrap()
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn a_server_is_added_from_the_connect_palette_after_it_was_tried() {
    if !cfg!(windows) {
        println!("skipped: the page is driven on Windows only");
        return;
    }
    let mut server = Server::start();
    server.session("alpha");
    let (app, pages) = application_to_drive("");
    let before = written(&app);
    assert_eq!(listed(&app), []);

    app.ctl(&["command", "lume.connect.open"]).ok("palette");
    until_page(
        &pages,
        "the palette to have the keyboard",
        "document.activeElement?.className === 'connect-input'",
    );
    press(&pages, &["Ctrl+Shift+A"]);
    until_page(
        &pages,
        "the dialog to have the keyboard",
        "document.activeElement?.id === 'connection-address'",
    );
    // Nothing can be added that is not an address.
    press(&pages, &["text:not an address"]);
    until_page(
        &pages,
        "the address to be refused",
        "document.querySelector('.connection-read[data-tone=warn]') !== null",
    );
    press(&pages, &["Enter"]);
    assert_eq!(written(&app), before);
    page(
        &pages,
        "(() => { document.querySelector('#connection-address').select(); return true; })()",
    );
    press(&pages, &[&format!("text:{}", server.address)]);
    until_page(
        &pages,
        "the address to be read",
        "document.querySelector('.connection-read[data-tone=good]') !== null",
    );
    // With the server gone the test says so, and the address can be added all the same.
    server.stop();
    until_page(&pages, "Test", &pressing(".connection-foot", "Test"));
    until_texts(&pages, ".connection-status", &["It could not be reached"]);
    server.run();
    until_page(&pages, "Test", &pressing(".connection-foot", "Test"));
    until_texts(&pages, ".connection-status", &["Connection works"]);
    let steps = texts(&pages, ".connection-step");
    assert!(
        steps[1].contains("rex ") && steps[1].contains("Windows"),
        "{steps:?}"
    );
    // What was tried is let go of, and is in no list.
    assert_eq!(listed(&app), []);

    page(
        &pages,
        "(() => { document.querySelector('#connection-name').focus(); return true; })()",
    );
    press(&pages, &["text:Mine", "Enter"]);
    until("the server to be listed", || {
        (listed(&app) == [("Mine".to_owned(), server.address.clone())]).then_some(())
    });
    // The selector shows it, and connects to it.
    until_texts(
        &pages,
        ".selector-host[aria-selected=true] .selector-name",
        &["Mine"],
    );
    mark(&pages, "Connect");
    press(&pages, &["Enter"]);
    until_texts(&pages, ".selector-row .selector-name", &["alpha"]);
    press(&pages, &["Escape"]);
    let text = written(&app);
    assert!(
        text.contains(&format!(
            "[[rex.host]]\nlabel = \"Mine\"\naddress = \"{}\"\n",
            server.address
        )),
        "{text}"
    );
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn the_selector_renames_a_server_and_changes_how_it_is_reached() {
    if !cfg!(windows) {
        println!("skipped: the page is driven on Windows only");
        return;
    }
    let server = Server::start();
    server.session("alpha");
    // The address of no server: it is put right.
    let wrong = format!("{}-not", server.address);
    let (app, pages) = application_to_drive(&format!(
        "\n# The machine in the cellar.\n[[rex.host]]\nlabel = 'old'\naddress = '{wrong}'\n"
    ));
    selector(&pages);
    look_at(&pages, "old");
    mark(&pages, "Connect");
    press(&pages, &["Enter"]);
    until_page(
        &pages,
        "the server to be said to be out of reach",
        "document.querySelector('.selector-why')?.textContent.startsWith('Unreachable')",
    );
    mark(&pages, "Edit connection…");
    press(&pages, &["Enter"]);
    until_page(
        &pages,
        "the dialog to have the keyboard",
        "document.activeElement?.id === 'connection-address' \
         && document.activeElement.value.endsWith('-not')",
    );
    press(
        &pages,
        &["End", "Backspace", "Backspace", "Backspace", "Backspace"],
    );
    until_page(
        &pages,
        "the address to be read",
        &format!(
            "document.activeElement.value === {} \
             && document.querySelector('.connection-read[data-tone=good]') !== null",
            json!(server.address)
        ),
    );
    press(&pages, &["Enter"]);
    until("the address to be put right", || {
        (listed(&app) == [("old".to_owned(), server.address.clone())]).then_some(())
    });
    mark(&pages, "Connect");
    press(&pages, &["Enter"]);
    until_texts(&pages, ".selector-row .selector-name", &["alpha"]);

    // Called something else in place, it stays connected.
    press(&pages, &["Tab"]);
    until_page(
        &pages,
        "the hosts to have the arrows",
        "document.querySelector('.selector-host[data-marked] .selector-name')\
         ?.textContent === 'old'",
    );
    press(&pages, &["F2"]);
    until_page(
        &pages,
        "the name to have the keyboard",
        "document.activeElement?.className === 'selector-rename'",
    );
    press(&pages, &["text:cellar", "Enter"]);
    until("the server to be renamed", || {
        (listed(&app) == [("cellar".to_owned(), server.address.clone())]).then_some(())
    });
    assert_eq!(host(&app)["state"]["kind"], "connected");
    let text = written(&app);
    assert!(
        text.contains("# The machine in the cellar.\n[[rex.host]]\nlabel = \"cellar\""),
        "{text}"
    );
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn the_settings_window_adds_a_server_and_takes_it_out() {
    if !cfg!(windows) {
        println!("skipped: the page is driven on Windows only");
        return;
    }
    const SETTINGS: &str = "settings.html";
    let server = Server::start();
    server.session("alpha");
    let (app, pages) = application_to_drive("");
    let before = written(&app);
    app.ctl(&["command", "lume.settings.open", r#"{"section":"hosts"}"#])
        .ok("opening the settings");
    let until_settings = |what: &str, expression: &str| {
        until(what, || {
            (pages.evaluate(SETTINGS, expression) == Some(json!(true))).then_some(())
        });
    };
    until_settings("Add host…", &pressing(".settings-main", "Add host…"));
    until_settings(
        "the dialog to have the keyboard",
        "document.activeElement?.id === 'connection-address'",
    );
    pages.press(SETTINGS, &[&format!("text:{}", server.address)]);
    until_settings("Test", &pressing(".connection-foot", "Test"));
    until_settings(
        "the test to have gone well",
        "document.querySelector('.connection-status')?.textContent === 'Connection works'",
    );
    pages.evaluate(
        SETTINGS,
        "(() => { document.querySelector('#connection-name').focus(); return true; })()",
    );
    pages.press(SETTINGS, &["text:Mine", "Enter"]);
    until("the server to be listed", || {
        (listed(&app) == [("Mine".to_owned(), server.address.clone())]).then_some(())
    });
    let badge = |says: &str| {
        format!(
            "document.querySelector('.host-row .host-badge')?.textContent === {}",
            json!(says)
        )
    };
    until_settings("its row to be there", &badge("Idle"));
    // The page follows how the server stands.
    command(&app, "lume.rex.connect", json!({ "host": "Mine" })).ok("connect");
    until_settings("the server to be said to be connected", &badge("Connected"));

    until_settings("Remove", &pressing(".host-row", "Remove"));
    until_settings("the question", &pressing(".host-row", "Keep"));
    until_settings("Remove again", &pressing(".host-row", "Remove"));
    until_settings(
        "the question again",
        "[...document.querySelectorAll('.addon-ask')].length === 1",
    );
    until_settings("the answer", &pressing(".host-row", "Remove"));
    until("the server to be taken out", || {
        listed(&app).is_empty().then_some(())
    });
    until_settings(
        "its row to be gone",
        "document.querySelector('.host-row') === null",
    );
    assert_eq!(written(&app), before);
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn a_machine_reached_over_ssh_is_added_and_is_listed_with_the_profiles() {
    if !cfg!(windows) {
        println!("skipped: the page is driven on Windows only");
        return;
    }
    let (app, pages) = application_to_drive("");
    app.ctl(&["command", "lume.connection.add", r#"{"kind":"ssh"}"#])
        .ok("the dialog");
    until_page(
        &pages,
        "the dialog to have the keyboard",
        "document.activeElement?.id === 'connection-host'",
    );
    // Nothing listens there: the test says so, and the machine can be added all the same.
    press(&pages, &["text:127.0.0.1", "Tab", "text:nobody", "Tab"]);
    page(
        &pages,
        "(() => { document.querySelector('#connection-port').select(); return true; })()",
    );
    press(&pages, &[&format!("text:{}", free_port())]);
    until_page(&pages, "Test", &pressing(".connection-foot", "Test"));
    until_texts(&pages, ".connection-status", &["It could not be reached"]);
    press(&pages, &["Enter"]);
    // The Connect palette shows it.
    until_page(
        &pages,
        "the profile to be listed",
        "[...document.querySelectorAll('.connect-row-name')]\
         .some((one) => one.textContent === '127.0.0.1')",
    );
    until_page(
        &pages,
        "the machines reached over SSH to be what is listed",
        "document.querySelector('.connect-chip[data-filter=ssh]')?.ariaPressed === 'true'",
    );
    let text = written(&app);
    assert!(text.contains("[[profile]]\nid = \"127.0.0.1\"\n"), "{text}");
    assert!(text.contains("user = \"nobody\"\n"), "{text}");
    press(&pages, &["Escape"]);
    app.quit();
}

/// The question an account with a password is asked.
const ASKS: &str = "me@127.0.0.1's password: ";
const PASSWORD: &str = "correct horse 7351";

/// A machine that is signed in to with a password and has the server of a test: the SSH
/// client that reaches it is the stand-in for it.
struct Machine {
    folder: PathBuf,
    port: u16,
    _bridge: tokio::runtime::Runtime,
}

impl Machine {
    fn of(server: &Server) -> Self {
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let folder = std::env::temp_dir().join(format!("lume-ssh-{}-{number}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let bridge = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        // What the client forwards to is a port; the server of the test listens elsewhere.
        let address = server.address.clone();
        let port = bridge.block_on(async move {
            let door = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = door.local_addr().unwrap().port();
            tokio::spawn(async move {
                while let Ok((mut near, _)) = door.accept().await {
                    let address = address.clone();
                    tokio::spawn(async move {
                        #[cfg(windows)]
                        let far = tokio::net::windows::named_pipe::ClientOptions::new()
                            .open(address.trim_start_matches("npipe://").replace('/', "\\"));
                        #[cfg(unix)]
                        let far =
                            tokio::net::UnixStream::connect(address.trim_start_matches("unix://"))
                                .await;
                        let _ = tokio::io::copy_bidirectional(&mut near, &mut far.unwrap()).await;
                    });
                }
            });
            port
        });
        let script = json!({
            "asks": [{
                "words": ASKS,
                "right": PASSWORD,
                "tries": 3,
                "again": "Permission denied, please try again.",
                "failure": "me@127.0.0.1: Permission denied (publickey,password).",
            }],
            "status": r#"{"endpoint":"unix:///run/rex/server.sock"}"#,
            "target": format!("127.0.0.1:{port}"),
            "cut": folder.join("cut"),
            "alive": folder.join("alive"),
            "log": folder.join("log"),
        });
        std::fs::write(folder.join("script"), script.to_string()).unwrap();
        Self {
            folder,
            port,
            _bridge: bridge,
        }
    }

    fn address(&self) -> String {
        format!("ssh://me@127.0.0.1:{}", self.port)
    }

    fn settings(&self) -> String {
        format!(
            "\n[[rex.host]]\nlabel = '{HOST}'\naddress = '{}'\n",
            self.address()
        )
    }

    /// What has an application use the stand-in for its SSH client.
    fn environment(&self) -> Vec<(&'static str, String)> {
        let name = format!("stand_in_ssh{}", std::env::consts::EXE_SUFFIX);
        let client = application().with_file_name("examples").join(name);
        assert!(client.is_file(), "{} is not there", client.display());
        let script = self.folder.join("script");
        vec![
            ("LUME_TEST_SSH_CLIENT", client.display().to_string()),
            ("STAND_IN_SSH", script.display().to_string()),
        ]
    }

    /// The application, with the stand-in for its SSH client.
    fn application(&self, more: &str) -> (App, Pages) {
        application_to_drive_with(more, &self.environment())
    }

    /// What happened to the client so far.
    fn log(&self) -> Vec<String> {
        std::fs::read_to_string(self.folder.join("log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

/// Waits until the SSH client that forwards has been ended.
fn until_ended(machine: &Machine) {
    let alive = machine.folder.join("alive");
    until("the SSH client to end", || {
        let _ = std::fs::remove_file(&alive);
        std::thread::sleep(Duration::from_millis(400));
        (!alive.exists()).then_some(())
    });
}

/// Waits until the window asks what the SSH client wants to know, with the keyboard in it.
fn until_asked(pages: &Pages, again: bool) {
    until_page(
        pages,
        "the question to have the keyboard",
        &format!(
            "document.activeElement?.matches('dialog.sign-in[open] input') === true \
             && (document.querySelector('.sign-in .prompt-warning') !== null) === {again}"
        ),
    );
}

fn until_not_asked(pages: &Pages) {
    until_page(
        pages,
        "the question to be gone",
        "document.querySelector('dialog.sign-in') === null",
    );
}

/// What is asserted of an application that was told the password: that it wrote it
/// nowhere.
fn wrote_nothing_down(app: &App) {
    let log = std::fs::read_to_string(&app.log).unwrap();
    assert!(!log.contains(PASSWORD), "the password is in the log");
    assert!(
        !log.contains("LUME_ASKPASS"),
        "the environment is in the log"
    );
    let mut folders = vec![app.home.join("config"), app.home.join("state")];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                folders.push(path);
            } else if let Ok(bytes) = std::fs::read(&path) {
                let held = bytes
                    .windows(PASSWORD.len())
                    .any(|part| part == PASSWORD.as_bytes());
                assert!(!held, "the password is in {}", path.display());
            }
        }
    }
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn a_machine_that_takes_a_password_is_signed_in_to_in_the_window() {
    if !cfg!(windows) {
        println!("skipped: the page is driven on Windows only");
        return;
    }
    let server = Server::start();
    server.session("alpha");
    let machine = Machine::of(&server);
    let (app, pages) = machine.application(&machine.settings());
    connect_palette(&app, &pages, HOST);
    press(&pages, &["Enter"]);
    until_asked(&pages, false);
    assert_eq!(texts(&pages, ".sign-in h2"), [format!("Sign in to {HOST}")]);
    assert_eq!(texts(&pages, ".sign-in pre"), [ASKS.trim()]);
    assert_eq!(texts(&pages, ".sign-in-address"), [machine.address()]);

    // Nobody answers for the user.
    for proceed in [true, false] {
        let answered = command(&app, "lume.window.answer", json!({ "proceed": proceed }));
        assert_eq!(answered.json["code"], "forbidden", "{}", answered.json);
    }
    // Keys do not reach what is behind.
    press(&pages, &["Tab", "Tab", "Tab", "Tab", "Tab"]);
    assert_eq!(
        page(
            &pages,
            "document.activeElement === document.body \
             || document.activeElement?.closest('dialog.sign-in') !== null"
        ),
        true
    );
    page(
        &pages,
        "(() => { document.querySelector('.sign-in input').focus(); return true; })()",
    );
    press(&pages, &["text:wrong horse", "Enter"]);
    until_asked(&pages, true);
    press(&pages, &[&format!("text:{PASSWORD}"), "Enter"]);
    until_not_asked(&pages);
    // The palette stayed, lists the sessions and has the keyboard again.
    until_texts(&pages, ".connect-session-name", &["alpha"]);
    until_page(
        &pages,
        "the palette to have the keyboard",
        "document.activeElement?.className === 'connect-input'",
    );
    // The second login was not asked for.
    assert_eq!(
        machine.log(),
        [
            "may ask", "asks", "wrong", "asks", "right", "told", "may ask", "asks", "right",
            "forwards"
        ]
    );
    press(&pages, &["Enter"]);
    until_on_show(&app, "alpha");

    // The connection is lost, and cannot be made again without asking.
    std::fs::write(machine.folder.join("cut"), "").unwrap();
    until_texts(&pages, ".notice-action", &["Connect again"]);
    assert_eq!(host(&app)["state"]["code"], "ssh");
    assert!(machine.log().contains(&"quiet".to_owned()));
    std::fs::remove_file(machine.folder.join("cut")).unwrap();
    page(
        &pages,
        "(() => { document.querySelector('.notice-action').click(); return true; })()",
    );
    until_asked(&pages, false);
    press(&pages, &[&format!("text:{PASSWORD}"), "Enter"]);
    until("the session to be on show again", || {
        (host(&app)["on_show"].is_string() && host(&app)["state"]["kind"] == "connected")
            .then_some(())
    });
    wrote_nothing_down(&app);
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn a_sign_in_that_is_put_away_tries_nothing_and_is_not_held_against_the_server() {
    if !cfg!(windows) {
        println!("skipped: the page is driven on Windows only");
        return;
    }
    let server = Server::start();
    server.session("alpha");
    let machine = Machine::of(&server);
    let (app, pages) = machine.application(&machine.settings());

    // Asked for from outside the window, it asks nobody.
    let refused = command(&app, "lume.rex.connect", json!({ "host": HOST }));
    assert_eq!(refused.json["code"], "ssh", "{}", refused.json);
    assert_eq!(machine.log(), ["quiet", "refused"]);
    std::fs::remove_file(machine.folder.join("log")).unwrap();

    connect_palette(&app, &pages, HOST);
    press(&pages, &["Enter"]);
    until_asked(&pages, false);
    press(&pages, &["Escape"]);
    until_not_asked(&pages);
    until("the server to stand as before", || {
        (host(&app)["state"]["kind"] == "idle").then_some(())
    });
    until_texts(&pages, ".connect-row-line", &["Not connected"]);
    until_page(
        &pages,
        "the palette to have the keyboard",
        "document.activeElement?.className === 'connect-input'",
    );
    // The client was ended before it could take that for an empty password.
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(machine.log(), ["may ask", "asks"]);
    assert_eq!(texts(&pages, ".notice"), Vec::<String>::new());

    // Disconnected while it is asked, the question is taken back.
    press(&pages, &["Enter"]);
    until_asked(&pages, false);
    command(&app, "lume.rex.disconnect", json!({ "host": HOST })).ok("disconnect");
    until_not_asked(&pages);
    assert_eq!(host(&app)["state"]["kind"], "idle");

    // Three wrong ones are the machine's own no.
    press(&pages, &["Enter"]);
    for again in [false, true, true] {
        until_asked(&pages, again);
        press(&pages, &["text:wrong horse", "Enter"]);
        // The next question is one of its own.
        until_page(
            &pages,
            "the field to be another",
            "!document.querySelector('.sign-in input')?.value",
        );
    }
    until_texts(&pages, ".connect-row-line", &["SSH failed"]);
    let why = texts(&pages, ".connect-call");
    assert!(why[0].contains("Permission denied"), "{why:?}");
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn a_test_of_a_connection_asks_and_stops_asking_when_nobody_waits_for_it() {
    if !cfg!(windows) {
        println!("skipped: the page is driven on Windows only");
        return;
    }
    let server = Server::start();
    let machine = Machine::of(&server);
    let (app, pages) = machine.application("");
    let dialog = || {
        app.ctl(&["command", "lume.connection.add"]).ok("dialog");
        until_page(
            &pages,
            "the dialog to have the keyboard",
            "document.activeElement?.id === 'connection-address'",
        );
        press(&pages, &[&format!("text:{}", machine.address())]);
        until_page(&pages, "Test", &pressing(".connection-foot", "Test"));
        until_asked(&pages, false);
        assert_eq!(texts(&pages, ".sign-in h2"), ["Sign in to 127.0.0.1"]);
    };
    dialog();
    press(&pages, &["Escape"]);
    until_not_asked(&pages);
    until_texts(&pages, ".connection-status", &["You did not sign in"]);
    until_page(
        &pages,
        "the dialog to be there",
        "document.querySelector('.connection') !== null",
    );
    until_page(&pages, "Test", &pressing(".connection-foot", "Test"));
    until_asked(&pages, false);
    press(&pages, &[&format!("text:{PASSWORD}"), "Enter"]);
    until_texts(&pages, ".connection-status", &["Connection works"]);

    // The dialog goes while the question is on show: so does the question.
    until_page(&pages, "Test", &pressing(".connection-foot", "Test"));
    until_asked(&pages, false);
    page(
        &pages,
        "(() => { document.querySelector('.connection-scrim').dispatchEvent(\
         new PointerEvent('pointerdown', { bubbles: true })); return true; })()",
    );
    until_not_asked(&pages);
    assert_eq!(
        page(&pages, "document.querySelector('.connection') === null"),
        true
    );
    assert!(!machine.log().contains(&"empty".to_owned()));
    wrote_nothing_down(&app);
    app.quit();
}

/// Starts `lume` as the user would in a console, for an application that runs.
fn lume_with(app: &App, arguments: &[&str]) {
    let status = Command::new(lume())
        .args(["--endpoint", &app.endpoint])
        .args(arguments)
        .stdin(Stdio::null())
        .status()
        .expect("lume runs");
    assert!(status.success(), "lume {arguments:?}: {status}");
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn the_application_is_started_on_a_server_and_is_handed_one_while_it_runs() {
    if !cfg!(windows) {
        println!("skipped: the page is driven on Windows only");
        return;
    }
    let mut server = Server::start();
    server.session("alpha");
    server.session("beta");
    let (app, pages) =
        application_started("", &settings(&server), &[], &[&format!("--rex={HOST}")]);
    until_on_show(&app, "alpha");
    until_button(&pages, HOST, "alpha");
    open(&app, "beta");
    command(&app, "lume.workspace.switch", json!({ "name": "default" })).ok("switch");
    until("the own workspace to be on show", || {
        own_workspace_on_show(&app).then_some(())
    });
    // The session last shown is the one a start shows.
    lume_with(&app, &["--rex", HOST]);
    until_on_show(&app, "beta");
    command(&app, "lume.workspace.switch", json!({ "name": "default" })).ok("switch");
    command(&app, "lume.rex.disconnect", json!({ "host": HOST })).ok("disconnect");

    // With the server gone it is said, and can be tried again.
    server.stop();
    lume_with(&app, &["--rex", &server.address]);
    until_texts(&pages, ".notice-action", &["Try again"]);
    assert!(own_workspace_on_show(&app));
    server.run();
    page(
        &pages,
        "(() => { document.querySelector('.notice-action').click(); return true; })()",
    );
    until("a session to be on show", || {
        (!own_workspace_on_show(&app)).then_some(())
    });
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn a_start_on_a_server_without_sessions_or_that_is_not_listed_offers_what_can_be_done() {
    if !cfg!(windows) {
        println!("skipped: the page is driven on Windows only");
        return;
    }
    let server = Server::start();
    let (app, pages) =
        application_started("", &settings(&server), &[], &[&format!("--rex={HOST}")]);
    until_page(
        &pages,
        "the palette to mark the server",
        &format!(
            "document.querySelector('.connect-row[aria-selected=true] .connect-row-name')\
             ?.textContent === {}",
            json!(HOST)
        ),
    );
    assert!(own_workspace_on_show(&app));
    press(&pages, &["Escape"]);
    until_page(
        &pages,
        "the palette to be gone",
        "document.querySelector('.connect') === null",
    );

    lume_with(&app, &["--rex", "nobody"]);
    until_texts(
        &pages,
        ".notice-message",
        &["There is no Rex server called nobody."],
    );
    lume_with(&app, &["--rex=ssh://me@nas.example"]);
    until_page(
        &pages,
        "the dialog to hold the address",
        "document.querySelector('#connection-address')?.value === 'ssh://me@nas.example'",
    );
    press(&pages, &["Escape"]);
    app.quit();
}

#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn the_settings_start_the_application_on_a_server_unless_it_is_started_for_something() {
    if !cfg!(windows) {
        println!("skipped: the page is driven on Windows only");
        return;
    }
    let server = Server::start();
    server.session("alpha");
    let machine = Machine::of(&server);
    let rex = format!("start-on = '{}'\n", machine.address());
    let more = machine.settings();
    let environment = machine.environment();
    let started = |arguments: &[&str]| application_started(&rex, &more, &environment, arguments);
    let (app, pages) = started(&[]);
    until_asked(&pages, false);
    press(&pages, &["Escape"]);
    until_not_asked(&pages);
    until("the server to stand as before", || {
        (host(&app)["state"]["kind"] == "idle").then_some(())
    });
    assert!(own_workspace_on_show(&app));
    assert_eq!(texts(&pages, ".notice"), Vec::<String>::new());
    until_page(
        &pages,
        "the terminal to have the keyboard",
        "document.activeElement?.classList.contains('terminal-input') === true",
    );
    app.quit();

    let (app, pages) = started(&[]);
    until_asked(&pages, false);
    press(&pages, &[&format!("text:{PASSWORD}"), "Enter"]);
    until_on_show(&app, "alpha");
    // What carries the connection ends with the application.
    assert!(machine.folder.join("alive").exists());
    app.quit();
    until_ended(&machine);

    let (app, pages) = started(&["--workspace", "site"]);
    app.status_with_session();
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(host(&app)["state"]["kind"], "idle");
    until_not_asked(&pages);
    app.quit();
}

/// A place within a tab: where it begins, across and down, and how wide and high it is,
/// each in hundredths of the tab.
type Area = [i64; 4];

/// A session's layout, as far as a window and the server both have it, by the names its
/// things were given.
#[derive(Debug, Default, PartialEq)]
struct Shape {
    tabs: Vec<ShapedTab>,
    /// The tab on show.
    shown: usize,
    /// The pane that has the keyboard.
    keyboard: String,
    /// What floats over the tab on show.
    floating: BTreeMap<String, Area>,
}

#[derive(Debug, PartialEq)]
struct ShapedTab {
    name: String,
    panes: BTreeMap<String, Area>,
    zoomed: Option<String>,
}

impl Shape {
    /// The names of a tab's panes.
    fn panes(&self, tab: usize) -> Vec<&str> {
        let tab = self.tabs.get(tab).into_iter();
        tab.flat_map(|tab| tab.panes.keys())
            .map(String::as_str)
            .collect()
    }

    fn area(&self, tab: usize, pane: &str) -> Option<Area> {
        self.tabs.get(tab)?.panes.get(pane).copied()
    }

    fn zoomed(&self, tab: usize) -> Option<&str> {
        self.tabs.get(tab)?.zoomed.as_deref()
    }
}

fn hundredths(of: [f64; 4]) -> Area {
    of.map(|part| (part * 100.0).round() as i64)
}

/// Where the panes of a tab's tree are, worked out from the shares of its splits.
fn areas(node: &Value, within: [f64; 4], found: &mut BTreeMap<String, Area>) {
    if node["type"] == "pane" {
        let name = node["title"].as_str().unwrap_or_default().to_owned();
        found.insert(name, hundredths(within));
        return;
    }
    let side = if node["axis"] == "row" { 0 } else { 1 };
    let mut part = within;
    for (child, share) in node["children"]
        .as_array()
        .unwrap()
        .iter()
        .zip(node["ratios"].as_array().unwrap())
    {
        part[side + 2] = within[side + 2] * share.as_f64().unwrap() / 1_000_000.0;
        areas(child, part, found);
        part[side] += part[side + 2];
    }
}

/// The names of the panes of a window's layout, by their ids.
fn pane_names(window: &Value) -> BTreeMap<String, String> {
    fn gather(node: &Value, found: &mut BTreeMap<String, String>) {
        if let Some(id) = node["id"].as_str().filter(|_| node["type"] == "pane") {
            let name = node["title"].as_str().unwrap_or_default();
            found.insert(id.to_owned(), name.to_owned());
        }
        for child in node["children"].as_array().into_iter().flatten() {
            gather(child, found);
        }
    }
    let mut found = BTreeMap::new();
    for tab in window["centre"]["tabs"].as_array().unwrap() {
        gather(&tab["root"], &mut found);
    }
    for floating in window["floating"].as_array().unwrap() {
        let pane = &floating["pane"];
        found.insert(
            pane["id"].as_str().unwrap().to_owned(),
            pane["title"].as_str().unwrap_or_default().to_owned(),
        );
    }
    found
}

/// The layout on show in the application's window.
fn shape_here(app: &App) -> Shape {
    let layout = app.layout();
    let window = &layout["workspace"]["windows"][0];
    let names = pane_names(window);
    let name_of = |id: &Value| names.get(id.as_str()?).cloned();
    let tabs = window["centre"]["tabs"].as_array().unwrap();
    Shape {
        shown: tabs
            .iter()
            .position(|tab| tab["id"] == window["centre"]["active"])
            .unwrap_or_default(),
        tabs: tabs
            .iter()
            .map(|tab| {
                let mut panes = BTreeMap::new();
                areas(&tab["root"], [0.0, 0.0, 1.0, 1.0], &mut panes);
                ShapedTab {
                    name: tab["title"].as_str().unwrap_or_default().to_owned(),
                    panes,
                    zoomed: name_of(&tab["zoomed"]),
                }
            })
            .collect(),
        keyboard: name_of(&window["focus"]).unwrap_or_default(),
        floating: window["floating"]
            .as_array()
            .unwrap()
            .iter()
            .map(|floating| {
                let part = |name: &str| floating["rect"][name].as_f64().unwrap() / 1_000_000.0;
                (
                    floating["pane"]["title"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    hundredths([part("x"), part("y"), part("w"), part("h")]),
                )
            })
            .collect(),
    }
}

/// The name a thing of a server was given. What was given none has one of the server's
/// own, which the application does not show.
fn given(label: &Value) -> String {
    let label = label.as_str().unwrap_or_default();
    let own = label == "shell" || label.starts_with("Window ");
    if own { "" } else { label }.to_owned()
}

/// The same layout as the server has it, read by another client of it.
fn shape_there(server: &Server, session: &str) -> Shape {
    fn rects(node: &Value, names: &BTreeMap<String, String>, found: &mut BTreeMap<String, Area>) {
        if let Some(block) = node["block_id"].as_str() {
            let rect: [f64; 4] = serde_json::from_value(node["rect"].clone()).unwrap();
            found.insert(
                names.get(block).cloned().unwrap_or_default(),
                hundredths(rect),
            );
        }
        for side in ["before", "after"] {
            if node["split"][side].is_object() {
                rects(&node["split"][side], names, found);
            }
        }
    }
    let view = server.view(session);
    let windows = view["windows"].as_array().unwrap();
    let mut shape = Shape::default();
    for (index, window) in windows.iter().enumerate() {
        let layers = window["layers"].as_array().unwrap();
        let mut names = BTreeMap::new();
        for block in layers
            .iter()
            .flat_map(|layer| layer["blocks"].as_array().into_iter().flatten())
        {
            names.insert(
                block["block_id"].as_str().unwrap().to_owned(),
                given(&block["label"]),
            );
        }
        let name_of = |id: &Value| names.get(id.as_str()?).cloned();
        let on_show = window["window_id"] == view["active_window_id"];
        let mut panes = BTreeMap::new();
        for layer in layers {
            if layer["kind"] == "tiled" {
                rects(&layer["layout"], &names, &mut panes);
            } else if on_show {
                // A layer has one block here, and so is as large as the block.
                let bounds: [f64; 4] = serde_json::from_value(layer["bounds"].clone()).unwrap();
                let name = given(&layer["blocks"][0]["label"]);
                shape.floating.insert(name, hundredths(bounds));
            }
        }
        if on_show {
            shape.shown = index;
            shape.keyboard = name_of(&window["focused_block_id"]).unwrap_or_default();
        }
        shape.tabs.push(ShapedTab {
            name: given(&window["label"]),
            panes,
            zoomed: name_of(&window["zoomed_block_id"]),
        });
    }
    shape
}

/// Waits until the window and the server have the same layout, and it is as `wanted`
/// says: one that only one of them has yet is not waited for any less.
fn agreed(app: &App, server: &Server, what: &str, wanted: impl Fn(&Shape) -> bool) -> Shape {
    let deadline = Instant::now() + SOON;
    loop {
        let here = shape_here(app);
        let there = shape_there(server, WALKED);
        if here == there && wanted(&here) {
            return here;
        }
        assert!(
            Instant::now() < deadline,
            "waited too long for {what}:\n  the window has {here:?}\n  the server has {there:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The panes that the window's page shows as cards.
fn cards(pages: &Pages) -> Vec<String> {
    let found = page(
        pages,
        "[...document.querySelectorAll('.pane-card')] \
         .filter((card) => card.checkVisibility({ visibilityProperty: true }) \
           && card.getBoundingClientRect().width > 0) \
         .map((card) => card.dataset.pane).sort()",
    );
    serde_json::from_value(found).unwrap_or_default()
}

/// Waits until the page shows these panes as cards and no others.
fn until_cards(app: &App, pages: &Pages, wanted: &[&str]) {
    let names = pane_names(&app.layout()["workspace"]["windows"][0]);
    let mut ids: Vec<String> = names
        .into_iter()
        .filter(|(_, name)| wanted.contains(&name.as_str()))
        .map(|(id, _)| id)
        .collect();
    ids.sort();
    assert_eq!(ids.len(), wanted.len(), "{wanted:?}");
    let deadline = Instant::now() + SOON;
    loop {
        let shown = cards(pages);
        if shown == ids {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the page shows {shown:?} as cards, not {ids:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Lines as they are compared: blanks at their ends and empty lines at the end of the
/// whole do not count.
fn plain_lines<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut lines: Vec<String> = lines.map(|line| line.trim_end().to_owned()).collect();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

/// What the page's own terminal holds, as it would copy it whole.
fn copied_whole(app: &App, pages: &Pages) -> Option<Vec<String>> {
    watch(pages);
    asked(pages);
    app.ctl(&["command", "lume.terminal.select-all"])
        .ok("select all");
    // Whoever runs the test has a clipboard, which is not to be written to. The page has
    // just answered, so nothing seen means that it is not watched.
    assert!(!asked(pages).is_empty(), "the page is not watched");
    app.ctl(&["command", "lume.terminal.copy"]).ok("copy");
    let text = (0..20).find_map(|_| {
        let text = copied(pages);
        if text.is_none() {
            std::thread::sleep(Duration::from_millis(50));
        }
        text
    })?;
    Some(plain_lines(text.lines()))
}

/// Waits until the terminal of the window's page, the application and `rex capture` all
/// have the same screen, the whole of it, and gives its size. The pane has the keyboard.
fn same_screen(
    app: &App,
    pages: Option<&Pages>,
    server: &Server,
    pane: &str,
    block: &str,
) -> (u64, u64) {
    let deadline = Instant::now() + SOON;
    loop {
        let captured = app.ctl(&["capture", "--pane", pane]).ok("capture");
        let lines = captured["lines"].as_array().unwrap().iter();
        let here = plain_lines(lines.map(|line| line.as_str().unwrap()));
        let there = server.captured(block);
        let there = plain_lines(there.iter().map(String::as_str));
        let size = (
            captured["size"]["cols"].as_u64().unwrap(),
            captured["size"]["rows"].as_u64().unwrap(),
        );
        // A copy joins the lines that are one line of the program's, and begins at the
        // first thing there is to see.
        let joined = server.rex(&["capture", "-b", block, "--trim", "--unwrap"]);
        let joined = plain_lines(joined.trim_start().lines());
        let in_page = pages.map(|pages| copied_whole(app, pages).unwrap_or_default());
        let same = here == there
            && captured["alternate"] == true
            && size == server.size(WALKED, block)
            && in_page.as_ref().is_none_or(|page| *page == joined);
        if same {
            assert!(!here.is_empty());
            return size;
        }
        assert!(
            Instant::now() < deadline,
            "the screens differ at {size:?}, the server's being {:?}:\n  \
             the application: {here:?}\n  the server: {there:?}\n  the page: {in_page:?}\n  \
             the server, joined: {joined:?}",
            server.size(WALKED, block)
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The session that is walked through.
const WALKED: &str = "walk";

/// Everything a user does with a session, start to end: found among the others, opened,
/// typed into, taken apart from both sides, left and taken up again. After each step the
/// window's layout is the one another client is given by the server, and the screen of
/// the program is the one `rex capture` prints.
#[test]
#[ignore = "needs a Rex server; run with `cargo xtask rex-e2e`"]
fn a_session_is_walked_through_and_is_the_same_here_and_on_the_server_at_every_step() {
    let server = Server::start();
    // Lines of more than one script, and one that is longer than any terminal is wide.
    let mut text =
        String::from("line number 1\nw\u{f6}rter, \u{65e5}\u{672c}\u{8a9e}, \u{43d}\u{430}\n");
    text += &format!("{}\n", "a long line of words ".repeat(30));
    for line in 4..=300 {
        text += &format!("line number {line}\n");
    }
    let Some(block) = server.editor(WALKED, &text, "line number 4") else {
        return;
    };
    server.session("beside it");
    let first_window = server.view(WALKED)["windows"][0]["window_id"]
        .as_str()
        .unwrap()
        .to_owned();
    server.rex(&["block", "rename", "-s", WALKED, &block, "editor"]);
    server.rex(&["window", "rename", "-s", WALKED, &first_window, "first"]);

    let (app, pages) = application_with_pages(&server);
    let pages = pages.as_ref();
    if pages.is_none() {
        println!("what the page shows is not looked at here");
    }
    let cards_are = |app: &App, pages: Option<&Pages>, wanted: &[&str]| {
        if let Some(pages) = pages {
            until_cards(app, pages, wanted);
        }
    };

    // Listed, and opened: its layout is a card.
    command(&app, "lume.rex.connect", json!({ "host": HOST })).ok("connect");
    let listed = host(&app);
    let names: Vec<&Value> = listed["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|session| &session["label"])
        .collect();
    assert_eq!(names, [WALKED, "beside it"]);
    open(&app, WALKED);
    let shape = agreed(&app, &server, "the session as it was", |shape| {
        shape.keyboard == "editor"
    });
    assert_eq!(shape.panes(0), ["editor"]);
    cards_are(&app, pages, &["editor"]);
    let pane = only_pane(&app);
    let whole = same_screen(&app, pages, &server, &pane, &block);

    // Typed into, in the page where that is possible. Back to its first line each time it
    // is asked to take text: it drops what is typed while it settles.
    let typed = "typed-in-the-window ";
    until("the editor to take text", || {
        app.ctl(&["send", "--pane", &pane, "--escapes", r"\x1c\x0eggI"])
            .ok("take text");
        std::thread::sleep(Duration::from_secs(1));
        let there = server.captured(&block);
        let takes = there.iter().any(|line| line.contains("-- INSERT"));
        takes.then_some(())
    });
    match pages {
        Some(pages) => {
            let typing = format!(
                "(() => {{ document.querySelector('.terminal-input').focus(); \
                 return document.execCommand('insertText', false, {}); }})()",
                json!(typed)
            );
            assert_eq!(page(pages, &typing), json!(true), "the page took no typing");
        }
        None => {
            app.ctl(&["send", "--pane", &pane, "--", typed]).ok("type");
        }
    }
    app.wait_text_in(&pane, typed);
    until("the server's screen to have what was typed", || {
        let there = server.captured(&block);
        (there[0] == format!("{typed}line number 1")).then_some(())
    });
    // Back to waiting for commands, by keys that no report of a focus is taken for.
    app.ctl(&["send", "--pane", &pane, "--escapes", r"\x1c\x0e"])
        .ok("stop typing");
    same_screen(&app, pages, &server, &pane, &block);

    // Split here: there are two on the server, and the editor has half the height.
    let shell = plain_shell();
    let another = json!({ "type": "terminal", "launcher": {
        "type": "shell", "program": shell[0], "args": &shell[1..], "login": false,
    }});
    apply(
        &app,
        json!({
            "type": "split", "pane": pane, "dir": "down", "ratio": 500_000, "content": another,
        }),
    );
    let panes = until_panes(&app, &[2]);
    let below = panes[0].iter().find(|id| **id != pane).unwrap().clone();
    apply(
        &app,
        json!({ "type": "pane_rename", "pane": below, "title": "below" }),
    );
    agreed(&app, &server, "a split made here", |shape| {
        shape.area(0, "editor") == Some([0, 0, 100, 50])
            && shape.area(0, "below") == Some([0, 50, 100, 50])
    });
    cards_are(&app, pages, &["editor", "below"]);
    let focus = |pane: &str| json!({ "type": "focus", "target": { "type": "pane", "pane": pane } });
    let editor_has_the_keyboard = |app: &App, pane: &str| {
        apply(app, focus(pane));
        agreed(app, &server, "the editor to have the keyboard", |shape| {
            shape.keyboard == "editor"
        });
    };
    editor_has_the_keyboard(&app, &pane);
    let filled = |app: &App, pane: &str, than: u64| {
        until("the editor to take its new size", || {
            // One that is busy can miss that its terminal changed, and sees it when it
            // is next asked to draw.
            app.ctl(&["send", "--pane", pane, "--", "\u{c}"])
                .ok("redraw");
            std::thread::sleep(Duration::from_millis(300));
            let (_, rows) = server.size(WALKED, &block);
            let drawn = server.captured(&block).len() as u64;
            (rows != than && (drawn == rows || drawn + 1 == rows)).then_some(())
        });
    };
    let halved = {
        let sized = until("the terminal to be halved", || {
            let size = server.size(WALKED, &block);
            (size.1 < whole.1).then_some(size)
        });
        // Whatever the editor makes of it, both show the same.
        same_screen(&app, pages, &server, &pane, &block);
        filled(&app, &pane, whole.1);
        assert_eq!(same_screen(&app, pages, &server, &pane, &block), sized);
        sized
    };

    // The border moved here: the third size.
    let layout = app.layout();
    let split = layout["workspace"]["windows"][0]["centre"]["tabs"][0]["root"]["id"].clone();
    apply(
        &app,
        json!({ "type": "resize", "split": split, "ratios": [300_000, 700_000] }),
    );
    agreed(&app, &server, "a border moved here", |shape| {
        shape.area(0, "editor") == Some([0, 0, 100, 30])
    });
    let third = until("the terminal to be smaller", || {
        let size = server.size(WALKED, &block);
        (size.1 < halved.1).then_some(size)
    });
    same_screen(&app, pages, &server, &pane, &block);
    filled(&app, &pane, halved.1);
    assert_eq!(same_screen(&app, pages, &server, &pane, &block), third);

    // Swapped, zoomed and back, floated and back, a tab made, named and closed: here.
    apply(&app, json!({ "type": "swap", "a": pane, "b": below }));
    agreed(&app, &server, "a swap made here", |shape| {
        shape.area(0, "editor") == Some([0, 30, 100, 70])
    });
    apply(&app, json!({ "type": "zoom", "pane": pane, "on": true }));
    agreed(&app, &server, "a zoom made here", |shape| {
        shape.zoomed(0) == Some("editor")
    });
    apply(&app, json!({ "type": "zoom", "pane": pane, "on": false }));
    agreed(&app, &server, "a zoom ended here", |shape| {
        shape.zoomed(0).is_none()
    });
    apply(
        &app,
        json!({
            "type": "float", "pane": below,
            "rect": { "x": 250_000, "y": 250_000, "w": 500_000, "h": 400_000 },
        }),
    );
    agreed(&app, &server, "a pane floated here", |shape| {
        shape.floating.get("below") == Some(&[25, 25, 50, 40]) && shape.panes(0) == ["editor"]
    });
    cards_are(&app, pages, &["editor", "below"]);
    let place = json!({ "type": "beside", "pane": pane, "edge": "right" });
    apply(&app, json!({ "type": "dock", "pane": below, "to": place }));
    agreed(&app, &server, "a pane put back here", |shape| {
        shape.area(0, "below") == Some([50, 0, 50, 100]) && shape.floating.is_empty()
    });
    let window = app.layout()["workspace"]["windows"][0]["id"].clone();
    apply(
        &app,
        json!({ "type": "tab_new", "window": window, "content": another }),
    );
    let tabs = until_panes(&app, &[2, 1]);
    apply(
        &app,
        json!({ "type": "pane_rename", "pane": tabs[1][0], "title": "elsewhere" }),
    );
    let layout = app.layout();
    let second = layout["workspace"]["windows"][0]["centre"]["tabs"][1]["id"].clone();
    apply(
        &app,
        json!({ "type": "tab_rename", "tab": second, "title": "second" }),
    );
    agreed(&app, &server, "a tab made here", |shape| {
        shape.shown == 1 && shape.tabs[1].name == "second" && shape.panes(1) == ["elsewhere"]
    });
    cards_are(&app, pages, &["elsewhere"]);
    apply(
        &app,
        json!({ "type": "tab_move", "tab": second, "index": 0 }),
    );
    agreed(&app, &server, "a tab moved here", |shape| {
        shape.tabs[0].name == "second" && shape.shown == 0
    });
    apply(&app, json!({ "type": "tab_close", "tab": second }));
    agreed(&app, &server, "a tab closed here", |shape| {
        shape.tabs.len() == 1
    });
    editor_has_the_keyboard(&app, &pane);
    same_screen(&app, pages, &server, &pane, &block);

    // The same from the other side: what `rex` changes is changed in the window.
    let id_of = |name: &str| -> String {
        let view = server.view(WALKED);
        let windows = view["windows"].as_array().unwrap().iter();
        windows
            .flat_map(|window| window["layers"].as_array().unwrap())
            .flat_map(|layer| layer["blocks"].as_array().unwrap())
            .find(|found| found["label"] == name)
            .unwrap_or_else(|| panic!("the server has no {name}: {view}"))["block_id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let beside = id_of("below");
    let mut run = vec!["split", "--split", "below", "--ratio", "25", "-b", &beside];
    run.extend(["--label", "theirs", "--shell=none", "--"]);
    run.extend(shell);
    server.rex(&run);
    agreed(&app, &server, "a split made there", |shape| {
        shape.area(0, "theirs") == Some([50, 75, 50, 25]) && shape.keyboard == "theirs"
    });
    cards_are(&app, pages, &["editor", "below", "theirs"]);
    let resize = json!({
        "direction": "horizontal", "ratio": 0.4,
        "before_block_id": block, "after_block_id": beside,
    });
    server.rex(&[
        "api",
        "call",
        "-s",
        WALKED,
        "session.resize_pane",
        &resize.to_string(),
    ]);
    agreed(&app, &server, "a border moved there", |shape| {
        shape.area(0, "editor") == Some([0, 0, 40, 100])
    });
    server.rex(&["swap", "-s", WALKED, "editor", "theirs"]);
    agreed(&app, &server, "a swap made there", |shape| {
        shape.area(0, "theirs") == Some([0, 0, 40, 100])
    });
    server.rex(&["move", "-s", WALKED, "theirs", "above", "below"]);
    agreed(&app, &server, "a move made there", |shape| {
        shape.area(0, "theirs").map(|area| area[2]) == Some(100)
    });
    server.rex(&["zoom", "-s", WALKED, "below", "--on"]);
    agreed(&app, &server, "a zoom made there", |shape| {
        shape.zoomed(0) == Some("below")
    });
    server.rex(&["zoom", "-s", WALKED, "below", "--off"]);
    agreed(&app, &server, "a zoom ended there", |shape| {
        shape.zoomed(0).is_none()
    });
    server.rex(&["block", "rename", "-s", WALKED, "theirs", "ours now"]);
    let mut tab = vec!["window", "new", "-s", WALKED, "third", "--shell=none", "--"];
    tab.extend(shell);
    server.rex(&tab);
    let shape = agreed(&app, &server, "a tab made there", |shape| {
        shape.tabs.len() == 2 && shape.tabs[1].name == "third" && shape.shown == 1
    });
    assert!(shape.panes(0).contains(&"ours now"), "{shape:?}");
    server.rex(&["window", "move", "-s", WALKED, "third", "--index", "1"]);
    agreed(&app, &server, "a tab moved there", |shape| {
        shape.tabs[0].name == "third"
    });
    server.rex(&["window", "focus", "-s", WALKED, "first"]);
    agreed(&app, &server, "a tab shown there", |shape| shape.shown == 1);
    server.rex(&["window", "close", "-s", WALKED, "third"]);
    agreed(&app, &server, "a tab closed there", |shape| {
        shape.tabs.len() == 1
    });
    server.rex(&["block", "close", "-s", WALKED, "ours now"]);
    agreed(&app, &server, "a pane closed there", |shape| {
        shape.panes(0) == ["below", "editor"]
    });
    server.rex(&["focus", "-s", WALKED, "editor"]);
    agreed(&app, &server, "the keyboard given there", |shape| {
        shape.keyboard == "editor"
    });
    cards_are(&app, pages, &["editor", "below"]);
    let left = same_screen(&app, pages, &server, &pane, &block);

    // Left: nobody of this name is at the server, and the program goes on.
    command(&app, "lume.rex.disconnect", json!({ "host": HOST })).ok("disconnect");
    assert!(own_workspace_on_show(&app));
    until("the server to be rid of the application", || {
        let clients = server.rex(&["client", "ls", "--all"]);
        (!clients.contains("Lume")).then_some(())
    });
    server.rex(&["window", "rename", "-s", WALKED, "first", "meanwhile"]);
    until("the editor to go on while nobody looks", || {
        // It drops what is typed while it settles.
        server.rex(&["send", "-b", &block, "20Gzt"]);
        std::thread::sleep(Duration::from_secs(1));
        let there = server.captured(&block);
        (there[0] == "line number 20").then_some(())
    });

    // And taken up again, by this application and then by one started anew.
    open(&app, WALKED);
    agreed(&app, &server, "what was done meanwhile", |shape| {
        shape.tabs[0].name == "meanwhile" && shape.panes(0) == ["below", "editor"]
    });
    let pane = until("the editor's pane", || {
        let names = pane_names(&app.layout()["workspace"]["windows"][0]);
        names.into_iter().find(|(_, name)| name == "editor")
    });
    editor_has_the_keyboard(&app, &pane.0);
    assert_eq!(same_screen(&app, pages, &server, &pane.0, &block), left);
    app.quit();

    let (app, pages) = application_with_pages(&server);
    let pages = pages.as_ref();
    open(&app, WALKED);
    agreed(&app, &server, "the layout after a start", |shape| {
        shape.panes(0) == ["below", "editor"]
    });
    let pane = until("the editor's pane", || {
        let names = pane_names(&app.layout()["workspace"]["windows"][0]);
        names.into_iter().find(|(_, name)| name == "editor")
    });
    editor_has_the_keyboard(&app, &pane.0);
    cards_are(&app, pages, &["editor", "below"]);
    same_screen(&app, pages, &server, &pane.0, &block);
    until("the editor to be gone", || {
        app.ctl(&["send", "--pane", &pane.0, "--escapes", r"\x1c\x0e:q!\r"]);
        std::thread::sleep(Duration::from_secs(1));
        (shape_here(&app).panes(0) == ["below"]).then_some(())
    });
    agreed(&app, &server, "the editor to be gone there", |shape| {
        shape.panes(0) == ["below"]
    });
    app.quit();
}
