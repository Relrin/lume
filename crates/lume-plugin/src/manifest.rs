//! A plugin's manifest: `plugin.toml` at the root of its package.
//!
//! The manifest says what the plugin is, what it adds and what it needs, before any of
//! its code runs. Lume shows what it adds (menu items, status items, settings) from the
//! manifest alone, and starts the plugin's code only when one of the events it names
//! happens. A manifest that Lume reads today must mean the same tomorrow, so it is
//! strict: a key it does not know is an error, not something passed over.
//!
//! The things a plugin adds are named without the plugin's id (`refresh`, not
//! `acme.metrics.refresh`); see [`crate::id`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer};

use crate::id::{Reference, check_name, check_plugin_id};
use crate::permissions::{Permissions, parse_table};
use crate::places;

/// The manifest's file name.
pub const MANIFEST_FILE: &str = "plugin.toml";

/// The version of the manifest's format this build reads.
pub const MANIFEST_SCHEMA: u32 = 1;

/// The major version of the plugin API this build offers.
pub const API_MAJOR: u32 = 1;

/// How much memory a plugin's code has when its manifest does not say, and the least
/// and the most it may ask for, in MiB.
pub const MEMORY_DEFAULT_MIB: u32 = 64;
pub const MEMORY_MIN_MIB: u32 = 16;
pub const MEMORY_MAX_MIB: u32 = 256;

/// The longest a plugin's name may be, and its description.
const NAME_MAX_CHARS: usize = 64;
const DESCRIPTION_MAX_CHARS: usize = 300;

/// The longest a title, label or other short text a plugin shows may be.
const LABEL_MAX_CHARS: usize = 64;

/// The kinds of session event a plugin can be started by.
pub const SESSION_EVENTS: [&str; 11] = [
    "title",
    "cwd",
    "bell",
    "command_started",
    "command_finished",
    "program_status",
    "foreground",
    "activity",
    "disconnected",
    "state",
    "exit",
];

/// A manifest, read and checked.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[schemars(
    title = "Lume plugin manifest",
    description = "plugin.toml at the root of a plugin's package"
)]
pub struct Manifest {
    /// The version of this file's format: 1.
    pub schema: u32,
    /// The plugin's id: dotted lower-case words, as in `acme.metrics`.
    pub id: String,
    /// What the plugin is called.
    pub name: String,
    /// The plugin's version, as semver: `1.2.0`.
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    /// An SVG file in the package.
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    /// Where the plugin's source is. Two packages with the same id and repository are one
    /// plugin at two versions.
    #[serde(default)]
    pub repository: Option<String>,
    /// The major version of the plugin API the plugin is written for, as in `"1"`.
    #[serde(deserialize_with = "api_version")]
    #[schemars(with = "ApiVersion")]
    pub api: u32,
    /// The plugin's code: a JavaScript file in the package. A plugin that only adds
    /// themes, keys or menus of the application's commands needs none.
    #[serde(default)]
    pub main: Option<String>,
    /// The events that start the plugin's code: `onStartup`, `onCommand:<name>`,
    /// `onView:<name>`, `onSessionEvent:<kind>`, `onService:<name>`.
    #[serde(default)]
    pub activation: Vec<String>,
    /// What the plugin may do; see the documentation of permissions.
    #[serde(default)]
    #[schemars(schema_with = "permissions_schema")]
    pub permissions: toml::Table,
    #[serde(default)]
    pub limits: Limits,
    /// Other plugins this one uses, by id, with the versions it works with.
    #[serde(default)]
    pub dependencies: BTreeMap<String, Dependency>,
    #[serde(default)]
    pub load: Load,
    #[serde(default)]
    pub contributes: Contributes,

    /// The permissions, read from the table above.
    #[serde(skip)]
    #[schemars(skip)]
    pub requested: Permissions,
    /// The activation events, read from the list above.
    #[serde(skip)]
    #[schemars(skip)]
    pub events: Vec<Activation>,
}

/// The API version as written: a string of digits, or a number.
#[derive(JsonSchema)]
#[serde(untagged)]
#[allow(dead_code, reason = "describes the field for the schema only")]
enum ApiVersion {
    Text(String),
    Number(u32),
}

fn api_version<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Written {
        Text(String),
        Number(u32),
    }
    match Written::deserialize(deserializer)? {
        Written::Number(number) => Ok(number),
        Written::Text(text) => text.parse().map_err(|_| {
            serde::de::Error::custom(format!("`{text}` is not an API version, such as \"1\""))
        }),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Limits {
    /// The memory the plugin's code may use, in MiB: 16 to 256, 64 if not given.
    #[serde(default)]
    pub memory: Option<u32>,
}

impl Limits {
    pub fn memory_mib(&self) -> u32 {
        self.memory.unwrap_or(MEMORY_DEFAULT_MIB)
    }
}

/// A plugin another depends on: the versions that will do, and whether it can go
/// without.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Dependency {
    /// `"acme.core" = "^1.2"`
    Version(String),
    /// `"acme.core" = { version = "^1.2", optional = true }`
    Table(DependencyTable),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct DependencyTable {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub optional: bool,
}

impl Dependency {
    /// The versions that will do; any if none are named.
    pub fn requirement(&self) -> semver::VersionReq {
        let written = match self {
            Self::Version(version) => Some(version.as_str()),
            Self::Table(table) => table.version.as_deref(),
        };
        written
            .and_then(|written| semver::VersionReq::parse(written).ok())
            .unwrap_or(semver::VersionReq::STAR)
    }

    pub fn optional(&self) -> bool {
        matches!(self, Self::Table(table) if table.optional)
    }

    fn written_requirement(&self) -> Option<&str> {
        match self {
            Self::Version(version) => Some(version),
            Self::Table(table) => table.version.as_deref(),
        }
    }
}

/// Where the plugin goes in the order plugins load in, beyond its dependencies.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Load {
    /// Plugins this one loads before, when they are installed.
    #[serde(default)]
    pub before: Vec<String>,
}

/// An event that starts a plugin's code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Activation {
    Startup,
    /// One of its commands is run.
    Command(String),
    /// One of its status items or panels comes on screen.
    View(String),
    /// A session reports something of this kind.
    SessionEvent(String),
    /// Another plugin calls one of its services.
    Service(String),
}

