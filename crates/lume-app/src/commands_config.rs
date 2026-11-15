//! The commands a terminal window's page calls about the configuration.
//!
//! A page reads the configuration once with `config_view` and is told of every change
//! from then on. Changing a setting writes it to the user's own file, comments and all
//! else in the file kept, and the change then reaches every page the way a change made
//! in an editor would.

use std::sync::Arc;

use serde_json::Value;
use tauri::State;

use crate::error::CommandError;
use crate::opener;
use crate::services::Services;

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
