//! The application's own commands: what each is called, and what the ones that run in
//! the application do.
//!
//! Most of them are an intent with the blanks filled in: "split right" is a split of
//! whichever pane has the keyboard in the window the command was run in. A command
//! that names its pane or tab in its arguments acts on that one instead, which is how
//! a button in a pane's header closes its own pane and not the focused one.
//!
//! Commands that act on what only a page has (a terminal's selection, its scrollback,
//! the font size of a window) are listed here and carried out there.

use lume_layout::{
    Area, Dir, FocusTarget, GroupIcon, GroupId, Intent, NewContent, PaneId, Place, SpawnRequest,
    TabColor, TabId, TabTarget, UNITS, Window, WindowId, group_add, group_remove, group_step,
    tab_step, tabs_beside,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use lume_session::{BackendId, Router, SessionId};

use crate::carrier::SessionKey;
use crate::configsvc::ConfigService;
use crate::control::Shell;
use crate::error::CommandError;
use crate::host;
use crate::opener;
use crate::panes::{Panes, equal_ratios};
use crate::registry::{CommandSpec, Owner, Registry, Runs};
use crate::services::Asker;
use crate::workspaces::Find;

/// One row of the table of commands: id, title, category, where it runs, and whether
/// the palette offers it.
type Row = (&'static str, &'static str, &'static str, Runs, bool);

/// Where the application's source and its issues are.
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

const COMMANDS: &[Row] = &[
    ("lume.app.quit", "Quit", "Application", Runs::Core, true),
    (
        "lume.app.quit-end-sessions",
        "Quit and End Sessions",
        "Application",
        Runs::Core,
        true,
    ),
    (
        "lume.host.restart",
        "Restart Session Host",
        "Application",
        Runs::Core,
        true,
    ),
    (
        "lume.session.detached",
        "Detached Sessions…",
        "Session",
        Runs::Page,
        true,
    ),
    (
        "lume.session.reattach",
        "Show Detached Session",
        "Session",
        Runs::Core,
        false,
    ),
    (
        "lume.session.end",
        "End Detached Session",
        "Session",
        Runs::Core,
        false,
    ),
    (
        "lume.app.show-logs",
        "Show Logs",
        "Application",
        Runs::Core,
        true,
    ),
    (
        "lume.app.report-issue",
        "Report an Issue",
        "Application",
        Runs::Core,
        true,
    ),
    (
        "lume.app.whats-new",
        "What's New",
        "Application",
        Runs::Core,
        true,
    ),
    (
        "lume.config.open",
        "Open Configuration File",
        "Configuration",
        Runs::Core,
        true,
    ),
    (
        "lume.config.reload",
        "Reload Configuration",
        "Configuration",
        Runs::Core,
        true,
    ),
    ("lume.tab.new", "New Tab", "Tab", Runs::Core, true),
    (
        "lume.terminal.new-with-profile",
        "New Terminal with Profile…",
        "Tab",
        Runs::Core,
        true,
    ),
    (
        "lume.profile.refresh",
        "Look for Profiles Again",
        "Application",
        Runs::Core,
        true,
    ),
    ("lume.tab.close", "Close Tab", "Tab", Runs::Core, true),
    ("lume.tab.next", "Next Tab", "Tab", Runs::Core, true),
    ("lume.tab.previous", "Previous Tab", "Tab", Runs::Core, true),
    ("lume.tab.activate", "Go to Tab", "Tab", Runs::Core, false),
    ("lume.tab.rename", "Rename Tab", "Tab", Runs::Core, false),
    ("lume.tab.move", "Move Tab", "Tab", Runs::Core, false),
    (
        "lume.tab.rename-start",
        "Rename Tab…",
        "Tab",
        Runs::Page,
        true,
    ),
    (
        "lume.tab.move-left",
        "Move Tab Left",
        "Tab",
        Runs::Core,
        true,
    ),
    (
        "lume.tab.move-right",
        "Move Tab Right",
        "Tab",
        Runs::Core,
        true,
    ),
    (
        "lume.tab.close-left",
        "Close Tabs to the Left",
        "Tab",
        Runs::Core,
        true,
    ),
    (
        "lume.tab.close-right",
        "Close Tabs to the Right",
        "Tab",
        Runs::Core,
        true,
    ),
    ("lume.tab.color", "Set Tab Colour", "Tab", Runs::Core, false),
    (
        "lume.tab.lock",
        "Lock or Unlock Tab",
        "Tab",
        Runs::Core,
        true,
    ),
    (
        "lume.tab.merge",
        "Move Tab into Another Tab",
        "Tab",
        Runs::Core,
        false,
    ),
    (
        "lume.group.new",
        "Add Tab to New Group",
        "Tab",
        Runs::Core,
        true,
    ),
    (
        "lume.group.add",
        "Add Tab to Group",
        "Tab",
        Runs::Core,
        false,
    ),
    (
        "lume.group.remove",
        "Remove Tab from Group",
        "Tab",
        Runs::Core,
        true,
    ),
    ("lume.group.set", "Change Group", "Tab", Runs::Core, false),
    (
        "lume.group.toggle",
        "Collapse or Expand Group",
        "Tab",
        Runs::Core,
        true,
    ),
    (
        "lume.group.move-left",
        "Move Group Left",
        "Tab",
        Runs::Core,
        true,
    ),
    (
        "lume.group.move-right",
        "Move Group Right",
        "Tab",
        Runs::Core,
        true,
    ),
    (
        "lume.group.ungroup",
        "Ungroup Tabs",
        "Tab",
        Runs::Core,
        true,
    ),
    ("lume.group.close", "Close Group", "Tab", Runs::Core, true),
    (
        "lume.group.reopen",
        "Reopen Group",
        "Tab",
        Runs::Core,
        false,
    ),
    (
        "lume.group.forget",
        "Forget Closed Group",
        "Tab",
        Runs::Core,
        false,
    ),
    (
        "lume.group.move-to-workspace",
        "Move Group to Workspace…",
        "Tab",
        Runs::Core,
        true,
    ),
    (
        "lume.group.bring-back",
        "Bring Group Back",
        "Tab",
        Runs::Core,
        false,
    ),
    ("lume.pane.close", "Close Pane", "Pane", Runs::Core, true),
    (
        "lume.pane.split-right",
        "Split Right",
        "Pane",
        Runs::Core,
        true,
    ),
    (
        "lume.pane.split-down",
        "Split Down",
        "Pane",
        Runs::Core,
        true,
    ),
    (
        "lume.pane.split-left",
        "Split Left",
        "Pane",
        Runs::Core,
        true,
    ),
    ("lume.pane.split-up", "Split Up", "Pane", Runs::Core, true),
    (
        "lume.pane.duplicate",
        "Duplicate Pane",
        "Pane",
        Runs::Core,
        true,
    ),
    (
        "lume.pane.show-in-new-tab",
        "Show Pane's Session in New Tab",
        "Pane",
        Runs::Core,
        true,
    ),
    (
        "lume.pane.show-in-new-window",
        "Show Pane's Session in New Window",
        "Pane",
        Runs::Core,
        true,
    ),
    (
        "lume.pane.reconnect",
        "Reconnect or Restart Pane",
        "Pane",
        Runs::Core,
        true,
    ),
    (
        "lume.pane.copy-cwd",
        "Copy Working Directory",
        "Pane",
        Runs::Core,
        true,
    ),
    (
        "lume.pane.open-cwd",
        "Open Working Directory in File Manager",
        "Pane",
        Runs::Core,
        true,
    ),
    (
        "lume.pane.focus-left",
        "Focus Pane on the Left",
        "Pane",
        Runs::Core,
        true,
    ),
    (
        "lume.pane.focus-right",
        "Focus Pane on the Right",
        "Pane",
        Runs::Core,
        true,
    ),
    (
        "lume.pane.focus-up",
        "Focus Pane Above",
        "Pane",
        Runs::Core,
        true,
    ),
    (
        "lume.pane.focus-down",
        "Focus Pane Below",
        "Pane",
        Runs::Core,
        true,
    ),
    (
        "lume.pane.focus-next",
        "Focus Next Pane",
        "Pane",
        Runs::Core,
        true,
    ),
    (
        "lume.pane.focus-previous",
        "Focus Previous Pane",
        "Pane",
        Runs::Core,
        true,
    ),
    (
        "lume.pane.focus-last",
        "Focus Last Used Pane",
        "Pane",
        Runs::Core,
        true,
    ),
    (
        "lume.pane.zoom",
        "Toggle Pane Zoom",
        "Pane",
        Runs::Core,
        true,
    ),
    (
        "lume.pane.equalise",
        "Equalise Panes",
        "Pane",
        Runs::Core,
        true,
    ),
    ("lume.pane.rename", "Rename Pane", "Pane", Runs::Core, false),
    (
        "lume.pane.float",
        "Float or Dock Pane",
        "Pane",
        Runs::Core,
        true,
    ),
    (
        "lume.pane.minimise",
        "Minimise or Restore Pane",
        "Pane",
        Runs::Core,
        true,
    ),
    (
        "lume.pane.scratch",
        "Toggle Scratch Terminal",
        "Pane",
        Runs::Core,
        true,
    ),
    (
        "lume.pane.move-to-new-tab",
        "Move Pane to New Tab",
        "Pane",
        Runs::Core,
        true,
    ),
    ("lume.window.new", "New Window", "Window", Runs::Core, true),
    (
        "lume.window.close",
        "Close Window",
        "Window",
        Runs::Core,
        true,
    ),
    (
        "lume.panel.toggle",
        "Toggle Side Panel",
        "Panel",
        Runs::Core,
        true,
    ),
    ("lume.panel.show", "Show Panel", "Panel", Runs::Core, false),
    (
        "lume.panel.close",
        "Close Panel",
        "Panel",
        Runs::Core,
        false,
    ),
    (
        "lume.panel.expand",
        "Open a Row of a Panel",
        "Panel",
        Runs::Core,
        false,
    ),
    (
        "lume.workspace.switch",
        "Switch Workspace",
        "Workspace",
        Runs::Core,
        true,
    ),
    (
        "lume.workspace.new",
        "New Workspace",
        "Workspace",
        Runs::Core,
        true,
    ),
    (
        "lume.workspace.rename",
        "Rename Workspace",
        "Workspace",
        Runs::Core,
        true,
    ),
    (
        "lume.workspace.delete",
        "Delete Workspace",
        "Workspace",
        Runs::Core,
        true,
    ),
    (
        "lume.workspace.set-root",
        "Set Workspace Folder",
        "Workspace",
        Runs::Core,
        true,
    ),
    (
        "lume.workspace.save",
        "Save Workspace",
        "Workspace",
        Runs::Core,
        true,
    ),
    (
        "lume.settings.open",
        "Settings",
        "Application",
        Runs::Core,
        true,
    ),
    (
        "lume.palette.open",
        "Command Palette",
        "View",
        Runs::Page,
        false,
    ),
    (
        "lume.connect.open",
        "Open Terminal…",
        "Tab",
        Runs::Page,
        true,
    ),
    (
        "lume.search.open",
        "Find in Terminal",
        "Search",
        Runs::Page,
        true,
    ),
    ("lume.search.next", "Find Next", "Search", Runs::Page, true),
    (
        "lume.search.previous",
        "Find Previous",
        "Search",
        Runs::Page,
        true,
    ),
    (
        "lume.search.close",
        "Close Find",
        "Search",
        Runs::Page,
        true,
    ),
    (
        "lume.window.focus-bar",
        "Move Keyboard to the Window's Bar",
        "Window",
        Runs::Page,
        true,
    ),
    ("lume.terminal.copy", "Copy", "Terminal", Runs::Page, true),
    (
        "lume.window.answer",
        "Answer the Question on Show",
        "Window",
        Runs::Page,
        false,
    ),
    ("lume.terminal.paste", "Paste", "Terminal", Runs::Page, true),
    (
        "lume.terminal.select-all",
        "Select All",
        "Terminal",
        Runs::Page,
        true,
    ),
    (
        "lume.terminal.clear",
        "Clear Terminal",
        "Terminal",
        Runs::Page,
        true,
    ),
    (
        "lume.terminal.scroll-page-up",
        "Scroll Up One Page",
        "Terminal",
        Runs::Page,
        true,
    ),
    (
        "lume.terminal.scroll-page-down",
        "Scroll Down One Page",
        "Terminal",
        Runs::Page,
        true,
    ),
    (
        "lume.terminal.scroll-top",
        "Scroll to Top",
        "Terminal",
        Runs::Page,
        true,
    ),
    (
        "lume.terminal.scroll-bottom",
        "Scroll to Bottom",
        "Terminal",
        Runs::Page,
        true,
    ),
    (
        "lume.view.zoom-in",
        "Increase Font Size",
        "View",
        Runs::Page,
        true,
    ),
    (
        "lume.view.zoom-out",
        "Decrease Font Size",
        "View",
        Runs::Page,
        true,
    ),
    (
        "lume.view.zoom-reset",
        "Reset Font Size",
        "View",
        Runs::Page,
        true,
    ),
];

/// The arguments of the commands that take any, as JSON Schema.
fn arguments(id: &str) -> Option<Value> {
    let pane =
        json!({ "type": "string", "description": "A pane's id; the focused pane if left out." });
    let tab =
        json!({ "type": "string", "description": "A tab's id; the tab on show if left out." });
    let profile = json!({
        "type": "string",
        "description": "A profile's id; the default profile if left out.",
    });
    let group = json!({
        "type": "string",
        "description": "A group's id; the group of the tab on show if left out.",
    });
    let colors = json!([
        "red", "orange", "yellow", "green", "teal", "blue", "purple", "pink"
    ]);
    let workspace = json!({
        "type": "string",
        "description": "The workspace to move it to",
        "x-lume-choices": "workspaces",
    });
    let object = |properties: Value, required: &[&str]| json!({ "type": "object", "properties": properties, "required": required });
    match id {
        "lume.window.new" => Some(object(json!({ "profile": profile }), &[])),
        "lume.window.answer" => Some(object(
            json!({
                "proceed": { "type": "boolean", "description": "Whether to go ahead, as the button that does would." },
                "item": { "type": "string", "description": "What to choose, for a choice from a list." },
            }),
            &["proceed"],
        )),
        "lume.tab.new" => Some(object(
            json!({
                "profile": profile,
                "group": { "type": "string", "description": "A group to open it in, after its last tab." },
            }),
            &[],
        )),
        "lume.tab.rename-start"
        | "lume.tab.move-left"
        | "lume.tab.move-right"
        | "lume.tab.close-left"
        | "lume.tab.close-right"
        | "lume.group.new"
        | "lume.group.remove" => Some(object(json!({ "tab": tab }), &[])),
        "lume.tab.color" => Some(object(
            json!({ "tab": tab, "color": {
                "type": ["string", "null"],
                "enum": ["red", "orange", "yellow", "green", "teal", "blue", "purple", "pink", null],
                "description": "The tab's colour; none if left out.",
            } }),
            &[],
        )),
        "lume.tab.lock" => Some(object(
            json!({ "tab": tab, "on": { "type": "boolean", "description": "Locked or not; the other way round if left out." } }),
            &[],
        )),
        "lume.tab.merge" => Some(object(
            json!({ "tab": tab, "into": { "type": "string", "description": "The tab whose panes it joins." } }),
            &["into"],
        )),
        "lume.group.add" => Some(object(
            json!({ "tab": tab, "group": { "type": "string", "description": "The group to put it in." } }),
            &["group"],
        )),
        "lume.group.set" => Some(object(
            json!({
                "group": group,
                "name": { "type": ["string", "null"] },
                "color": { "type": "string", "enum": colors },
                "icon": { "type": ["string", "null"], "enum": ["bug", "flame", "rocket", "flask", "wrench", "layers", "server", "book", null] },
                "collapsed": { "type": "boolean" },
                "kept": { "type": "boolean" },
            }),
            &[],
        )),
        "lume.group.toggle"
        | "lume.group.move-left"
        | "lume.group.move-right"
        | "lume.group.ungroup"
        | "lume.group.close" => Some(object(json!({ "group": group }), &[])),
        "lume.group.reopen" | "lume.group.forget" => Some(object(
            json!({ "group": { "type": "string", "description": "A closed group's id." } }),
            &["group"],
        )),
        "lume.group.move-to-workspace" => Some(object(
            json!({ "group": group, "workspace": workspace }),
            &["workspace"],
        )),
        "lume.group.bring-back" => Some(object(
            json!({
                "workspace": { "type": "string", "description": "The workspace the group is in" },
                "group": { "type": "string", "description": "The group's id there." },
            }),
            &["workspace", "group"],
        )),
        "lume.terminal.new-with-profile" => Some(object(
            json!({ "profile": {
                "type": "string",
                "description": "The profile to open",
                "x-lume-choices": "profiles",
            } }),
            &["profile"],
        )),
        "lume.pane.split-right"
        | "lume.pane.split-down"
        | "lume.pane.split-left"
        | "lume.pane.split-up" => Some(object(
            json!({
                "pane": pane,
                "profile": {
                    "type": "string",
                    "description": "A profile's id; the profile of the pane that is split if left out.",
                },
            }),
            &[],
        )),
        "lume.tab.activate" => Some(object(
            json!({ "index": { "type": "integer", "minimum": 1, "description": "The tab's place, counted from one." } }),
            &["index"],
        )),
        "lume.tab.close" => Some(object(json!({ "tab": tab }), &[])),
        "lume.tab.rename" => Some(object(
            json!({ "tab": tab, "title": { "type": ["string", "null"] } }),
            &[],
        )),
        "lume.tab.move" => Some(object(
            json!({ "tab": tab, "index": { "type": "integer", "minimum": 0 } }),
            &["index"],
        )),
        "lume.pane.rename" => Some(object(
            json!({ "pane": pane, "title": { "type": ["string", "null"] } }),
            &[],
        )),
        "lume.pane.close"
        | "lume.pane.duplicate"
        | "lume.pane.reconnect"
        | "lume.pane.copy-cwd"
        | "lume.pane.open-cwd"
        | "lume.pane.zoom"
        | "lume.pane.float"
        | "lume.pane.minimise"
        | "lume.pane.show-in-new-tab"
        | "lume.pane.show-in-new-window"
        | "lume.pane.move-to-new-tab" => Some(object(json!({ "pane": pane }), &[])),
        "lume.panel.toggle" => Some(object(
            json!({ "area": {
                "type": "string",
                "enum": ["left", "right", "bottom"],
                "description": "The side to fold away or open; the one with the tabs if left out.",
            } }),
            &[],
        )),
        "lume.panel.show" | "lume.panel.close" => Some(object(
            json!({ "panel": {
                "type": "string",
                "description": "A plugin's panel, by its id: the plugin's and the panel's name, as in acme.tools.processes.",
            } }),
            &["panel"],
        )),
        "lume.panel.expand" => Some(object(
            json!({
                "panel": {
                    "type": "string",
                    "description": "A plugin's panel, by its id: the plugin's and the panel's name, as in acme.tools.processes.",
                },
                "path": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "The row whose rows the plugin is asked for, by the ids of the rows from the top down to it.",
                },
            }),
            &["panel", "path"],
        )),
        "lume.workspace.switch" => Some(object(
            json!({ "name": {
                "type": "string",
                "description": "The workspace to show",
                "x-lume-choices": "workspaces",
            } }),
            &["name"],
        )),
        "lume.workspace.delete" => Some(object(
            json!({ "name": {
                "type": "string",
                "description": "The workspace to delete",
                "x-lume-choices": "workspaces",
            } }),
            &["name"],
        )),
        "lume.workspace.new" => Some(object(
            json!({
                "name": { "type": "string", "description": "A name for the new workspace" },
                "root": {
                    "type": ["string", "null"],
                    "description": "The folder its terminals start in",
                },
            }),
            &["name"],
        )),
        "lume.workspace.rename" => Some(object(
            json!({ "name": { "type": "string", "description": "The workspace's new name" } }),
            &["name"],
        )),
        "lume.search.open" => Some(object(
            json!({
                "query": { "type": "string", "description": "The text to look for" },
                "pane": pane,
            }),
            &[],
        )),
        "lume.search.next" | "lume.search.previous" | "lume.search.close" => {
            Some(object(json!({ "pane": pane }), &[]))
        }
        "lume.settings.open" => Some(object(
            json!({ "section": {
                "type": "string",
                "description": "The page of the settings to show",
            } }),
            &[],
        )),
        "lume.workspace.set-root" => Some(object(
            json!({ "root": {
                "type": "string",
                "description": "The folder new terminals start in; empty for none",
            } }),
            &["root"],
        )),
        "lume.app.quit-end-sessions" | "lume.host.restart" => Some(object(
            json!({ "confirmed": {
                "type": "boolean",
                "description": "The user has agreed; asked from a window, the window asks first otherwise",
            } }),
            &[],
        )),
        "lume.session.reattach" => Some(object(
            json!({
                "session": { "type": "string", "description": "The session's id" },
                "backend": { "type": "string", "description": "Its backend; the local one if left out" },
                "place": {
                    "type": "string",
                    "enum": ["tab", "split", "window"],
                    "description": "A new tab, beside the pane that has the keyboard, or a new window",
                },
            }),
            &["session"],
        )),
        "lume.session.end" => Some(object(
            json!({
                "session": { "type": "string", "description": "The session's id" },
                "backend": { "type": "string", "description": "Its backend; the local one if left out" },
            }),
            &["session"],
        )),
        _ => None,
    }
}

/// A registry holding the application's own commands.
pub fn registry() -> Registry {
    let registry = Registry::new();
    for (id, title, category, runs, palette) in COMMANDS {
        let spec = CommandSpec {
            id: (*id).to_owned(),
            title: (*title).to_owned(),
            category: (*category).to_owned(),
            runs: *runs,
            palette: *palette,
            args: arguments(id),
        };
        // A row that is refused is a mistake in the table above; a test holds every
        // row to being taken.
        if let Err(error) = registry.register(Owner::App, spec) {
            tracing::error!(%error, "one of the application's commands was not registered");
        }
    }
    registry
}

/// What a command that runs in the application acts on.
pub struct Context<'a> {
    pub panes: &'a Panes,
    pub config: &'a ConfigService,
    pub shell: &'a dyn Shell,
    /// The window the command was run in.
    pub window: WindowId,
    /// Who asked for the command.
    pub asker: Asker,
}

