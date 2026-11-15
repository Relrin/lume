//! The settings: their names, their types and their defaults.
//!
//! These types are the definition. The JSON Schema that editors complete from, the
//! check a file is held to, and the file a new user is given are all made from them.
//!
//! A newer file must still load in an older build, so a key these types do not know
//! is reported and ignored rather than refused. That is why none of them rejects
//! unknown fields: the check against the schema finds such keys first, with the line
//! they are on.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::paths::Platform;
use crate::tuning::{FONT_SIZE_MAX, FONT_SIZE_MIN, SCROLLBACK_LINES_MAX};

/// Everything that can be set in `config.toml`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    /// The application as a whole.
    pub app: AppConfig,
    /// The application's windows.
    pub window: WindowConfig,
    /// Workspaces.
    pub workspace: WorkspaceConfig,
    /// How things look: the theme and what overrides it.
    pub appearance: AppearanceConfig,
    /// The fonts.
    pub font: FontConfig,
    /// The terminals.
    pub terminal: TerminalConfig,
    /// Where things are placed in a window.
    pub layout: LayoutConfig,
    /// How terminals are drawn.
    pub renderer: RendererConfig,
    /// Keys and the commands they run. A key is a chord such as `"ctrl+shift+t"`, or a
    /// sequence of chords separated by spaces; its value is a command's id, a table
    /// with `command`, `when` and `args`, or `"unbound"` to take a default away.
    pub keybindings: Keybindings,
    /// Settings of plugins, by plugin id.
    pub plugins: BTreeMap<String, Value>,
}

/// The application as a whole.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case")]
pub struct AppConfig {
    /// Keep the application running when its last window closes.
    pub keep_running: bool,
    /// The workspace opened at start: a workspace's name, or `"last"` for the one
    /// that was open when the application closed.
    pub start_workspace: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            keep_running: false,
            start_workspace: "last".to_owned(),
        }
    }
}

/// Who draws a window's title bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TitleBar {
    /// The application draws it, as part of the top bar.
    Custom,
    /// The window system draws it.
    System,
}

/// When closing something asks first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Confirm {
    /// Never ask.
    #[default]
    Never,
    /// Always ask.
    Always,
}

/// The application's windows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case")]
pub struct WindowConfig {
    /// Who draws the title bar. Left out, the application draws it on Windows and
    /// macOS, and the window system does on Linux.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_bar: Option<TitleBar>,
    /// Close a window when its last tab closes.
    pub close_with_last_tab: bool,
    /// Whether closing a window with running programs asks first.
    pub confirm_close: Confirm,
    /// Open windows where they were when the workspace was last open.
    pub restore_bounds: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title_bar: None,
            close_with_last_tab: true,
            confirm_close: Confirm::Never,
            restore_bounds: true,
        }
    }
}

impl WindowConfig {
    /// Who draws the title bar on `platform`.
    pub fn title_bar_on(&self, platform: Platform) -> TitleBar {
        self.title_bar.unwrap_or(match platform {
            Platform::Windows | Platform::Macos => TitleBar::Custom,
            // Window managers differ in how they treat a window without a frame, so
            // the frame is theirs unless the user says otherwise.
            Platform::Linux => TitleBar::System,
        })
    }
}

/// What happens to a workspace's programs when another workspace is opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum OnSwitch {
    /// They are ended.
    #[default]
    Close,
    /// They keep running. Not available yet: programs are ended as with `close`.
    Keep,
}

/// Workspaces.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case")]
pub struct WorkspaceConfig {
    /// What happens to a workspace's programs when another workspace is opened.
    pub on_switch: OnSwitch,
}

/// When a pane shows its header.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PaneHeader {
    /// Always.
    #[default]
    Always,
    /// Only in a tab with more than one pane.
    Auto,
    /// Never.
    Never,
}

