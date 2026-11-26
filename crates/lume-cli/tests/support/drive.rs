//! What the tests of the application as a client of a Rex server share: asking it for
//! things by command, waiting for what comes of them, and driving the page of its window.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::*;

/// The label the server has in the application's configuration.
pub const HOST: &str = "Test server";

/// How long something may take to come about, on either side.
pub const SOON: Duration = Duration::from_secs(60);

/// The address of a window's page, as far as it tells it from others.
pub const WINDOW_PAGE: &str = "tauri.localhost";

/// A shell that keeps no history, as a program and its arguments.
pub fn plain_shell() -> &'static [&'static str] {
    if cfg!(windows) {
        &["cmd.exe", "/d"]
    } else {
        &["/bin/sh"]
    }
}

/// Waits until `found` finds something.
pub fn until<T>(what: &str, mut found: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + SOON;
    loop {
        if let Some(found) = found() {
            return found;
        }
        assert!(Instant::now() < deadline, "waited too long for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Runs a command of the application.
pub fn command(app: &App, id: &str, args: Value) -> Answer {
    app.ctl(&["command", id, &args.to_string()])
}

/// How the one server stands, as the application says.
pub fn host(app: &App) -> Value {
    let listed = app.ctl(&["rex"]).ok("rex");
    assert_eq!(
        listed["rex"]["hosts"].as_array().map(Vec::len),
        Some(1),
        "{listed}"
    );
    listed["rex"]["hosts"][0].clone()
}

/// The panes of the layout on show, tab by tab.
pub fn tabs(app: &App) -> Vec<Vec<String>> {
    tabs_of(&app.layout())
}

pub fn only_pane(app: &App) -> String {
    let tabs = tabs(app);
    assert_eq!(tabs.len(), 1, "{tabs:?}");
    assert_eq!(tabs[0].len(), 1, "{tabs:?}");
    tabs[0][0].clone()
}

/// Waits until the window shows as many panes in each tab as `wanted` says.
pub fn until_panes(app: &App, wanted: &[usize]) -> Vec<Vec<String>> {
    until(&format!("panes {wanted:?}"), || {
        let tabs = tabs(app);
        let counts: Vec<usize> = tabs.iter().map(Vec::len).collect();
        (counts == wanted).then_some(tabs)
    })
}

pub fn apply(app: &App, intent: Value) {
    app.ctl(&["layout", "apply", &intent.to_string()])
        .ok(&format!("{intent}"));
}

pub fn own_workspace_on_show(app: &App) -> bool {
    app.layout()["workspace"]["backend"].is_null()
}

/// What a shell prints for a line, told apart from the line as it was typed.
pub fn echo(app: &App, pane: &str, mark: &str) {
    let line = if cfg!(windows) {
        format!("set /a 41+1 >nul & echo {mark}-%OS%")
    } else {
        format!("echo {mark}-$((41+1))")
    };
    app.type_line_in(pane, &line);
    app.wait_text_in(pane, &printed(mark));
}

pub fn printed(mark: &str) -> String {
    if cfg!(windows) {
        format!("{mark}-Windows_NT")
    } else {
        format!("{mark}-42")
    }
}

/// Starts the application, with more of a configuration, so that its page can be looked
/// into and driven.
pub fn application_to_drive(more: &str) -> (App, Pages) {
    application_to_drive_with(more, &[])
}

/// The same, with these variables added to the application's environment.
pub fn application_to_drive_with(more: &str, environment: &[(&str, String)]) -> (App, Pages) {
    let (app, pages) = application_started("", more, environment, &[]);
    app.status_with_session();
    (app, pages)
}

/// The same, with these settings of Rex and these arguments added to the application's,
/// whatever it shows then.
pub fn application_started(
    rex: &str,
    more: &str,
    environment: &[(&str, String)],
    arguments: &[&str],
) -> (App, Pages) {
    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    let endpoint = format!("e2e-{}-{number}", std::process::id());
    let home = std::env::temp_dir().join(format!("lume-{endpoint}"));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("config")).unwrap();
    std::fs::write(
        home.join("config").join("config.toml"),
        // What the tests' applications are set to ends with the settings of Rex.
        configuration() + rex + IN_PROCESS_HOST + more,
    )
    .unwrap();
    application_in(home, endpoint, environment, arguments)
}

/// The same, in a home that is made already.
pub fn application_in(
    home: std::path::PathBuf,
    endpoint: String,
    environment: &[(&str, String)],
    arguments: &[&str],
) -> (App, Pages) {
    let pages = Pages { port: free_port() };
    let mut variables = vec![
        (
            "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
            format!("--remote-debugging-port={}", pages.port),
        ),
        (
            "WEBVIEW2_USER_DATA_FOLDER",
            home.join("webview").display().to_string(),
        ),
    ];
    variables.extend_from_slice(environment);
    let app = App::start_as(home.clone(), false, endpoint, &variables, arguments);
    (app, pages)
}

/// What `expression` gives in the window's page.
pub fn page(pages: &Pages, expression: &str) -> Value {
    pages
        .evaluate(WINDOW_PAGE, expression)
        .unwrap_or(Value::Null)
}

/// Writes down what a page asks the application for, by name, and answers for the
/// application where an answer would be the user's: their clipboard and their browser.
const WATCHED: &str = "(() => { \
    if (window.__asked) return true; \
    window.__asked = []; \
    window.__copied = null; \
    const reach = window.fetch; \
    window.fetch = (what, how) => { \
      const at = new URL(typeof what === 'string' ? what : what.url, location.href); \
      if (at.hostname !== 'ipc.localhost') return reach(what, how); \
      const asked = decodeURIComponent(at.pathname.slice(1)); \
      window.__asked.push(asked); \
      if (!['clipboard_write', 'open_link'].includes(asked)) return reach(what, how); \
      if (asked === 'clipboard_write') window.__copied = JSON.parse(how.body).text; \
      return Promise.resolve(new Response('null', { headers: { \
        'Tauri-Response': 'ok', 'content-type': 'application/json' } })); \
    }; \
    return true; })()";

/// Begins to write down what the window's page asks the application for. From here on
/// the clipboard and the browser of whoever runs the test are not reached.
pub fn watch(pages: &Pages) {
    assert_eq!(page(pages, WATCHED), json!(true));
}

/// What a watched page asked for since it was last looked at, each once.
pub fn asked(pages: &Pages) -> Vec<String> {
    let found = page(
        pages,
        "(() => { const all = [...new Set(window.__asked)].sort(); \
         window.__asked = []; return all; })()",
    );
    serde_json::from_value(found).expect("the page is watched")
}

/// What a watched page would have put on the clipboard last, taken from it.
pub fn copied(pages: &Pages) -> Option<String> {
    let found = page(
        pages,
        "(() => { const last = window.__copied; window.__copied = null; return last; })()",
    );
    found.as_str().map(str::to_owned)
}

/// Waits until `expression` holds in the window's page.
pub fn until_page(pages: &Pages, what: &str, expression: &str) {
    until(what, || (page(pages, expression) == true).then_some(()));
}

/// The text of everything in the window's page that `selector` finds.
pub fn texts(pages: &Pages, selector: &str) -> Vec<String> {
    let found = page(
        pages,
        &format!(
            "[...document.querySelectorAll({})].map((one) => one.textContent)",
            json!(selector)
        ),
    );
    serde_json::from_value(found).unwrap_or_default()
}

pub fn until_texts(pages: &Pages, selector: &str, wanted: &[&str]) {
    let deadline = Instant::now() + SOON;
    loop {
        let found = texts(pages, selector);
        if found == wanted {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{selector} says {found:?}, not {wanted:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn press(pages: &Pages, keys: &[&str]) {
    pages.press(WINDOW_PAGE, keys);
}

/// Opens the Connect palette on the Rex servers, with the keyboard in it, and marks the
/// one called `label`.
pub fn connect_palette(app: &App, pages: &Pages, label: &str) {
    app.ctl(&["command", "lume.connect.open"]).ok("palette");
    until_page(
        pages,
        "the palette to have the keyboard",
        "document.activeElement?.className === 'connect-input'",
    );
    press(pages, &["Alt+ArrowRight"]);
    until_page(
        pages,
        "the Rex servers to be listed",
        "document.querySelector('.connect-chip[data-filter=rex]')?.ariaPressed === 'true'",
    );
    let marked = format!(
        "document.querySelector('.connect-row[aria-selected=true] .connect-row-name')\
         ?.textContent === {}",
        json!(label)
    );
    until("the server to be marked", || {
        if page(pages, &marked) == true {
            return Some(());
        }
        press(pages, &["ArrowDown"]);
        None
    });
}

/// Opens the selector under the workspace's name, with the keyboard in it.
pub fn selector(pages: &Pages) {
    page(
        pages,
        "(() => { document.querySelector('.switcher-button').click(); return true; })()",
    );
    until_page(
        pages,
        "the selector to have the keyboard",
        "document.activeElement?.className === 'selector-input'",
    );
}

pub fn until_on_show(app: &App, name: &str) {
    until(&format!("{name} to be on show"), || {
        let layout = app.layout();
        (layout["workspace"]["name"] == name && layout["workspace"]["backend"].is_string())
            .then_some(())
    });
}
