//! The commands a terminal window's page calls about the layout, the sessions of its
//! panes, and the application's commands.
//!
//! A page keeps a copy of the layout. It reads the whole of it once with `layout_view`
//! and follows the notices from there; what it wants changed it asks for with
//! `layout_apply` or by running a command, and sees the result arrive as a notice like
//! any other page does.

use std::sync::Arc;

use lume_layout::{Applied, Intent, WindowId};
use serde_json::Value;
use tauri::{State, Webview};

use crate::error::CommandError;
use crate::panes::{PaneStart, SessionsStarted};
use crate::registry::CommandSpec;
use crate::services::{Asker, Services};
use crate::window::window_of;

type Shared<'a> = State<'a, Arc<Services>>;

/// The window of the layout a webview shows.
fn window_for(webview: &Webview) -> Result<WindowId, CommandError> {
    window_of(webview.label())
        .ok_or_else(|| CommandError::forbidden("this window shows no part of the layout"))
}

/// The layout as it is now, and which of its windows the calling page is. A page calls
/// this after opening its carrier, so no change can fall between the two.
#[tauri::command]
pub fn layout_view(webview: Webview, services: Shared<'_>) -> Result<Value, CommandError> {
    let window = window_for(&webview)?;
    // What happened before there was a page to tell.
    if let Some(page) = services.state.page_of(webview.label()) {
        for notice in services.panes.take_pending() {
            page.notify(&notice);
        }
    }
    Ok(services.panes.view(window))
}

/// Applies an intent to the layout.
#[tauri::command]
pub async fn layout_apply(
    webview: Webview,
    services: Shared<'_>,
    carrier: u32,
    intent: Intent,
) -> Result<Applied, CommandError> {
    window_for(&webview)?;
    services.state.check_carrier(webview.label(), carrier)?;
    services.panes.apply(intent).await
}

/// Starts sessions for panes of the calling window that have none, each at the grid
/// the page says its pane has.
#[tauri::command]
pub async fn sessions_start(
    webview: Webview,
    services: Shared<'_>,
    carrier: u32,
    panes: Vec<PaneStart>,
) -> Result<SessionsStarted, CommandError> {
    let window = window_for(&webview)?;
    services.state.check_carrier(webview.label(), carrier)?;
    Ok(services.panes.start_sessions(window, panes).await)
}

/// Every command there is.
#[tauri::command]
pub fn commands_list(services: Shared<'_>) -> Vec<CommandSpec> {
    services.registry.list()
}

/// Runs a command that is carried out in the application, in the calling window.
#[tauri::command]
pub async fn command_run(
    webview: Webview,
    services: Shared<'_>,
    carrier: u32,
    id: String,
    args: Option<Value>,
) -> Result<(), CommandError> {
    let window = window_for(&webview)?;
    services.state.check_carrier(webview.label(), carrier)?;
    let args = args.unwrap_or(Value::Null);
    services.run_command(window, &id, &args, Asker::Page).await
}