impl Activation {
    fn parse(written: &str) -> Result<Self, String> {
        if written == "onStartup" {
            return Ok(Self::Startup);
        }
        let (event, name) = written.split_once(':').ok_or_else(|| {
            format!(
                "`{written}` is not an activation event: onStartup, onCommand:<name>, \
                 onView:<name>, onSessionEvent:<kind> or onService:<name>"
            )
        })?;
        match event {
            "onCommand" => check_name(name).map(|()| Self::Command(name.to_owned())),
            "onView" => check_name(name).map(|()| Self::View(name.to_owned())),
            "onService" => check_name(name).map(|()| Self::Service(name.to_owned())),
            "onSessionEvent" if SESSION_EVENTS.contains(&name) => {
                Ok(Self::SessionEvent(name.to_owned()))
            }
            "onSessionEvent" => Err(format!(
                "`{name}` is not a kind of session event; the kinds are {}",
                SESSION_EVENTS.join(", ")
            )),
            _ => Err(format!("`{event}` is not an activation event")),
        }
    }
}

/// Everything the plugin adds.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Contributes {
    #[serde(default)]
    pub commands: Vec<Command>,
    #[serde(default)]
    pub keybindings: Vec<Keybinding>,
    #[serde(default)]
    pub menus: Vec<MenuItem>,
    #[serde(default)]
    pub sections: Vec<Section>,
    #[serde(default)]
    pub status: Vec<StatusItem>,
    #[serde(default)]
    pub settings: Vec<Setting>,
    #[serde(default)]
    pub setting_groups: Vec<SettingGroup>,
    #[serde(default)]
    pub themes: Vec<Theme>,
    #[serde(default)]
    pub panels: Vec<Panel>,
    #[serde(default)]
    pub services: Vec<Service>,
}

/// Where a thing goes among its neighbours. `before` and `after` name a neighbour; the
/// thing goes right next to it. `order` places it where neither is given, or where the
/// neighbour is not there: lower comes first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Placement {
    pub before: Option<String>,
    pub after: Option<String>,
    pub order: Option<i32>,
}

/// Gives each kind of thing that is placed among neighbours its placement.
macro_rules! placed {
    ($($kind:ty),*) => {$(
        impl $kind {
            pub fn placement(&self) -> Placement {
                Placement {
                    before: self.before.clone(),
                    after: self.after.clone(),
                    order: self.order,
                }
            }
        }
    )*};
}

placed!(MenuItem, Section, StatusItem, Setting, SettingGroup, Panel);

/// A command: something the user can run from the palette, a key or a menu, carried out
/// by the plugin's code.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Command {
    pub id: String,
    pub title: String,
    /// The heading the palette lists it under; the plugin's name if not given.
    #[serde(default)]
    pub category: Option<String>,
    /// Whether the palette offers it.
    #[serde(default = "yes")]
    pub palette: bool,
}

fn yes() -> bool {
    true
}

/// A key that runs a command.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Keybinding {
    /// The keys, as in `ctrl+shift+m`, or a sequence of chords.
    pub key: String,
    /// The keys on a Mac, where they differ.
    #[serde(default)]
    pub macos: Option<String>,
    /// The plugin's own command, or one of the application's.
    pub command: String,
    #[serde(default)]
    #[schemars(with = "Option<serde_json::Value>")]
    pub args: Option<toml::Value>,
    /// When the key holds, as in the configuration's keybindings.
    #[serde(default)]
    pub when: Option<String>,
}

/// An item of a menu, or a submenu.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct MenuItem {
    pub id: String,
    /// The menu it is in: one of the application's menus.
    pub location: String,
    /// The section of that menu it is in: one of the application's (`lume:<name>`), one
    /// of the plugin's own, or one of a plugin it depends on.
    #[serde(default)]
    pub section: Option<String>,
    /// The submenu it is in, instead of the menu itself.
    #[serde(default)]
    pub parent: Option<String>,
    /// The neighbour it goes right before.
    #[serde(default)]
    pub before: Option<String>,
    /// The neighbour it goes right after.
    #[serde(default)]
    pub after: Option<String>,
    /// Where it goes where no neighbour is named, or the one named is not there: lower
    /// comes first.
    #[serde(default)]
    pub order: Option<i32>,
    /// The command it runs. A submenu runs none.
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    #[schemars(with = "Option<serde_json::Value>")]
    pub args: Option<toml::Value>,
    /// What it is called; the command's title if not given. An item that runs one of the
    /// application's commands is always called what the command is.
    #[serde(default)]
    pub title: Option<String>,
    /// Whether it opens a menu of its own.
    #[serde(default)]
    pub submenu: bool,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub when: Option<String>,
}

/// A section of a menu: items a line sets apart, with a heading if it has a title.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Section {
    pub id: String,
    pub location: String,
    #[serde(default)]
    pub title: Option<String>,
    /// The neighbour it goes right before.
    #[serde(default)]
    pub before: Option<String>,
    /// The neighbour it goes right after.
    #[serde(default)]
    pub after: Option<String>,
    /// Where it goes where no neighbour is named, or the one named is not there: lower
    /// comes first.
    #[serde(default)]
    pub order: Option<i32>,
}

/// A status item: a small piece of information, or a button, in a bar.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct StatusItem {
    pub id: String,
    /// `topbar.left`, `topbar.center`, `topbar.right` or `pane.header`.
    pub slot: String,
    /// `text`, `icon`, `meter`, `sparkline` or `badge`.
    pub kind: String,
    /// What the item stands for, read out and shown until the plugin says more.
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    /// The neighbour it goes right before.
    #[serde(default)]
    pub before: Option<String>,
    /// The neighbour it goes right after.
    #[serde(default)]
    pub after: Option<String>,
    /// Where it goes where no neighbour is named, or the one named is not there: lower
    /// comes first.
    #[serde(default)]
    pub order: Option<i32>,
    /// A command the item runs when clicked, which makes it a button.
    #[serde(default)]
    pub command: Option<String>,
    /// A submenu of the plugin's own the item opens when clicked.
    #[serde(default)]
    pub menu: Option<String>,
    #[serde(default)]
    pub tooltip: Option<String>,
    #[serde(default)]
    pub when: Option<String>,
}

/// One of the plugin's settings, under `[plugins."<id>"]` in the configuration.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Setting {
    /// The setting's name inside the plugin's table.
    pub key: String,
    /// `boolean`, `integer`, `number` or `string`.
    #[serde(rename = "type")]
    pub kind: String,
    #[schemars(with = "serde_json::Value")]
    pub default: toml::Value,
    #[serde(default)]
    pub minimum: Option<f64>,
    #[serde(default)]
    pub maximum: Option<f64>,
    /// The values it may have, for a setting that is one of a few.
    #[serde(default)]
    pub choices: Vec<Choice>,
    /// What the settings window calls it. A setting without one is only in the file.
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub help: Option<String>,
    /// `switch`, `select`, `segmented`, `text` or `number`; from the type if not given.
    #[serde(default)]
    pub control: Option<String>,
    /// `self` for the plugin's own page, or the id of one of the application's pages.
    #[serde(default)]
    pub page: Option<String>,
    /// The group of that page it is in: one of the plugin's own, or `lume:<group>`.
    #[serde(default)]
    pub group: Option<String>,
    /// The neighbour it goes right before.
    #[serde(default)]
    pub before: Option<String>,
    /// The neighbour it goes right after.
    #[serde(default)]
    pub after: Option<String>,
    /// Where it goes where no neighbour is named, or the one named is not there: lower
    /// comes first.
    #[serde(default)]
    pub order: Option<i32>,
    /// What the value is counted in, as `px` or `s`.
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub step: Option<f64>,
    /// Another of the plugin's settings that must be on for this one to be shown.
    #[serde(default)]
    pub shown_when: Option<String>,
}