/// One of a command's arguments, if it was given.
fn argument<T: DeserializeOwned>(args: &Value, name: &str) -> Result<Option<T>, CommandError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|error| CommandError::bad_request(format!("the argument `{name}`: {error}"))),
    }
}

/// The session a command's arguments name, on the backend they name or the local one.
fn session_key(args: &Value) -> Result<SessionKey, CommandError> {
    let session: SessionId = argument(args, "session")?
        .ok_or_else(|| CommandError::bad_request("the argument `session` is missing"))?;
    let backend = argument::<BackendId>(args, "backend")?.unwrap_or_else(Router::local_id);
    Ok(SessionKey { backend, session })
}

/// Whether a command that ends programs is to go ahead now. Asked for in a window and
/// not yet agreed to, the window asks the user first, and runs it again on a yes;
/// asked for from outside, nobody is there to answer.
fn agreed(context: &Context<'_>, args: &Value) -> Result<bool, CommandError> {
    Ok(context.asker != Asker::Page || argument::<bool>(args, "confirmed")?.unwrap_or(false))
}

/// Writes the workspace, ends every session the host runs, and quits. Asks first in a
/// window where something other than a shell at its prompt runs.
async fn quit_ending_sessions(context: &Context<'_>, args: &Value) -> Result<(), CommandError> {
    let panes = context.panes;
    let state = context.state()?;
    if !agreed(context, args)? {
        let busy = panes
            .workspace()
            .panes()
            .into_iter()
            .filter(|pane| panes.busy(pane.id))
            .count();
        if busy > 0 {
            let message = format!(
                "{} still running a program. Quitting this way ends every program in every \
                 terminal, those of sessions no pane shows too.",
                if busy == 1 {
                    "A pane is".to_owned()
                } else {
                    format!("{busy} panes are")
                }
            );
            return host::ask_first(
                &state,
                context.window,
                "lume.app.quit-end-sessions",
                args,
                host::Question {
                    title: "Quit and end every program?".to_owned(),
                    message,
                    confirm: "Quit and end".to_owned(),
                },
            );
        }
    }
    // Written first, with every pane in it: the panes are not closed by their programs
    // ending, and come back with fresh ones at the next start.
    panes.close();
    let ended = host::end_all(&state, panes).await;
    tracing::info!(sessions = ended, "quitting with every session ended");
    context.shell.quit();
    Ok(())
}

