//! The application's window with a Rex server that is not to be trusted, or is none of
//! the user's revision: it shows what the server has, and nothing the server says or
//! prints does more than be shown.
//!
//! The server is the pretend one of the client's own tests, run as a program, so that
//! these tests need no Rex and run wherever `cargo xtask e2e` does. What the window's
//! page does is looked into where it can be, which is on Windows.

mod support;

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::drive::*;
use support::*;

/// The session the server has.
const SESSION: &str = "lume-fixture";

/// How long the server may take over what it is told. A busy machine takes its time.
const TOLD: Duration = Duration::from_secs(60);

/// How long the application may take to answer a script, whatever a server does.
const ANSWERED: Duration = Duration::from_secs(20);

/// What the page may ask the application for while a server has its say. Everything else
/// does something: opens a link, writes to the clipboard, changes a setting.
const SHOWING: &[&str] = &[
    "carrier_open",
    "carrier_send",
    "clipboard_read",
    "commands_list",
    "config_view",
    "folders_list",
    "frame_token",
    "layout_view",
    "page_marks",
    "page_reply",
    "profiles_list",
    "rex_hosts",
    "rex_hosts_look",
    "session_attach",
    "sessions_detached",
    "ui_contributions",
    "ui_visible",
    "window_state",
    "workspaces_list",
    // How what the application sends reaches a page.
    "plugin:__TAURI_CHANNEL__|fetch",
];

/// The pretend server, for one test.
struct Pretend {
    child: Child,
    words: ChildStdin,
    lines: mpsc::Receiver<Value>,
    address: String,
}

impl Pretend {
    fn start(arguments: &[&str]) -> Self {
        let name = format!("pretend_server{}", std::env::consts::EXE_SUFFIX);
        let program = application().with_file_name("examples").join(name);
        assert!(
            program.is_file(),
            "{} is not there. Run these tests with `cargo xtask e2e`.",
            program.display()
        );
        let mut child = Command::new(program)
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("the pretend server starts");
        let words = child.stdin.take().expect("its standard input");
        let said = child.stdout.take().expect("its standard output");
        let (say, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(said).lines().map_while(Result::ok) {
                let _ = say.send(serde_json::from_str(&line).unwrap_or(Value::String(line)));
            }
        });
        let mut server = Self {
            child,
            words,
            lines,
            address: String::new(),
        };
        server.address = server.next()["address"]
            .as_str()
            .expect("an address")
            .to_owned();
        server
    }

    fn next(&self) -> Value {
        self.lines
            .recv_timeout(TOLD)
            .expect("the pretend server answers")
    }

    /// Tells the server what to do, and gives what it says when it has done it.
    fn tell(&mut self, word: &str) -> Value {
        writeln!(self.words, "{word}").expect("the pretend server listens");
        let said = self.next();
        assert_eq!(said["done"], word, "{said}");
        said
    }

    fn settings(&self) -> String {
        format!(
            "\n[workspace]\non-switch = \"keep\"\n\
             \n[[rex.host]]\nlabel = '{HOST}'\naddress = 'http://{}'\n",
            self.address
        )
    }
}

