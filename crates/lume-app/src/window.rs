//! The application's windows.
//!
//! A window is described in the configuration file but opened here, because what it
//! shows depends on how the application was started, and because the handler that keeps
//! its title can only be given to a window that does not exist yet.
//!
//! A window that shows terminals is one window of the layout, and is labelled after it:
//! the layout's window `w3` is the webview window `w-3`. The commands a terminal's page
//! may call are granted to every label of that form. The window that only reports on
//! the webview, or runs a test suite, has a label of its own and is granted less.
//!
//! The layout says which windows there are, and the windows here follow it: one is
//! opened when the layout gains a window and closed when it loses one. In the other
//! direction a window tells the layout where it is on the desktop, a moment after it
//! was last moved, so that it can be opened in the same place another day.
//!
//! A window is drawn with the system's title bar or without one. Without, the page
//! draws the bar itself and asks for what a title bar does: moving the window, and
//! the three controls at its end.
//!
//! The settings have a window of their own. It is no part of the layout: it shows no
//! terminal, it is not kept with a workspace, and there is at most one. It is opened
//! when asked for, brought forward when asked for again, and goes when the last window
//! that shows terminals does.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use lume_layout::{Bounds, Intent, WindowId};
use serde::Serialize;
use serde_json::json;
use tauri::{
    App, AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

use crate::args::Mode;
use crate::bounds::{Reading, Screen, clamp_bounds, merge_bounds};
use crate::configsvc::ConfigService;
use crate::control::Shell;
use crate::state::AppState;
use crate::tuning::{
    BOUNDS_DEBOUNCE, SETTINGS_SECTION_MAX_BYTES, SETTINGS_WINDOW_MIN_SIZE, SETTINGS_WINDOW_SIZE,
    TITLE_MAX_BYTES,
};

/// The label of the window that shows no terminals: the webview report, or a test
/// suite.
pub const LABEL: &str = "main";

/// The label of the window that shows the settings.
pub const SETTINGS_LABEL: &str = "settings";

/// The page the settings window shows.
const SETTINGS_PAGE: &str = "settings.html";

/// The settings window's title.
const SETTINGS_TITLE: &str = "Settings";

/// What the label of a window of the layout begins with.
const LAYOUT_LABEL_PREFIX: &str = "w-";

/// The label of the webview window that shows a window of the layout.
pub fn label_of(window: WindowId) -> String {
    format!("{LAYOUT_LABEL_PREFIX}{}", window.get())
}

/// The window of the layout a webview window shows, if its label is one of a layout's
/// window.
pub fn window_of(label: &str) -> Option<WindowId> {
    let number: u32 = label.strip_prefix(LAYOUT_LABEL_PREFIX)?.parse().ok()?;
    let window = WindowId::new(number);
    // One spelling for each window: no sign, no leading zeros.
    (label_of(window) == label).then_some(window)
}

/// Whether a window with this label has a page that draws its own title bar when the
/// system does not: a window of the layout, or the settings window.
pub fn has_own_bar(label: &str) -> bool {
    label == SETTINGS_LABEL || window_of(label).is_some()
}

/// The section of the settings a request names, if it could be one: a short word of
/// lower-case letters, digits and hyphens. It becomes part of the page's address, so
/// nothing else is let through.
pub fn settings_section(asked: Option<&str>) -> Option<&str> {
    asked.filter(|section| {
        !section.is_empty()
            && section.len() <= SETTINGS_SECTION_MAX_BYTES
            && section
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    })
}

/// The address of the settings page, opened at a section if one is named.
fn settings_address(section: Option<&str>) -> String {
    match settings_section(section) {
        Some(section) => format!("{SETTINGS_PAGE}#{section}"),
        None => SETTINGS_PAGE.to_owned(),
    }
}

/// Where a window of `size` goes to be in the middle of `area`: its top left corner.
/// A window larger than the area starts at the area's own corner, so its title bar can
/// be reached.
fn centred(area: (i32, i32, u32, u32), size: (u32, u32)) -> (i32, i32) {
    let (left, top, width, height) = area;
    let spare = |room: u32, taken: u32| i32::try_from(room.saturating_sub(taken) / 2).unwrap_or(0);
    (left + spare(width, size.0), top + spare(height, size.1))
}

/// Whether closing a window waits for its page to ask the user first.
///
/// It does when the configuration says closing a window always asks, something still
/// runs in the window, there is a page to do the asking, and the user has not said yes
/// already. A window whose page is gone closes: nothing could ask, and a window that
/// cannot be closed is worse than a question that was not put.
pub fn close_asks(always: bool, running: bool, page: bool, confirmed: bool) -> bool {
    always && running && page && !confirmed
}

/// The window's title when the page has none worth showing.
const DEFAULT_TITLE: &str = "Lume";

/// The page a harness run shows in place of the terminal.
const HARNESS_PAGE: &str = "harness.html";

/// The window the configuration file describes, which every window starts from.
fn described(app: &AppHandle) -> Result<tauri::utils::config::WindowConfig, String> {
    app.config()
        .app
        .windows
        .first()
        .cloned()
        .ok_or_else(|| "the configuration describes no window".to_owned())
}

/// Keeps a window's title the title of its page. The page names its document after
/// the terminal's title, and a program in the terminal chooses that title, so it is
/// checked here as well as in the page.
fn follow_title(window: WebviewWindow, title: String) {
    if let Err(error) = window.set_title(&window_title(&title)) {
        tracing::debug!(%error, "the window's title could not be set");
    }
}

/// Opens the window that shows no terminals: the one that reports on the webview, or
/// runs a test suite.
pub fn open(app: &App, mode: Mode) -> Result<WebviewWindow, Box<dyn std::error::Error>> {
    let mut described = described(app.handle())?;
    if matches!(mode, Mode::Harness(_)) {
        described.url = WebviewUrl::App(HARNESS_PAGE.into());
    }
    let window = WebviewWindowBuilder::from_config(app.handle(), &described)?
        .on_document_title_changed(follow_title)
        .build()?;
    Ok(window)
}

/// What a window is doing, as its page is told: the page draws the window's controls
/// when the system does not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct WindowState {
    pub maximised: bool,
    pub fullscreen: bool,
    pub focused: bool,
}