/// Ends every session, restarts the session host, and starts every pane again. Always
/// asks first in a window.
async fn restart_host(context: &Context<'_>, args: &Value) -> Result<(), CommandError> {
    let state = context.state()?;
    if !agreed(context, args)? {
        return host::ask_first(
            &state,
            context.window,
            host::RESTART_COMMAND,
            args,
            host::Question {
                title: "Restart the session host?".to_owned(),
                message: "Every program in every terminal ends, those of sessions no pane \
                          shows too. The panes start new ones."
                    .to_owned(),
                confirm: "Restart".to_owned(),
            },
        );
    }
    let ended = host::restart(&state, context.panes).await?;
    context.panes.notice(
        "info",
        match ended {
            0 => "The session host was restarted.".to_owned(),
            1 => "The session host was restarted, and the program it ran was ended.".to_owned(),
            ended => format!(
                "The session host was restarted, and the {ended} programs it ran were ended."
            ),
        },
    );
    Ok(())
}

impl Context<'_> {
    /// The application's state, while the application runs.
    fn state(&self) -> Result<std::sync::Arc<crate::state::AppState>, CommandError> {
        self.panes
            .state
            .upgrade()
            .ok_or_else(|| CommandError::new("disconnected", "the application is closing"))
    }

    /// The pane a command acts on: the one its arguments name, or the focused one.
    fn pane(&self, args: &Value) -> Result<PaneId, CommandError> {
        argument(args, "pane")?
            .or_else(|| self.panes.focused_pane(self.window))
            .ok_or_else(|| CommandError::not_found("there is no pane to act on"))
    }

    /// The tab a command acts on: the one its arguments name, or the one on show.
    fn tab(&self, args: &Value) -> Result<TabId, CommandError> {
        let on_show = || {
            self.panes
                .workspace()
                .window(self.window)
                .and_then(|window| window.centre.active)
        };
        argument(args, "tab")?
            .or_else(on_show)
            .ok_or_else(|| CommandError::not_found("there is no tab to act on"))
    }

    /// The window the command was run in, as it is now.
    fn window_now(&self) -> Result<Window, CommandError> {
        self.panes
            .workspace()
            .window(self.window)
            .cloned()
            .ok_or_else(|| CommandError::not_found("the window is gone"))
    }

    /// The group a command acts on: the one its arguments name, or that of the tab on
    /// show.
    fn group(&self, args: &Value) -> Result<GroupId, CommandError> {
        if let Some(group) = argument(args, "group")? {
            return Ok(group);
        }
        let window = self.window_now()?;
        window
            .centre
            .active_tab()
            .and_then(|tab| tab.group)
            .ok_or_else(|| CommandError::not_found("the tab on show is in no group"))
    }

    fn tab_count(&self) -> usize {
        self.panes
            .workspace()
            .window(self.window)
            .map_or(0, |window| window.centre.tabs.len())
    }
}