/// One of the values a setting may have, and what it is called.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Choice {
    #[schemars(with = "serde_json::Value")]
    pub value: toml::Value,
    pub label: String,
}

/// A group of settings on a page, with a heading.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct SettingGroup {
    pub id: String,
    pub title: String,
    /// `self`, or the id of one of the application's pages.
    #[serde(default)]
    pub page: Option<String>,
    /// The neighbour it goes right before.
    #[serde(default)]
    pub before: Option<String>,
    /// The neighbour it goes right after.
    #[serde(default)]
    pub after: Option<String>,
    /// Where it goes where no neighbour is named, or the one named is not there: lower
    /// comes first.
    #[serde(default)]
    pub order: Option<i32>,
}

/// A theme file in the package.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Theme {
    pub id: String,
    pub file: String,
}

/// A panel in a dock area of the window, drawn by Lume from rows the plugin gives.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Panel {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub icon: Option<String>,
    /// How it is drawn: `list`.
    pub kind: String,
    /// The dock area it opens in the first time: `left`, `right` or `bottom`.
    pub area: String,
    /// The neighbour it goes right before.
    #[serde(default)]
    pub before: Option<String>,
    /// The neighbour it goes right after.
    #[serde(default)]
    pub after: Option<String>,
    /// Where it goes where no neighbour is named, or the one named is not there: lower
    /// comes first.
    #[serde(default)]
    pub order: Option<i32>,
}

