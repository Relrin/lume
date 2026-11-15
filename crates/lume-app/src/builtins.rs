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
    Dir, FocusTarget, Intent, NewContent, PaneId, SpawnRequest, TabId, TabTarget, UNITS, WindowId,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::configsvc::ConfigService;
use crate::control::Shell;
use crate::error::CommandError;
use crate::opener;
use crate::panes::{Panes, equal_ratios};
use crate::registry::{CommandSpec, Owner, Registry, Runs};

/// One row of the table of commands: id, title, category, where it runs, and whether
/// the palette offers it.
type Row = (&'static str, &'static str, &'static str, Runs, bool);

const COMMANDS: &[Row] = &[
    ("lume.app.quit", "Quit", "Application", Runs::Core, true),
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
    let object = |properties: Value, required: &[&str]| json!({ "type": "object", "properties": properties, "required": required });
    match id {
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
        | "lume.pane.split-right"
        | "lume.pane.split-down"
        | "lume.pane.split-left"
        | "lume.pane.split-up"
        | "lume.pane.zoom" => Some(object(json!({ "pane": pane }), &[])),
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

fn terminal() -> NewContent {
    NewContent::Terminal(SpawnRequest::default())
}

fn split(pane: PaneId, dir: Dir) -> Intent {
    Intent::Split {
        pane,
        dir,
        ratio: UNITS / 2,
        content: terminal(),
    }
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
        "lume.tab.new" => Intent::TabNew {
            window,
            content: terminal(),
            index: None,
            activate: true,
        },
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
        "lume.pane.split-right" => split(context.pane(args)?, Dir::Right),
        "lume.pane.split-down" => split(context.pane(args)?, Dir::Down),
        "lume.pane.split-left" => split(context.pane(args)?, Dir::Left),
        "lume.pane.split-up" => split(context.pane(args)?, Dir::Up),
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
    fn every_default_key_runs_a_command_there_is_or_one_still_to_come() {
        // The keys bound out of the box name these commands, which are not made yet.
        const TO_COME: [&str; 7] = [
            "lume.pane.float",
            "lume.pane.scratch",
            "lume.palette.open",
            "lume.search.open",
            "lume.settings.open",
            "lume.window.new",
            "lume.panel.toggle",
        ];
        let registry = registry();
        for platform in lume_config::Platform::ALL {
            for binding in lume_config::default_bindings(platform) {
                let known = registry.get(&binding.command).is_some()
                    || TO_COME.contains(&binding.command.as_str());
                assert!(known, "{} on {platform:?}", binding.command);
            }
        }
        for id in TO_COME {
            assert!(
                registry.get(id).is_none(),
                "{id} is made now: take it off the list"
            );
        }
    }

    #[test]
    fn a_command_with_arguments_describes_them() {
        let registry = registry();
        let activate = registry.get("lume.tab.activate").unwrap();
        assert_eq!(activate.args.unwrap()["required"], json!(["index"]));
        assert!(registry.get("lume.tab.new").unwrap().args.is_none());
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
