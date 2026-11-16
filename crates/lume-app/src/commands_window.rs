//! The commands a terminal window's page calls about its own window, and about the
//! workspaces there are.
//!
//! A window without the system's title bar has its bar drawn by the page, and the page
//! asks here for what a title bar does: taking hold of the window to move it, and the
//! controls that put it away, make it fill the screen, and close it. These are the
//! application's own commands rather than the webview library's, so the page is
//! granted these few things and nothing else of the window.

use std::sync::Arc;

use lume_proto::app::WorkspaceEntry;
use serde::Serialize;
use tauri::{State, WebviewWindow};

use crate::error::CommandError;
use crate::services::Services;
use crate::window::{self, Control, WindowState};

type Shared<'a> = State<'a, Arc<Services>>;

fn failed(error: tauri::Error) -> CommandError {
    CommandError::internal(error.to_string())
}

/// Takes hold of the calling window, as pressing on a title bar does: the window
/// follows the pointer until the button is let go.
#[tauri::command]
pub fn window_drag(window: WebviewWindow) -> Result<(), CommandError> {
    window.start_dragging().map_err(failed)
}

/// Does what one of a window's controls does to the calling window: `minimise`,
/// `maximise`, which also undoes itself, or `close`.
#[tauri::command]
pub fn window_control(window: WebviewWindow, action: String) -> Result<(), CommandError> {
    let control = Control::named(&action).ok_or_else(|| {
        CommandError::bad_request(format!("a window has no control called `{action}`"))
    })?;
    window::control(&window, control).map_err(failed)
}

/// What the calling window is doing.
#[tauri::command]
pub fn window_state(window: WebviewWindow) -> WindowState {
    window::state_of(&window)
}

/// The workspaces there are, and which of them is on show.
#[derive(Debug, Serialize)]
pub struct Workspaces {
    active: String,
    workspaces: Vec<WorkspaceEntry>,
}

/// Lists the workspaces.
#[tauri::command]
pub async fn workspaces_list(services: Shared<'_>) -> Result<Workspaces, CommandError> {
    let panes = Arc::clone(&services.panes);
    let (active, workspaces) = tokio::task::spawn_blocking(move || panes.workspaces())
        .await
        .map_err(|error| CommandError::internal(error.to_string()))??;
    Ok(Workspaces { active, workspaces })
}