impl Drop for Pretend {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts the application, with its page to look into where that can be done.
fn application_with(settings: &str) -> (App, Option<Pages>) {
    if cfg!(windows) {
        let (app, pages) = application_to_drive(settings);
        return (app, Some(pages));
    }
    let app = App::start_with(settings);
    app.status_with_session();
    (app, None)
}

/// Puts the server's session on show, and gives its panes.
fn open(app: &App) -> Vec<String> {
    command(
        app,
        "lume.rex.open",
        json!({ "host": HOST, "session": SESSION }),
    )
    .ok("open");
    until("the server's layout to be on show", || {
        let shown = app.layout();
        let there = shown["workspace"]["backend"].is_string();
        let panes = tabs_of(&shown);
        (there && panes.len() == 1 && panes[0].len() == 2).then(|| panes[0].clone())
    })
}

/// Waits until the two terminals of the server are shown as they are.
fn until_shown(app: &App, panes: &[String]) {
    for (nth, pane) in panes.iter().enumerate() {
        until(&format!("terminal {} to be shown", nth + 1), || {
            let captured = app.ctl(&["capture", "--pane", pane]);
            let text = captured.json["lines"].to_string();
            (captured.status == 0 && text.contains("of the pretend server")).then_some(())
        });
    }
}

/// What the page shows is all it should be: nothing failed, and nothing is asked.
fn window_is_whole(pages: &Pages, when: &str) {
    for (what, selector) in [
        ("something could not be drawn", ".part-failed"),
        ("the window could not be drawn", ".window-failed"),
        ("a question is asked", "dialog[open]"),
        ("what a server wrote was read as a page", "img[src='x']"),
    ] {
        assert_eq!(
            texts(pages, selector),
            Vec::<String>::new(),
            "{what}, {when}"
        );
    }
}

/// Opens what can be opened over the window, which lists what the server says, and puts
/// it away again.
fn overlays_open(app: &App, pages: &Pages, when: &str) {
    for (id, field) in [
        ("lume.palette.open", "palette-input"),
        ("lume.connect.open", "connect-input"),
    ] {
        app.ctl(&["command", id]).ok(id);
        until_page(
            pages,
            &format!("{field} to have the keyboard, {when}"),
            &format!("document.activeElement?.className === '{field}'"),
        );
        press(pages, &["text:e", "Escape"]);
        // One that has something typed into it is emptied first.
        let gone = format!("document.querySelector('.{field}') === null");
        until(&format!("{field} to be put away, {when}"), || {
            if page(pages, &gone) == true {
                return Some(());
            }
            press(pages, &["Escape"]);
            None
        });
    }
    selector(pages);
    let gone = "document.querySelector('.selector-input') === null";
    until(&format!("the selector to be put away, {when}"), || {
        press(pages, &["Escape"]);
        (page(pages, gone) == true).then_some(())
    });
}

/// A shell of the user's own, which is not on show while a server's session is.
struct Own {
    session: String,
    pid: u64,
    workspace: String,
    files: BTreeMap<PathBuf, Vec<u8>>,
    marks: u32,
}

fn files(dir: &Path, found: &mut BTreeMap<PathBuf, Vec<u8>>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, found);
        } else {
            found.insert(path.clone(), std::fs::read(&path).unwrap_or_default());
        }
    }
}

fn configuration_of(app: &App) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut found = BTreeMap::new();
    files(&app.home.join("config"), &mut found);
    found
}

impl Own {
    fn of(app: &App) -> Self {
        let status = app.status_with_session();
        let session = status["focused"]["session"].as_str().unwrap().to_owned();
        let pane = status["focused"]["pane"].as_str().unwrap().to_owned();
        Self {
            session,
            pid: u64::from(app.pid_of(&pane)),
            workspace: app.layout()["workspace"]["name"]
                .as_str()
                .unwrap()
                .to_owned(),
            files: configuration_of(app),
            marks: 0,
        }
    }

