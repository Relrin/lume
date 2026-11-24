//! The commands a page calls to add, change and take out what terminals can be opened
//! on: the profiles that reach another machine over SSH, and the Linux distributions
//! that were found. Rex servers are in [`crate::rex::commands`].
//!
//! Everything is written to the user's own file, which stays as it is otherwise, and
//! reaches every page the way a change made in an editor would.

use std::sync::Arc;

use lume_config::{ConfigError, ListedProfile, SshProfile};
use lume_session::Launcher;
use serde::Deserialize;
use serde_json::Value;
use tauri::State;

use crate::configsvc::ConfigService;
use crate::error::CommandError;
use crate::host_probe::{self, Probe};
use crate::services::Services;
use crate::ssh_try;

type Shared<'a> = State<'a, Arc<Services>>;

/// Writes to the user's file, away from the threads that wait for answers.
pub(crate) async fn write<T: Send + 'static>(
    services: &Services,
    change: impl FnOnce(&ConfigService) -> Result<T, ConfigError> + Send + 'static,
) -> Result<T, CommandError> {
    let config = Arc::clone(&services.config);
    tokio::task::spawn_blocking(move || change(&config))
        .await
        .map_err(|error| CommandError::internal(error.to_string()))?
        .map_err(CommandError::from)
}

/// A machine reached over SSH, as it is typed.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SshHost {
    host: String,
    user: Option<String>,
    port: Option<u16>,
    /// Arguments for the client, before the host.
    #[serde(default)]
    args: Vec<String>,
}

impl SshHost {
    fn launcher(self) -> Launcher {
        Launcher::Ssh {
            host: self.host.trim().to_owned(),
            user: self.user.filter(|user| !user.is_empty()),
            port: self.port,
            extra_args: self.args,
            client: None,
        }
    }
}

/// How long the machine takes to answer on its SSH port, or why that is not known.
#[tauri::command]
pub async fn ssh_reach(host: SshHost) -> Result<Probe, CommandError> {
    Ok(host_probe::probe(host.launcher()).await)
}

/// Whether the machine can be signed in to over SSH without anything being asked.
#[tauri::command]
pub async fn ssh_try(host: SshHost) -> Result<(), CommandError> {
    ssh_try::sign_in(host.launcher()).await
}

/// Writes a profile that opens a shell on a machine reached over SSH: a new one, or one
/// of the user's own in place of what it was. Gives its id.
#[tauri::command]
pub async fn ssh_profile_put(
    services: Shared<'_>,
    id: Option<String>,
    name: String,
    host: SshHost,
) -> Result<String, CommandError> {
    let profile = SshProfile {
        name,
        host: host.host,
        user: host.user,
        port: host.port,
        extra_args: host.args,
    };
    write(&services, move |config| {
        config.put_ssh_profile(id.as_deref(), &profile)
    })
    .await
}

/// Takes a profile of the user's own out of their file.
#[tauri::command]
pub async fn profile_remove(services: Shared<'_>, id: String) -> Result<(), CommandError> {
    write(&services, move |config| config.remove_profile(&id)).await
}

/// Whether a profile that was found is listed, and where its terminals start.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Listed {
    id: String,
    listed: bool,
    cwd: Option<String>,
}

/// Says of profiles that were found whether they are listed and where they start.
#[tauri::command]
pub async fn profiles_listed(
    services: Shared<'_>,
    changes: Vec<Listed>,
) -> Result<Value, CommandError> {
    let changes: Vec<ListedProfile> = changes
        .into_iter()
        .map(|change| ListedProfile {
            id: change.id,
            listed: change.listed,
            cwd: change.cwd,
        })
        .collect();
    write(&services, move |config| config.list_profiles(&changes)).await?;
    Ok(services.panes.profiles.view())
}