/// How things look.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case")]
pub struct AppearanceConfig {
    /// The theme to use whatever the system's colour scheme is. Left out, the theme
    /// follows the system: `theme-light` or `theme-dark`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// The theme used when the system's colour scheme is light.
    pub theme_light: String,
    /// The theme used when the system's colour scheme is dark.
    pub theme_dark: String,
    /// The radius of a pane's corners, in pixels. Left out, the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 64.0))]
    pub card_radius: Option<f64>,
    /// The space between panes and around them, in pixels. Left out, the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 64.0))]
    pub gap: Option<f64>,
    /// When a pane shows its header.
    pub pane_header: PaneHeader,
    /// How much a pane without the focus is dimmed, from 0 (not at all) to 1. Left
    /// out, the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub dim_unfocused: Option<f64>,
    /// A picture shown behind the panes: the path of an image file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_image: Option<String>,
    /// How much the picture is blurred, in pixels. Left out, the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 200.0))]
    pub background_blur: Option<f64>,
    /// How opaque the picture is, from 0 to 1. Left out, the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub background_opacity: Option<f64>,
    /// Design tokens set over the theme's, written as a theme file writes them: a
    /// table for each group, such as `[appearance.tokens.color]` with `accent = "#ff8800"`.
    pub tokens: BTreeMap<String, Value>,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: None,
            theme_light: "lume-light".to_owned(),
            theme_dark: "lume-dark".to_owned(),
            card_radius: None,
            gap: None,
            pane_header: PaneHeader::Always,
            dim_unfocused: None,
            background_image: None,
            background_blur: None,
            background_opacity: None,
            tokens: BTreeMap::new(),
        }
    }
}

/// The fonts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case")]
pub struct FontConfig {
    /// The font terminals are drawn in.
    pub family: String,
    /// The fonts tried, in order, for a character the first has no glyph for, or when
    /// it is not installed.
    pub fallback: Vec<String>,
    /// The size of terminal text, in pixels.
    #[schemars(range(min = FONT_SIZE_MIN, max = FONT_SIZE_MAX))]
    pub size: f64,
    /// The height of a line of terminal text, as a multiple of the font's size.
    #[schemars(range(min = 0.8, max = 3.0))]
    pub line_height: f64,
    /// The weight of ordinary text, as fonts number weights: 400 is regular.
    #[schemars(range(min = 1, max = 1000))]
    pub weight: u16,
    /// The weight of bold text.
    #[schemars(range(min = 1, max = 1000))]
    pub bold_weight: u16,
    /// The font of everything that is not a terminal: tabs, headers, menus.
    pub ui_family: String,
    /// The size of that text, in pixels.
    #[schemars(range(min = 8.0, max = 32.0))]
    pub ui_size: f64,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: "JetBrains Mono".to_owned(),
            fallback: [
                "Cascadia Mono",
                "Consolas",
                "Menlo",
                "DejaVu Sans Mono",
                "monospace",
            ]
            .map(str::to_owned)
            .to_vec(),
            size: 14.0,
            line_height: 1.2,
            weight: 400,
            bold_weight: 700,
            ui_family: "system-ui".to_owned(),
            ui_size: 13.0,
        }
    }
}

/// The shape of the cursor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CursorStyle {
    /// A filled cell.
    #[default]
    Block,
    /// A line before the character.
    Bar,
    /// A line under the character.
    Underline,
}

/// What happens to a pane when its program ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CloseOnExit {
    /// The pane closes if the program ended without an error, and stays otherwise.
    #[default]
    Clean,
    /// The pane closes however the program ended.
    Always,
    /// The pane stays, and says how the program ended.
    Never,
}