/// What one of a window's controls does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Minimise,
    /// Fills the screen, or goes back to the size it had.
    Maximise,
    Close,
}

impl Control {
    /// The control a page names.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "minimise" => Some(Control::Minimise),
            "maximise" => Some(Control::Maximise),
            "close" => Some(Control::Close),
            _ => None,
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a window is doing now.
pub fn state_of(window: &WebviewWindow) -> WindowState {
    WindowState {
        maximised: window.is_maximized().unwrap_or(false),
        fullscreen: window.is_fullscreen().unwrap_or(false),
        focused: window.is_focused().unwrap_or(false),
    }
}

/// Does what one of a window's controls does.
pub fn control(window: &WebviewWindow, control: Control) -> tauri::Result<()> {
    match control {
        Control::Minimise => window.minimize(),
        Control::Maximise if window.is_maximized()? => window.unmaximize(),
        Control::Maximise => window.maximize(),
        // Asked, not forced: the application decides what closing this window means.
        Control::Close => window.close(),
    }
}

/// The application's windows: opening and closing the ones the layout names, and
/// telling the layout and the pages what the windows do.
pub struct WindowShell {
    app: AppHandle,
    state: Arc<AppState>,
    config: Arc<ConfigService>,
    /// Counts the times each window was moved or resized. A window's place is kept
    /// once it has stopped changing: when the count has not gone up for a while.
    moved: Mutex<HashMap<String, u64>>,
    /// What each window's page was last told the window is doing.
    told: Mutex<HashMap<String, WindowState>>,
    /// The windows whose closing the user has agreed to, each until it next closes.
    confirmed: Mutex<HashSet<String>>,
}

impl WindowShell {
    pub fn new(app: AppHandle, state: Arc<AppState>, config: Arc<ConfigService>) -> Arc<Self> {
        Arc::new(Self {
            app,
            state,
            config,
            moved: Mutex::new(HashMap::new()),
            told: Mutex::new(HashMap::new()),
            confirmed: Mutex::new(HashSet::new()),
        })
    }

