//! The commands a terminal window's page calls about the layout, the sessions of its
//! panes, and the application's commands.
//!
//! A page keeps a copy of the layout. It reads the whole of it once with `layout_view`
//! and follows the notices from there; what it wants changed it asks for with
//! `layout_apply` or by running a command, and sees the result arrive as a notice like
//! any other page does.

use std::sync::Arc;

use lume_layout::{Applied, Intent, PaneId, TabId, WindowId};
use serde_json::Value;
use tauri::{State, Webview};

use crate::error::CommandError;
use crate::panes::{LayoutOpened, LayoutReapplied, LayoutSaved, PaneStart, SessionsStarted};
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
// Run beside the main thread, which the windows need while they start.
#[tauri::command(async)]
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

/// Which of the calling window's `panes` run something that closing them would end.
/// A pane that is not in the window runs nothing of the window's.
// Run beside the main thread, which the windows need while they start.
#[tauri::command(async)]
pub fn panes_busy(
    webview: Webview,
    services: Shared<'_>,
    panes: Vec<PaneId>,
) -> Result<Vec<PaneId>, CommandError> {
    let window = window_for(&webview)?;
    let workspace = services.panes.workspace();
    Ok(panes
        .into_iter()
        .filter(|pane| workspace.window_of(*pane) == Some(window))
        .filter(|pane| services.panes.busy(*pane))
        .collect())
}

/// The sessions on this machine that no pane of any workspace shows, for the user to
/// show again or end.
#[tauri::command]
pub async fn sessions_detached(services: Shared<'_>) -> Result<Value, CommandError> {
    let sessions = crate::host::detached(&services.state, &services.panes).await?;
    Ok(serde_json::json!({ "sessions": sessions }))
}

/// Every command there is.
// Run beside the main thread, which the windows need while they start.
#[tauri::command(async)]
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

/// That `tab` is a tab of the calling window.
fn own_tab(webview: &Webview, services: &Services, tab: TabId) -> Result<(), CommandError> {
    let window = window_for(webview)?;
    let workspace = services.panes.workspace();
    let theirs = workspace
        .windows
        .iter()
        .find(|found| found.centre.tab(tab).is_some())
        .map(|found| found.id);
    if theirs == Some(window) {
        Ok(())
    } else {
        Err(CommandError::not_found(format!(
            "this window has no tab {tab}"
        )))
    }
}

/// Opens the tabs of a layout file in the calling window.
#[tauri::command]
pub async fn layouts_open(
    webview: Webview,
    services: Shared<'_>,
    carrier: u32,
    id: String,
) -> Result<LayoutOpened, CommandError> {
    let window = window_for(&webview)?;
    services.state.check_carrier(webview.label(), carrier)?;
    services.panes.open_layout(&id, Some(window)).await
}

/// Gives a tab of the calling window the shape its layout file has now.
#[tauri::command]
pub async fn layouts_reapply(
    webview: Webview,
    services: Shared<'_>,
    carrier: u32,
    tab: TabId,
) -> Result<LayoutReapplied, CommandError> {
    services.state.check_carrier(webview.label(), carrier)?;
    own_tab(&webview, &services, tab)?;
    services.panes.reapply_layout(tab).await
}

/// Lets a tab of the calling window go of its layout file.
#[tauri::command]
pub async fn layouts_detach(
    webview: Webview,
    services: Shared<'_>,
    carrier: u32,
    tab: TabId,
) -> Result<(), CommandError> {
    services.state.check_carrier(webview.label(), carrier)?;
    own_tab(&webview, &services, tab)?;
    services.panes.detach_layout(tab).await
}

/// Writes a tab of the calling window as a layout file called `name`.
#[tauri::command]
pub async fn layouts_save_tab(
    webview: Webview,
    services: Shared<'_>,
    carrier: u32,
    tab: TabId,
    name: String,
    replace: Option<bool>,
) -> Result<LayoutSaved, CommandError> {
    services.state.check_carrier(webview.label(), carrier)?;
    own_tab(&webview, &services, tab)?;
    services
        .panes
        .save_tab_as_layout(tab, &name, replace.unwrap_or(false))
        .await
}