/// The terminals.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case")]
pub struct TerminalConfig {
    /// The program a new terminal runs. Empty, it is the user's own shell.
    pub shell: String,
    /// The arguments that program is started with.
    pub shell_args: Vec<String>,
    /// How many lines that have scrolled off the top are kept.
    #[schemars(range(min = 0, max = SCROLLBACK_LINES_MAX))]
    pub scrollback_lines: u32,
    /// The cursor's shape, until a program chooses another.
    pub cursor_style: CursorStyle,
    /// Whether the cursor blinks, until a program says otherwise.
    pub cursor_blink: bool,
    /// The space between a pane's edge and its text, in pixels. Left out, the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 64.0))]
    pub padding: Option<f64>,
    /// What happens to a pane when its program ends.
    pub close_on_exit: CloseOnExit,
    /// Whether closing a pane asks first.
    pub confirm_close: Confirm,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            shell: String::new(),
            shell_args: Vec::new(),
            scrollback_lines: 10_000,
            cursor_style: CursorStyle::Block,
            cursor_blink: true,
            padding: None,
            close_on_exit: CloseOnExit::Clean,
            confirm_close: Confirm::Never,
        }
    }
}

/// Where the list of tabs is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TabPlacement {
    /// In the bar at the top of the window.
    #[default]
    Top,
    /// In a panel on the left.
    Left,
    /// In a panel on the right.
    Right,
}

/// The directory a new pane's program starts in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum NewPaneCwd {
    /// The workspace's folder, or the home directory if it has none.
    #[default]
    Workspace,
    /// The home directory.
    Home,
}

/// Where things are placed in a window.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case")]
pub struct LayoutConfig {
    /// Where the list of tabs is.
    pub tab_placement: TabPlacement,
    /// The directory a new pane's program starts in.
    pub new_pane_cwd: NewPaneCwd,
}

/// What a terminal is drawn with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RendererTier {
    /// The best the machine offers.
    #[default]
    Auto,
    Webgpu,
    Webgl2,
    Canvas2d,
}

/// How terminals are drawn.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case")]
pub struct RendererConfig {
    /// What terminals are drawn with. Anything but `auto` is for finding out why
    /// drawing goes wrong on a machine.
    pub tier: RendererTier,
}

/// One binding of a key, written out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub struct BindingEntry {
    /// The id of the command the key runs.
    pub command: String,
    /// Where the key does so: context names joined by `&&`, each of which may be
    /// negated with `!`. Left out, everywhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// What the command is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
}

/// What a key is bound to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum BindingValue {
    /// A command's id, or `"unbound"`.
    Command(String),
    /// A command with a context or arguments.
    Entry(BindingEntry),
    /// Several, for a key that does different things in different contexts.
    Entries(Vec<BindingEntry>),
}

/// Keys and what they run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Keybindings {
    /// Bindings that hold on Windows only.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub windows: BTreeMap<String, BindingValue>,
    /// Bindings that hold on macOS only.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub macos: BTreeMap<String, BindingValue>,
    /// Bindings that hold on Linux only.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub linux: BTreeMap<String, BindingValue>,
    /// Bindings that hold everywhere.
    #[serde(flatten)]
    pub all: BTreeMap<String, BindingValue>,
}

impl Keybindings {
    /// The bindings that hold on one platform only.
    pub fn on(&self, platform: Platform) -> &BTreeMap<String, BindingValue> {
        match platform {
            Platform::Windows => &self.windows,
            Platform::Macos => &self.macos,
            Platform::Linux => &self.linux,
        }
    }
}

/// The value that takes a default binding away.
pub const UNBOUND: &str = "unbound";

/// The sections a workspace's file may not set: they are about the application, not
/// about one workspace.
pub const NOT_IN_A_WORKSPACE: [&str; 2] = ["app", "workspace"];

/// What a workspace's file says besides settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct WorkspaceSettings {
    /// The workspace's folder, set by the user.
    pub root: Option<String>,
    /// The profile new terminals in the workspace use.
    pub default_profile: Option<String>,
}

/// The keys a workspace's file has at its top besides the sections.
pub const WORKSPACE_ROOT_KEY: &str = "root";
pub const WORKSPACE_PROFILE_KEY: &str = "default-profile";