    /// The window the user is working in: the one that has the keyboard, else the one
    /// the layout says last had it, else any.
    fn front(&self) -> Option<WebviewWindow> {
        // The settings window is not one the user works in.
        let windows: HashMap<String, WebviewWindow> = self
            .app
            .webview_windows()
            .into_iter()
            .filter(|(label, _)| window_of(label).is_some())
            .collect();
        let focused = windows
            .values()
            .find(|window| window.is_focused().unwrap_or(false));
        let last = || {
            let window = self.state.panes()?.focused_window()?;
            windows.get(&label_of(window))
        };
        focused
            .or_else(last)
            .or_else(|| windows.values().next())
            .cloned()
    }

    /// The screens there are, the main one first.
    fn screens(&self) -> Vec<Screen> {
        let main = self.app.primary_monitor().ok().flatten();
        let mut monitors = self.app.available_monitors().unwrap_or_default();
        if let Some(main) = &main {
            monitors.sort_by_key(|monitor| monitor.position() != main.position());
        }
        monitors
            .iter()
            .map(|monitor| {
                let area = monitor.work_area();
                Screen {
                    x: area.position.x,
                    y: area.position.y,
                    width: area.size.width,
                    height: area.size.height,
                    scale: monitor.scale_factor(),
                }
            })
            .collect()
    }

