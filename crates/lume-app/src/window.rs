//! The application's window.
//!
//! The window is described in the configuration file but opened here, because what it
//! shows depends on how the application was started, and because the handler that keeps
//! its title can only be given to a window that does not exist yet.

use tauri::{App, AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::args::Mode;
use crate::control::Shell;
use crate::tuning::TITLE_MAX_BYTES;

/// The window's label, which the capability file grants the commands to.
pub const LABEL: &str = "main";

/// The window's title when the page has none worth showing.
const DEFAULT_TITLE: &str = "Lume";

/// The page a harness run shows in place of the terminal.
const HARNESS_PAGE: &str = "harness.html";

/// Opens the window the configuration describes.
pub fn open(app: &App, mode: Mode) -> Result<(), Box<dyn std::error::Error>> {
    let mut described = app
        .config()
        .app
        .windows
        .first()
        .cloned()
        .ok_or("the configuration describes no window")?;
    if matches!(mode, Mode::Harness(_)) {
        described.url = WebviewUrl::App(HARNESS_PAGE.into());
    }
    WebviewWindowBuilder::from_config(app.handle(), &described)?
        // The page names its document after the terminal's title, and the window follows
        // the document. A program in the terminal chooses that title, so it is checked
        // here as well as in the page.
        .on_document_title_changed(|window, title| {
            if let Err(error) = window.set_title(&window_title(&title)) {
                tracing::debug!(%error, "the window's title could not be set");
            }
        })
        .build()?;
    Ok(())
}

/// The window as the control endpoint reaches it.
pub struct WindowShell {
    app: AppHandle,
}

impl WindowShell {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl Shell for WindowShell {
    fn activate(&self) {
        let Some(window) = self.app.get_webview_window(LABEL) else {
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