/// An argument that may be given as null to take something away: left out it is
/// `None`, null is `Some(None)`.
fn given<T: DeserializeOwned>(args: &Value, name: &str) -> Result<Option<Option<T>>, CommandError> {
    match args.get(name) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(value) => serde_json::from_value(value.clone())
            .map(|value| Some(Some(value)))
            .map_err(|error| CommandError::bad_request(format!("the argument `{name}`: {error}"))),
    }
}

/// The group a command about a closed group names.
fn required_group(args: &Value) -> Result<GroupId, CommandError> {
    argument(args, "group")?.ok_or_else(|| CommandError::bad_request("which group: `group`"))
}

/// The workspace's name a command was given.
fn name(args: &Value) -> Result<String, CommandError> {
    argument::<String>(args, "name")?
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| CommandError::bad_request("which workspace: `name`"))
}

/// Opens a place of the application's own: a page of its repository in the browser,
/// or one of its files shown in its folder.
async fn open_link(link: String) -> Result<(), CommandError> {
    tokio::task::spawn_blocking(move || opener::open(&link))
        .await
        .map_err(|error| CommandError::internal(error.to_string()))?
}

fn terminal() -> NewContent {
    NewContent::Terminal(SpawnRequest::default())
}

/// A new terminal that runs `profile`, or the default one if none is named, opened
/// from the pane `from`: it starts in the directory that pane's shell is in, where
/// that is known and the new terminal will be in the same place.
async fn terminal_with(
    context: &Context<'_>,
    profile: Option<String>,
    from: Option<PaneId>,
) -> NewContent {
    let cwd = match from {
        Some(from) => context.panes.inherited_dir(from, profile.as_deref()).await,
        None => None,
    };
    NewContent::Terminal(SpawnRequest {
        profile,
        cwd,
        ..SpawnRequest::default()
    })
}