    /// The shell runs as it did and answers, as the application does; the configuration
    /// is the user's; nothing panicked.
    fn stands(&mut self, app: &App, when: &str) {
        let asked = Instant::now();
        let listed = app.ctl(&["list"]).ok(&format!("list, {when}"));
        assert!(asked.elapsed() < ANSWERED, "{when}: {:?}", asked.elapsed());
        let pids: Vec<u64> = listed["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|session| session["info"]["pid"].as_u64())
            .collect();
        // A server's programs are another machine's, and have no number here.
        assert_eq!(pids, [self.pid], "{when}: {listed}");

        self.marks += 1;
        let mark = format!("own{}", self.marks);
        let line = if cfg!(windows) {
            format!("echo {mark}-%OS%")
        } else {
            format!("echo {mark}-$((41+1))")
        };
        let of = ["--session", self.session.as_str()];
        app.ctl(&[&["send"], &of[..], &["--enter", "--", &line]].concat())
            .ok(&format!("send, {when}"));
        app.ctl(&[&["wait"], &of[..], &["--text", &printed(&mark)]].concat())
            .ok(&format!("the own shell, {when}"));
        assert!(asked.elapsed() < ANSWERED, "{when}: {:?}", asked.elapsed());

        assert_eq!(configuration_of(app), self.files, "{when}");
        let errors = app.errors();
        assert!(!errors.contains("panicked"), "{when}: {errors}");
    }
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_servers_terminals_are_shown_and_typed_into() {
    let mut server = Pretend::start(&[]);
    let (app, pages) = application_with(&server.settings());
    let mut own = Own::of(&app);
    let panes = open(&app);
    until_shown(&app, &panes);

    for (nth, pane) in panes.iter().enumerate() {
        let line = format!("typed into {nth}");
        app.type_line_in(pane, &line);
        app.wait_text_in(pane, &line);
    }
    let said = server.tell("count");
    let typed: Vec<&str> = said["typed"]
        .as_object()
        .unwrap()
        .values()
        .map(|typed| typed.as_str().unwrap())
        .collect();
    assert_eq!(typed.len(), 2, "{said}");
    for nth in 0..2 {
        let line = format!("typed into {nth}\r");
        assert!(typed.contains(&line.as_str()), "{said}");
    }
    // The terminals take the size of their panes, which only the page knows.
    until("the terminals to take their panes' sizes", || {
        let said = server.tell("count");
        let called = said["called"].as_array().unwrap();
        called
            .contains(&json!("com.superlogical.terminal.resize"))
            .then_some(())
    });
    if let Some(pages) = &pages {
        // The page's own terminal holds what the server's printed: it finds it.
        app.ctl(&["command", "lume.search.open", r#"{"query":"typed into"}"#])
            .ok("search");
        until_page(
            pages,
            "the page to find what was printed",
            "/[1-9]/.test(document.querySelector('.search-count')?.textContent ?? '')",
        );
        app.ctl(&["command", "lume.search.close"]).ok("close");
        window_is_whole(pages, "with a sound server");
    }

    command(&app, "lume.rex.disconnect", json!({ "host": HOST })).ok("disconnect");
    until("the own workspace to be on show", || {
        own_workspace_on_show(&app).then_some(())
    });
    own.stands(&app, "after the server was let go of");
    app.quit();
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn a_server_of_another_revision_is_turned_down_with_the_reason() {
    for (arguments, reasons) in [
        // Ones that say so themselves, and ones that would go on all the same.
        (
            ["--revision", "18"].as_slice(),
            ["the server speaks 18", "update the server"],
        ),
        (
            &["--revision", "20"],
            ["the server speaks 20", "update the client"],
        ),
        (
            &["--revision", "18", "--welcomes"],
            ["speaks 18 to 18", "update the Rex server"],
        ),
        (
            &["--revision", "20", "--welcomes"],
            ["speaks 20 to 20", "update this application"],
        ),
    ] {
        let mut server = Pretend::start(arguments);
        let (app, pages) = application_with(&server.settings());
        let mut own = Own::of(&app);
        let opened = command(&app, "lume.rex.open", json!({ "host": HOST }));
        assert_ne!(opened.status, 0, "{arguments:?}: {}", opened.json);
        let state = until("the server to be turned down", || {
            let state = host(&app)["state"].clone();
            (state["kind"] == "failed").then_some(state)
        });
        assert_eq!(state["code"], "version_mismatch", "{arguments:?}: {state}");
        let error = state["error"].as_str().unwrap();
        for reason in reasons {
            assert!(error.contains(reason), "{arguments:?}: {error}");
        }
        // It is not tried again: another try would not change it.
        let tried = server.tell("count")["connected"].clone();
        std::thread::sleep(Duration::from_secs(2));
        assert_eq!(server.tell("count")["connected"], tried);
        assert!(own_workspace_on_show(&app));

        if let Some(pages) = &pages {
            connect_palette(&app, pages, HOST);
            until_texts(pages, ".connect-row-line", &["Other version"]);
            let words = pages.until_text(WINDOW_PAGE, &reasons);
            assert!(words.contains(HOST), "{words}");
            press(pages, &["Escape"]);
            window_is_whole(pages, "with a server of another revision");
        }
        own.stands(&app, "with a server of another revision");
        app.quit();
    }
}

#[test]
#[ignore = "needs a display and the built application; run with `cargo xtask e2e`"]
fn whatever_a_server_does_the_window_and_what_is_the_users_own_stand() {
    let mut server = Pretend::start(&[]);
    let (app, pages) = application_with(&server.settings());
    let mut own = Own::of(&app);
    until_shown(&app, &open(&app));
    if let Some(pages) = &pages {
        watch(pages);
    }

    for word in [
        "names", "reports", "layouts", "streams", "copies", "flood", "answers", "silence", "close",
    ] {
        // What is said is said to somebody: both terminals are on show.
        let before = until("the terminals to be streamed", || {
            let stands = server.tell("count");
            (stands["streams"] == 2).then_some(stands)
        });
        let said = server.tell(word);
        let when = format!("after {word}");
        // Six damaged copies, and each was the beginning of a stream.
        if word == "copies" {
            assert!(
                said["streamed"].as_u64() >= before["streamed"].as_u64().map(|all| all + 6),
                "{said}"
            );
        }
        own.stands(&app, &when);
        if let Some(pages) = &pages {
            until_page(pages, &format!("the page to answer, {when}"), "true");
            window_is_whole(pages, &when);
            overlays_open(&app, pages, &when);
            window_is_whole(pages, &when);
        }
        // And it is sound again: its session is shown as if nothing had been.
        server.tell("calm");
        if word == "silence" {
            // What nobody answered is given up in its own time. Nobody waits for it.
            command(&app, "lume.rex.disconnect", json!({ "host": HOST })).ok("disconnect");
        }
        let panes = until(&format!("the session to be shown again, {when}"), || {
            let args = json!({ "host": HOST, "session": SESSION });
            let opened = command(&app, "lume.rex.open", args);
            let shown = app.layout();
            let panes = tabs_of(&shown);
            (opened.status == 0
                && shown["workspace"]["backend"].is_string()
                && shown["workspace"]["name"] == SESSION
                && panes.len() == 1
                && panes[0].len() == 2)
                .then(|| panes[0].clone())
        });
        until_shown(&app, &panes);
        let line = format!("typed {when}");
        until(&format!("typing to arrive, {when}"), || {
            let sent = app.ctl(&["send", "--pane", &panes[0], "--enter", "--", &line]);
            let said = server.tell("count");
            let arrived = said["typed"].to_string().contains(&line);
            (sent.status == 0 && arrived).then_some(())
        });
        if word == "close" {
            let again = server.tell("count");
            assert!(
                again["connected"].as_u64() > before["connected"].as_u64(),
                "{again}"
            );
        }
        if let Some(pages) = &pages {
            let asked = asked(pages);
            assert!(
                asked.contains(&"carrier_send".to_owned()),
                "the page is not watched, {when}: {asked:?}"
            );
            let more: Vec<String> = asked
                .into_iter()
                .filter(|one| !SHOWING.contains(&one.as_str()))
                .collect();
            assert_eq!(more, Vec::<String>::new(), "{when}");
        }
    }

    // What a program prints in a pane without the keyboard never reaches the clipboard.
    // Only where the page is watched: the clipboard is the user's.
    if let Some(pages) = &pages {
        let panes = tabs(&app)[0].clone();
        apply(
            &app,
            json!({ "type": "focus", "target": { "type": "pane", "pane": panes[0] } }),
        );
        server.tell("clipboard");
        std::thread::sleep(Duration::from_secs(1));
        let asked = asked(pages);
        assert!(!asked.contains(&"clipboard_write".to_owned()), "{asked:?}");
        // Seen, if it were done: in the pane that has the keyboard it is let through.
        apply(
            &app,
            json!({ "type": "focus", "target": { "type": "pane", "pane": panes[1] } }),
        );
        until("a pane with the keyboard to write to the clipboard", || {
            server.tell("clipboard");
            let found = page(pages, "window.__asked.includes('clipboard_write')");
            (found == true).then_some(())
        });
    }

    command(&app, "lume.rex.disconnect", json!({ "host": HOST })).ok("disconnect");
    until("the own workspace to be on show", || {
        let shown = app.layout();
        (shown["workspace"]["backend"].is_null() && shown["workspace"]["name"] == own.workspace)
            .then_some(())
    });
    own.stands(&app, "after the server was let go of");
    app.quit();
}
