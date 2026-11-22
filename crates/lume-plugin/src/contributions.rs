//! What the plugins that can be used add to the window, worked out once for all of
//! them.
//!
//! Each plugin's manifest says what it adds and where; this puts all of it together,
//! with the application's own nodes, in the order everything is shown: the menus with
//! their sections and submenus, the status items of each bar, the panels of each dock
//! area. Every name a manifest writes is made full here (`refresh` becomes
//! `acme.metrics.refresh`, `acme.core:open` becomes the command of the install it is
//! bound to), pictures and shaders are read from the packages and checked, themes are
//! read with the configuration's own reader, and the plugins' settings become a part of
//! the configuration's schema, with the rows the settings window shows for them.
//!
//! What could not be done as a manifest asks is left out or placed by its order, and
//! said in the warnings, by plugin.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::assets;
use crate::id::{Reference, full_id};
use crate::manifest::{Manifest, OWN_PAGE, Placement, Setting, TABLE_KIND};
use crate::motion;
use crate::placement::{Anchor, BUILTIN_RANK, Placed, Side, Warning, arrange};
use crate::places::{self, NodeKind};

/// The order of a plugin's thing that says none: after the application's own.
const NO_ORDER: i32 = 10_000;

/// How deep submenus may be inside each other.
const SUBMENU_DEPTH_MAX: usize = 3;

/// The heading of a plugin's own settings that are in no group of its own.
const OWN_GROUP: &str = "Settings";

/// The heading the plugins' own pages are listed under in the settings window.
pub const ADD_ONS: &str = "Add-ons";

/// What a page of a plugin's own settings is called by: its install.
pub fn page_id(local: &str) -> String {
    format!("addon:{local}")
}

/// One install that can be used, as this needs it.
pub struct Install<'a> {
    /// The id it is installed under.
    pub local: &'a str,
    /// What the user calls it.
    pub name: String,
    pub manifest: &'a Manifest,
    /// Where its package is.
    pub dir: PathBuf,
    /// The hash of its package, as its lock entry has it: what was read of a package
    /// is kept while this stays the same.
    pub content: &'a str,
    /// Where it is in the order plugins load in.
    pub rank: i64,
    /// For each plugin it depends on, by manifest id, the install it is bound to.
    pub bound: &'a BTreeMap<String, String>,
}

/// What this needs to know of the application's own commands.
pub trait Commands {
    /// The title of the command `id`, and whether a plugin's menu item or key may run
    /// it; nothing when there is no such command.
    fn builtin(&self, id: &str) -> Option<(String, bool)>;
}

/// A picture a plugin's thing shows: one of the application's, by name, or one of the
/// plugins', by its key among the pictures.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Icon {
    Glyph(String),
    Image(String),
}

/// A picture of a plugin's, as the window draws it. A window is told its hash and
/// asks for the picture itself when it first draws it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ImageOut {
    #[serde(skip)]
    pub url: String,
    /// The hash of the picture's bytes, which a window asks for it by.
    pub hash: String,
    pub tint: bool,
}

/// One node of a menu, in its place.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MenuNode {
    /// `section`, `item` or `submenu`.
    pub kind: &'static str,
    /// `lume:<key>` for the application's own, `<install>:<name>` for a plugin's.
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The command an item runs, by its full id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
    /// The conditions it is shown under, each perhaps after `!`; all must hold.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<MenuNode>,
}

/// A status item, in its place.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StatusDef {
    /// `<install>.<name>`.
    pub id: String,
    pub plugin: String,
    /// The place it is in: where its manifest puts it, or, for an item a setting
    /// moves, where that setting puts it until the user changes it.
    pub slot: String,
    pub kind: String,
    /// Where it is among the items of the bars, or of the panes' headers. The items of
    /// the bars are put in order together, so one that moves keeps its place among the
    /// others of the place it goes to.
    pub order: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    /// The parts of a segmented item, in the order they are shown.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<StatusChoiceDef>,
    /// The command it runs when clicked, by its full id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The key of the submenu in `menu.status` it opens when clicked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub menu: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<String>,
    /// The plugin's switch that must be on for it to be shown, with its default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shown_when: Option<ShownWhen>,
    /// The plugin's setting that chooses which of the bars' places it is in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot_setting: Option<SlotSetting>,
    /// Whether a thin rule is drawn after it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub separator: bool,
}

/// One part of a segmented status item.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StatusChoiceDef {
    pub value: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
}

/// A setting of the plugin's that chooses where a status item goes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SlotSetting {
    pub key: String,
    /// The places it may choose, in the order its choices are given.
    pub slots: Vec<String>,
}

/// A switch of the plugin's settings a thing is shown under.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ShownWhen {
    pub key: String,
    pub default: bool,
}

/// A panel, in its place.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PanelDef {
    /// `<install>.<name>`, the panel's id in the layout too.
    pub id: String,
    pub plugin: String,
    /// The name its manifest gives it, which the layout calls its view.
    pub view: String,
    pub title: String,
    pub kind: String,
    /// The dock area it opens in the first time.
    pub area: String,
    /// Where it is among the plugins' panels of its area.
    pub order: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
    /// The page a `frame` panel shows, as a path in the package.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
}

/// A view a plugin draws in a frame, which a pane can show.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PaneViewDef {
    /// `<install>.<name>`.
    pub id: String,
    pub plugin: String,
    /// The name its manifest gives it, which the layout calls its view.
    pub view: String,
    pub title: String,
    /// The page it shows, as a path in the package.
    pub entry: String,
    pub persist: bool,
    /// Whether the settings window shows it on the plugin's page.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub settings: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
}

/// Files or addresses a plugin's pane view opens.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OpenerDef {
    pub plugin: String,
    /// The pane view's name in its manifest.
    pub view: String,
    pub title: String,
    /// File name patterns, lower case.
    pub patterns: Vec<String>,
    pub schemes: Vec<String>,
}

/// An effect a plugin offers for a place, checked.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MotionOut {
    /// `<install>/<name>`, as the settings choose it.
    pub id: String,
    pub plugin: String,
    pub title: String,
    pub slot: String,
    /// The effect as the window plays it.
    pub spec: Value,
}

/// A cursor effect a plugin's shader draws, checked.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CursorEffectOut {
    /// `<install>/<name>`, as the settings choose it.
    pub id: String,
    pub plugin: String,
    pub title: String,
    pub kind: String,
    pub duration: u32,
    pub reach: u32,
    #[serde(skip)]
    pub source: String,
    /// The hash of the shader's text, which a window asks for it by.
    pub hash: String,
}

/// A key a plugin binds, with its names made full.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyOut {
    pub plugin: String,
    pub key: String,
    pub macos: Option<String>,
    pub command: String,
    pub args: Value,
    pub when: Option<String>,
}

/// A verb a plugin adds to the command line, with its command's name made full.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VerbOut {
    /// The verb as it is typed, its words with a space between them.
    pub verb: String,
    #[serde(skip)]
    pub words: Vec<String>,
    pub plugin: String,
    /// `<install>.<name>`.
    pub command: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
}

/// What a plugin adds to the settings: the part of the configuration's schema for its
/// table, and its own page if it has one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsOut {
    /// The schema of `[plugins."<install>"]`, by install.
    pub fragments: BTreeMap<String, Value>,
    /// The plugins' own pages, as the schema lists pages, in the order plugins load in.
    pub pages: Vec<Value>,
}

/// Everything the plugins that can be used add.
#[derive(Clone, Debug, Default)]
pub struct Contributions {
    /// Each menu's nodes, by the menu's name.
    pub menus: BTreeMap<String, Vec<MenuNode>>,
    pub status: Vec<StatusDef>,
    pub panels: Vec<PanelDef>,
    pub pane_views: Vec<PaneViewDef>,
    pub openers: Vec<OpenerDef>,
    /// The plugins' pictures, by key.
    pub images: BTreeMap<String, ImageOut>,
    pub motions: Vec<MotionOut>,
    pub cursor_effects: Vec<CursorEffectOut>,
    pub themes: Vec<lume_config::Theme>,
    pub keys: Vec<KeyOut>,
    pub settings: SettingsOut,
    /// The verbs the plugins add to the command line.
    pub verbs: Vec<VerbOut>,
    /// What could not be done as asked, by install.
    pub warnings: Vec<(String, String)>,
}

/// What has been read of the installs' packages, kept from one working-out to the next
/// so a package is read again only when it is another one: by install, the hash of the
/// package the files were read from, and the files.
#[derive(Debug, Default)]
pub struct Read {
    by_install: BTreeMap<String, (String, Files)>,
}

/// The files read of one package, by their path in it.
#[derive(Debug, Default)]
struct Files {
    images: BTreeMap<String, Result<Picture, String>>,
    shaders: BTreeMap<String, Result<String, String>>,
    themes: BTreeMap<String, Result<String, String>>,
}

