//! A window's page that stops answering, and how the window comes back from it.
//!
//! WebKit runs a page and the frames in it on one thread: a plugin's view that never
//! lets go of it, in a frame of its own, stops the window's page with it, and nothing in
//! the page can end it. WebView2 runs a frame apart, but a page of its own can stop all
//! the same. So each window's page is asked now and then whether it is there. One that
//! says nothing for a while, in a window that is on show, has stopped; one that has not
//! answered yet is given longer, as it may still be starting. The plugins' views that
//! were open in it are closed, as the page would have closed them, and those still in
//! the window are held back: which of them stopped the page cannot be told, and one that
//! opened again by itself would stop the next page too. The page's process is then ended
//! and the page loaded afresh, and it finds its terminals as after any reload. The user
//! is told, and opens a view that was held back again from where it is shown.

use std::sync::Arc;

use lume_layout::Window;
use lume_proto::app::{PageRequestBody, codes};
use tauri::{AppHandle, Manager};

use crate::services::Services;
use crate::state::AppState;
use crate::tuning::{PAGE_PING_EVERY, PAGE_STOPPED_AFTER};
use crate::window::{WindowShell, window_of};

/// What was heard from a page when it was asked whether it is there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heard {
    /// It answered, whatever it said.
    Answered,
    /// It said nothing in time, in a window that is on show.
    Silent,
    /// It said nothing in time, but its window was not on show meanwhile: a webview
    /// may hold the page of a window that is not seen back.
    Unseen,
}

/// What to do about a page after hearing from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    Watch,
    Restart,
}

/// How many times in a row a page that has not answered yet may say nothing before it is
/// taken to have stopped: it may still be starting. A view in a panel the page opens as
/// it starts can stop it before it ever answers.
const STARTING_SILENCES: u32 = 3;

/// What has been heard of one page.
#[derive(Debug, Default)]
pub struct Watch {
    answered: bool,
    /// How many times it said nothing, in a window on show, before it ever answered.
    starting_silences: u32,
}

impl Watch {
    pub fn heard(&mut self, heard: Heard) -> Next {
        match heard {
            Heard::Answered => {
                self.answered = true;
                Next::Watch
            }
            Heard::Silent if self.answered => Next::Restart,
            Heard::Silent => {
                self.starting_silences += 1;
                if self.starting_silences >= STARTING_SILENCES {
                    Next::Restart
                } else {
                    Next::Watch
                }
            }
            Heard::Unseen => Next::Watch,
        }
    }
}

/// Watches the page of the window called `label`, which has just connected on carrier
/// `carrier`, for as long as that carrier is the window's.
pub fn watch(app: &AppHandle, label: &str, carrier: u32) {
    // Only the windows of the layout show views that can stop a page, and answer.
    if window_of(label).is_none() {
        return;
    }
    let Some(state) = app
        .try_state::<Arc<AppState>>()
        .map(|state| Arc::clone(&state))
    else {
        return;
    };
    let app = app.clone();
    let label = label.to_owned();
    let runtime = state.runtime().clone();
    runtime.spawn(async move {
        let mut watch = Watch::default();
        loop {
            tokio::time::sleep(PAGE_PING_EVERY).await;
            let current = || state.page_of(&label).filter(|link| link.id() == carrier);
            // Another page came, or none is there.
            let Some(link) = current() else {
                return;
            };
            let expected = || {
                app.try_state::<Arc<WindowShell>>()
                    .is_some_and(|windows| windows.page_expected_to_answer(&label))
            };
            if !expected() {
                continue;
            }
            let heard = match link.ask(PageRequestBody::Ping {}, PAGE_STOPPED_AFTER).await {
                Err(error) if error.code == codes::TIMEOUT => {
                    if expected() {
                        Heard::Silent
                    } else {
                        Heard::Unseen
                    }
                }
                // The page went while it was asked.
                Err(error) if error.code == codes::NO_WINDOW => return,
                // Any answer, even a refusal, is the page's code running.
                Ok(_) | Err(_) => Heard::Answered,
            };
            if watch.heard(heard) == Next::Restart {
                if current().is_some() {
                    restart(&app, &state, &label);
                }
                return;
            }
        }
    });
}

