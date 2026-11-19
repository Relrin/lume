//! Starting the shells of a window before its page asks for them.
//!
//! A page asks for its panes' sessions only once it has laid itself out, which on a
//! cold start comes most of a second after the process began, mostly spent waiting for
//! the webview. The shells are started at once instead, at the grid each pane is
//! expected to have, and their output waits in the host. The page attaches to them with
//! the grid it really has, and the host resizes each session then.

use std::sync::Arc;

use lume_config::schema::FontConfig;
use lume_layout::{PaneContent, UNITS, UnitRect, Window, WindowId, Workspace, tab_rects};
use lume_session::TermSize;

use crate::panes::{PaneStart, Panes};
use crate::tuning::{
    PLANNED_CELL_WIDTH_EM, PLANNED_CHROME_HEIGHT_PX, PLANNED_CHROME_WIDTH_PX,
    PLANNED_WINDOW_HEIGHT_PX, PLANNED_WINDOW_WIDTH_PX,
};

/// The panes of each window that have no session yet, each with the grid it is
/// expected to have under `font`; the tab on show first in each window.
pub fn planned(workspace: &Workspace, font: &FontConfig) -> Vec<(WindowId, Vec<PaneStart>)> {
    workspace
        .windows
        .iter()
        .filter_map(|window| {
            let starts = planned_in(window, font);
            (!starts.is_empty()).then_some((window.id, starts))
        })
        .collect()
}

fn planned_in(window: &Window, font: &FontConfig) -> Vec<PaneStart> {
    let (width, height) = window.bounds.map_or(
        (PLANNED_WINDOW_WIDTH_PX, PLANNED_WINDOW_HEIGHT_PX),
        |bounds| (f64::from(bounds.width), f64::from(bounds.height)),
    );
    let width = (width - PLANNED_CHROME_WIDTH_PX).max(1.0);
    let height = (height - PLANNED_CHROME_HEIGHT_PX).max(1.0);
    let sessionless = |pane| {
        window.panes().into_iter().any(|found| {
            found.id == pane
                && matches!(&found.content,
                    PaneContent::Terminal(terminal) if terminal.session.is_none())
        })
    };
    let active = window.centre.active_tab().map(|tab| tab.id);
    let tabs = window
        .centre
        .tabs
        .iter()
        .filter(|tab| Some(tab.id) == active)
        .chain(
            window
                .centre
                .tabs
                .iter()
                .filter(|tab| Some(tab.id) != active),
        );
    let mut starts = Vec::new();
    for tab in tabs {
        for (pane, rect) in tab_rects(&tab.root) {
            // A zoomed pane fills its tab.
            let rect = if tab.zoomed == Some(pane) {
                UnitRect::FULL
            } else {
                rect
            };
            if !sessionless(pane) {
                continue;
            }
            let share = |units: u32| f64::from(units) / f64::from(UNITS);
            if let Some(size) = grid(width * share(rect.w), height * share(rect.h), font) {
                starts.push(PaneStart { pane, size });
            }
        }
    }
    starts
}

/// The grid a terminal of `width` by `height` logical pixels is expected to have under
/// `font`.
fn grid(width: f64, height: f64, font: &FontConfig) -> Option<TermSize> {
    let cell_width = (font.size * PLANNED_CELL_WIDTH_EM).max(1.0);
    let cell_height = (font.size * font.line_height).ceil().max(1.0);
    let cols = (width / cell_width).floor().clamp(2.0, f64::from(u16::MAX));
    let rows = (height / cell_height)
        .floor()
        .clamp(1.0, f64::from(u16::MAX));
    TermSize::new(cols as u16, rows as u16).ok()
}

/// Starts the shells of every window's panes that have none, in the background.
pub fn start(panes: &Arc<Panes>, font: &FontConfig) {
    let planned = planned(&panes.workspace(), font);
    if planned.is_empty() {
        return;
    }
    let panes = Arc::clone(panes);
    tauri::async_runtime::spawn(async move {
        for (window, starts) in planned {
            let outcome = panes.start_sessions(window, starts).await;
            for failed in outcome.failed {
                // The page asks again for the panes it shows, and says why it failed.
                tracing::warn!(pane = %failed.pane, message = %failed.message,
                    "a shell started before the page asked could not be started");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font() -> FontConfig {
        FontConfig {
            size: 10.0,
            line_height: 2.0,
            ..FontConfig::default()
        }
    }

    #[test]
    fn a_grid_follows_the_font() {
        let size = grid(600.0, 400.0, &font()).expect("a grid");
        assert_eq!((size.cols(), size.rows()), (100, 20));
    }

    #[test]
    fn a_new_windows_pane_is_planned_at_the_windows_size() {
        let mut workspace = Workspace::new(lume_layout::WorkspaceId::new(1).expect("an id"), "w");
        lume_layout::apply(
            &mut workspace,
            lume_layout::Intent::WindowNew {
                content: lume_layout::NewContent::Terminal(Default::default()),
                bounds: None,
            },
            None,
        )
        .expect("a window is made");
        let planned = planned(&workspace, &font());
        assert_eq!(planned.len(), 1);
        let (_, starts) = &planned[0];
        assert_eq!(starts.len(), 1);
        let expected = grid(
            PLANNED_WINDOW_WIDTH_PX - PLANNED_CHROME_WIDTH_PX,
            PLANNED_WINDOW_HEIGHT_PX - PLANNED_CHROME_HEIGHT_PX,
            &font(),
        );
        assert_eq!(Some(starts[0].size), expected);
    }

    #[test]
    fn a_tiny_area_still_has_a_grid() {
        let size = grid(0.5, 0.5, &font()).expect("a grid");
        assert_eq!((size.cols(), size.rows()), (2, 1));
    }
}