/// A picture as it was read: its address, the hash and the count of its bytes.
#[derive(Clone, Debug)]
struct Picture {
    url: String,
    hash: String,
    bytes: u64,
}

impl Read {
    /// Forgets the packages that are not among the installs any more.
    fn keep(&mut self, installs: &[Install<'_>]) {
        self.by_install.retain(|local, (content, _)| {
            installs
                .iter()
                .any(|install| install.local == local && install.content == content)
        });
    }

    fn files(&mut self, install: &Install<'_>) -> &mut Files {
        &mut self
            .by_install
            .entry(install.local.to_owned())
            .or_insert_with(|| (install.content.to_owned(), Files::default()))
            .1
    }

    fn image(&mut self, install: &Install<'_>, file: &str) -> Result<Picture, String> {
        let dir = &install.dir;
        self.files(install)
            .images
            .entry(file.to_owned())
            .or_insert_with(|| {
                assets::read_image(dir, file).map(|image| Picture {
                    url: image.data_url(),
                    hash: crate::hash::hex(&image.bytes),
                    bytes: image.bytes.len() as u64,
                })
            })
            .clone()
    }

    fn shader(&mut self, install: &Install<'_>, file: &str) -> Result<String, String> {
        let dir = &install.dir;
        self.files(install)
            .shaders
            .entry(file.to_owned())
            .or_insert_with(|| assets::read_shader(dir, file))
            .clone()
    }

    fn theme(&mut self, install: &Install<'_>, file: &str) -> Result<String, String> {
        let path = install.dir.join(file);
        self.files(install)
            .themes
            .entry(file.to_owned())
            .or_insert_with(|| std::fs::read_to_string(&path).map_err(|error| error.to_string()))
            .clone()
    }

    /// How many packages it holds files of.
    pub fn len(&self) -> usize {
        self.by_install.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_install.is_empty()
    }
}

impl Contributions {
    /// What a window is sent: everything but the pictures and the shaders themselves,
    /// which it asks for by their hashes when it needs them.
    pub fn to_json(&self) -> Value {
        json!({
            "menus": self.menus,
            "status": self.status,
            "panels": self.panels,
            "pane_views": self.pane_views,
            "openers": self.openers,
            "images": self.images,
            "motions": self.motions,
            "cursor_effects": self.cursor_effects,
        })
    }

    /// A picture, as the address a window draws it from, or a shader's text, by its
    /// hash.
    pub fn asset(&self, hash: &str) -> Option<&str> {
        self.images
            .values()
            .find(|image| image.hash == hash)
            .map(|image| image.url.as_str())
            .or_else(|| {
                self.cursor_effects
                    .iter()
                    .find(|effect| effect.hash == hash)
                    .map(|effect| effect.source.as_str())
            })
    }

    fn warn(&mut self, local: &str, message: String) {
        self.warnings.push((local.to_owned(), message));
    }
}

/// How one install's manifest names things, made full.
struct Names<'a> {
    install: &'a Install<'a>,
}

impl Names<'_> {
    fn local(&self) -> &str {
        self.install.local
    }

    /// The key of a thing a manifest names, among the things of a place: its own, the
    /// application's, or a dependency's through its binding.
    fn key(&self, written: &str) -> Option<String> {
        match Reference::parse(written).ok()? {
            Reference::Own(name) => Some(format!("{}:{name}", self.local())),
            Reference::Builtin(key) => Some(format!("lume:{key}")),
            Reference::Other { plugin, name } => self
                .install
                .bound
                .get(&plugin)
                .map(|local| format!("{local}:{name}")),
        }
    }

    /// The full id of a command a manifest names.
    fn command(&self, written: &str) -> Option<String> {
        match Reference::parse(written).ok()? {
            Reference::Own(name) => Some(full_id(self.local(), &name)),
            Reference::Builtin(id) => Some(id),
            Reference::Other { plugin, name } => self
                .install
                .bound
                .get(&plugin)
                .map(|local| full_id(local, &name)),
        }
    }

    /// A condition as the window reads it: the plugin's own contexts, written
    /// `plugin:<key>`, become `plugin:<install>.<key>`; a dependency's,
    /// `plugin:<id>:<key>`, the bound install's.
    fn when(&self, written: Option<&String>) -> Vec<String> {
        let Some(written) = written else {
            return Vec::new();
        };
        written
            .split("&&")
            .map(|term| {
                let term = term.trim();
                let (bang, name) = match term.strip_prefix('!') {
                    Some(name) => ("!", name.trim()),
                    None => ("", term),
                };
                let name = match name.strip_prefix("plugin:") {
                    Some(rest) => match rest.split_once(':') {
                        Some((plugin, key)) => match self.install.bound.get(plugin) {
                            Some(local) => format!("plugin:{local}.{key}"),
                            // A dependency that is not there holds no condition.
                            None => format!("plugin:{plugin}:{key}"),
                        },
                        None => format!("plugin:{}.{rest}", self.local()),
                    },
                    None => name.to_owned(),
                };
                format!("{bang}{name}")
            })
            .collect()
    }

    fn anchor(&self, placement: &Placement) -> Option<Anchor> {
        let (side, written) = match (&placement.before, &placement.after) {
            (Some(before), _) => (Side::Before, before),
            (None, Some(after)) => (Side::After, after),
            (None, None) => return None,
        };
        self.key(written).map(|key| Anchor { side, key })
    }

    fn icon(&self, written: Option<&String>, images: &BTreeMap<String, ImageOut>) -> Option<Icon> {
        let written = written?;
        let own = format!("{}:{written}", self.local());
        if images.contains_key(&own) {
            return Some(Icon::Image(own));
        }
        if written.contains('/') || assets::is_image_path(written) {
            // A picture that could not be read shows nothing.
            return None;
        }
        Some(Icon::Glyph(written.clone()))
    }
}

/// A plugin's thing to be placed in a container, with what it is.
fn placed<T>(
    names: &Names<'_>,
    name: &str,
    placement: &Placement,
    index: usize,
    value: T,
) -> Placed<T> {
    Placed {
        key: format!("{}:{name}", names.local()),
        anchor: names.anchor(placement),
        order: placement.order.unwrap_or(NO_ORDER),
        rank: names.install.rank,
        plugin: names.local().to_owned(),
        index,
        value,
    }
}

fn toml_json(value: Option<&toml::Value>) -> Option<Value> {
    value.and_then(|value| serde_json::to_value(value).ok())
}

/// Says each placement warning against the plugin it is about.
fn say(out: &mut Contributions, warnings: Vec<Warning>) {
    for warning in warnings {
        if !warning.plugin.is_empty() {
            out.warn(&warning.plugin, warning.message);
        }
    }
}

