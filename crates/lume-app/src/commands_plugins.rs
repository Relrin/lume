//! The commands a terminal window's page calls about what plugins add to it.
//!
//! A page fetches everything the plugins add once, with what their code shows in it
//! now, and again whenever it is told that changed; what their code shows from then on
//! reaches it as notices. The page says in turn which of the plugins' things it has on
//! screen, which starts a plugin that waits for one of its things to be seen and lets
//! the machine be read for meters only while they are seen.

use std::sync::Arc;

use serde_json::Value;
use tauri::{State, WebviewWindow};

use crate::services::Services;

type Shared<'a> = State<'a, Arc<Services>>;

/// Everything the plugins add to a window, with what their code shows in it now.
#[tauri::command]
pub fn ui_contributions(services: Shared<'_>) -> Value {
    services.plugins.snapshot()
}

/// The plugins' things the calling window has on screen: status items and panels, by
/// their full ids. A window that is minimised or hidden says it has none.
#[tauri::command]
pub fn ui_visible(services: Shared<'_>, window: WebviewWindow, items: Vec<String>) {
    // A page says what it draws; a list without end is not that.
    let items: Vec<String> = items
        .into_iter()
        .filter(|item| item.len() <= 128)
        .take(512)
        .collect();
    services.plugins.visible(window.label(), items);
}