/// The directory the shell in the pane a command means is in. A pane whose shell has
/// not said where it is has none to give.
async fn live_dir(
    context: &Context<'_>,
    args: &Value,
) -> Result<(lume_session::Launcher, String), CommandError> {
    let pane = context.pane(args)?;
    context.panes.live_dir(pane).await.ok_or_else(|| {
        CommandError::not_found(format!(
            "the shell in pane {pane} has not said which directory it is in"
        ))
    })
}

/// The profile a command's arguments name. It has to be one there is: a pane that
/// quietly started something else would be worse than a refusal.
async fn profile(context: &Context<'_>, args: &Value) -> Result<Option<String>, CommandError> {
    let Some(id) = argument::<String>(args, "profile")?.filter(|id| !id.trim().is_empty()) else {
        return Ok(None);
    };
    match context.panes.profiles.resolve(&id).await {
        Some(_) => Ok(Some(id)),
        None => Err(CommandError::not_found(format!(
            "there is no profile called `{id}`"
        ))),
    }
}

/// A split of `pane`. The new pane runs the profile the arguments name, or else the
/// profile of the pane that is split: a second terminal on the same machine or in the
/// same distribution is what splitting one asks for.
async fn split(context: &Context<'_>, args: &Value, dir: Dir) -> Result<Intent, CommandError> {
    let pane = context.pane(args)?;
    let named = profile(context, args).await?;
    let inherited = || {
        context
            .panes
            .terminal(pane)
            .and_then(|terminal| terminal.profile)
    };
    Ok(Intent::Split {
        pane,
        dir,
        ratio: UNITS / 2,
        content: terminal_with(context, named.or_else(inherited), Some(pane)).await,
    })
}