/// Works out everything the installs add. They come in the order they load in. What
/// `read` holds of a package from an earlier time is used again.
pub fn build(installs: &[Install<'_>], commands: &dyn Commands, read: &mut Read) -> Contributions {
    let mut out = Contributions::default();
    read.keep(installs);
    let by_local: BTreeMap<&str, &Install<'_>> = installs
        .iter()
        .map(|install| (install.local, install))
        .collect();

    for install in installs {
        images(install, read, &mut out);
    }
    menus(installs, &by_local, commands, &mut out);
    status(installs, commands, &mut out);
    panels(installs, &mut out);
    views(installs, &mut out);
    for install in installs {
        let names = Names { install };
        let local = install.local;
        let contributes = &install.manifest.contributes;
        for (index, written) in contributes.motions.iter().enumerate() {
            match motion::check(written, &format!("contributes.motions[{index}]")) {
                Ok(spec) => out.motions.push(MotionOut {
                    id: format!("{local}/{}", written.id),
                    plugin: local.to_owned(),
                    title: written.title.clone(),
                    slot: written.slot.clone(),
                    spec,
                }),
                Err(problem) => out.warn(local, problem),
            }
        }
        for effect in &contributes.cursor_effects {
            match read.shader(install, &effect.shader) {
                Ok(source) => out.cursor_effects.push(CursorEffectOut {
                    id: format!("{local}/{}", effect.id),
                    plugin: local.to_owned(),
                    title: effect.title.clone(),
                    kind: effect.kind.clone(),
                    duration: effect.duration_ms(),
                    reach: effect.reach_px(),
                    hash: crate::hash::hex(source.as_bytes()),
                    source,
                }),
                Err(problem) => out.warn(
                    local,
                    format!("the cursor effect {} is not used: {problem}", effect.id),
                ),
            }
        }
        for theme in &contributes.themes {
            let id = format!("{local}/{}", theme.id);
            let file = install.dir.join(&theme.file);
            let parsed = read.theme(install, &theme.file).and_then(|text| {
                lume_config::parse_theme(&id, &text, Some(&file)).map_err(|problem| problem.message)
            });
            match parsed {
                Ok((parsed, problems)) => {
                    for problem in problems {
                        out.warn(
                            local,
                            format!("the theme {}: {}", theme.id, problem.message),
                        );
                    }
                    out.themes.push(parsed);
                }
                Err(problem) => out.warn(
                    local,
                    format!("the theme {} is not used: {problem}", theme.id),
                ),
            }
        }
        for binding in &contributes.keybindings {
            let Some(command) = names.command(&binding.command) else {
                out.warn(
                    local,
                    format!(
                        "the key {} is not bound: {} is not installed",
                        binding.key, binding.command
                    ),
                );
                continue;
            };
            if command.starts_with("lume.")
                && !commands.builtin(&command).is_some_and(|(_, open)| open)
            {
                out.warn(
                    local,
                    format!(
                        "the key {} is not bound: plugins may not run {command}",
                        binding.key
                    ),
                );
                continue;
            }
            let when = names.when(binding.when.as_ref());
            out.keys.push(KeyOut {
                plugin: local.to_owned(),
                key: binding.key.clone(),
                macos: binding.macos.clone(),
                command,
                args: toml_json(binding.args.as_ref()).unwrap_or(Value::Null),
                when: (!when.is_empty()).then(|| when.join(" && ")),
            });
        }
    }
    settings(installs, &mut out);
    verbs(installs, &mut out);
    out
}

/// The verbs the installs add to the command line. A verb belongs to the first plugin
/// that adds it, in the order plugins load in; another that adds the same is told.
fn verbs(installs: &[Install<'_>], out: &mut Contributions) {
    for install in installs {
        let local = install.local;
        for written in &install.manifest.contributes.cli {
            let Ok(words) = crate::cli::parse_verb(&written.verb) else {
                continue;
            };
            if let Some(taken) = out.verbs.iter().find(|verb| verb.words == words) {
                let message = format!(
                    "the verb `{}` is not added: {} adds it already",
                    written.verb, taken.plugin
                );
                out.warn(local, message);
                continue;
            }
            out.verbs.push(VerbOut {
                verb: words.join(" "),
                words,
                plugin: local.to_owned(),
                command: full_id(local, &written.command),
                summary: written.summary.clone(),
                usage: written.usage.clone(),
            });
        }
    }
}

/// Reads the pictures a plugin declares, and those its things name by path.
fn images(install: &Install<'_>, read: &mut Read, out: &mut Contributions) {
    let local = install.local;
    let contributes = &install.manifest.contributes;
    let mut wanted: Vec<(String, String, bool)> = contributes
        .images
        .iter()
        .map(|image| (image.id.clone(), image.file.clone(), image.tint))
        .collect();
    let named = contributes
        .menus
        .iter()
        .filter_map(|item| item.icon.as_ref())
        .chain(
            contributes
                .status
                .iter()
                .filter_map(|item| item.icon.as_ref()),
        )
        .chain(
            contributes
                .panels
                .iter()
                .filter_map(|panel| panel.icon.as_ref()),
        )
        .chain(install.manifest.icon.as_ref());
    let mut paths = BTreeSet::new();
    for path in named {
        if (path.contains('/') || assets::is_image_path(path)) && paths.insert(path.clone()) {
            wanted.push((path.clone(), path.clone(), false));
        }
    }
    let mut total = 0u64;
    for (id, file, tint) in wanted {
        match read.image(install, &file) {
            Ok(image) => {
                total += image.bytes;
                if total > assets::IMAGES_MAX_BYTES {
                    out.warn(
                        local,
                        format!(
                            "{file} is left out: a plugin's pictures may hold {} KiB together",
                            assets::IMAGES_MAX_BYTES / 1024
                        ),
                    );
                    continue;
                }
                out.images.insert(
                    format!("{local}:{id}"),
                    ImageOut {
                        url: image.url,
                        hash: image.hash,
                        tint,
                    },
                );
            }
            Err(problem) => out.warn(local, format!("a picture is left out: {problem}")),
        }
    }
}

/// What a container of a menu is: the top of a menu, one of its sections, or a
/// submenu.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Container {
    Top(&'static str),
    Section(&'static str, String),
    Submenu(&'static str, String),
}

/// A node waiting to be placed in its container.
#[derive(Clone, Debug)]
struct Pending {
    node: MenuNode,
    /// The container a submenu's or section's own children are in.
    holds: Option<Container>,
}

fn menu_location(written: &str) -> Option<&'static str> {
    places::MENUS.iter().copied().find(|menu| *menu == written)
}

/// Puts the menus together: the application's own nodes, the plugins' sections and
/// items, each in its container, and each container in order.
fn menus(
    installs: &[Install<'_>],
    by_local: &BTreeMap<&str, &Install<'_>>,
    commands: &dyn Commands,
    out: &mut Contributions,
) {
    let mut containers: BTreeMap<Container, Vec<Placed<Pending>>> = BTreeMap::new();
    // The application's own nodes.
    for menu in places::MENUS {
        for (at, node) in places::skeleton(menu).iter().enumerate() {
            let top_order = i32::try_from(at + 1).unwrap_or(i32::MAX);
            let holds = (node.kind == NodeKind::Section)
                .then(|| Container::Section(menu, format!("lume:{}", node.key)));
            let step = if node.kind == NodeKind::Section {
                places::SECTION_STEP
            } else {
                places::ITEM_STEP
            };
            containers
                .entry(Container::Top(menu))
                .or_default()
                .push(Placed {
                    key: format!("lume:{}", node.key),
                    anchor: None,
                    order: top_order * step,
                    rank: BUILTIN_RANK,
                    plugin: String::new(),
                    index: at,
                    value: Pending {
                        node: MenuNode {
                            kind: node.kind.as_str(),
                            key: format!("lume:{}", node.key),
                            plugin: None,
                            title: None,
                            command: None,
                            args: None,
                            icon: None,
                            when: Vec::new(),
                            children: Vec::new(),
                        },
                        holds: holds.clone(),
                    },
                });
            for (child_at, child) in node.children.iter().enumerate() {
                let order = i32::try_from(child_at + 1).unwrap_or(i32::MAX) * places::ITEM_STEP;
                if let Some(section) = &holds {
                    containers.entry(section.clone()).or_default().push(Placed {
                        key: format!("lume:{}", child.key),
                        anchor: None,
                        order,
                        rank: BUILTIN_RANK,
                        plugin: String::new(),
                        index: child_at,
                        value: Pending {
                            node: MenuNode {
                                kind: child.kind.as_str(),
                                key: format!("lume:{}", child.key),
                                plugin: None,
                                title: None,
                                command: None,
                                args: None,
                                icon: None,
                                when: Vec::new(),
                                children: Vec::new(),
                            },
                            holds: None,
                        },
                    });
                }
            }
        }
    }

    // The plugins' sections, then their items.
    let mut sections: BTreeSet<(&'static str, String)> = BTreeSet::new();
    let mut submenus: BTreeSet<(&'static str, String)> = BTreeSet::new();
    for install in installs {
        let names = Names { install };
        let contributes = &install.manifest.contributes;
        for (index, section) in contributes.sections.iter().enumerate() {
            let Some(menu) = menu_location(&section.location) else {
                continue;
            };
            if menu == "menu.app" {
                out.warn(
                    install.local,
                    format!(
                        "{}:{} is left out: the menu bar holds only menus",
                        install.local, section.id
                    ),
                );
                continue;
            }
            let key = format!("{}:{}", install.local, section.id);
            sections.insert((menu, key.clone()));
            let node = MenuNode {
                kind: "section",
                key: key.clone(),
                plugin: Some(install.local.to_owned()),
                title: section.title.clone(),
                command: None,
                args: None,
                icon: None,
                when: Vec::new(),
                children: Vec::new(),
            };
            containers
                .entry(Container::Top(menu))
                .or_default()
                .push(placed(
                    &names,
                    &section.id,
                    &section.placement(),
                    index,
                    Pending {
                        node,
                        holds: Some(Container::Section(menu, key)),
                    },
                ));
        }
        for item in &contributes.menus {
            if item.submenu
                && let Some(menu) = menu_location(&item.location)
            {
                submenus.insert((menu, format!("{}:{}", install.local, item.id)));
            }
        }
    }
    for install in installs {
        let names = Names { install };
        let local = install.local;
        for (index, item) in install.manifest.contributes.menus.iter().enumerate() {
            let Some(mut menu) = menu_location(&item.location) else {
                continue;
            };
            let key = format!("{local}:{}", item.id);
            // Where it goes: inside a submenu, in a section, or at the top.
            let container = if let Some(parent) = &item.parent {
                match names.key(parent) {
                    Some(parent) if parent.starts_with("lume:") => {
                        match places::opened_by(menu, &parent["lume:".len()..]) {
                            Some(opened) => {
                                menu = menu_location(opened).unwrap_or(menu);
                                Container::Top(menu)
                            }
                            None => {
                                out.warn(
                                    local,
                                    format!(
                                        "{key} is left out: {parent} in {menu} takes no items of plugins"
                                    ),
                                );
                                continue;
                            }
                        }
                    }
                    Some(parent) if submenus.contains(&(menu, parent.clone())) => {
                        Container::Submenu(menu, parent)
                    }
                    _ => {
                        out.warn(
                            local,
                            format!("{key} is left out: its submenu {parent} is not in {menu}"),
                        );
                        continue;
                    }
                }
            } else if let Some(section) = &item.section {
                match names.key(section) {
                    Some(section)
                        if (section.starts_with("lume:")
                            && places::find(menu, &section["lume:".len()..])
                                .is_some_and(|found| found.node.kind == NodeKind::Section))
                            || sections.contains(&(menu, section.clone())) =>
                    {
                        Container::Section(menu, section)
                    }
                    _ => {
                        out.warn(
                            local,
                            format!(
                                "{key} is at the top of {menu}: its section {section} is not there"
                            ),
                        );
                        Container::Top(menu)
                    }
                }
            } else {
                Container::Top(menu)
            };
            if container == Container::Top("menu.app") && !item.submenu {
                out.warn(
                    local,
                    format!("{key} is left out: the menu bar holds only menus"),
                );
                continue;
            }
            // What it runs, and what it is called.
            let mut title = item.title.clone();
            let command = match &item.command {
                None => None,
                Some(written) => match names.command(written) {
                    Some(id) if id.starts_with("lume.") => match commands.builtin(&id) {
                        Some((builtin, true)) => {
                            // An item that runs one of the application's commands is
                            // called what the command is, whatever the manifest says.
                            title = Some(builtin);
                            Some(id)
                        }
                        Some((_, false)) => {
                            out.warn(
                                local,
                                format!("{key} is left out: plugins may not run {id}"),
                            );
                            continue;
                        }
                        None => {
                            out.warn(local, format!("{key} is left out: there is no {id}"));
                            continue;
                        }
                    },
                    Some(id) => {
                        if title.is_none() {
                            title = command_title(&id, by_local);
                        }
                        Some(id)
                    }
                    None => {
                        out.warn(
                            local,
                            format!("{key} is left out: {written} is not installed"),
                        );
                        continue;
                    }
                },
            };
            let node = MenuNode {
                kind: if item.submenu { "submenu" } else { "item" },
                key: key.clone(),
                plugin: Some(local.to_owned()),
                title,
                command,
                args: toml_json(item.args.as_ref()),
                icon: names.icon(item.icon.as_ref(), &out.images),
                when: names.when(item.when.as_ref()),
                children: Vec::new(),
            };
            let holds = item.submenu.then(|| Container::Submenu(menu, key.clone()));
            containers.entry(container).or_default().push(placed(
                &names,
                &item.id,
                &item.placement(),
                index,
                Pending { node, holds },
            ));
        }
    }

    // Each container in order.
    let mut arranged: BTreeMap<Container, Vec<Pending>> = BTreeMap::new();
    let mut warnings = Vec::new();
    for (container, things) in containers {
        let ordered = arrange(things, &mut warnings);
        arranged.insert(
            container,
            ordered.into_iter().map(|thing| thing.value).collect(),
        );
    }
    say(out, warnings);

    // The tree of each menu, from its top.
    // A key names one thing in one menu: the application's own keys come again in other
    // menus (a tab's menu has "new-tab" too), so what has been placed is counted for each
    // menu apart, and a section apart from an item.
    let mut reached: BTreeSet<String> = BTreeSet::new();
    /// Counts everything inside a container as dealt with, for what is left out whole.
    fn mark(
        container: &Container,
        arranged: &BTreeMap<Container, Vec<Pending>>,
        reached: &mut BTreeSet<String>,
    ) {
        for pending in arranged.get(container).into_iter().flatten() {
            if reached.insert(pending.node.key.clone())
                && let Some(holds) = &pending.holds
            {
                mark(holds, arranged, reached);
            }
        }
    }
    fn grow(
        container: &Container,
        arranged: &BTreeMap<Container, Vec<Pending>>,
        depth: usize,
        seen: &mut BTreeSet<(&'static str, String)>,
        reached: &mut BTreeSet<String>,
        out: &mut Contributions,
    ) -> Vec<MenuNode> {
        let mut nodes = Vec::new();
        for pending in arranged.get(container).into_iter().flatten() {
            let mut node = pending.node.clone();
            if !seen.insert((node.kind, node.key.clone())) {
                continue;
            }
            reached.insert(node.key.clone());
            if let Some(holds) = &pending.holds {
                let deeper = matches!(holds, Container::Submenu(..));
                if deeper && depth >= SUBMENU_DEPTH_MAX {
                    if let Some(plugin) = &node.plugin {
                        out.warn(
                            plugin,
                            format!(
                                "{} is left out: submenus go {SUBMENU_DEPTH_MAX} deep at most",
                                node.key
                            ),
                        );
                    }
                    mark(holds, arranged, reached);
                    continue;
                }
                node.children = grow(
                    holds,
                    arranged,
                    depth + usize::from(deeper),
                    seen,
                    reached,
                    out,
                );
            }
            nodes.push(node);
        }
        nodes
    }
    for menu in places::MENUS {
        let mut seen = BTreeSet::new();
        let nodes = grow(
            &Container::Top(menu),
            &arranged,
            0,
            &mut seen,
            &mut reached,
            out,
        );
        out.menus.insert(menu.to_owned(), nodes);
    }
    // What was never reached: submenus inside each other in a circle.
    for pending in arranged.values() {
        for one in pending {
            if !reached.contains(&one.node.key)
                && let Some(plugin) = &one.node.plugin
            {
                out.warn(
                    plugin,
                    format!("{} is left out: its submenus hold each other", one.node.key),
                );
            }
        }
    }
}

/// The title the manifest gives a plugin's command, by the command's full id.
fn command_title(id: &str, by_local: &BTreeMap<&str, &Install<'_>>) -> Option<String> {
    by_local.iter().find_map(|(local, install)| {
        let name = id.strip_prefix(local)?.strip_prefix('.')?;
        install
            .manifest
            .contributes
            .commands
            .iter()
            .find(|command| command.id == name)
            .map(|command| command.title.clone())
    })
}

fn status(installs: &[Install<'_>], commands: &dyn Commands, out: &mut Contributions) {
    // The items of the bars are put in order together, those of the panes' headers apart.
    let mut containers: BTreeMap<&'static str, Vec<Placed<StatusDef>>> = BTreeMap::new();
    for install in installs {
        let names = Names { install };
        let local = install.local;
        for (index, item) in install.manifest.contributes.status.iter().enumerate() {
            let command = match &item.command {
                None => None,
                Some(written) => match names.command(written) {
                    Some(id) if id.starts_with("lume.") => {
                        if commands.builtin(&id).is_some_and(|(_, open)| open) {
                            Some(id)
                        } else {
                            out.warn(
                                local,
                                format!(
                                    "{local}.{} runs nothing: plugins may not run {id}",
                                    item.id
                                ),
                            );
                            None
                        }
                    }
                    found => found,
                },
            };
            let settings = &install.manifest.contributes.settings;
            let slot_setting = item.slot_setting.as_ref().and_then(|key| {
                let setting = settings.iter().find(|setting| setting.key == *key)?;
                let slots: Vec<String> = setting
                    .choices
                    .iter()
                    .filter_map(|choice| choice.value.as_str())
                    .filter(|place| places::BAR_SLOTS.contains(place))
                    .map(str::to_owned)
                    .collect();
                let first = setting
                    .default
                    .as_str()
                    .filter(|place| slots.iter().any(|slot| slot == place))
                    .map(str::to_owned)
                    .or_else(|| slots.first().cloned())?;
                Some((
                    first,
                    SlotSetting {
                        key: key.clone(),
                        slots,
                    },
                ))
            });
            let slot = match (&item.slot, &slot_setting) {
                (Some(slot), _) => slot.clone(),
                (None, Some((first, _))) => first.clone(),
                (None, None) => {
                    out.warn(
                        local,
                        format!("{local}.{} is left out: it has no place", item.id),
                    );
                    continue;
                }
            };
            let container = if slot == "pane.header" {
                "pane.header"
            } else {
                "bars"
            };
            let def = StatusDef {
                id: full_id(local, &item.id),
                plugin: local.to_owned(),
                slot,
                kind: item.kind.clone(),
                order: 0,
                label: item.label.clone(),
                icon: names.icon(item.icon.as_ref(), &out.images),
                tooltip: item.tooltip.clone(),
                choices: item
                    .choices
                    .iter()
                    .map(|choice| StatusChoiceDef {
                        value: choice.value.clone(),
                        label: choice.label.clone(),
                        icon: names.icon(choice.icon.as_ref(), &out.images),
                    })
                    .collect(),
                command,
                menu: item.menu.as_ref().map(|menu| format!("{local}:{menu}")),
                when: names.when(item.when.as_ref()),
                shown_when: item.shown_when.as_ref().map(|key| ShownWhen {
                    key: key.clone(),
                    default: install
                        .manifest
                        .contributes
                        .settings
                        .iter()
                        .find(|setting| setting.key == *key)
                        .and_then(|setting| setting.default.as_bool())
                        .unwrap_or(true),
                }),
                slot_setting: slot_setting.map(|(_, setting)| setting),
                separator: item.separator,
            };
            containers.entry(container).or_default().push(placed(
                &names,
                &item.id,
                &item.placement(),
                index,
                def,
            ));
        }
    }
    let mut warnings = Vec::new();
    for (_, things) in containers {
        for (order, thing) in arrange(things, &mut warnings).into_iter().enumerate() {
            let mut def = thing.value;
            def.order = order;
            out.status.push(def);
        }
    }
    say(out, warnings);
}

/// The pane views and openers of every install, in the order installs load in.
fn views(installs: &[Install<'_>], out: &mut Contributions) {
    for install in installs {
        let names = Names { install };
        let local = install.local;
        let contributes = &install.manifest.contributes;
        for view in &contributes.pane_views {
            out.pane_views.push(PaneViewDef {
                id: full_id(local, &view.id),
                plugin: local.to_owned(),
                view: view.id.clone(),
                title: view.title.clone(),
                entry: view.entry.clone(),
                persist: view.persist,
                settings: view.settings,
                icon: names.icon(view.icon.as_ref(), &out.images),
            });
        }
        for opener in &contributes.openers {
            let title = opener.title.clone().unwrap_or_else(|| {
                contributes
                    .pane_views
                    .iter()
                    .find(|view| view.id == opener.view)
                    .map(|view| view.title.clone())
                    .unwrap_or_default()
            });
            out.openers.push(OpenerDef {
                plugin: local.to_owned(),
                view: opener.view.clone(),
                title,
                patterns: opener.patterns.iter().map(|p| p.to_lowercase()).collect(),
                schemes: opener.schemes.clone(),
            });
        }
    }
}

fn panels(installs: &[Install<'_>], out: &mut Contributions) {
    let mut areas: BTreeMap<String, Vec<Placed<PanelDef>>> = BTreeMap::new();
    for install in installs {
        let names = Names { install };
        let local = install.local;
        for (index, panel) in install.manifest.contributes.panels.iter().enumerate() {
            let def = PanelDef {
                id: full_id(local, &panel.id),
                plugin: local.to_owned(),
                view: panel.id.clone(),
                title: panel.title.clone(),
                kind: panel.kind.clone(),
                area: panel.area.clone(),
                order: 0,
                icon: names.icon(panel.icon.as_ref(), &out.images),
                entry: panel.entry.clone(),
            };
            areas.entry(panel.area.clone()).or_default().push(placed(
                &names,
                &panel.id,
                &panel.placement(),
                index,
                def,
            ));
        }
    }
    let mut warnings = Vec::new();
    for (_, things) in areas {
        for (order, thing) in arrange(things, &mut warnings).into_iter().enumerate() {
            let mut def = thing.value;
            def.order = order;
            out.panels.push(def);
        }
    }
    say(out, warnings);
}

/// The schema of one plugin setting, with how the settings window shows it.
fn setting_schema(setting: &Setting) -> Value {
    let mut schema = Map::new();
    // A table is an object to the schema, open: what is in it is the plugin's to read.
    let kind = if setting.kind == TABLE_KIND {
        "object"
    } else {
        setting.kind.as_str()
    };
    schema.insert("type".to_owned(), json!(kind));
    if let Some(default) = toml_json(Some(&setting.default)) {
        schema.insert("default".to_owned(), default);
    }
    if let Some(minimum) = setting.minimum {
        schema.insert("minimum".to_owned(), json!(minimum));
    }
    if let Some(maximum) = setting.maximum {
        schema.insert("maximum".to_owned(), json!(maximum));
    }
    if !setting.choices.is_empty() {
        let values: Vec<Value> = setting
            .choices
            .iter()
            .filter_map(|choice| toml_json(Some(&choice.value)))
            .collect();
        schema.insert("enum".to_owned(), Value::Array(values));
    }
    if let Some(help) = &setting.help {
        schema.insert("description".to_owned(), json!(help));
    }
    Value::Object(schema)
}

/// A setting waiting to be placed among its group's: its key and its schema.
type Row = (String, Value);

/// The control a setting is shown with when its manifest does not say.
fn control_of(setting: &Setting) -> &str {
    if let Some(control) = &setting.control {
        return control;
    }
    if !setting.choices.is_empty() {
        return "select";
    }
    match setting.kind.as_str() {
        "boolean" => "switch",
        "integer" | "number" => "number",
        _ => "text",
    }
}

/// The plugins' settings: the schema of each one's table, the rows the settings window
/// shows on its pages and on the plugins' own.
fn settings(installs: &[Install<'_>], out: &mut Contributions) {
    for install in installs {
        let names = Names { install };
        let local = install.local;
        let manifest = install.manifest;
        let contributes = &manifest.contributes;
        if contributes.settings.is_empty() {
            continue;
        }
        // The plugin's own groups, by page, in order.
        let mut group_titles: BTreeMap<String, (String, String)> = BTreeMap::new();
        let mut own_groups: Vec<Placed<String>> = Vec::new();
        for (index, group) in contributes.setting_groups.iter().enumerate() {
            let page = group.page.clone().unwrap_or_else(|| OWN_PAGE.to_owned());
            group_titles.insert(group.id.clone(), (page.clone(), group.title.clone()));
            if page == OWN_PAGE {
                own_groups.push(placed(
                    &names,
                    &group.id,
                    &group.placement(),
                    index,
                    group.title.clone(),
                ));
            }
        }
        let mut warnings = Vec::new();
        let mut headings: Vec<String> = arrange(own_groups, &mut warnings)
            .into_iter()
            .map(|placed| placed.value)
            .collect();

        // Each row, by the page and heading it is under.
        let mut rows: BTreeMap<(String, String), Vec<Placed<Row>>> = BTreeMap::new();
        let mut properties = Map::new();
        for (index, setting) in contributes.settings.iter().enumerate() {
            let mut schema = setting_schema(setting);
            // A setting without a label, and a table with or without one, is only in
            // the file.
            let label = match &setting.label {
                Some(label) if setting.kind != TABLE_KIND => label,
                _ => {
                    properties.insert(setting.key.clone(), schema);
                    continue;
                }
            };
            let page = setting.page.clone().unwrap_or_else(|| OWN_PAGE.to_owned());
            let heading = match &setting.group {
                Some(group) => match Reference::parse(group) {
                    Ok(Reference::Builtin(slug)) => lume_config::settings::pages()
                        .into_iter()
                        .filter(|known| known.id == page)
                        .flat_map(|known| known.groups.iter())
                        .find(|title| title.to_ascii_lowercase().replace(' ', "-") == slug)
                        .map(|title| (*title).to_owned()),
                    Ok(Reference::Own(name)) => {
                        group_titles.get(&name).map(|(_, title)| title.clone())
                    }
                    _ => None,
                },
                None => Some(OWN_GROUP.to_owned()),
            };
            let Some(heading) = heading else {
                out.warn(
                    local,
                    format!(
                        "the setting {} is not shown: its group is not on its page",
                        setting.key
                    ),
                );
                properties.insert(setting.key.clone(), schema);
                continue;
            };
            if page == OWN_PAGE && !headings.contains(&heading) {
                if heading == OWN_GROUP {
                    headings.insert(0, heading.clone());
                } else {
                    headings.push(heading.clone());
                }
            }
            let mut annotation = json!({
                "page": if page == OWN_PAGE { page_id(local) } else { page.clone() },
                "group": heading,
                "label": label,
                "control": control_of(setting),
                "plugin": install.name,
            });
            if let Some(help) = &setting.help {
                annotation["help"] = json!(help);
            }
            if !setting.choices.is_empty() {
                annotation["options"] = Value::Array(
                    setting
                        .choices
                        .iter()
                        .filter_map(|choice| {
                            toml_json(Some(&choice.value))
                                .map(|value| json!({ "value": value, "label": choice.label }))
                        })
                        .collect(),
                );
            }
            if let Some(unit) = &setting.unit {
                annotation["unit"] = json!(unit);
            }
            if control_of(setting) == "number" {
                annotation["step"] = json!(setting.step.unwrap_or(1.0));
            }
            if let Some(other) = &setting.shown_when {
                annotation["when"] = json!({
                    "path": lume_config::source::join_path(&["plugins", local, other.as_str()]),
                    "is": "on",
                });
            }
            schema["x-lume-settings"] = annotation;
            rows.entry((page, heading)).or_default().push(placed(
                &names,
                &setting.key,
                &setting.placement(),
                index,
                (setting.key.clone(), schema),
            ));
        }
        for ((page, _), things) in rows {
            let explicit: BTreeMap<String, i32> = contributes
                .settings
                .iter()
                .filter_map(|setting| setting.order.map(|order| (setting.key.clone(), order)))
                .collect();
            for (position, thing) in arrange(things, &mut warnings).into_iter().enumerate() {
                let (key, mut schema) = thing.value;
                let position = i64::try_from(position).unwrap_or(i64::MAX);
                // On a page of the application's a row goes after its rows unless its
                // order puts it among them; on its own page it goes where it falls.
                let order = match explicit.get(&key) {
                    Some(order) if page != OWN_PAGE => i64::from(*order),
                    _ if page != OWN_PAGE => 1000 + position * 10,
                    _ => (position + 1) * 10,
                };
                schema["x-lume-settings"]["order"] = json!(order);
                properties.insert(key, schema);
            }
        }
        for warning in warnings {
            out.warn(local, warning.message);
        }
        out.settings.fragments.insert(
            local.to_owned(),
            json!({
                "type": "object",
                "properties": properties,
                "additionalProperties": false,
            }),
        );
        if contributes.settings.iter().any(|setting| {
            setting.label.is_some() && setting.page.as_deref().unwrap_or(OWN_PAGE) == OWN_PAGE
        }) {
            let icon = manifest
                .icon
                .as_ref()
                .and_then(|icon| out.images.get(&format!("{local}:{icon}")))
                .map(|image| image.url.clone());
            let mut page = json!({
                "id": page_id(local),
                "label": install.name,
                "icon": "plugin",
                "group": ADD_ONS,
                "blurb": manifest.description.clone().unwrap_or_default(),
                "groups": headings,
                "meta": format!("v{}", manifest.version),
                "plugin": local,
            });
            if let Some(icon) = icon {
                page["image"] = json!(icon);
            }
            out.settings.pages.push(page);
        }
    }
}

/// The paths of the application's settings whose choices plugins add to, and the place
/// or kind of effect their choices come from.
const CHOICES: [(&str, &str); 3] = [
    ("appearance.tab-highlight", "tab.highlight"),
    ("terminal.cursor-trail", "cursor:trail"),
    ("terminal.cursor-pulse", "cursor:pulse"),
];

/// The settings of the effects in the window's places: one for each place but the
/// tab's mark, which has its own setting, under `appearance.motion`.
pub fn motion_setting(slot: &str) -> String {
    slot.replace('.', "-")
}

/// The configuration's schema with the plugins' parts in it: each plugin's table and
/// pages, their themes' and effects' choices among the choices of the settings they
/// are for, and the rows of the places a plugin's motion can play in.
pub fn compose_schema(base: &Value, contributions: &Contributions) -> Value {
    let mut schema = base.clone();
    if let Some(plugins) = schema.pointer_mut("/properties/plugins")
        && let Some(object) = plugins.as_object_mut()
    {
        let properties = object
            .entry("properties")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(properties) = properties.as_object_mut() {
            for (local, fragment) in &contributions.settings.fragments {
                properties.insert(local.clone(), fragment.clone());
            }
        }
    }
    if let Some(Value::Array(pages)) = schema.get_mut(lume_config::settings::PAGES_KEY) {
        pages.extend(contributions.settings.pages.iter().cloned());
    }
    for (path, from) in CHOICES {
        let pointer = format!(
            "/properties/{}/x-lume-settings/options",
            path.replace('.', "/properties/")
        );
        let Some(Value::Array(options)) = schema.pointer_mut(&pointer) else {
            continue;
        };
        let offered: Vec<(String, String)> = match from.strip_prefix("cursor:") {
            Some(kind) => contributions
                .cursor_effects
                .iter()
                .filter(|effect| effect.kind == kind)
                .map(|effect| (effect.id.clone(), effect.title.clone()))
                .collect(),
            None => contributions
                .motions
                .iter()
                .filter(|motion| motion.slot == from)
                .map(|motion| (motion.id.clone(), motion.title.clone()))
                .collect(),
        };
        for (value, label) in offered {
            options.push(json!({ "value": value, "label": label }));
        }
    }
    // The places of motion a plugin offers something for get a row each.
    let mut order = 10;
    for slot in places::MOTION_SLOTS {
        if slot == "tab.highlight" {
            continue;
        }
        let offered: Vec<&MotionOut> = contributions
            .motions
            .iter()
            .filter(|motion| motion.slot == slot)
            .collect();
        if offered.is_empty() {
            continue;
        }
        let pointer = format!(
            "/properties/appearance/properties/motion/properties/{}",
            motion_setting(slot)
        );
        let Some(setting) = schema.pointer_mut(&pointer) else {
            continue;
        };
        let options: Vec<Value> = offered
            .iter()
            .map(|motion| json!({ "value": motion.id, "label": motion.title }))
            .collect();
        setting["x-lume-settings"] = json!({
            "page": "appearance",
            "group": "Motion",
            "order": order,
            "label": slot_label(slot),
            "control": "select",
            "options": options,
            "unset": "Lume's own",
        });
        order += 10;
    }
    schema
}

/// What a place of motion is called in the settings.
fn slot_label(slot: &str) -> &'static str {
    match slot {
        "menu.enter" => "Menus open",
        "toast.enter" => "Notices appear",
        "toast.exit" => "Notices leave",
        "palette.enter" => "Palette opens",
        "panel.enter" => "Panels open",
        "float.enter" => "Floating panes appear",
        "pill.enter" => "Pills of panes put away appear",
        "status.change" => "Status items change",
        _ => "Motion",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testing::TempDir;

    struct Known;

    impl Commands for Known {
        fn builtin(&self, id: &str) -> Option<(String, bool)> {
            match id {
                "lume.tab.new" => Some(("New tab".to_owned(), true)),
                "lume.app.quit" => Some(("Quit Lume".to_owned(), false)),
                _ => None,
            }
        }
    }

    const HEAD: &str =
        "schema = 1\nname = \"P\"\nversion = \"1.2.0\"\napi = \"1\"\nmain = \"a.js\"\n";

    fn manifest(id: &str, rest: &str) -> Manifest {
        Manifest::parse(&format!("{HEAD}id = \"{id}\"\n{rest}"))
            .unwrap_or_else(|problems| panic!("{problems:#?}"))
    }

    fn install<'a>(
        local: &'a str,
        manifest: &'a Manifest,
        dir: &TempDir,
        rank: i64,
        bound: &'a BTreeMap<String, String>,
    ) -> Install<'a> {
        Install {
            local,
            name: manifest.name.clone(),
            manifest,
            dir: dir.path().to_owned(),
            content: "sha256:test",
            rank,
            bound,
        }
    }

    fn keys(nodes: &[MenuNode]) -> Vec<&str> {
        nodes.iter().map(|node| node.key.as_str()).collect()
    }

    #[test]
    fn menus_hold_the_applications_nodes_and_the_plugins_in_their_places() {
        let dir = TempDir::new("menus");
        let one = manifest(
            "acme.tools",
            r#"
[[contributes.commands]]
id = "refresh"
title = "Refresh meters"

[[contributes.sections]]
id = "meters"
location = "menu.app.view"
title = "Meters"
after = "lume:zoom"

[[contributes.menus]]
id = "refresh-item"
location = "menu.app.view"
section = "meters"
command = "refresh"

[[contributes.menus]]
id = "zoom-extra"
location = "menu.app.view"
section = "lume:zoom"
before = "lume:zoom-reset"
command = "lume.tab.new"
title = "Not what it is called"

[[contributes.menus]]
id = "tools"
location = "menu.app"
title = "Tools"
submenu = true
after = "lume:view"

[[contributes.menus]]
id = "in-tools"
location = "menu.app"
parent = "tools"
command = "refresh"
when = "plugin:busy && !terminal"

[[contributes.menus]]
id = "quit"
location = "menu.pane"
command = "lume.app.quit"

[[contributes.menus]]
id = "in-view"
location = "menu.app"
parent = "lume:view"
command = "refresh"
order = 5
"#,
        );
        let bound = BTreeMap::new();
        let installs = [install("acme.tools", &one, &dir, 0, &bound)];
        let out = build(&installs, &Known, &mut Read::default());

        let view = &out.menus["menu.app.view"];
        assert_eq!(
            keys(view),
            [
                "acme.tools:in-view",
                "lume:splits",
                "lume:zoom",
                "acme.tools:meters",
                "lume:panels"
            ]
        );
        assert_eq!(
            keys(&view[2].children),
            [
                "lume:zoom-in",
                "lume:zoom-out",
                "acme.tools:zoom-extra",
                "lume:zoom-reset"
            ]
        );
        assert_eq!(view[2].children[2].title.as_deref(), Some("New tab"));
        assert_eq!(
            view[3].children[0].command.as_deref(),
            Some("acme.tools.refresh")
        );
        assert_eq!(view[3].children[0].title.as_deref(), Some("Refresh meters"));

        let bar = &out.menus["menu.app"];
        assert_eq!(
            keys(bar),
            ["lume:lume", "lume:view", "acme.tools:tools", "lume:help"]
        );
        assert_eq!(keys(&bar[2].children), ["acme.tools:in-tools"]);
        assert_eq!(
            bar[2].children[0].when,
            ["plugin:acme.tools.busy", "!terminal"]
        );

        assert_eq!(keys(&out.menus["menu.pane"]), ["lume:name", "lume:show"]);
        // The application's keys that come again in other menus are in each.
        let tab = &out.menus["menu.tab"];
        let position = tab.iter().find(|node| node.key == "lume:position").unwrap();
        assert_eq!(
            keys(&position.children),
            [
                "lume:new-tab",
                "lume:move-left",
                "lume:move-right",
                "lume:move-into"
            ]
        );
        let lume = &out.menus["menu.app.lume"];
        assert_eq!(keys(&lume[0].children)[1], "lume:new-tab");
        assert!(
            out.warnings
                .iter()
                .any(|(_, warning)| warning.contains("plugins may not run lume.app.quit")),
            "{:?}",
            out.warnings
        );
    }

    #[test]
    fn a_dependencys_things_are_named_through_its_binding() {
        let dir = TempDir::new("bound");
        let core = manifest(
            "acme.core",
            r#"
[[contributes.commands]]
id = "open"
title = "Open the core"

[[contributes.sections]]
id = "core"
location = "menu.tab"
title = "Core"
"#,
        );
        let user = manifest(
            "acme.user",
            r#"
[dependencies]
"acme.core" = "^1"

[[contributes.menus]]
id = "open"
location = "menu.tab"
section = "acme.core:core"
command = "acme.core:open"

[[contributes.menus]]
id = "first"
location = "menu.tab"
section = "acme.core:core"
before = "acme.core:missing"
command = "acme.core:open"
order = 1
"#,
        );
        let none = BTreeMap::new();
        let mut bound = BTreeMap::new();
        bound.insert("acme.core".to_owned(), "fork.core".to_owned());
        let installs = [
            install("fork.core", &core, &dir, 0, &none),
            install("acme.user", &user, &dir, 1, &bound),
        ];
        let out = build(&installs, &Known, &mut Read::default());
        let tab = &out.menus["menu.tab"];
        let section = tab
            .iter()
            .find(|node| node.key == "fork.core:core")
            .unwrap();
        assert_eq!(
            keys(&section.children),
            ["acme.user:first", "acme.user:open"]
        );
        assert_eq!(
            section.children[1].command.as_deref(),
            Some("fork.core.open")
        );
        assert_eq!(section.children[1].title.as_deref(), Some("Open the core"));
        assert!(out.warnings.iter().any(
            |(plugin, warning)| plugin == "acme.user" && warning.contains("fork.core:missing")
        ));
    }

    #[test]
    fn status_items_and_panels_are_placed_across_plugins() {
        let dir = TempDir::new("status");
        dir.write(
            "icons/cpu.svg",
            "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
        );
        let one = manifest(
            "acme.meters",
            r#"
activation = ["onView:cpu"]

[[contributes.images]]
id = "chip"
file = "icons/cpu.svg"
tint = true

[[contributes.status]]
id = "cpu"
slot = "topbar.right"
kind = "meter"
label = "CPU"
icon = "chip"
menu = "options"

[[contributes.status]]
id = "net"
slot = "topbar.right"
kind = "text"
icon = "wifi"
after = "cpu"
separator = true

[[contributes.menus]]
id = "options"
location = "menu.status"
title = "Options"
submenu = true

[[contributes.panels]]
id = "procs"
title = "Processes"
kind = "list"
area = "right"
"#,
        );
        let two = manifest(
            "acme.other",
            r#"
[[contributes.status]]
id = "first"
slot = "topbar.right"
kind = "badge"
order = 1
"#,
        );
        let bound = BTreeMap::new();
        let installs = [
            install("acme.meters", &one, &dir, 0, &bound),
            install("acme.other", &two, &dir, 1, &bound),
        ];
        let out = build(&installs, &Known, &mut Read::default());
        let ids: Vec<&str> = out.status.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(
            ids,
            ["acme.other.first", "acme.meters.cpu", "acme.meters.net"]
        );
        // A rule is drawn after the item that asks for one, and after no other.
        assert!(out.status[2].separator && !out.status[1].separator);
        let sent = serde_json::to_value(&out.status).unwrap();
        assert_eq!(sent[2]["separator"], true);
        assert!(sent[1].get("separator").is_none());
        assert_eq!(
            out.status[1].icon,
            Some(Icon::Image("acme.meters:chip".to_owned()))
        );
        assert_eq!(out.status[2].icon, Some(Icon::Glyph("wifi".to_owned())));
        assert_eq!(out.status[1].menu.as_deref(), Some("acme.meters:options"));
        assert!(out.images["acme.meters:chip"].tint);
        assert!(
            out.images["acme.meters:chip"]
                .url
                .starts_with("data:image/svg+xml;base64,")
        );
        assert_eq!(out.panels[0].id, "acme.meters.procs");
        assert_eq!(keys(&out.menus["menu.status"]), ["acme.meters:options"]);

        // A window is told a picture's hash, not the picture, and has it for the asking.
        let chip = &out.images["acme.meters:chip"];
        let sent = out.to_json();
        assert_eq!(sent["images"]["acme.meters:chip"]["hash"], chip.hash);
        assert!(sent["images"]["acme.meters:chip"].get("url").is_none());
        assert_eq!(out.asset(&chip.hash), Some(chip.url.as_str()));
        assert_eq!(out.asset("none"), None);
    }

    #[test]
    fn a_package_is_read_again_only_when_it_is_another_one() {
        let dir = TempDir::new("read-again");
        dir.write(
            "icons/chip.svg",
            "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
        );
        let one = manifest(
            "acme.meters",
            "[[contributes.images]]\nid = \"chip\"\nfile = \"icons/chip.svg\"\n",
        );
        let bound = BTreeMap::new();
        let mut read = Read::default();
        let installs = [install("acme.meters", &one, &dir, 0, &bound)];
        let first = build(&installs, &Known, &mut read).images["acme.meters:chip"].clone();
        assert_eq!(read.len(), 1);

        // The same package: the file is not read again, whatever is in it now.
        dir.write(
            "icons/chip.svg",
            "<svg xmlns=\"http://www.w3.org/2000/svg\"><g/></svg>",
        );
        let again = build(&installs, &Known, &mut read).images["acme.meters:chip"].clone();
        assert_eq!(again, first);

        // Another package under the same name: read anew.
        let mut other = install("acme.meters", &one, &dir, 0, &bound);
        other.content = "sha256:other";
        let changed = build(&[other], &Known, &mut read).images["acme.meters:chip"].clone();
        assert_ne!(changed.hash, first.hash);

        // An install that went is forgotten.
        build(&[], &Known, &mut read);
        assert!(read.is_empty());
    }

    #[test]
    fn a_status_item_a_setting_moves_keeps_its_place_among_the_bars_items() {
        let dir = TempDir::new("moved");
        let one = manifest(
            "acme.meters",
            r#"
[[contributes.status]]
id = "cpu"
slot-setting = "place"
kind = "meter"
label = "CPU"
order = 10

[[contributes.status]]
id = "net"
slot = "topbar.right"
kind = "text"
after = "cpu"

[[contributes.status]]
id = "branch"
slot = "statusbar.left"
kind = "text"

[[contributes.settings]]
key = "place"
type = "string"
default = "statusbar.right"
choices = [
  { value = "topbar.right", label = "Top bar" },
  { value = "statusbar.right", label = "Status bar" },
]
"#,
        );
        let bound = BTreeMap::new();
        let installs = [install("acme.meters", &one, &dir, 0, &bound)];
        let out = build(&installs, &Known, &mut Read::default());
        let placed: Vec<(&str, &str, usize)> = out
            .status
            .iter()
            .map(|item| (item.id.as_str(), item.slot.as_str(), item.order))
            .collect();
        // One order for the bars: the meter comes before what follows it wherever it
        // goes, and is where its setting puts it until the user moves it.
        assert_eq!(
            placed,
            [
                ("acme.meters.cpu", "statusbar.right", 0),
                ("acme.meters.net", "topbar.right", 1),
                ("acme.meters.branch", "statusbar.left", 2),
            ]
        );
        assert_eq!(
            out.status[0].slot_setting,
            Some(SlotSetting {
                key: "place".to_owned(),
                slots: vec!["topbar.right".to_owned(), "statusbar.right".to_owned()],
            })
        );
        assert_eq!(out.status[2].slot_setting, None);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    }

    #[test]
    fn settings_become_part_of_the_schema_with_their_rows() {
        let dir = TempDir::new("settings");
        let one = manifest(
            "acme.meters",
            r#"
description = "Meters in the top bar."

[[contributes.setting-groups]]
id = "meters"
title = "Status bar meters"

[[contributes.setting-groups]]
id = "sampling"
title = "Sampling"

[[contributes.settings]]
key = "cpu"
type = "boolean"
default = true
label = "CPU"
group = "meters"

[[contributes.settings]]
key = "rate"
type = "integer"
default = 2
choices = [{ value = 1, label = "1 s" }, { value = 2, label = "2 s" }]
label = "Refresh every"
group = "sampling"
shown-when = "cpu"

[[contributes.settings]]
key = "glow"
type = "boolean"
default = false
label = "Glow"
page = "terminal"
group = "lume:cursor"

[[contributes.settings]]
key = "secret"
type = "string"
default = ""
"#,
        );
        let bound = BTreeMap::new();
        let installs = [install("acme.meters", &one, &dir, 0, &bound)];
        let out = build(&installs, &Known, &mut Read::default());
        let fragment = &out.settings.fragments["acme.meters"];
        assert_eq!(fragment["additionalProperties"], false);
        assert_eq!(fragment["properties"]["rate"]["enum"], json!([1, 2]));
        let rate = &fragment["properties"]["rate"]["x-lume-settings"];
        assert_eq!(rate["page"], "addon:acme.meters");
        assert_eq!(rate["group"], "Sampling");
        assert_eq!(rate["control"], "select");
        assert_eq!(rate["when"]["path"], "plugins.\"acme.meters\".cpu");
        let glow = &fragment["properties"]["glow"]["x-lume-settings"];
        assert_eq!(glow["page"], "terminal");
        assert_eq!(glow["group"], "Cursor");
        assert_eq!(glow["order"], 1000);
        assert_eq!(glow["plugin"], "P");
        assert!(
            fragment["properties"]["secret"]
                .get("x-lume-settings")
                .is_none()
        );
        let page = &out.settings.pages[0];
        assert_eq!(page["groups"], json!(["Status bar meters", "Sampling"]));
        assert_eq!(page["group"], ADD_ONS);
        assert_eq!(page["meta"], "v1.2.0");

        let base = lume_config::config_schema();
        let composed = compose_schema(base, &out);
        assert_eq!(
            composed["properties"]["plugins"]["properties"]["acme.meters"],
            *fragment
        );
        let pages = composed[lume_config::settings::PAGES_KEY]
            .as_array()
            .unwrap();
        assert_eq!(pages.last().unwrap()["id"], "addon:acme.meters");
    }

    #[test]
    fn effects_shaders_and_themes_are_read_from_the_package() {
        let dir = TempDir::new("effects");
        dir.write(
            "fx/glow.glsl",
            "vec4 lume_effect(vec2 p) { return vec4(lume_color, 1.0 - lume_progress); }",
        );
        dir.write(
            "fx/bad.glsl",
            "vec4 lume_effect(vec2 p) { while (true) {} }",
        );
        dir.write(
            "themes/night.toml",
            "name = \"Night\"\ninherits = \"lume-dark\"\n",
        );
        let one = manifest(
            "acme.fx",
            r#"
[[contributes.cursor-effects]]
id = "glow"
title = "Glow"
kind = "pulse"
shader = "fx/glow.glsl"

[[contributes.cursor-effects]]
id = "bad"
title = "Bad"
kind = "trail"
shader = "fx/bad.glsl"

[[contributes.motions]]
id = "pop"
title = "Pop"
slot = "menu.enter"
duration = 160
keyframes = [{ opacity = 0, transform = "scale(0.96)" }, { opacity = 1 }]

[[contributes.themes]]
id = "night"
file = "themes/night.toml"
"#,
        );
        let bound = BTreeMap::new();
        let installs = [install("acme.fx", &one, &dir, 0, &bound)];
        let out = build(&installs, &Known, &mut Read::default());
        assert_eq!(out.cursor_effects.len(), 1);
        assert_eq!(out.cursor_effects[0].id, "acme.fx/glow");
        assert_eq!(out.cursor_effects[0].duration, 400);
        assert!(
            out.warnings
                .iter()
                .any(|(_, warning)| warning.contains("bad"))
        );
        assert_eq!(out.motions[0].id, "acme.fx/pop");
        assert_eq!(out.themes[0].id, "acme.fx/night");
        assert_eq!(out.themes[0].name, "Night");

        let composed = compose_schema(lume_config::config_schema(), &out);
        let pulses = composed
            .pointer("/properties/terminal/properties/cursor-pulse/x-lume-settings/options")
            .unwrap();
        assert!(
            pulses
                .as_array()
                .unwrap()
                .iter()
                .any(|option| option["value"] == "acme.fx/glow")
        );
        let menus = composed
            .pointer(
                "/properties/appearance/properties/motion/properties/menu-enter/x-lume-settings",
            )
            .unwrap();
        assert_eq!(menus["options"][0]["value"], "acme.fx/pop");
        assert_eq!(menus["unset"], "Lume's own");
    }

    #[test]
    fn a_table_setting_is_an_open_object_and_has_no_row() {
        let dir = TempDir::new("contributions-table");
        let one = manifest(
            "acme.tables",
            r#"
[[contributes.settings]]
key = "shown"
type = "boolean"
default = true
label = "Shown"

[[contributes.settings]]
key = "sources"
type = "table"
default = { local = { path = "." } }
label = "Sources"
help = "By name."
"#,
        );
        let bound = BTreeMap::new();
        let installs = [install("acme.tables", &one, &dir, 0, &bound)];
        let out = build(&installs, &Known, &mut Read::default());
        let fragment = &out.settings.fragments["acme.tables"];
        let sources = &fragment["properties"]["sources"];
        assert_eq!(sources["type"], "object");
        assert_eq!(sources["default"], json!({ "local": { "path": "." } }));
        assert_eq!(sources["description"], "By name.");
        assert!(sources.get("x-lume-settings").is_none(), "{sources}");
        assert!(sources.get("properties").is_none(), "{sources}");
        assert!(
            fragment["properties"]["shown"]
                .get("x-lume-settings")
                .is_some()
        );
        assert_eq!(out.warnings, Vec::new());
    }

    #[test]
    fn a_verb_belongs_to_the_first_plugin_that_adds_it() {
        let dir = TempDir::new("contributions-verbs");
        let verbs = r#"
[[contributes.commands]]
id = "run"
title = "Run"

[[contributes.cli]]
verb = "agent run"
command = "run"
summary = "Runs once"
usage = "lume agent run <prompt>"

[[contributes.cli]]
verb = "agent"
command = "run"
summary = "Says what there is"
"#;
        let one = manifest("acme.one", verbs);
        let two = manifest("acme.two", verbs);
        let bound = BTreeMap::new();
        let installs = [
            install("acme.one", &one, &dir, 0, &bound),
            install("acme.two", &two, &dir, 1, &bound),
        ];
        let out = build(&installs, &Known, &mut Read::default());
        assert_eq!(out.verbs.len(), 2);
        assert_eq!(out.verbs[0].verb, "agent run");
        assert_eq!(out.verbs[0].words, ["agent", "run"]);
        assert_eq!(out.verbs[0].plugin, "acme.one");
        assert_eq!(out.verbs[0].command, "acme.one.run");
        assert_eq!(out.verbs[1].verb, "agent");
        assert_eq!(
            out.warnings,
            [
                (
                    "acme.two".to_owned(),
                    "the verb `agent run` is not added: acme.one adds it already".to_owned()
                ),
                (
                    "acme.two".to_owned(),
                    "the verb `agent` is not added: acme.one adds it already".to_owned()
                ),
            ]
        );
        assert_eq!(
            serde_json::to_value(&out.verbs[1]).unwrap(),
            json!({
                "verb": "agent",
                "plugin": "acme.one",
                "command": "acme.one.run",
                "summary": "Says what there is",
            })
        );
    }
}