    /// Gives a window that is being made the title bar the configuration asks for.
    fn with_title_bar<'a>(
        &self,
        builder: WebviewWindowBuilder<'a, tauri::Wry, AppHandle>,
    ) -> WebviewWindowBuilder<'a, tauri::Wry, AppHandle> {
        let custom = self.config.title_bar_custom();
        // A Mac keeps its three buttons and lays them over the page's own bar; elsewhere
        // a window without the system's bar has no frame at all.
        #[cfg(target_os = "macos")]
        let builder = if custom {
            builder
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .hidden_title(true)
        } else {
            builder
        };
        #[cfg(not(target_os = "macos"))]
        let builder = builder.decorations(!custom);
        builder
    }

    /// Opens the settings window at a section, in the middle of the screen the window
    /// that asked is on.
    fn build_settings(
        &self,
        section: Option<&str>,
        from: Option<WindowId>,
    ) -> Result<WebviewWindow, Box<dyn std::error::Error>> {
        let mut described = described(&self.app)?;
        described.label = SETTINGS_LABEL.to_owned();
        described.title = SETTINGS_TITLE.to_owned();
        described.url = WebviewUrl::App(settings_address(section).into());
        described.width = f64::from(SETTINGS_WINDOW_SIZE.0);
        described.height = f64::from(SETTINGS_WINDOW_SIZE.1);
        described.min_width = Some(f64::from(SETTINGS_WINDOW_MIN_SIZE.0));
        described.min_height = Some(f64::from(SETTINGS_WINDOW_MIN_SIZE.1));
        // Shown once it is where it belongs, so it is not seen to jump there.
        let builder = WebviewWindowBuilder::from_config(&self.app, &described)?.visible(false);
        let opened = self.with_title_bar(builder).build()?;

        let beside = from.and_then(|window| self.app.get_webview_window(&label_of(window)));
        let screen = beside
            .and_then(|window| window.current_monitor().ok().flatten())
            .or_else(|| opened.current_monitor().ok().flatten());
        if let Some(screen) = screen {
            let area = screen.work_area();
            let size = opened.outer_size()?;
            let (x, y) = centred(
                (
                    area.position.x,
                    area.position.y,
                    area.size.width,
                    area.size.height,
                ),
                (size.width, size.height),
            );
            opened.set_position(PhysicalPosition::new(x, y))?;
        }
        opened.show()?;
        opened.set_focus()?;
        Ok(opened)
    }

    /// Closes the settings window if no window that shows terminals is left but the
    /// one called `closing`, which is on its way out. Without this the application
    /// would go on with nothing but its settings to show.
    pub fn close_settings_with(&self, closing: &str) {
        if window_of(closing).is_none() {
            return;
        }
        let others = self
            .layout_windows()
            .into_iter()
            .any(|label| label != closing);
        if !others {
            self.close_window(SETTINGS_LABEL);
        }
    }

    /// Notes that the user agreed to the closing of a window.
    pub fn confirm_close(&self, label: &str) {
        lock(&self.confirmed).insert(label.to_owned());
    }

    /// Whether the user agreed to the closing of a window. Asking uses the agreement
    /// up: it holds for the one closing it was given for.
    pub fn take_confirmed(&self, label: &str) -> bool {
        lock(&self.confirmed).remove(label)
    }

    fn build(
        &self,
        window: WindowId,
        bounds: Option<Bounds>,
    ) -> Result<WebviewWindow, Box<dyn std::error::Error>> {
        let mut described = described(&self.app)?;
        described.label = label_of(window);
        // Shown once it is where it belongs, so it is not seen to jump there.
        let builder = WebviewWindowBuilder::from_config(&self.app, &described)?
            .on_document_title_changed(follow_title)
            .visible(false);
        let opened = self.with_title_bar(builder).build()?;

        let restore = self.config.current().config.window.restore_bounds;
        let placement = bounds
            .filter(|_| restore)
            .and_then(|bounds| clamp_bounds(&bounds, &self.screens()));
        if let Some(placement) = placement {
            opened.set_size(PhysicalSize::new(placement.width, placement.height))?;
            opened.set_position(PhysicalPosition::new(placement.x, placement.y))?;
            if placement.maximised {
                opened.maximize()?;
            }
            if placement.fullscreen {
                opened.set_fullscreen(true)?;
            }
        }
        opened.show()?;
        opened.set_focus()?;
        Ok(opened)
    }

    /// What a window says of where it is.
    fn read(window: &WebviewWindow) -> tauri::Result<Reading> {
        let position = window.outer_position()?;
        let size = window.inner_size()?;
        Ok(Reading {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
            scale: window.scale_factor()?,
            maximised: window.is_maximized()?,
            fullscreen: window.is_fullscreen()?,
            minimised: window.is_minimized()?,
        })
    }

    /// The place to keep for a window, if it differs from the one that is kept.
    fn place_to_keep(&self, label: &str) -> Option<(WindowId, Bounds)> {
        let window = window_of(label)?;
        let webview = self.app.get_webview_window(label)?;
        let panes = self.state.panes()?;
        let kept = panes.workspace().window(window)?.bounds;
        let reading = match Self::read(&webview) {
            Ok(reading) => reading,
            Err(error) => {
                tracing::debug!(%error, label, "a window did not say where it is");
                return None;
            }
        };
        let bounds = merge_bounds(kept, &reading)?;
        (Some(bounds) != kept).then_some((window, bounds))
    }

    /// Notes that a window was moved or resized. Where it is now is kept with the
    /// layout once it has been left alone for a moment.
    pub fn moved(self: &Arc<Self>, label: &str) {
        if window_of(label).is_none() {
            return;
        }
        let turn = {
            let mut moved = lock(&self.moved);
            let turn = moved.entry(label.to_owned()).or_insert(0);
            *turn += 1;
            *turn
        };
        let shell = Arc::clone(self);
        let label = label.to_owned();
        self.state.runtime().spawn(async move {
            tokio::time::sleep(BOUNDS_DEBOUNCE).await;
            if lock(&shell.moved).get(&label).copied() != Some(turn) {
                return;
            }
            let Some((window, bounds)) = shell.place_to_keep(&label) else {
                return;
            };
            if let Some(panes) = shell.state.panes() {
                let _ = panes
                    .apply_core(Intent::WindowBounds { window, bounds })
                    .await;
            }
        });
    }

    /// Keeps where a window is now, at once: it is about to close. Call from the
    /// thread the windows live on.
    pub fn keep_place(&self, label: &str) {
        let Some((window, bounds)) = self.place_to_keep(label) else {
            return;
        };
        if let Some(panes) = self.state.panes() {
            // Recording a place starts nothing and waits for nothing.
            let _ = tauri::async_runtime::block_on(
                panes.apply_core(Intent::WindowBounds { window, bounds }),
            );
        }
    }

    /// Tells a window's page what the window is doing, if that has changed since the
    /// page was last told.
    pub fn tell_state(&self, label: &str) {
        if !has_own_bar(label) {
            return;
        }
        let Some(window) = self.app.get_webview_window(label) else {
            return;
        };
        let now = state_of(&window);
        if lock(&self.told).insert(label.to_owned(), now) == Some(now) {
            return;
        }
        if let Some(page) = self.state.page_of(label) {
            page.notify(&json!({
                "type": "window_state",
                "maximised": now.maximised,
                "fullscreen": now.fullscreen,
                "focused": now.focused,
            }));
        }
    }

    /// Forgets what was kept about a window that is gone.
    pub fn forget(&self, label: &str) {
        lock(&self.moved).remove(label);
        lock(&self.told).remove(label);
        lock(&self.confirmed).remove(label);
    }
}