/// Carries out a command that runs in the application.
pub async fn run(context: &Context<'_>, id: &str, args: &Value) -> Result<(), CommandError> {
    let window = context.window;
    let panes = context.panes;
    let intent = match id {
        "lume.app.quit" => {
            context.shell.quit();
            return Ok(());
        }
        "lume.app.quit-end-sessions" => return quit_ending_sessions(context, args).await,
        "lume.host.restart" => return restart_host(context, args).await,
        "lume.session.reattach" => {
            let key = session_key(args)?;
            let place = argument::<host::Place>(args, "place")?.unwrap_or_default();
            return host::reattach(panes, window, key, place).await;
        }
        "lume.pane.show-in-new-tab" | "lume.pane.show-in-new-window" => {
            let pane = context.pane(args)?;
            let key = panes
                .session_of(pane)
                .ok_or_else(|| CommandError::not_found(format!("pane {pane} shows no session")))?;
            let place = if id == "lume.pane.show-in-new-window" {
                host::Place::Window
            } else {
                host::Place::Tab
            };
            return host::reattach(panes, window, key, place).await;
        }
        "lume.session.end" => {
            let state = context.state()?;
            return host::end(&state, &session_key(args)?).await;
        }
        "lume.config.open" => {
            context.config.ensure_default_file();
            let dir = context.config.dirs().config.clone();
            let file = context.config.dirs().config_file();
            return tokio::task::spawn_blocking(move || opener::open_settings_file(&dir, &file))
                .await
                .map_err(|error| CommandError::internal(error.to_string()))?;
        }
        "lume.config.reload" => return Ok(context.config.reload()?),
        "lume.app.show-logs" => {
            let shown = crate::log::log_file()
                .and_then(|file| url::Url::from_file_path(file).ok())
                .ok_or_else(|| CommandError::not_found("the application keeps no log"))?;
            return open_link(shown.into()).await;
        }
        "lume.settings.open" => {
            let section: Option<String> = argument(args, "section")?;
            context
                .shell
                .open_settings(section.as_deref(), Some(window));
            return Ok(());
        }
        "lume.app.report-issue" => return open_link(format!("{REPOSITORY}/issues/new")).await,
        "lume.app.whats-new" => return open_link(format!("{REPOSITORY}/releases")).await,
        "lume.tab.new" => Intent::TabNew {
            window,
            // A terminal opened from the window's pane starts where that pane is.
            content: terminal_with(
                context,
                profile(context, args).await?,
                panes.focused_pane(window),
            )
            .await,
            index: None,
            activate: true,
            group: argument(args, "group")?,
        },
        "lume.terminal.new-with-profile" => {
            let named = profile(context, args)
                .await?
                .ok_or_else(|| CommandError::bad_request("which profile: `profile`"))?;
            Intent::TabNew {
                window,
                content: terminal_with(context, Some(named), panes.focused_pane(window)).await,
                index: None,
                activate: true,
                group: None,
            }
        }
        "lume.profile.refresh" => {
            context.panes.profiles.refresh().await;
            return Ok(());
        }
        "lume.tab.close" => Intent::TabClose {
            tab: context.tab(args)?,
        },
        "lume.tab.next" => Intent::TabActivate {
            target: TabTarget::Next { window },
        },
        "lume.tab.previous" => Intent::TabActivate {
            target: TabTarget::Previous { window },
        },
        "lume.tab.activate" => {
            let place: u32 = argument(args, "index")?
                .filter(|place| *place >= 1)
                .ok_or_else(|| CommandError::bad_request("which tab: `index`, counted from one"))?;
            // A key for a tab that is not there does nothing, rather than complain.
            if place as usize > context.tab_count() {
                return Ok(());
            }
            Intent::TabActivate {
                target: TabTarget::Index {
                    window,
                    index: place - 1,
                },
            }
        }
        "lume.tab.rename" => Intent::TabRename {
            tab: context.tab(args)?,
            title: argument(args, "title")?,
        },
        "lume.tab.move" => Intent::TabMove {
            tab: context.tab(args)?,
            index: argument(args, "index")?
                .ok_or_else(|| CommandError::bad_request("where to: `index`"))?,
        },
        "lume.tab.move-left" | "lume.tab.move-right" => {
            let tab = context.tab(args)?;
            let forward = id == "lume.tab.move-right";
            // A tab at the end has nowhere to go, and a key held there does nothing.
            match tab_step(&context.window_now()?, tab, forward) {
                Some(intent) => intent,
                None => return Ok(()),
            }
        }
        "lume.tab.close-left" | "lume.tab.close-right" => Intent::TabsClose {
            tabs: tabs_beside(
                &context.window_now()?,
                context.tab(args)?,
                id == "lume.tab.close-right",
            ),
        },
        "lume.tab.color" => Intent::TabColor {
            tab: context.tab(args)?,
            color: argument::<TabColor>(args, "color")?,
        },
        "lume.tab.lock" => {
            let tab = context.tab(args)?;
            let locked = panes.workspace().tab(tab).is_some_and(|found| found.locked);
            Intent::TabLock {
                tab,
                on: argument(args, "on")?.unwrap_or(!locked),
            }
        }
        "lume.tab.merge" => Intent::TabMerge {
            tab: context.tab(args)?,
            into: argument(args, "into")?
                .ok_or_else(|| CommandError::bad_request("which tab: `into`"))?,
        },
        "lume.group.new" => Intent::GroupNew {
            tab: context.tab(args)?,
        },
        "lume.group.add" => {
            let tab = context.tab(args)?;
            let group: GroupId = argument(args, "group")?
                .ok_or_else(|| CommandError::bad_request("which group: `group`"))?;
            group_add(&context.window_now()?, tab, group).ok_or_else(|| {
                CommandError::not_found(format!("this window has no group {group}"))
            })?
        }
        "lume.group.remove" => {
            let tab = context.tab(args)?;
            match group_remove(&context.window_now()?, tab) {
                Some(intent) => intent,
                None => return Ok(()),
            }
        }
        "lume.group.set" => Intent::GroupSet {
            group: context.group(args)?,
            name: given(args, "name")?,
            color: argument(args, "color")?,
            icon: given::<GroupIcon>(args, "icon")?,
            collapsed: argument(args, "collapsed")?,
            kept: argument(args, "kept")?,
        },
        "lume.group.toggle" => {
            let group = context.group(args)?;
            let collapsed = context
                .window_now()?
                .centre
                .group(group)
                .is_some_and(|found| found.collapsed);
            Intent::GroupSet {
                group,
                name: None,
                color: None,
                icon: None,
                collapsed: Some(!collapsed),
                kept: None,
            }
        }
        "lume.group.move-left" | "lume.group.move-right" => {
            let group = context.group(args)?;
            match group_step(&context.window_now()?, group, id == "lume.group.move-right") {
                Some(intent) => intent,
                None => return Ok(()),
            }
        }
        "lume.group.ungroup" => Intent::GroupUngroup {
            group: context.group(args)?,
        },
        "lume.group.close" => Intent::GroupClose {
            group: context.group(args)?,
        },
        "lume.group.reopen" => Intent::GroupReopen {
            group: required_group(args)?,
            window,
        },
        "lume.group.forget" => Intent::GroupForget {
            group: required_group(args)?,
        },
        "lume.group.move-to-workspace" => {
            let to: String = argument(args, "workspace")?
                .ok_or_else(|| CommandError::bad_request("which workspace: `workspace`"))?;
            return panes.move_group(context.group(args)?, to.trim()).await;
        }
        "lume.group.bring-back" => {
            let from: String = argument(args, "workspace")?
                .ok_or_else(|| CommandError::bad_request("which workspace: `workspace`"))?;
            return panes
                .bring_group_back(from.trim(), required_group(args)?, window)
                .await;
        }
        "lume.pane.close" => Intent::Close {
            pane: context.pane(args)?,
        },
        "lume.pane.split-right" => split(context, args, Dir::Right).await?,
        "lume.pane.split-down" => split(context, args, Dir::Down).await?,
        "lume.pane.split-left" => split(context, args, Dir::Left).await?,
        "lume.pane.split-up" => split(context, args, Dir::Up).await?,
        "lume.pane.duplicate" => {
            let pane = context.pane(args)?;
            let terminal = panes.terminal(pane).ok_or_else(|| {
                CommandError::bad_request(format!("pane {pane} is not a terminal"))
            })?;
            // The same thing again, beside it: what was asked for, not the session.
            // It starts where the first one's shell is now, or else where that started.
            let cwd = match panes.live_dir(pane).await {
                Some((_, dir)) => Some(dir),
                None => terminal.cwd,
            };
            Intent::Split {
                pane,
                dir: Dir::Right,
                ratio: UNITS / 2,
                content: NewContent::Terminal(SpawnRequest {
                    backend: terminal.backend,
                    profile: terminal.profile,
                    launcher: terminal.launcher,
                    cwd,
                    env: terminal.env,
                    size: None,
                    session: None,
                }),
            }
        }
        "lume.pane.reconnect" => {
            let pane = context.pane(args)?;
            return panes.restart(pane).await.map(drop);
        }
        "lume.pane.copy-cwd" => {
            let (_, dir) = live_dir(context, args).await?;
            context.shell.copy_text(dir);
            return Ok(());
        }
        "lume.pane.open-cwd" => {
            let folder = match live_dir(context, args).await? {
                // A distribution's directory is reached from this machine as a share.
                (lume_session::Launcher::Wsl { distro, .. }, dir) => {
                    lume_session::wsl::linux_to_unc(&distro, &dir).ok_or_else(|| {
                        CommandError::bad_request(format!(
                            "`{dir}` has no path the file manager could be given"
                        ))
                    })?
                }
                (_, dir) => dir,
            };
            let opened = tokio::task::spawn_blocking(move || {
                crate::opener::open_folder(std::path::Path::new(&folder))
            });
            return opened
                .await
                .map_err(|error| CommandError::internal(error.to_string()))?;
        }
        "lume.pane.focus-left" => Intent::FocusDir {
            window,
            dir: Dir::Left,
        },
        "lume.pane.focus-right" => Intent::FocusDir {
            window,
            dir: Dir::Right,
        },
        "lume.pane.focus-up" => Intent::FocusDir {
            window,
            dir: Dir::Up,
        },
        "lume.pane.focus-down" => Intent::FocusDir {
            window,
            dir: Dir::Down,
        },
        "lume.pane.focus-next" => Intent::Focus {
            target: FocusTarget::Next { window },
        },
        "lume.pane.focus-previous" => Intent::Focus {
            target: FocusTarget::Previous { window },
        },
        "lume.pane.focus-last" => Intent::Focus {
            target: FocusTarget::Back { window },
        },
        "lume.pane.zoom" => {
            let pane = context.pane(args)?;
            Intent::Zoom {
                pane,
                on: !panes.is_zoomed(pane),
            }
        }
        "lume.pane.equalise" => {
            for (split, children) in panes.splits_on_show(window) {
                panes
                    .apply(Intent::Resize {
                        split,
                        ratios: equal_ratios(children),
                    })
                    .await?;
            }
            return Ok(());
        }
        "lume.pane.rename" => Intent::PaneRename {
            pane: context.pane(args)?,
            title: argument(args, "title")?,
        },
        "lume.pane.float" => {
            let pane = context.pane(args)?;
            if panes.is_floating(pane) {
                Intent::Dock { pane, to: None }
            } else {
                Intent::Float { pane, rect: None }
            }
        }
        "lume.pane.minimise" => {
            let pane = context.pane(args)?;
            Intent::Minimise {
                pane,
                on: !panes.is_minimised(pane),
            }
        }
        "lume.pane.scratch" => {
            if panes.scratch_shown_in(window) {
                Intent::ScratchHide {}
            } else {
                Intent::ScratchShow {
                    window,
                    content: Some(terminal()),
                }
            }
        }
        "lume.pane.move-to-new-tab" => Intent::Move {
            pane: context.pane(args)?,
            to: Place::NewTab {
                window,
                index: None,
            },
        },
        "lume.window.new" => Intent::WindowNew {
            // A terminal opened from the window's pane starts where that pane is.
            content: terminal_with(
                context,
                profile(context, args).await?,
                panes.focused_pane(window),
            )
            .await,
            bounds: None,
        },
        "lume.window.close" => {
            // Asked for in the window itself, it is the window's own close control by
            // another name, and the user may be asked first. Asked for from outside,
            // nobody is there to answer.
            if context.asker == Asker::Page {
                context.shell.ask_close(window);
                return Ok(());
            }
            // The last window closes the application, which keeps the layout as it
            // is; any other takes its tabs with it.
            if panes.workspace().windows.len() <= 1 {
                // Unless the application keeps running in the background, which keeps
                // the window ready instead.
                if context.config.current().config.app.keep_running {
                    panes.keep_ready(window, context.shell);
                } else {
                    context.shell.quit();
                }
                return Ok(());
            }
            Intent::WindowClose { window }
        }
        "lume.panel.toggle" => {
            let area = match argument::<Area>(args, "area")? {
                Some(Area::Top) => {
                    return Err(CommandError::bad_request(
                        "the bar at the top cannot be folded away",
                    ));
                }
                Some(area) => Some(area),
                None => panes.dock_to_toggle(window),
            };
            // With no panel at any side there is nothing to fold away.
            let Some(area) = area else {
                return Ok(());
            };
            Intent::PanelToggle {
                window,
                area,
                on: None,
            }
        }
        "lume.workspace.switch" => {
            return panes.switch(&name(args)?, Find::Existing).await;
        }
        "lume.workspace.new" => {
            panes.switch(&name(args)?, Find::New).await?;
            return match argument::<String>(args, "root")? {
                Some(root) if !root.trim().is_empty() => panes.set_root(Some(&root)).await,
                _ => Ok(()),
            };
        }
        "lume.workspace.rename" => return panes.rename(&name(args)?).await,
        "lume.workspace.delete" => return panes.delete(&name(args)?).await,
        "lume.workspace.set-root" => {
            let root: Option<String> = argument(args, "root")?;
            return panes.set_root(root.as_deref()).await;
        }
        "lume.workspace.save" => {
            panes.save().await;
            return Ok(());
        }
        _ => {
            return Err(CommandError::not_found(format!(
                "the application has no command called `{id}`"
            )));
        }
    };
    panes.apply(intent).await.map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_in_the_table_is_registered() {
        let registry = registry();
        assert_eq!(registry.list().len(), COMMANDS.len());
        for (id, ..) in COMMANDS {
            assert!(registry.get(id).is_some(), "{id}");
        }
    }

    #[test]
    fn every_default_key_runs_a_command_there_is() {
        let registry = registry();
        for platform in lume_config::Platform::ALL {
            for binding in lume_config::default_bindings(platform) {
                assert!(
                    registry.get(&binding.command).is_some(),
                    "{} on {platform:?}",
                    binding.command
                );
            }
        }
    }

    #[test]
    fn searching_runs_in_the_page_and_the_settings_open_in_the_application() {
        let registry = registry();
        for id in [
            "lume.connect.open",
            "lume.search.open",
            "lume.search.next",
            "lume.search.previous",
            "lume.search.close",
            "lume.window.focus-bar",
        ] {
            let spec = registry.get(id).unwrap();
            assert_eq!(spec.runs, Runs::Page, "{id}");
            assert!(spec.palette, "{id}");
        }
        let open = registry.get("lume.search.open").unwrap().args.unwrap();
        assert_eq!(open["properties"]["query"]["type"], "string");
        assert_eq!(open["required"], json!([]));
        let settings = registry.get("lume.settings.open").unwrap();
        assert_eq!(settings.runs, Runs::Core);
        assert_eq!(
            settings.args.unwrap()["properties"]["section"]["type"],
            "string"
        );
    }

    #[test]
    fn a_command_with_arguments_describes_them() {
        let registry = registry();
        let activate = registry.get("lume.tab.activate").unwrap();
        assert_eq!(activate.args.unwrap()["required"], json!(["index"]));
        // A new tab may name its profile and need not.
        let new_tab = registry.get("lume.tab.new").unwrap().args.unwrap();
        assert_eq!(new_tab["properties"]["profile"]["type"], "string");
        assert_eq!(new_tab["required"], json!([]));
        let with_profile = registry
            .get("lume.terminal.new-with-profile")
            .unwrap()
            .args
            .unwrap();
        assert_eq!(with_profile["required"], json!(["profile"]));
        assert_eq!(
            with_profile["properties"]["profile"]["x-lume-choices"],
            "profiles"
        );
        assert!(registry.get("lume.app.quit").unwrap().args.is_none());
        assert!(!registry.get("lume.tab.activate").unwrap().palette);
    }

    #[test]
    fn an_argument_of_the_wrong_kind_is_refused_and_a_missing_one_is_nothing() {
        let args = json!({ "pane": "p3", "index": "two", "title": null });
        assert_eq!(
            argument::<PaneId>(&args, "pane").unwrap(),
            Some(PaneId::new(3))
        );
        assert_eq!(argument::<String>(&args, "title").unwrap(), None);
        assert_eq!(argument::<u32>(&args, "absent").unwrap(), None);
        assert_eq!(
            argument::<u32>(&args, "index").unwrap_err().code,
            "bad_request"
        );
        assert_eq!(argument::<u32>(&Value::Null, "index").unwrap(), None);
    }
}
