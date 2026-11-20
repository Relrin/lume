//! The commands pages call about plugins.
//!
//! A terminal window's page fetches everything the plugins add once, with what their
//! code shows in it now, and again whenever it is told that changed; what their code
//! shows from then on reaches it as notices. Pictures and shaders are named there by
//! their hashes, and the page asks for one when it first needs it. The page says in turn which of the
//! plugins' things it has on screen, which starts a plugin that waits for one of its
//! things to be seen and lets the machine be read for meters only while they are seen.
//!
//! The settings window manages what is installed: it lists the plugins with how each
//! fares, turns them on and off, removes them, moves them in the order they load in,
//! and installs one from a folder once the user has seen what it would be able to do.

use std::sync::Arc;

use serde_json::Value;
use tauri::{State, WebviewWindow};

use crate::error::CommandError;
use crate::services::Services;

type Shared<'a> = State<'a, Arc<Services>>;

/// Everything the plugins add to a window, with what their code shows in it now.
// Run beside the main thread, which the windows need while they start.
#[tauri::command(async)]
pub fn ui_contributions(services: Shared<'_>, window: WebviewWindow) -> Value {
    services.plugins.snapshot(window.label())
}

/// How many pictures and shaders a page may ask for at a time.
const ASSETS_AT_ONCE: usize = 64;

/// The plugins' pictures, as the addresses a window draws them from, and the shaders
/// of their cursor effects, by the hashes the window was told. One that is not there
/// any more is left out.
// Run beside the main thread, which the windows need while they start.
#[tauri::command(async)]
pub fn ui_assets(services: Shared<'_>, hashes: Vec<String>) -> Value {
    let found: serde_json::Map<String, Value> = hashes
        .into_iter()
        .filter(|hash| hash.len() <= 128)
        .take(ASSETS_AT_ONCE)
        .filter_map(|hash| {
            let body = services.plugins.asset(&hash)?;
            Some((hash, Value::String(body)))
        })
        .collect();
    Value::Object(found)
}

/// The user opened a row of a plugin's list panel whose rows are given when it is
/// opened: the plugin is asked for them, and they arrive as the panel's rows do.
#[tauri::command]
pub async fn ui_panel_expand(
    services: Shared<'_>,
    panel: String,
    path: Vec<String>,
) -> Result<(), CommandError> {
    services.plugins.expand(&panel, path).await
}

/// The page opened one of a plugin's views in a frame: a pane view (`place = "pane"`)
/// or a panel that shows a page (`"panel"`). `view` is the page's own name for it.
/// Gives the instance the plugin's code knows it by.
#[tauri::command]
pub fn view_opened(
    services: Shared<'_>,
    window: WebviewWindow,
    view: String,
    plugin: String,
    name: String,
    place: String,
) -> Result<String, CommandError> {
    let place = crate::plugins::ViewPlace::parse(&place)
        .ok_or_else(|| CommandError::bad_request("a view is in a pane or a panel"))?;
    services
        .plugins
        .view_opened(window.label(), &view, &plugin, &name, place)
}

/// The page closed one of its views.
#[tauri::command]
pub fn view_closed(services: Shared<'_>, window: WebviewWindow, view: String) {
    services.plugins.view_closed(window.label(), &view);
}

/// A call one of the page's open views made of its plugin's code. A failure of the
/// code's comes back with the code's own kind of failure.
#[tauri::command]
pub async fn view_call(
    services: Shared<'_>,
    window: WebviewWindow,
    view: String,
    name: String,
    args: Value,
) -> Result<Value, CommandError> {
    services
        .plugins
        .view_call(window.label(), &view, &name, args)
        .await
        .map_err(|error| {
            let code = serde_json::to_value(error.code)
                .ok()
                .and_then(|code| code.as_str().map(str::to_owned))
                .unwrap_or_else(|| "failed".to_owned());
            CommandError::new(code, error.message)
        })
}

/// The plugins' things the calling window has on screen: status items and panels, by
/// their full ids. A window that is minimised or hidden says it has none.
// Run beside the main thread, which the windows need while they start.
#[tauri::command(async)]
pub fn ui_visible(services: Shared<'_>, window: WebviewWindow, items: Vec<String>) {
    // A page says what it draws; a list without end is not that.
    let items: Vec<String> = items
        .into_iter()
        .filter(|item| item.len() <= 128)
        .take(512)
        .collect();
    services.plugins.visible(window.label(), items);
}

/// Every plugin installed, in the order they load in, with how each fares.
#[tauri::command]
pub async fn addons_list(services: Shared<'_>) -> Result<Value, CommandError> {
    Ok(services.plugins.addons().await)
}

/// Turns a plugin on or off.
#[tauri::command]
pub async fn addon_set_enabled(
    services: Shared<'_>,
    id: String,
    enabled: bool,
) -> Result<(), CommandError> {
    services.plugins.set_addon_enabled(&id, enabled).await
}

/// Removes a plugin; its data is kept.
#[tauri::command]
pub async fn addon_remove(services: Shared<'_>, id: String) -> Result<(), CommandError> {
    services.plugins.remove_addon(&id).await
}

/// Moves a plugin one place earlier or later in the order plugins load in.
#[tauri::command]
pub async fn addon_move(
    services: Shared<'_>,
    id: String,
    earlier: bool,
) -> Result<(), CommandError> {
    services.plugins.move_addon(&id, earlier).await
}

/// Copies and checks the package in a folder, and says what installing it would do.
#[tauri::command]
pub async fn addon_inspect(
    services: Shared<'_>,
    folder: String,
    install_as: Option<String>,
) -> Result<Value, CommandError> {
    services.plugins.inspect_addon(folder, install_as).await
}

/// Installs a package the user agreed to, by the token its inspection gave.
#[tauri::command]
pub async fn addon_install(services: Shared<'_>, token: u64) -> Result<Value, CommandError> {
    services.plugins.install_addon(token).await
}

/// Lets go of a package the user did not install.
#[tauri::command]
pub fn addon_cancel(services: Shared<'_>, token: u64) {
    services.plugins.cancel_addon(token);
}