impl Shell for WindowShell {
    fn activate(&self) {
        let Some(window) = self.front() else {
            return;
        };
        // A window that is minimised or hidden is not brought forward by being focused.
        let shown = window
            .unminimize()
            .and_then(|()| window.show())
            .and_then(|()| window.set_focus());
        if let Err(error) = shown {
            tracing::debug!(%error, "the window could not be brought to the front");
        }
    }

    fn quit(&self) {
        // The same way out as closing the window: the sessions are ended on the way.
        self.app.exit(0);
    }

    fn bundled_ui(&self) -> bool {
        !tauri::is_dev()
    }

    fn open_window(&self, window: WindowId, bounds: Option<Bounds>) {
        if self.app.get_webview_window(&label_of(window)).is_some() {
            return;
        }
        if let Err(error) = self.build(window, bounds) {
            tracing::error!(%window, %error, "a window could not be opened");
        }
    }

    fn close_window(&self, label: &str) {
        let Some(window) = self.app.get_webview_window(label) else {
            return;
        };
        if let Err(error) = window.destroy() {
            tracing::debug!(%error, label, "the window could not be closed");
        }
    }

    fn layout_windows(&self) -> Vec<String> {
        self.app
            .webview_windows()
            .into_keys()
            .filter(|label| window_of(label).is_some())
            .collect()
    }

    fn open_settings(&self, section: Option<&str>, from: Option<WindowId>) {
        let Some(open) = self.app.get_webview_window(SETTINGS_LABEL) else {
            if let Err(error) = self.build_settings(section, from) {
                tracing::error!(%error, "the settings window could not be opened");
            }
            return;
        };
        // A window that is minimised or hidden is not brought forward by being focused.
        let shown = open
            .unminimize()
            .and_then(|()| open.show())
            .and_then(|()| open.set_focus());
        if let Err(error) = shown {
            tracing::debug!(%error, "the settings window could not be brought to the front");
        }
        if let Some(page) = self.state.page_of(SETTINGS_LABEL) {
            page.notify(&json!({
                "type": "settings_show",
                "section": settings_section(section),
            }));
        }
    }

    fn ask_close(&self, window: WindowId) {
        let Some(open) = self.app.get_webview_window(&label_of(window)) else {
            return;
        };
        // Asked as the window's own control asks, so what closing means, and whether
        // the user is asked first, is decided in one place.
        if let Err(error) = open.close() {
            tracing::debug!(%error, %window, "the window could not be asked to close");
        }
    }

    fn set_title_bar(&self, custom: bool) {
        for (label, window) in self.app.webview_windows() {
            if !has_own_bar(&label) {
                continue;
            }
            #[cfg(target_os = "macos")]
            let set = window.set_title_bar_style(if custom {
                tauri::TitleBarStyle::Overlay
            } else {
                tauri::TitleBarStyle::Visible
            });
            #[cfg(not(target_os = "macos"))]
            let set = window.set_decorations(!custom);
            if let Err(error) = set {
                tracing::debug!(%error, label, "the window's title bar could not be changed");
            }
        }
    }
}