/// Brings back a window whose page has stopped answering.
fn restart(app: &AppHandle, state: &Arc<AppState>, label: &str) {
    tracing::warn!(
        label,
        "a window's page stopped answering; it is loaded again"
    );
    let held = match (app.try_state::<Arc<Services>>(), window_of(label)) {
        (Some(services), Some(window)) => {
            let workspace = services.panes.workspace();
            let still_shown = |host: &str| {
                workspace
                    .window(window)
                    .is_some_and(|window| shows(window, host))
            };
            services.plugins.views_of_page_stopped(label, still_shown)
        }
        _ => 0,
    };
    tracing::debug!(
        label,
        held,
        "the views of a page that stopped answering were closed"
    );
    // Its sessions wait for the page that comes next, as they do across any reload.
    state.retire(label);
    if let Some(windows) = app.try_state::<Arc<WindowShell>>() {
        windows.restart_page(label);
    }
    if let Some(panes) = state.panes() {
        // With no other window to show it, it waits for the page that comes next.
        panes.notice("warning", stopped_notice(held));
    }
    tracing::debug!(label, "a page that stopped answering is being loaded again");
}

/// What the user is told of a window whose page stopped answering, with how many of
/// the plugins' views open in it are held back.
fn stopped_notice(held: usize) -> String {
    let said = "A window stopped answering and was loaded again.";
    match held {
        0 => said.to_owned(),
        1 => format!(
            "{said} A plugin's view that was open in it was stopped; load it again from where it is shown."
        ),
        _ => format!(
            "{said} The {held} plugins' views that were open in it were stopped; load each again from where it is shown."
        ),
    }
}

/// Whether a window has the pane or the panel a page named as where it shows a view.
fn shows(window: &Window, host: &str) -> bool {
    let docked = [
        &window.dock.top,
        &window.dock.left,
        &window.dock.right,
        &window.dock.bottom,
    ];
    window
        .panes()
        .iter()
        .any(|pane| pane.id.to_string() == host)
        || docked
            .iter()
            .any(|area| area.panels.iter().any(|panel| panel.id.as_str() == host))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_that_answered_and_then_says_nothing_in_sight_is_restarted() {
        let mut watch = Watch::default();
        assert_eq!(watch.heard(Heard::Answered), Next::Watch);
        assert_eq!(watch.heard(Heard::Answered), Next::Watch);
        assert_eq!(watch.heard(Heard::Silent), Next::Restart);
    }

    #[test]
    fn a_page_that_never_answered_is_given_a_while_to_start() {
        let mut watch = Watch::default();
        for _ in 1..STARTING_SILENCES {
            assert_eq!(watch.heard(Heard::Silent), Next::Watch);
        }
        assert_eq!(watch.heard(Heard::Answered), Next::Watch);
        assert_eq!(watch.heard(Heard::Silent), Next::Restart);

        // One that never answers at all has stopped too, a while later.
        let mut watch = Watch::default();
        for _ in 1..STARTING_SILENCES {
            assert_eq!(watch.heard(Heard::Silent), Next::Watch);
            assert_eq!(watch.heard(Heard::Unseen), Next::Watch);
        }
        assert_eq!(watch.heard(Heard::Silent), Next::Restart);
    }

    #[test]
    fn a_page_out_of_sight_may_be_held_back_by_its_webview() {
        let mut watch = Watch::default();
        watch.heard(Heard::Answered);
        for _ in 0..5 {
            assert_eq!(watch.heard(Heard::Unseen), Next::Watch);
        }
        assert_eq!(watch.heard(Heard::Answered), Next::Watch);
    }

    #[test]
    fn the_user_is_told_how_many_views_were_stopped() {
        assert_eq!(
            stopped_notice(0),
            "A window stopped answering and was loaded again."
        );
        assert!(stopped_notice(1).contains("A plugin's view that was open in it was stopped"));
        assert!(stopped_notice(3).contains("The 3 plugins' views"));
    }

    #[test]
    fn a_view_is_still_shown_where_the_window_has_its_pane_or_its_panel() {
        use lume_layout::{Floating, Pane, PaneContent, PaneId, UnitRect, WindowId};

        // A new window has the top bar's panels, and a pane floats over it.
        let mut window = Window::new(WindowId::new(1));
        window.floating.push(Floating {
            pane: Pane {
                id: PaneId::new(4),
                title: None,
                icon: None,
                content: PaneContent::PluginView {
                    plugin: "acme.tools".to_owned(),
                    view: "board".to_owned(),
                    state: serde_json::Value::Null,
                },
                key: None,
            },
            rect: UnitRect::FULL,
            minimised: None,
        });
        assert!(shows(&window, "p4"));
        assert!(shows(&window, "lume.topbar.left"));
        assert!(!shows(&window, "p5"));
        assert!(!shows(&window, "acme.tools.board"));
    }
}