fn generate() -> Value {
    let settings = SchemaSettings::draft2020_12().with(|settings| {
        // One self-contained tree: whoever walks it needs no references resolved.
        settings.inline_subschemas = true;
    });
    let mut schema = settings
        .into_generator()
        .into_root_schema_for::<Config>()
        .to_value();
    if let Some(object) = schema.as_object_mut() {
        object.insert("title".to_owned(), Value::from("Lume configuration"));
        object.insert(
            "description".to_owned(),
            Value::from("The settings of config.toml."),
        );
    }
    schema
}

/// The JSON Schema of `config.toml`. It is self-contained: nothing in it refers to
/// another part of it.
pub fn config_schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(generate)
}

/// The file a new user is given: every setting, commented out, at its default.
pub fn default_config_text() -> &'static str {
    include_str!("default-config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_ones_the_application_is_built_to() {
        let config = Config::default();
        assert_eq!(config.font.family, "JetBrains Mono");
        assert_eq!(config.font.fallback[0], "Cascadia Mono");
        assert_eq!(config.font.fallback.last().unwrap(), "monospace");
        assert_eq!(config.terminal.close_on_exit, CloseOnExit::Clean);
        assert_eq!(config.workspace.on_switch, OnSwitch::Close);
        assert_eq!(config.appearance.theme, None);
        assert_eq!(config.appearance.theme_dark, "lume-dark");
        assert_eq!(config.layout.tab_placement, TabPlacement::Top);
        assert!(config.keybindings.all.is_empty());
    }

    #[test]
    fn the_title_bar_is_the_window_systems_on_linux_unless_the_user_says_otherwise() {
        let window = WindowConfig::default();
        assert_eq!(window.title_bar_on(Platform::Windows), TitleBar::Custom);
        assert_eq!(window.title_bar_on(Platform::Macos), TitleBar::Custom);
        assert_eq!(window.title_bar_on(Platform::Linux), TitleBar::System);
        let chosen = WindowConfig {
            title_bar: Some(TitleBar::Custom),
            ..WindowConfig::default()
        };
        assert_eq!(chosen.title_bar_on(Platform::Linux), TitleBar::Custom);
    }

    #[test]
    fn the_defaults_round_trip_through_json() {
        let config = Config::default();
        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(json["font"]["size"], 14.0);
        assert_eq!(json["terminal"]["close-on-exit"], "clean");
        // A setting with no default is left out rather than written as nothing.
        assert!(json["appearance"].get("theme").is_none());
        let back: Config = serde_json::from_value(json).unwrap();
        assert_eq!(back, config);
    }

    #[test]
    fn keybindings_keep_the_platform_tables_apart_from_the_keys() {
        let json = serde_json::json!({
            "ctrl+shift+t": "lume.tab.new",
            "ctrl+a c": { "command": "lume.tab.new", "when": "terminal" },
            "macos": { "cmd+d": "lume.pane.split-right" },
        });
        let keys: Keybindings = serde_json::from_value(json).unwrap();
        assert_eq!(keys.all.len(), 2);
        assert_eq!(
            keys.all["ctrl+shift+t"],
            BindingValue::Command("lume.tab.new".to_owned())
        );
        assert_eq!(keys.on(Platform::Macos).len(), 1);
        assert!(keys.on(Platform::Linux).is_empty());
    }

    #[test]
    fn the_schema_describes_every_section_without_references() {
        let schema = config_schema();
        let sections = schema["properties"].as_object().unwrap();
        for section in [
            "app",
            "window",
            "workspace",
            "appearance",
            "font",
            "terminal",
            "layout",
            "renderer",
            "keybindings",
            "plugins",
        ] {
            assert!(sections.contains_key(section), "{section}");
        }
        assert!(!schema.to_string().contains("$ref"));
        assert_eq!(
            schema["properties"]["font"]["properties"]["size"]["minimum"],
            FONT_SIZE_MIN
        );
        assert_eq!(
            schema["properties"]["terminal"]["properties"]["scrollback-lines"]["maximum"],
            SCROLLBACK_LINES_MAX
        );
    }
}
