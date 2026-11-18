//! The commands there are.
//!
//! A command is something the application can be told to do by name: from a key, from
//! the palette, from the command line. The registry holds what each is called and where
//! it is carried out. The application's own commands are put in when it starts; a
//! plugin's are added under the plugin's own name.
//!
//! Names are dotted and lower case. The application's begin with `lume.` and an area
//! from a short list, as in `lume.pane.split-right`. A plugin's begin with the plugin's
//! id. The areas are kept from plugins, at any depth, so a command that looks like the
//! application's is the application's; everything else under `lume.` is free, which
//! leaves room for plugins the project itself ships.

use std::collections::BTreeMap;
use std::sync::{PoisonError, RwLock};

use serde::Serialize;
use serde_json::Value;

/// The longest id a command may have.
const ID_MAX_LEN: usize = 96;

/// The first part of every id of the application's own commands.
const OWN_PREFIX: &str = "lume";

/// The areas the application's own commands are in. No plugin's id may be
/// `lume.<area>` or begin with it.
pub const AREAS: [&str; 17] = [
    "app",
    "host",
    "session",
    "window",
    "workspace",
    "tab",
    "group",
    "pane",
    "panel",
    "profile",
    "terminal",
    "view",
    "config",
    "palette",
    "connect",
    "search",
    "settings",
];

/// Where a command is carried out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Runs {
    /// In the application: it changes the layout, the configuration or the application
    /// itself.
    Core,
    /// In the page of a window: it acts on what only the page has, such as a
    /// terminal's selection.
    Page,
}

/// What a command is, for whoever lists commands or runs one.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CommandSpec {
    pub id: String,
    /// What the command is called in the palette.
    pub title: String,
    /// The heading it is listed under.
    pub category: String,
    pub runs: Runs,
    /// Whether the palette offers it. A command that only makes sense with arguments a
    /// key supplies does not belong there.
    pub palette: bool,
    /// A JSON Schema of the command's arguments, for a command that takes any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
}

/// Who adds a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner<'a> {
    /// The application itself.
    App,
    /// The plugin with this id.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "nothing loads plugins yet; the rule for their names is here already"
        )
    )]
    Plugin(&'a str),
}

/// Why a command was not added.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("`{0}` is not a command id: lower-case words joined by dots, as in `a.b.do-it`")]
    Malformed(String),
    #[error("`{id}` is not in one of the application's areas")]
    UnknownArea { id: String },
    #[error("`{id}` does not begin with `{plugin}.`, the id of the plugin that adds it")]
    Foreign { id: String, plugin: String },
    #[error("a plugin cannot be called `{0}`: that name is the application's")]
    Reserved(String),
    #[error("there is already a command called `{0}`")]
    Exists(String),
}

