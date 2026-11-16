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
    Area, Dir, FocusTarget, Intent, NewContent, PaneId, Place, SpawnRequest, TabId, TabTarget,
    UNITS, WindowId,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::configsvc::ConfigService;
use crate::control::Shell;
use crate::error::CommandError;
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
    let object = |properties: Value, required: &[&str]| json!({ "type": "object", "properties": properties, "required": required });
    match id {
        "lume.tab.new" | "lume.window.new" => Some(object(json!({ "profile": profile }), &[])),
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
        | "lume.pane.zoom"
        | "lume.pane.float"
        | "lume.pane.minimise"
        | "lume.pane.move-to-new-tab" => Some(object(json!({ "pane": pane }), &[])),
        "lume.panel.toggle" => Some(object(
            json!({ "area": {
                "type": "string",
                "enum": ["left", "right", "bottom"],
                "description": "The side to fold away or open; the one with the tabs if left out.",
            } }),
            &[],
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

impl Context<'_> {
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

    fn tab_count(&self) -> usize {
        self.panes
            .workspace()
            .window(self.window)
            .map_or(0, |window| window.centre.tabs.len())
    }
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

/// A new terminal that runs `profile`, or the default one if none is named.
fn terminal_with(profile: Option<String>) -> NewContent {
    NewContent::Terminal(SpawnRequest {
        profile,
        ..SpawnRequest::default()
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
        content: terminal_with(named.or_else(inherited)),
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
            content: terminal_with(profile(context, args).await?),
            index: None,
            activate: true,
        },
        "lume.terminal.new-with-profile" => {
            let named = profile(context, args)
                .await?
                .ok_or_else(|| CommandError::bad_request("which profile: `profile`"))?;
            Intent::TabNew {
                window,
                content: terminal_with(Some(named)),
                index: None,
                activate: true,
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
            Intent::Split {
                pane,
                dir: Dir::Right,
                ratio: UNITS / 2,
                content: NewContent::Terminal(SpawnRequest {
                    backend: terminal.backend,
                    profile: terminal.profile,
                    launcher: terminal.launcher,
                    cwd: terminal.cwd,
                    env: terminal.env,
                    size: None,
                }),
            }
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
            content: terminal_with(profile(context, args).await?),
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
                context.shell.quit();
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