/// What the window may be called, given what a page asked for: no control characters,
/// nothing that changes the direction text is read in, and no more than a line of it.
fn window_title(asked: &str) -> String {
    let mut title = String::new();
    for character in asked.chars() {
        if character.is_control() || is_direction_mark(character) {
            continue;
        }
        if title.len() + character.len_utf8() > TITLE_MAX_BYTES {
            break;
        }
        title.push(character);
    }
    if title.trim().is_empty() {
        DEFAULT_TITLE.to_owned()
    } else {
        title
    }
}

/// Whether a character overrides or isolates the direction of the text around it. Such
/// characters can make a title read as something other than what it is.
fn is_direction_mark(character: char) -> bool {
    matches!(character, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_of_the_layout_has_one_label_and_no_other_label_names_it() {
        let window = WindowId::new(12);
        assert_eq!(label_of(window), "w-12");
        assert_eq!(window_of("w-12"), Some(window));
        for label in ["main", "w-", "w-012", "w-+12", "w12", "w-12 ", "x-12"] {
            assert_eq!(window_of(label), None, "{label}");
        }
    }

    #[test]
    fn the_settings_window_is_no_window_of_the_layout_and_draws_its_own_bar() {
        assert_eq!(window_of(SETTINGS_LABEL), None);
        assert!(has_own_bar(SETTINGS_LABEL));
        assert!(has_own_bar("w-3"));
        assert!(!has_own_bar(LABEL));
    }

    #[test]
    fn a_section_is_a_short_plain_word_and_anything_else_is_none() {
        assert_eq!(settings_section(Some("appearance")), Some("appearance"));
        assert_eq!(settings_section(Some("add-on-2")), Some("add-on-2"));
        for asked in ["", "Appearance", "a b", "a#b", "a/b", "a?b=c", "..", "é"] {
            assert_eq!(settings_section(Some(asked)), None, "{asked}");
        }
        let long = "a".repeat(SETTINGS_SECTION_MAX_BYTES + 1);
        assert_eq!(settings_section(Some(&long)), None);
        assert_eq!(settings_section(None), None);
    }

    #[test]
    fn the_settings_page_opens_at_the_section_named() {
        assert_eq!(settings_address(Some("terminal")), "settings.html#terminal");
        assert_eq!(settings_address(None), "settings.html");
        assert_eq!(settings_address(Some("x#y")), "settings.html");
    }

    #[test]
    fn a_window_is_centred_on_its_screen_and_kept_inside_a_small_one() {
        assert_eq!(centred((0, 0, 1920, 1040), (980, 680)), (470, 180));
        // A second screen to the left of the main one, and above it.
        assert_eq!(centred((-1280, -200, 1280, 984), (980, 680)), (-1130, -48));
        assert_eq!(centred((100, 50, 800, 600), (980, 680)), (100, 50));
    }

    #[test]
    fn closing_asks_only_when_told_to_with_something_running_and_a_page_to_ask() {
        assert!(close_asks(true, true, true, false));
        // The setting is off, nothing runs, no page could ask, or the user said yes.
        assert!(!close_asks(false, true, true, false));
        assert!(!close_asks(true, false, true, false));
        assert!(!close_asks(true, true, false, false));
        assert!(!close_asks(true, true, true, true));
    }

    #[test]
    fn an_ordinary_title_is_kept() {
        assert_eq!(window_title("vim: notes.txt"), "vim: notes.txt");
        assert_eq!(window_title("日本語 ~ zsh"), "日本語 ~ zsh");
    }

    #[test]
    fn an_empty_title_becomes_the_applications_name() {
        assert_eq!(window_title(""), "Lume");
        assert_eq!(window_title("  \u{7}\n"), "Lume");
    }

    #[test]
    fn control_characters_and_direction_marks_are_dropped() {
        assert_eq!(window_title("a\u{1b}[31mb\r\nc"), "a[31mbc");
        assert_eq!(window_title("safe\u{202e}txt.exe"), "safetxt.exe");
    }

    #[test]
    fn a_long_title_is_cut_between_characters() {
        let title = window_title(&"é".repeat(TITLE_MAX_BYTES));
        assert_eq!(title.len(), TITLE_MAX_BYTES);
        assert!(title.chars().all(|character| character == 'é'));
    }
}
