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
//! and installs one from a folder or a repository once the user has seen what it would
//! be able to do.

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

/// The page opened one of a plugin's views in a frame: a pane view (`place = "pane"`),
/// a panel that shows a page (`"panel"`), or the view a plugin is set up in, on its page
/// in the settings window (`"settings"`). `view` is the page's own name for it, and
/// `host` the pane or the panel it is shown in.
/// Gives the instance the plugin's code knows it by.
#[tauri::command]
pub fn view_opened(
    services: Shared<'_>,
    window: WebviewWindow,
    view: String,
    plugin: String,
    name: String,
    place: String,
    host: Option<String>,
) -> Result<String, CommandError> {
    let place = crate::plugins::ViewPlace::parse(&place).ok_or_else(|| {
        CommandError::bad_request("a view is in a pane, a panel or the settings window")
    })?;
    services
        .plugins
        .view_opened(window.label(), &view, &plugin, &name, place, host)
}

/// Keeps what one of the page's open views wants to find again when it next starts.
#[tauri::command]
pub async fn view_state(
    services: Shared<'_>,
    window: WebviewWindow,
    view: String,
    state: Value,
) -> Result<(), CommandError> {
    services
        .plugins
        .view_state(window.label(), &view, state)
        .await
}

/// One of the page's open views opens a grant to one of its plugin's streams, as a
/// stream of the page's carrier. Says whether the stream is read, written or both.
// Run beside the main thread: it starts threads and takes locks.
#[tauri::command(async)]
pub fn view_stream_open(
    services: Shared<'_>,
    window: WebviewWindow,
    carrier: u32,
    view: String,
    grant: String,
    stream: u32,
) -> Result<Value, CommandError> {
    if grant.len() > 64 {
        return Err(CommandError::not_found("there is no such grant"));
    }
    services
        .plugins
        .view_stream_open(window.label(), carrier, &view, &grant, stream)
}

/// The page closed one of its views.
#[tauri::command]
pub fn view_closed(services: Shared<'_>, window: WebviewWindow, view: String) {
    services.plugins.view_closed(window.label(), &view);
}

/// A call one of the page's open views made of its plugin's code. A failure of the
/// code's comes back with the code's own kind of failure and what it said of it.
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
            let mut failed = CommandError::new(error.shown_code(), error.message);
            failed.data = error.data;
            failed
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

/// Takes back what a plugin may run on another machine.
#[tauri::command]
pub async fn addon_revoke_machine(
    services: Shared<'_>,
    id: String,
    machine: String,
) -> Result<(), CommandError> {
    services.plugins.revoke_machine(&id, &machine).await
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

/// Fetches the package in a repository, checks it, and says what installing it would
/// do.
#[tauri::command]
pub async fn addon_inspect_git(
    services: Shared<'_>,
    source: String,
    path: Option<String>,
    install_as: Option<String>,
) -> Result<Value, CommandError> {
    if source.len() > 2048 || path.as_ref().is_some_and(|path| path.len() > 1024) {
        return Err(CommandError::bad_request(
            "that is not a repository's address",
        ));
    }
    services
        .plugins
        .inspect_addon_git(source, path, install_as)
        .await
}

/// Looks in the repository a plugin was installed from for something newer, and says
/// what installing it would do.
#[tauri::command]
pub async fn addon_check_update(services: Shared<'_>, id: String) -> Result<Value, CommandError> {
    services.plugins.check_addon_update(id).await
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