/// Whether text is dotted, lower-case words: letters, digits and hyphens, each part
/// beginning with a letter.
fn well_formed(id: &str) -> bool {
    let part = |part: &str| {
        part.starts_with(|first: char| first.is_ascii_lowercase())
            && part
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    id.len() <= ID_MAX_LEN && id.split('.').all(part)
}

/// Whether an id is one a plugin may not have: the application's prefix alone, or
/// anything inside one of its areas.
fn reserved(plugin: &str) -> bool {
    let mut parts = plugin.split('.');
    match (parts.next(), parts.next()) {
        (Some(OWN_PREFIX), None) => true,
        (Some(OWN_PREFIX), Some(area)) => AREAS.contains(&area),
        _ => false,
    }
}

/// The commands there are, by id.
#[derive(Default)]
pub struct Registry {
    commands: RwLock<BTreeMap<String, CommandSpec>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a command. `owner` says who adds it, which decides what it may be called.
    pub fn register(&self, owner: Owner<'_>, spec: CommandSpec) -> Result<(), RegistryError> {
        let id = spec.id.as_str();
        if !well_formed(id) {
            return Err(RegistryError::Malformed(id.to_owned()));
        }
        match owner {
            Owner::App => {
                let mut parts = id.split('.');
                let own = parts.next() == Some(OWN_PREFIX)
                    && parts.next().is_some_and(|area| AREAS.contains(&area))
                    && parts.next().is_some();
                if !own {
                    return Err(RegistryError::UnknownArea { id: id.to_owned() });
                }
            }
            Owner::Plugin(plugin) => {
                if !well_formed(plugin) || reserved(plugin) {
                    return Err(RegistryError::Reserved(plugin.to_owned()));
                }
                let within = id
                    .strip_prefix(plugin)
                    .is_some_and(|rest| rest.len() > 1 && rest.starts_with('.'));
                if !within {
                    return Err(RegistryError::Foreign {
                        id: id.to_owned(),
                        plugin: plugin.to_owned(),
                    });
                }
            }
        }
        let mut commands = self
            .commands
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        if commands.contains_key(id) {
            return Err(RegistryError::Exists(id.to_owned()));
        }
        commands.insert(id.to_owned(), spec);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<CommandSpec> {
        self.commands
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned()
    }

    /// Every command, in the order of their ids.
    pub fn list(&self) -> Vec<CommandSpec> {
        self.commands
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str) -> CommandSpec {
        CommandSpec {
            id: id.to_owned(),
            title: "Do It".to_owned(),
            category: "Test".to_owned(),
            runs: Runs::Core,
            palette: true,
            args: None,
        }
    }

    #[test]
    fn the_applications_commands_are_in_its_areas() {
        let registry = Registry::new();
        registry
            .register(Owner::App, spec("lume.pane.split-right"))
            .unwrap();
        for id in ["lume.metrics.show", "lume.pane", "other.pane.close", "lume"] {
            assert_eq!(
                registry.register(Owner::App, spec(id)),
                Err(RegistryError::UnknownArea { id: id.to_owned() }),
                "{id}"
            );
        }
        assert_eq!(
            registry.register(Owner::App, spec("lume.pane.split-right")),
            Err(RegistryError::Exists("lume.pane.split-right".to_owned()))
        );
    }

    #[test]
    fn a_plugins_commands_begin_with_its_own_id() {
        let registry = Registry::new();
        registry
            .register(Owner::Plugin("lume.metrics"), spec("lume.metrics.show"))
            .unwrap();
        registry
            .register(Owner::Plugin("acme.tools"), spec("acme.tools.deploy.now"))
            .unwrap();
        for id in ["acme.toolsx.run", "acme.tools", "lume.pane.close", "run"] {
            assert!(
                matches!(
                    registry.register(Owner::Plugin("acme.tools"), spec(id)),
                    Err(RegistryError::Foreign { .. })
                ),
                "{id}"
            );
        }
    }

    #[test]
    fn a_plugin_cannot_take_the_applications_name_or_one_of_its_areas() {
        let registry = Registry::new();
        for plugin in ["lume", "lume.pane", "lume.terminal", "lume.pane.extras"] {
            assert_eq!(
                registry.register(Owner::Plugin(plugin), spec(&format!("{plugin}.run"))),
                Err(RegistryError::Reserved(plugin.to_owned())),
                "{plugin}"
            );
        }
        // Other names under the prefix are free.
        registry
            .register(Owner::Plugin("lume.panes"), spec("lume.panes.run"))
            .unwrap();
    }

    #[test]
    fn ids_are_dotted_lower_case_words() {
        let registry = Registry::new();
        for id in [
            "lume.pane.Split",
            "lume..close",
            "lume.pane.close ",
            "lume.pane.9lives",
            "lume.pane.do_it",
        ] {
            assert_eq!(
                registry.register(Owner::App, spec(id)),
                Err(RegistryError::Malformed(id.to_owned())),
                "{id}"
            );
        }
    }

    #[test]
    fn commands_are_listed_in_the_order_of_their_ids() {
        let registry = Registry::new();
        for id in ["lume.tab.new", "lume.pane.close", "lume.app.quit"] {
            registry.register(Owner::App, spec(id)).unwrap();
        }
        let ids: Vec<String> = registry.list().into_iter().map(|spec| spec.id).collect();
        assert_eq!(ids, ["lume.app.quit", "lume.pane.close", "lume.tab.new"]);
        assert!(registry.get("lume.tab.new").is_some());
        assert!(registry.get("lume.tab.old").is_none());
    }
}
