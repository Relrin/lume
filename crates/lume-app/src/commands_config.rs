//! The commands a terminal window's page calls about the configuration.
//!
//! A page reads the configuration once with `config_view` and is told of every change
//! from then on. Changing a setting writes it to the user's own file, comments and all
//! else in the file kept, and the change then reaches every page the way a change made
//! in an editor would.
//!
//! The page that shows the settings also asks what there is to choose from: what can be
//! set at all, and the themes and the shells there are.

use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use tauri::State;
use tauri::ipc::Response;

use crate::background;
use crate::configsvc::ThemeEntry;
use crate::error::CommandError;
use crate::opener;
use crate::services::Services;
use crate::tuning::BACKGROUND_IMAGE_MAX_BYTES;

type Shared<'a> = State<'a, Arc<Services>>;

/// The configuration in use: the settings, where each came from, what was wrong in the
/// files, every design token's value and the keys in effect.
#[tauri::command]
pub fn config_view(services: Shared<'_>) -> Value {
    services.config.view()
}

/// Sets one setting in the user's file. `path` is the setting's dotted path, such as
/// `font.size`.
#[tauri::command]
pub async fn config_set(
    services: Shared<'_>,
    path: String,
    value: Value,
) -> Result<(), CommandError> {
    let config = Arc::clone(&services.config);
    tokio::task::spawn_blocking(move || config.set(&path, &value))
        .await
        .map_err(|error| CommandError::internal(error.to_string()))?
        .map_err(CommandError::from)
}

/// Takes one setting out of the user's file, so that its default holds again.
#[tauri::command]
pub async fn config_unset(services: Shared<'_>, path: String) -> Result<(), CommandError> {
    let config = Arc::clone(&services.config);
    tokio::task::spawn_blocking(move || config.unset(&path))
        .await
        .map_err(|error| CommandError::internal(error.to_string()))?
        .map_err(CommandError::from)
}

/// What can be set, as a JSON Schema: every setting with its kind, its limits and what
/// it is for, and how the settings window shows the ones it shows.
#[tauri::command]
pub fn config_schema(services: Shared<'_>) -> Value {
    services.config.schema()
}

/// The themes there are to choose from.
#[tauri::command]
pub fn themes_list(services: Shared<'_>) -> Vec<ThemeEntry> {
    services.config.themes()
}

/// A shell a user can choose as the one new terminals run.
#[derive(Debug, Serialize)]
pub struct ShellEntry {
    /// What a person calls it.
    name: &'static str,
    /// What `terminal.shell` is set to for it.
    program: String,
}

/// The well-known shells installed on this machine.
#[tauri::command]
pub async fn shells_list() -> Result<Vec<ShellEntry>, CommandError> {
    let shells = tokio::task::spawn_blocking(lume_session::known_shells)
        .await
        .map_err(|error| CommandError::internal(error.to_string()))?;
    Ok(shells
        .into_iter()
        .map(|shell| ShellEntry {
            name: shell.name,
            program: shell.program,
        })
        .collect())
}

/// The profiles in effect: the ways there are to start a terminal, the id of the one
/// new terminals use, and what there is to say about ones that are not there. A page
/// reads this once and is told of every change from then on.
#[tauri::command]
pub fn profiles_list(services: Shared<'_>) -> Value {
    services.panes.profiles.view()
}

/// The picture shown behind the panes, as the bytes of its file; no bytes when the
/// settings name no picture. The page says nothing of which file: it is the one the
/// settings name and no other.
#[tauri::command]
pub async fn background_image(services: Shared<'_>) -> Result<Response, CommandError> {
    let config = Arc::clone(&services.config);
    let bytes = tokio::task::spawn_blocking(move || match config.background_file() {
        Some(file) => background::read(&file, BACKGROUND_IMAGE_MAX_BYTES),
        None => Ok(Vec::new()),
    })
    .await
    .map_err(|error| CommandError::internal(error.to_string()))??;
    Ok(Response::new(bytes))
}

/// Opens one of the user's settings files for editing: `config`, which is the default,
/// or `workspace` for the file of the workspace in use.
#[tauri::command]
pub async fn config_open(services: Shared<'_>, file: Option<String>) -> Result<(), CommandError> {
    let config = Arc::clone(&services.config);
    let which = file.unwrap_or_else(|| "config".to_owned());
    tokio::task::spawn_blocking(move || {
        let path = config.file(&which).ok_or_else(|| {
            CommandError::bad_request(format!("there is no file called `{which}`"))
        })?;
        // A file the user has not written yet is given to them to begin from.
        if which == "config" {
            config.ensure_default_file();
        }
        opener::open_settings_file(&config.dirs().config, &path)
    })
    .await
    .map_err(|error| CommandError::internal(error.to_string()))?
}