/// A function other plugins may call.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Service {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// Something wrong with a manifest, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// The line and column, counted from 1, where the reader knows them.
    pub at: Option<(usize, usize)>,
    /// What is wrong, beginning with the key it is about where there is one.
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.at {
            Some((line, column)) => write!(f, "line {line}, column {column}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// The line and column of a byte offset in a text.
fn position(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    (line, column)
}

impl Manifest {
    /// Reads a manifest's text and checks everything that can be checked without the
    /// rest of the package.
    pub fn parse(text: &str) -> Result<Self, Vec<Problem>> {
        let mut manifest: Self = toml::from_str(text).map_err(|error| {
            let message = error.message().trim().to_owned();
            vec![Problem {
                at: error.span().map(|span| position(text, span.start)),
                message,
            }]
        })?;
        let problems = manifest.check();
        if problems.is_empty() {
            Ok(manifest)
        } else {
            Err(problems
                .into_iter()
                .map(|message| Problem { at: None, message })
                .collect())
        }
    }

    /// The version, read.
    pub fn version(&self) -> semver::Version {
        semver::Version::parse(&self.version).unwrap_or_else(|_| semver::Version::new(0, 0, 0))
    }

    /// Checks what serde does not, and fills in what is read from the raw tables.
    fn check(&mut self) -> Vec<String> {
        let mut problems = Checker::default();
        let p = &mut problems;

        if self.schema != MANIFEST_SCHEMA {
            p.say(format!(
                "schema: this Lume reads manifests of schema {MANIFEST_SCHEMA}, not {}",
                self.schema
            ));
        }
        if let Err(problem) = check_plugin_id(&self.id) {
            p.say(format!("id: {problem}"));
        }
        p.text("name", &self.name, NAME_MAX_CHARS, true);
        if let Some(description) = &self.description {
            p.text("description", description, DESCRIPTION_MAX_CHARS, false);
        }
        if let Err(error) = semver::Version::parse(&self.version) {
            p.say(format!(
                "version: `{}` is not a version such as 1.2.0: {error}",
                self.version
            ));
        }
        if self.api != API_MAJOR {
            p.say(format!(
                "api: the plugin is written for version {} of the plugin API; this Lume \
                 offers version {API_MAJOR}",
                self.api
            ));
        }
        if let Some(icon) = &self.icon {
            p.file("icon", icon, &["svg"]);
        }
        if let Some(main) = &self.main {
            p.file("main", main, &["js"]);
        }
        if let Some(memory) = self.limits.memory
            && !(MEMORY_MIN_MIB..=MEMORY_MAX_MIB).contains(&memory)
        {
            p.say(format!(
                "limits.memory: {memory} MiB is outside {MEMORY_MIN_MIB} to {MEMORY_MAX_MIB}"
            ));
        }
        match parse_table(&self.permissions) {
            Ok(permissions) => self.requested = permissions,
            Err(found) => p.0.extend(found),
        }
        for (id, dependency) in &self.dependencies {
            if let Err(problem) = check_plugin_id(id) {
                p.say(format!("dependencies.\"{id}\": {problem}"));
            } else if *id == self.id {
                p.say(format!(
                    "dependencies.\"{id}\": a plugin cannot depend on itself"
                ));
            }
            if let Some(written) = dependency.written_requirement()
                && let Err(error) = semver::VersionReq::parse(written)
            {
                p.say(format!(
                    "dependencies.\"{id}\": `{written}` is not a version requirement such \
                     as ^1.2: {error}"
                ));
            }
        }
        for id in &self.load.before {
            if let Err(problem) = check_plugin_id(id) {
                p.say(format!("load.before: {problem}"));
            } else if *id == self.id {
                p.say("load.before: a plugin cannot load before itself".to_owned());
            }
        }

        let dependencies: BTreeSet<&str> = self.dependencies.keys().map(String::as_str).collect();
        let contributes = &self.contributes;
        let own = Own::of(contributes);
        contributes.check(p, &own, &dependencies);

        self.events.clear();
        for written in &self.activation {
            match Activation::parse(written) {
                Ok(event) => {
                    let missing = match &event {
                        Activation::Command(name) if !own.commands.contains(name.as_str()) => {
                            Some("command")
                        }
                        Activation::View(name)
                            if !own.status.contains(name.as_str())
                                && !own.panels.contains(name.as_str()) =>
                        {
                            Some("status item or panel")
                        }
                        Activation::Service(name) if !own.services.contains(name.as_str()) => {
                            Some("service")
                        }
                        _ => None,
                    };
                    if let Some(what) = missing {
                        p.say(format!(
                            "activation: `{written}` names no {what} of the plugin's own"
                        ));
                    }
                    self.events.push(event);
                }
                Err(problem) => p.say(format!("activation: {problem}")),
            }
        }
        let needs_code = !contributes.commands.is_empty()
            || !contributes.services.is_empty()
            || !contributes.panels.is_empty()
            || !self.events.is_empty();
        if needs_code && self.main.is_none() {
            p.say(
                "main: a plugin with commands, services, panels or activation events needs \
                 its code, named by main"
                    .to_owned(),
            );
        }
        problems.0
    }
}

/// The problems found so far.
#[derive(Default)]
struct Checker(Vec<String>);

impl Checker {
    fn say(&mut self, problem: String) {
        self.0.push(problem);
    }

    /// Checks a short text the plugin shows.
    fn text(&mut self, key: &str, text: &str, max: usize, needed: bool) {
        if needed && text.trim().is_empty() {
            self.say(format!("{key}: must not be empty"));
        } else if text.chars().count() > max {
            self.say(format!("{key}: {max} characters at most"));
        } else if text.chars().any(char::is_control) {
            self.say(format!(
                "{key}: must be one line, without control characters"
            ));
        }
    }

    /// Checks a path to a file in the package: relative, with `/`, not going up, and of
    /// one of the kinds named.
    fn file(&mut self, key: &str, path: &str, kinds: &[&str]) {
        if let Err(problem) = check_package_path(path) {
            self.say(format!("{key}: {problem}"));
        } else if !kinds
            .iter()
            .any(|kind| path.to_ascii_lowercase().ends_with(&format!(".{kind}")))
        {
            self.say(format!(
                "{key}: `{path}` must be a .{} file",
                kinds.join(" or .")
            ));
        }
    }

    /// Checks a name the plugin gives one of its own things, and that it is not given
    /// twice.
    fn name(&mut self, key: &str, name: &str, seen: &mut BTreeSet<String>) {
        if let Err(problem) = check_name(name) {
            self.say(format!("{key}: {problem}"));
        } else if !seen.insert(name.to_owned()) {
            self.say(format!("{key}: `{name}` is given twice"));
        }
    }

    /// Checks a reference to something, of the plugin's own (among `own`), of the
    /// application's (when `builtin` allows it), or of a plugin it depends on.
    fn reference(
        &mut self,
        key: &str,
        written: &str,
        own: &BTreeSet<&str>,
        builtin: Builtin<'_>,
        dependencies: &BTreeSet<&str>,
    ) {
        match Reference::parse(written) {
            Err(problem) => self.say(format!("{key}: {problem}")),
            Ok(Reference::Own(name)) => {
                if !own.contains(name.as_str()) {
                    self.say(format!("{key}: `{name}` is nothing the plugin declares"));
                }
            }
            Ok(Reference::Builtin(name)) => match builtin {
                Builtin::None => {
                    self.say(format!("{key}: `{written}` cannot be the application's"));
                }
                Builtin::Commands if !written.starts_with("lume.") => self.say(format!(
                    "{key}: `{written}`: one of the application's commands is named by its \
                     id, as in lume.tab.new"
                )),
                Builtin::Keys(keys) if !keys.contains(&name.as_str()) => self.say(format!(
                    "{key}: `{written}` is not one of the application's here; they are {}",
                    keys.iter()
                        .map(|key| format!("lume:{key}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                Builtin::Places if written.starts_with("lume.") => self.say(format!(
                    "{key}: one of the application's places is named as lume:<name>, not \
                     `{written}`"
                )),
                _ => {}
            },
            Ok(Reference::Other { plugin, .. }) => {
                if !dependencies.contains(plugin.as_str()) {
                    self.say(format!(
                        "{key}: `{written}` belongs to {plugin}, which is not among the \
                         plugin's dependencies"
                    ));
                }
            }
        }
    }

    fn placement(
        &mut self,
        key: &str,
        placement: &Placement,
        own: &BTreeSet<&str>,
        builtin: Builtin<'_>,
        dependencies: &BTreeSet<&str>,
    ) {
        if placement.before.is_some() && placement.after.is_some() {
            self.say(format!("{key}: give before or after, not both"));
        }
        for (side, anchor) in [("before", &placement.before), ("after", &placement.after)] {
            if let Some(anchor) = anchor {
                self.reference(&format!("{key}.{side}"), anchor, own, builtin, dependencies);
            }
        }
    }

    fn when(&mut self, key: &str, when: Option<&String>) {
        if let Some(when) = when
            && let Err(problem) = check_when_syntax(when)
        {
            self.say(format!("{key}.when: {problem}"));
        }
    }
}

/// What of the application's a reference may name.
#[derive(Clone, Copy)]
enum Builtin<'a> {
    None,
    /// Its commands, by id.
    Commands,
    /// Its places in menus and bars, as `lume:<key>`; not checked further here, since
    /// the places are the application's to know.
    Places,
    /// One of these keys.
    Keys(&'a [&'a str]),
}

/// The names of the things the plugin declares, by kind.
#[derive(Default)]
struct Own<'a> {
    commands: BTreeSet<&'a str>,
    menus: BTreeSet<&'a str>,
    submenus: BTreeSet<&'a str>,
    sections: BTreeSet<&'a str>,
    status: BTreeSet<&'a str>,
    settings: BTreeSet<&'a str>,
    groups: BTreeSet<&'a str>,
    panels: BTreeSet<&'a str>,
    services: BTreeSet<&'a str>,
}

impl<'a> Own<'a> {
    fn of(contributes: &'a Contributes) -> Self {
        Self {
            commands: contributes.commands.iter().map(|c| c.id.as_str()).collect(),
            menus: contributes.menus.iter().map(|m| m.id.as_str()).collect(),
            submenus: contributes
                .menus
                .iter()
                .filter(|m| m.submenu)
                .map(|m| m.id.as_str())
                .collect(),
            sections: contributes.sections.iter().map(|s| s.id.as_str()).collect(),
            status: contributes.status.iter().map(|s| s.id.as_str()).collect(),
            settings: contributes
                .settings
                .iter()
                .map(|s| s.key.as_str())
                .collect(),
            groups: contributes
                .setting_groups
                .iter()
                .map(|g| g.id.as_str())
                .collect(),
            panels: contributes.panels.iter().map(|p| p.id.as_str()).collect(),
            services: contributes
                .services
                .iter()
                .map(|s| s.name.as_str())
                .collect(),
        }
    }
}

/// The kinds a setting may be.
const SETTING_KINDS: [&str; 4] = ["boolean", "integer", "number", "string"];

/// The controls a setting may be shown with.
const CONTROLS: [&str; 5] = ["switch", "select", "segmented", "text", "number"];

/// The page that is the plugin's own.
pub const OWN_PAGE: &str = "self";

/// The keys of the application's settings groups a plugin's settings can be put in: the
/// groups' headings, in lower case, with hyphens for spaces.
pub fn builtin_groups(page: &str) -> Vec<String> {
    lume_config::settings::pages()
        .into_iter()
        .filter(|candidate| candidate.id == page)
        .flat_map(|page| page.groups.iter())
        .map(|group| group.to_ascii_lowercase().replace(' ', "-"))
        .collect()
}

fn check_page(page: Option<&String>) -> Result<(), String> {
    match page.map(String::as_str) {
        None | Some(OWN_PAGE) => Ok(()),
        Some(page) if lume_config::settings::pages().iter().any(|p| p.id == page) => Ok(()),
        Some(page) => Err(format!(
            "`{page}` is not a page: `self` for the plugin's own, or one of {}",
            lume_config::settings::pages()
                .iter()
                .map(|p| p.id)
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// What kind of TOML value a setting's kind is.
fn of_kind(kind: &str, value: &toml::Value) -> bool {
    match kind {
        "boolean" => value.is_bool(),
        "integer" => value.is_integer(),
        "number" => value.is_integer() || value.is_float(),
        "string" => value.is_str(),
        _ => false,
    }
}

fn number(value: &toml::Value) -> Option<f64> {
    #[allow(
        clippy::cast_precision_loss,
        reason = "a setting's bounds are small numbers"
    )]
    value
        .as_float()
        .or_else(|| value.as_integer().map(|integer| integer as f64))
}

impl Contributes {
    fn check(&self, p: &mut Checker, own: &Own<'_>, dependencies: &BTreeSet<&str>) {
        let mut seen = BTreeSet::new();
        for (index, command) in self.commands.iter().enumerate() {
            let key = format!("contributes.commands[{index}]");
            p.name(&format!("{key}.id"), &command.id, &mut seen);
            p.text(
                &format!("{key}.title"),
                &command.title,
                LABEL_MAX_CHARS,
                true,
            );
            if let Some(category) = &command.category {
                p.text(&format!("{key}.category"), category, LABEL_MAX_CHARS, true);
            }
        }

        for (index, binding) in self.keybindings.iter().enumerate() {
            let key = format!("contributes.keybindings[{index}]");
            for keys in std::iter::once(&binding.key).chain(binding.macos.as_ref()) {
                if let Err(problem) = lume_config::KeySequence::parse(keys) {
                    p.say(format!("{key}: `{keys}`: {problem}"));
                }
            }
            p.reference(
                &format!("{key}.command"),
                &binding.command,
                &own.commands,
                Builtin::Commands,
                dependencies,
            );
            p.when(&key, binding.when.as_ref());
        }

        let mut seen = BTreeSet::new();
        for (index, section) in self.sections.iter().enumerate() {
            let key = format!("contributes.sections[{index}]");
            p.name(&format!("{key}.id"), &section.id, &mut seen);
            if !places::MENUS.contains(&section.location.as_str()) {
                p.say(format!(
                    "{key}.location: `{}` is not a menu; the menus are {}",
                    section.location,
                    places::MENUS.join(", ")
                ));
            }
            if let Some(title) = &section.title {
                p.text(&format!("{key}.title"), title, LABEL_MAX_CHARS, true);
            }
            p.placement(
                &key,
                &section.placement(),
                &own.sections,
                Builtin::Places,
                dependencies,
            );
        }

        let mut seen = BTreeSet::new();
        for (index, item) in self.menus.iter().enumerate() {
            let key = format!("contributes.menus[{index}]");
            p.name(&format!("{key}.id"), &item.id, &mut seen);
            if !places::MENUS.contains(&item.location.as_str()) {
                p.say(format!(
                    "{key}.location: `{}` is not a menu; the menus are {}",
                    item.location,
                    places::MENUS.join(", ")
                ));
            }
            match (&item.command, item.submenu) {
                (Some(_), true) => p.say(format!("{key}: a submenu runs no command")),
                (None, false) => p.say(format!("{key}: needs a command, or submenu = true")),
                (Some(command), false) => p.reference(
                    &format!("{key}.command"),
                    command,
                    &own.commands,
                    Builtin::Commands,
                    dependencies,
                ),
                (None, true) => {
                    if item.title.is_none() {
                        p.say(format!("{key}: a submenu needs a title"));
                    }
                }
            }
            if let Some(title) = &item.title {
                p.text(&format!("{key}.title"), title, LABEL_MAX_CHARS, true);
            }
            if let Some(section) = &item.section {
                p.reference(
                    &format!("{key}.section"),
                    section,
                    &own.sections,
                    Builtin::Places,
                    dependencies,
                );
            }
            if let Some(parent) = &item.parent {
                if item.section.is_some() {
                    p.say(format!("{key}: give a section or a parent, not both"));
                }
                p.reference(
                    &format!("{key}.parent"),
                    parent,
                    &own.submenus,
                    Builtin::Places,
                    dependencies,
                );
                if Reference::parse(parent) == Ok(Reference::Own(item.id.clone())) {
                    p.say(format!("{key}.parent: an item cannot be inside itself"));
                }
            }
            let mut neighbours = own.menus.clone();
            neighbours.extend(own.sections.iter());
            p.placement(
                &key,
                &item.placement(),
                &neighbours,
                Builtin::Places,
                dependencies,
            );
            if let Some(icon) = &item.icon {
                check_icon(p, &format!("{key}.icon"), icon);
            }
            p.when(&key, item.when.as_ref());
        }

        let mut seen = BTreeSet::new();
        for (index, item) in self.status.iter().enumerate() {
            let key = format!("contributes.status[{index}]");
            p.name(&format!("{key}.id"), &item.id, &mut seen);
            if !places::STATUS_SLOTS.contains(&item.slot.as_str()) {
                p.say(format!(
                    "{key}.slot: `{}` is not a place for status items; they are {}",
                    item.slot,
                    places::STATUS_SLOTS.join(", ")
                ));
            }
            if !places::STATUS_KINDS.contains(&item.kind.as_str()) {
                p.say(format!(
                    "{key}.kind: `{}` is not a kind of status item; they are {}",
                    item.kind,
                    places::STATUS_KINDS.join(", ")
                ));
            }
            if let Some(label) = &item.label {
                p.text(&format!("{key}.label"), label, LABEL_MAX_CHARS, true);
            }
            if let Some(tooltip) = &item.tooltip {
                p.text(
                    &format!("{key}.tooltip"),
                    tooltip,
                    DESCRIPTION_MAX_CHARS,
                    true,
                );
            }
            if item.command.is_some() && item.menu.is_some() {
                p.say(format!("{key}: give a command or a menu, not both"));
            }
            if let Some(command) = &item.command {
                p.reference(
                    &format!("{key}.command"),
                    command,
                    &own.commands,
                    Builtin::Commands,
                    dependencies,
                );
            }
            if let Some(menu) = &item.menu {
                p.reference(
                    &format!("{key}.menu"),
                    menu,
                    &own.submenus,
                    Builtin::None,
                    dependencies,
                );
            }
            if let Some(icon) = &item.icon {
                check_icon(p, &format!("{key}.icon"), icon);
            }
            p.placement(
                &key,
                &item.placement(),
                &own.status,
                Builtin::Places,
                dependencies,
            );
            p.when(&key, item.when.as_ref());
        }

        let mut seen = BTreeSet::new();
        for (index, group) in self.setting_groups.iter().enumerate() {
            let key = format!("contributes.setting-groups[{index}]");
            p.name(&format!("{key}.id"), &group.id, &mut seen);
            p.text(&format!("{key}.title"), &group.title, LABEL_MAX_CHARS, true);
            if let Err(problem) = check_page(group.page.as_ref()) {
                p.say(format!("{key}.page: {problem}"));
            }
            let page = group.page.as_deref().unwrap_or(OWN_PAGE);
            let builtin = builtin_groups(page);
            let keys: Vec<&str> = builtin.iter().map(String::as_str).collect();
            p.placement(
                &key,
                &group.placement(),
                &own.groups,
                Builtin::Keys(&keys),
                dependencies,
            );
        }

        let mut seen = BTreeSet::new();
        for (index, setting) in self.settings.iter().enumerate() {
            let key = format!("contributes.settings[{index}]");
            p.name(&format!("{key}.key"), &setting.key, &mut seen);
            if !SETTING_KINDS.contains(&setting.kind.as_str()) {
                p.say(format!(
                    "{key}.type: `{}` is not a type of setting; they are {}",
                    setting.kind,
                    SETTING_KINDS.join(", ")
                ));
                continue;
            }
            if !of_kind(&setting.kind, &setting.default) {
                p.say(format!("{key}.default: is not a {}", setting.kind));
            }
            let numeric = matches!(setting.kind.as_str(), "integer" | "number");
            if !numeric && (setting.minimum.is_some() || setting.maximum.is_some()) {
                p.say(format!("{key}: only a number has a minimum and a maximum"));
            }
            if let (Some(minimum), Some(maximum)) = (setting.minimum, setting.maximum)
                && minimum > maximum
            {
                p.say(format!("{key}: the minimum is above the maximum"));
            }
            if let Some(value) = number(&setting.default)
                && numeric
                && (setting.minimum.is_some_and(|minimum| value < minimum)
                    || setting.maximum.is_some_and(|maximum| value > maximum))
            {
                p.say(format!(
                    "{key}.default: is outside the minimum and the maximum"
                ));
            }
            for (choice_index, choice) in setting.choices.iter().enumerate() {
                if !of_kind(&setting.kind, &choice.value) {
                    p.say(format!(
                        "{key}.choices[{choice_index}].value: is not a {}",
                        setting.kind
                    ));
                }
                p.text(
                    &format!("{key}.choices[{choice_index}].label"),
                    &choice.label,
                    LABEL_MAX_CHARS,
                    true,
                );
            }
            if !setting.choices.is_empty()
                && !setting
                    .choices
                    .iter()
                    .any(|choice| choice.value == setting.default)
            {
                p.say(format!("{key}.default: is not one of the choices"));
            }
            if let Some(control) = &setting.control {
                let fits = match control.as_str() {
                    "switch" => setting.kind == "boolean",
                    "select" | "segmented" => !setting.choices.is_empty(),
                    "number" => numeric,
                    "text" => setting.kind == "string",
                    _ => {
                        p.say(format!(
                            "{key}.control: `{control}` is not a control; they are {}",
                            CONTROLS.join(", ")
                        ));
                        true
                    }
                };
                if !fits {
                    p.say(format!(
                        "{key}.control: a {control} cannot set a {} setting{}",
                        setting.kind,
                        if setting.choices.is_empty() {
                            " without choices"
                        } else {
                            ""
                        }
                    ));
                }
            }
            if let Some(label) = &setting.label {
                p.text(&format!("{key}.label"), label, LABEL_MAX_CHARS, true);
            }
            if let Some(help) = &setting.help {
                p.text(&format!("{key}.help"), help, DESCRIPTION_MAX_CHARS, true);
            }
            if let Err(problem) = check_page(setting.page.as_ref()) {
                p.say(format!("{key}.page: {problem}"));
            }
            let page = setting.page.as_deref().unwrap_or(OWN_PAGE);
            if page != OWN_PAGE && setting.group.is_none() {
                p.say(format!(
                    "{key}.group: a setting on one of the application's pages names the group \
                     it is in"
                ));
            }
            if let Some(group) = &setting.group {
                let builtin = builtin_groups(page);
                let keys: Vec<&str> = builtin.iter().map(String::as_str).collect();
                p.reference(
                    &format!("{key}.group"),
                    group,
                    &own.groups,
                    Builtin::Keys(&keys),
                    dependencies,
                );
            }
            p.placement(
                &key,
                &setting.placement(),
                &own.settings,
                Builtin::None,
                dependencies,
            );
            if let Some(other) = &setting.shown_when {
                let switch = self
                    .settings
                    .iter()
                    .any(|candidate| candidate.key == *other && candidate.kind == "boolean");
                if !switch {
                    p.say(format!(
                        "{key}.shown-when: `{other}` is not one of the plugin's boolean settings"
                    ));
                }
            }
        }

        let mut seen = BTreeSet::new();
        for (index, theme) in self.themes.iter().enumerate() {
            let key = format!("contributes.themes[{index}]");
            p.name(&format!("{key}.id"), &theme.id, &mut seen);
            p.file(&format!("{key}.file"), &theme.file, &["toml"]);
        }

        let mut seen = BTreeSet::new();
        for (index, panel) in self.panels.iter().enumerate() {
            let key = format!("contributes.panels[{index}]");
            p.name(&format!("{key}.id"), &panel.id, &mut seen);
            p.text(&format!("{key}.title"), &panel.title, LABEL_MAX_CHARS, true);
            if !places::PANEL_KINDS.contains(&panel.kind.as_str()) {
                p.say(format!(
                    "{key}.kind: `{}` is not a kind of panel; they are {}",
                    panel.kind,
                    places::PANEL_KINDS.join(", ")
                ));
            }
            if !places::DOCK_AREAS.contains(&panel.area.as_str()) {
                p.say(format!(
                    "{key}.area: `{}` is not a dock area; they are {}",
                    panel.area,
                    places::DOCK_AREAS.join(", ")
                ));
            }
            if let Some(icon) = &panel.icon {
                check_icon(p, &format!("{key}.icon"), icon);
            }
            p.placement(
                &key,
                &panel.placement(),
                &own.panels,
                Builtin::Places,
                dependencies,
            );
        }

        let mut seen = BTreeSet::new();
        for (index, service) in self.services.iter().enumerate() {
            let key = format!("contributes.services[{index}]");
            p.name(&format!("{key}.name"), &service.name, &mut seen);
            if let Some(description) = &service.description {
                p.text(
                    &format!("{key}.description"),
                    description,
                    DESCRIPTION_MAX_CHARS,
                    true,
                );
            }
        }
    }
}

/// Checks an icon: a name of one of the application's pictures, or an SVG file in the
/// package.
fn check_icon(p: &mut Checker, key: &str, icon: &str) {
    if icon.contains('/') || icon.to_ascii_lowercase().ends_with(".svg") {
        p.file(key, icon, &["svg"]);
    } else if check_name(icon).is_err() {
        p.say(format!(
            "{key}: `{icon}` is neither the name of a picture nor an SVG file in the package"
        ));
    }
}

/// Checks the shape of a `when` condition: terms joined by `&&`, each perhaps with `!`.
/// What the terms may be is the application's to say when it reads them.
pub fn check_when_syntax(when: &str) -> Result<(), String> {
    for term in when.split("&&") {
        let term = term.trim();
        let name = term.strip_prefix('!').unwrap_or(term).trim();
        let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_');
        if name.is_empty() || !name.chars().all(allowed) {
            return Err(format!(
                "`{when}` is not a condition: names joined by &&, each perhaps after !"
            ));
        }
    }
    Ok(())
}

/// Checks a path to a file in a package: relative, written with `/`, not going up and
/// not into a hidden part.
pub fn check_package_path(path: &str) -> Result<(), String> {
    let bad = path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..");
    if bad {
        Err(format!(
            "`{path}` is not a path in the package: relative, with `/` between folders, \
             never `..`"
        ))
    } else {
        Ok(())
    }
}

/// The schema of `[permissions]`: each kind, granted as `true`, as the list it covers,
/// or as a table with a scope and whether to ask each time; and the kinds that may be
/// asked for later.
fn permissions_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    use crate::permissions::{Kind, ScopeKind};
    let mut kinds = serde_json::Map::new();
    let mut later = serde_json::Map::new();
    for kind in Kind::ALL {
        let scope = serde_json::json!({ "type": "array", "items": { "type": "string" } });
        let prompt = serde_json::json!({ "enum": ["once", "each"] });
        let granted = match kind.scope() {
            ScopeKind::None => serde_json::json!({ "anyOf": [
                { "const": true },
                { "type": "object", "properties": { "prompt": prompt }, "additionalProperties": false }
            ]}),
            _ => serde_json::json!({ "anyOf": [
                scope.clone(),
                { "type": "object", "properties": { "scope": scope, "prompt": prompt },
                  "required": ["scope"], "additionalProperties": false }
            ]}),
        };
        kinds.insert(kind.as_str().to_owned(), granted);
        let may = match kind.scope() {
            ScopeKind::None => serde_json::json!({ "const": true }),
            _ => serde_json::json!({ "anyOf": [{ "const": true }, scope] }),
        };
        later.insert(kind.as_str().to_owned(), may);
    }
    kinds.insert(
        "optional".to_owned(),
        serde_json::json!({
            "description": "Kinds the plugin may ask for while it runs",
            "type": "object",
            "properties": later,
            "additionalProperties": false
        }),
    );
    schemars::json_schema!({
        "description": "What the plugin may do",
        "type": "object",
        "properties": kinds,
        "additionalProperties": false
    })
}

/// The files made from this crate's definitions and kept in the repository: the JSON
/// Schema of manifests.
pub fn generated_files() -> Vec<lume_config::generate::Generated> {
    vec![lume_config::generate::Generated {
        kind: lume_config::generate::GeneratedKind::Schema,
        path: "schemas/plugin.schema.json",
        text: lume_config::generate::pretty_json(&manifest_schema()),
    }]
}

/// The JSON Schema of manifests, for an editor's completion.
pub fn manifest_schema() -> serde_json::Value {
    let settings = schemars::generate::SchemaSettings::draft2020_12();
    let generator = settings.into_generator();
    let schema = generator.into_root_schema_for::<Manifest>();
    serde_json::to_value(schema).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
schema = 1
id = "acme.metrics"
name = "Metrics"
version = "0.1.0"
api = "1"
"#;

    fn problems(text: &str) -> Vec<String> {
        match Manifest::parse(text) {
            Ok(_) => Vec::new(),
            Err(problems) => problems.iter().map(ToString::to_string).collect(),
        }
    }

    fn says(text: &str, what: &str) {
        let found = problems(text);
        assert!(
            found.iter().any(|problem| problem.contains(what)),
            "expected a problem saying {what:?}, got {found:#?}"
        );
    }

    #[test]
    fn a_minimal_manifest_is_read() {
        let manifest = Manifest::parse(MINIMAL).unwrap();
        assert_eq!(manifest.id, "acme.metrics");
        assert_eq!(manifest.api, 1);
        assert_eq!(manifest.version(), semver::Version::new(0, 1, 0));
        assert_eq!(manifest.limits.memory_mib(), MEMORY_DEFAULT_MIB);
    }

    #[test]
    fn a_full_manifest_is_read() {
        let text = format!(
            r#"{MINIMAL}
main = "dist/host.js"
icon = "icon.svg"
activation = ["onView:cpu", "onCommand:refresh", "onSessionEvent:command_finished"]

[permissions]
"system.metrics" = true

[limits]
memory = 32

[dependencies]
"acme.core" = "^1.2"
"acme.extra" = {{ version = "1", optional = true }}

[load]
before = ["acme.later"]

[[contributes.commands]]
id = "refresh"
title = "Refresh meters"

[[contributes.keybindings]]
key = "ctrl+alt+m"
command = "refresh"

[[contributes.sections]]
id = "meters"
location = "menu.app.view"
title = "Meters"
after = "lume:zoom"

[[contributes.menus]]
id = "tools"
location = "menu.app"
title = "Tools"
submenu = true
after = "lume:view"

[[contributes.menus]]
id = "refresh-item"
location = "menu.app.view"
section = "meters"
command = "refresh"

[[contributes.menus]]
id = "new-tab"
location = "menu.app"
parent = "tools"
command = "lume.tab.new"

[[contributes.menus]]
id = "core-open"
location = "menu.app"
parent = "tools"
command = "acme.core:open"

[[contributes.status]]
id = "cpu"
slot = "topbar.right"
kind = "meter"
label = "CPU"
menu = "tools"

[[contributes.setting-groups]]
id = "sampling"
title = "Sampling"

[[contributes.settings]]
key = "show-cpu"
type = "boolean"
default = true
label = "CPU"

[[contributes.settings]]
key = "rate"
type = "integer"
default = 2
choices = [{{ value = 1, label = "1 s" }}, {{ value = 2, label = "2 s" }}]
control = "select"
group = "sampling"
shown-when = "show-cpu"

[[contributes.settings]]
key = "cursor-glow"
type = "boolean"
default = false
page = "terminal"
group = "lume:cursor"
label = "Glow"

[[contributes.themes]]
id = "night"
file = "themes/night.toml"

[[contributes.panels]]
id = "processes"
title = "Processes"
kind = "list"
area = "right"

[[contributes.services]]
name = "latest"
"#
        );
        let manifest = Manifest::parse(&text).unwrap_or_else(|problems| {
            panic!("{problems:#?}");
        });
        assert_eq!(manifest.events.len(), 3);
        assert_eq!(manifest.requested.required.len(), 1);
        assert!(manifest.dependencies["acme.extra"].optional());
        assert!(!manifest.dependencies["acme.core"].optional());
        assert_eq!(manifest.limits.memory_mib(), 32);
    }

    #[test]
    fn an_unknown_key_is_an_error_with_its_place() {
        let found = problems(&format!("{MINIMAL}colour = \"red\"\n"));
        assert_eq!(found.len(), 1);
        assert!(found[0].starts_with("line 7, column 1:"), "{found:?}");
        assert!(found[0].contains("colour"), "{found:?}");
        says(
            &format!(
                "{MINIMAL}[[contributes.commands]]\nid = \"x\"\ntitle = \"X\"\nicon = \"y\"\n"
            ),
            "icon",
        );
        says(&format!("{MINIMAL}[contributes.panes]\n"), "panes");
    }

    #[test]
    fn what_is_wrong_is_said_for_every_part() {
        let manifest = |extra: &str| format!("{MINIMAL}{extra}");
        says(
            &MINIMAL.replace("acme.metrics", "lume.metrics"),
            "application's",
        );
        says(&MINIMAL.replace("acme.metrics", "Acme"), "not a plugin id");
        says(&MINIMAL.replace("0.1.0", "1.2"), "not a version");
        says(
            &MINIMAL.replace("api = \"1\"", "api = \"2\""),
            "version 2 of the plugin API",
        );
        says(
            &MINIMAL.replace("schema = 1", "schema = 2"),
            "of schema 1, not 2",
        );
        says(&manifest("main = \"../host.js\"\n"), "never `..`");
        says(&manifest("main = \"host.ts\"\n"), ".js file");
        says(&manifest("[limits]\nmemory = 1024\n"), "outside 16 to 256");
        says(
            &manifest("[permissions]\n\"fs.read\" = true\n"),
            "needs a scope",
        );
        says(
            &manifest("[dependencies]\n\"acme.metrics\" = \"1\"\n"),
            "itself",
        );
        says(
            &manifest("[dependencies]\n\"acme.core\" = \"one\"\n"),
            "version requirement",
        );
        says(
            &manifest("activation = [\"onCommand:nope\"]\nmain = \"a.js\"\n"),
            "names no command",
        );
        says(
            &manifest("activation = [\"onSessionEvent:typing\"]\nmain = \"a.js\"\n"),
            "kind of session event",
        );
        says(
            &manifest("activation = [\"onBoot\"]\n"),
            "not an activation event",
        );
        says(
            &manifest("[[contributes.commands]]\nid = \"go\"\ntitle = \"Go\"\n"),
            "needs its code",
        );
        says(
            &manifest(
                "main = \"a.js\"\n[[contributes.commands]]\nid = \"go\"\ntitle = \"Go\"\n\
                 [[contributes.commands]]\nid = \"go\"\ntitle = \"Again\"\n",
            ),
            "given twice",
        );
        says(
            &manifest(
                "[[contributes.menus]]\nid = \"m\"\nlocation = \"menu.app.view\"\ncommand = \"acme.core:open\"\n",
            ),
            "not among the plugin's dependencies",
        );
        says(
            &manifest(
                "[[contributes.menus]]\nid = \"m\"\nlocation = \"menu.side\"\ncommand = \"lume.tab.new\"\n",
            ),
            "not a menu",
        );
        says(
            &manifest(
                "[[contributes.menus]]\nid = \"m\"\nlocation = \"menu.app\"\ncommand = \"lume:tab\"\n",
            ),
            "named by its id",
        );
        says(
            &manifest(
                "[[contributes.menus]]\nid = \"m\"\nlocation = \"menu.app\"\ncommand = \"lume.tab.new\"\nbefore = \"a\"\nafter = \"b\"\n",
            ),
            "not both",
        );
        says(
            &manifest("[[contributes.status]]\nid = \"s\"\nslot = \"bottom\"\nkind = \"meter\"\n"),
            "not a place for status items",
        );
        says(
            &manifest(
                "[[contributes.status]]\nid = \"s\"\nslot = \"topbar.left\"\nkind = \"gauge\"\n",
            ),
            "not a kind of status item",
        );
        says(
            &manifest(
                "[[contributes.settings]]\nkey = \"n\"\ntype = \"integer\"\ndefault = \"two\"\n",
            ),
            "is not a integer",
        );
        says(
            &manifest(
                "[[contributes.settings]]\nkey = \"n\"\ntype = \"integer\"\ndefault = 9\nmaximum = 5\n",
            ),
            "outside the minimum",
        );
        says(
            &manifest(
                "[[contributes.settings]]\nkey = \"n\"\ntype = \"string\"\ndefault = \"x\"\ncontrol = \"switch\"\n",
            ),
            "cannot set a string",
        );
        says(
            &manifest(
                "[[contributes.settings]]\nkey = \"n\"\ntype = \"boolean\"\ndefault = true\npage = \"terminal\"\n",
            ),
            "names the group",
        );
        says(
            &manifest(
                "[[contributes.settings]]\nkey = \"n\"\ntype = \"boolean\"\ndefault = true\npage = \"terminal\"\ngroup = \"lume:nowhere\"\n",
            ),
            "lume:cursor",
        );
        says(
            &manifest(
                "[[contributes.settings]]\nkey = \"n\"\ntype = \"boolean\"\ndefault = true\npage = \"sidebar\"\n",
            ),
            "not a page",
        );
        says(
            &manifest(
                "[[contributes.panels]]\nid = \"p\"\ntitle = \"P\"\nkind = \"frame\"\narea = \"top\"\n",
            ),
            "not a kind of panel",
        );
        says(
            &manifest(
                "[[contributes.keybindings]]\nkey = \"ctrl+nope\"\ncommand = \"lume.tab.new\"\n",
            ),
            "ctrl+nope",
        );
    }

    #[test]
    fn the_schema_describes_the_manifest() {
        let schema = manifest_schema();
        let properties = schema["properties"].as_object().unwrap();
        for key in [
            "schema",
            "id",
            "api",
            "permissions",
            "contributes",
            "dependencies",
            "load",
        ] {
            assert!(properties.contains_key(key), "{key}");
        }
    }
}
