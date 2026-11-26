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
use crate::profiles::ProfileEntry;
use crate::rex::RexConfig;
use crate::settings::{PAGES_KEY, Shown, pages};
use crate::tuning::{
    ACCENT_PRESETS, BACKGROUND_BLUR_MAX, CURSOR_EFFECT_LENGTH_MAX, CURSOR_EFFECT_LENGTH_MIN,
    FONT_SIZE_MAX, FONT_SIZE_MIN, HIDDEN_RELEASE_SECONDS_DEFAULT, HIDDEN_RELEASE_SECONDS_MAX,
    HIDDEN_RELEASE_SECONDS_MIN, HISTORY_RELEASE_SECONDS_DEFAULT, HISTORY_RELEASE_SECONDS_MAX,
    HISTORY_RELEASE_SECONDS_MIN, HOST_IDLE_EXIT_SECONDS_DEFAULT, HOST_IDLE_EXIT_SECONDS_MAX,
    LINE_HEIGHT_MAX, LINE_HEIGHT_MIN, LONG_COMMAND_SECONDS_DEFAULT, LONG_COMMAND_SECONDS_MAX,
    LONG_COMMAND_SECONDS_MIN, PANE_LENGTH_MAX, SCROLLBACK_LINES_MAX, SETTINGS_STEP_EFFECT_LENGTH,
    SETTINGS_STEP_LENGTH, SETTINGS_STEP_LINE_HEIGHT, SETTINGS_STEP_LINES, SETTINGS_STEP_PERCENT,
    SETTINGS_STEP_SECONDS, UI_FONT_SIZE_MAX, UI_FONT_SIZE_MIN, WINDOW_HISTORY_LINES_DEFAULT,
    WINDOW_HISTORY_LINES_MAX, WINDOW_HISTORY_LINES_MIN,
};

/// The two answers to whether closing asks first, as the settings window calls them.
const CONFIRM_CHOICES: [(&str, &str); 2] = [("never", "Never"), ("always", "Always")];

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
    /// The session host, which runs the programs in the terminals.
    pub host: HostSettings,
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
    /// Terminals on other machines.
    pub ssh: SshConfig,
    /// Rex servers, whose sessions can be shown.
    pub rex: RexConfig,
    /// The profile new terminals use, by its id. Empty, new terminals run the shell of
    /// `terminal.shell`.
    #[schemars(extend("x-lume-settings" = Shown::select("terminal", "New terminals", 10, "Default terminal")
        .help("Used for new terminal tabs and panes. A workspace's own choice comes first")
        .list("profiles")
        .unset("System default")))]
    pub default_profile: String,
    /// Ways to start a terminal. Each is a `[[profile]]` table with an `id`; one with
    /// the id of a profile the application made changes that profile.
    #[serde(rename = "profile", skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<ProfileEntry>,
}

/// Terminals on other machines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case")]
pub struct SshConfig {
    /// Let terminals to one machine share a connection, so that only the first has to
    /// sign in. It applies to the hosts listed from the OpenSSH configuration, and
    /// only where the OpenSSH client can do it, which the one that comes with Windows
    /// cannot.
    pub connection_sharing: bool,
    /// How long a shared connection is kept after its last terminal closes, as
    /// OpenSSH's `ControlPersist` takes it: `10m`, `yes`, `no`.
    pub control_persist: String,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            connection_sharing: true,
            control_persist: "10m".to_owned(),
        }
    }
}

/// The application as a whole.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case")]
pub struct AppConfig {
    /// Keep the application running when its last window closes: the window is hidden
    /// with a fresh shell started in each of its panes, and opening Lume shows it again
    /// at once. Quitting ends it.
    #[schemars(extend("x-lume-settings" = Shown::switch("general", "Startup", 20, "Keep Lume ready in the background")
        .help("Closing the last window hides it with fresh shells, so Lume opens again at once")))]
    pub keep_running: bool,
    /// The workspace opened at start: a workspace's name, or `"last"` for the one
    /// that was open when the application closed.
    #[schemars(extend("x-lume-settings" = Shown::select("general", "Startup", 10, "On launch")
        .help("The workspace Lume opens with")
        .options(&[("last", "Last workspace")])
        .list("workspaces")))]
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
    #[schemars(extend("x-lume-settings" = Shown::segmented("general", "Windows", 10, "Title bar")
        .help("Who draws the bar at the top of a window")
        .options(&[("custom", "Lume"), ("system", "System")])))]
    pub title_bar: Option<TitleBar>,
    /// Close a window when its last tab closes.
    #[schemars(extend("x-lume-settings" = Shown::switch("general", "Windows", 20, "Close a window with its last tab")))]
    pub close_with_last_tab: bool,
    /// Whether closing a window with running programs asks first.
    #[schemars(extend("x-lume-settings" = Shown::select("general", "Closing", 10, "Confirm before closing a window")
        .help("Ask before closing a window while programs are still running in it")
        .options(&CONFIRM_CHOICES)))]
    pub confirm_close: Confirm,
    /// Open windows where they were when the workspace was last open.
    #[schemars(extend("x-lume-settings" = Shown::switch("general", "Windows", 30, "Restore window positions")
        .help("Open windows where they were when the workspace was last open")))]
    pub restore_bounds: bool,
    /// Ask for attention when a program rings the bell in a window that is not in use:
    /// the taskbar button flashes, or the dock icon bounces.
    #[schemars(extend("x-lume-settings" = Shown::switch("notifications", "Notify me when", 10, "Flash the window on a bell")
        .help("When a program rings the bell in a window you are not using")))]
    pub bell_attention: bool,
    /// How round the corners of a window's own frame are, where the window system lets
    /// the application choose: Windows 11.
    #[schemars(extend("x-lume-settings" = Shown::segmented("appearance", "Panes", 12, "Window corners")
        .help("The corners of the window itself")
        .options(&[("round", "Round"), ("small", "Small"), ("square", "Square")])
        .only_on("windows")))]
    pub corners: WindowCorners,
}

/// How round the corners of a window's own frame are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum WindowCorners {
    /// As round as the window system makes them.
    #[default]
    Round,
    /// Rounded a little.
    Small,
    /// Not rounded.
    Square,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title_bar: None,
            close_with_last_tab: true,
            confirm_close: Confirm::Never,
            restore_bounds: true,
            bell_attention: true,
            corners: WindowCorners::Round,
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
    /// They keep running, and the workspace's panes show them again when it is opened
    /// again.
    Keep,
}

/// Where the session host runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HostMode {
    /// In a process of its own, which keeps the programs running when the application
    /// closes, crashes or is updated; the application shows them again when it starts.
    Daemon,
    /// Inside the application: the programs end when it closes.
    #[default]
    InProcess,
}

/// The session host, which runs the programs in the terminals.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case")]
pub struct HostSettings {
    /// Where the session host runs. Read when the application starts.
    #[schemars(extend("x-lume-settings" = Shown::select("general", "Sessions", 10, "Keep programs running")
        .help("After Lume closes, programs keep running and come back when it starts again. Takes effect the next time Lume starts")
        .options(&[("in-process", "Only while Lume runs"), ("daemon", "After Lume closes")])))]
    pub mode: HostMode,
    /// How long, in seconds, the session host stays with no window and no running
    /// program before it exits, when programs are kept running after the application
    /// closes. Zero keeps it running.
    #[schemars(range(max = HOST_IDLE_EXIT_SECONDS_MAX))]
    #[schemars(extend("x-lume-settings" = Shown::number("general", "Sessions", 20, "Session host exits when idle after", SETTINGS_STEP_SECONDS)
        .help("With no window open and no program running. 0 keeps it running")
        .seconds()))]
    pub idle_exit: u32,
}

impl Default for HostSettings {
    fn default() -> Self {
        Self {
            mode: HostMode::InProcess,
            idle_exit: HOST_IDLE_EXIT_SECONDS_DEFAULT,
        }
    }
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
    #[schemars(extend("x-lume-settings" = Shown::select("appearance", "Theme", 10, "Theme")
        .help("Colours for every window, terminal and pane")
        .list("themes")
        .unset("Follow system")))]
    pub theme: Option<String>,
    /// The theme used when the system's colour scheme is light.
    #[schemars(extend("x-lume-settings" = Shown::select("appearance", "Theme", 20, "In light mode")
        .help("The theme used while the system is light")
        .list("themes")
        .when_unset("appearance.theme")))]
    pub theme_light: String,
    /// The theme used when the system's colour scheme is dark.
    #[schemars(extend("x-lume-settings" = Shown::select("appearance", "Theme", 30, "In dark mode")
        .help("The theme used while the system is dark")
        .list("themes")
        .when_unset("appearance.theme")))]
    pub theme_dark: String,
    /// The colour focus, the selected tab, selected text and the window's glow are
    /// drawn in: `mint`, `blue`, `lilac`, `amber`, `coral` or `graphite`, each of which
    /// suits both dark and light themes, or a colour of your own written `#rrggbb`.
    /// Left out, the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("x-lume-format" = "accent"))]
    #[schemars(extend("x-lume-settings" = Shown::swatch("appearance", "Theme", 40, "Accent colour", &ACCENT_PRESETS)
        .help("Focus, selection, the tab highlight and the window's glow")
        .unset("Theme's")
        .token("color.accent")))]
    pub accent: Option<String>,
    /// The radius of a pane's corners, in pixels. Menus and palettes follow it. Left
    /// out, the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = PANE_LENGTH_MAX))]
    #[schemars(extend("x-lume-settings" = Shown::number("appearance", "Panes", 10, "Pane corners", SETTINGS_STEP_LENGTH)
        .help("The radius of a pane's corners, and of menus and palettes")
        .pixels()
        .unset("Theme's")
        .token("radius.card")))]
    pub card_radius: Option<f64>,
    /// The radius of the corners of tabs, buttons, fields and menu items, in pixels.
    /// Nothing smaller inside them is rounder. Left out, the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = PANE_LENGTH_MAX))]
    #[schemars(extend("x-lume-settings" = Shown::number("appearance", "Panes", 11, "Control corners", SETTINGS_STEP_LENGTH)
        .help("The radius of tabs, buttons, fields and menu items")
        .pixels()
        .unset("Theme's")
        .token("radius.control")))]
    pub control_radius: Option<f64>,
    /// The space between panes and around them, in pixels. Left out, the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = PANE_LENGTH_MAX))]
    #[schemars(extend("x-lume-settings" = Shown::number("appearance", "Panes", 20, "Gap between panes", SETTINGS_STEP_LENGTH)
        .pixels()
        .unset("Theme's")
        .token("space.gap")))]
    pub gap: Option<f64>,
    /// When a pane shows its header.
    #[schemars(extend("x-lume-settings" = Shown::segmented("appearance", "Panes", 30, "Pane header")
        .help("Auto hides it in a tab with a single pane")
        .options(&[("always", "Always"), ("auto", "Auto"), ("never", "Never")])))]
    pub pane_header: PaneHeader,
    /// How much a pane without the focus is dimmed, from 0 (not at all) to 1. Left
    /// out, the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    #[schemars(extend("x-lume-settings" = Shown::number("appearance", "Panes", 40, "Dim unfocused panes", SETTINGS_STEP_PERCENT)
        .help("How much a pane without the keyboard is dimmed")
        .percent()
        .unset("Theme's")
        .token("opacity.dim")))]
    pub dim_unfocused: Option<f64>,
    /// A picture shown behind the panes: the path of an image file.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("x-lume-settings" = Shown::text("appearance", "Background", 10, "Picture")
        .help("The path of an image file shown behind the panes")
        .unset("None")))]
    pub background_image: Option<String>,
    /// How much the picture is blurred, in pixels. Left out, the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = BACKGROUND_BLUR_MAX))]
    #[schemars(extend("x-lume-settings" = Shown::number("appearance", "Background", 20, "Blur", SETTINGS_STEP_LENGTH)
        .pixels()
        .unset("Theme's")
        .token("background.blur")))]
    pub background_blur: Option<f64>,
    /// How opaque the picture is, from 0 to 1. Left out, the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    #[schemars(extend("x-lume-settings" = Shown::number("appearance", "Background", 30, "Opacity", SETTINGS_STEP_PERCENT)
        .percent()
        .unset("Theme's")
        .token("opacity.background-image")))]
    pub background_opacity: Option<f64>,
    /// Design tokens set over the theme's, written as a theme file writes them: a
    /// table for each group, such as `[appearance.tokens.color]` with `accent = "#ff8800"`.
    pub tokens: BTreeMap<String, Value>,
    /// The tile a pane's header and its tab show for a program, by the program's name
    /// without its folder or `.exe`, such as `nvim = "editor"`. These come before the
    /// tiles the application knows.
    pub program_icons: BTreeMap<String, ProgramIcon>,
    /// Which program tiles are drawn in their colours. The others are drawn in the
    /// theme's muted ink.
    #[schemars(extend("x-lume-settings" = Shown::select("appearance", "Panes", 60, "Program icon colour")
        .help("Which tabs and pane headers show program icons in colour")
        .options(&[
            ("always", "Always"),
            ("active-pane", "Active pane"),
            ("active-tab", "Active tab"),
            ("never", "Never"),
        ])))]
    pub tile_color: TileColor,
    /// How the mark on the tab on show moves to another tab.
    #[schemars(extend("x-lume-settings" = Shown::select("appearance", "Tabs", 10, "Tab highlight")
        .help("How the highlight moves when another tab is shown")
        .options(&[
            ("glow", "Glow"),
            ("glide", "Glide"),
            ("stretch", "Stretch"),
            ("spring", "Spring"),
            ("instant", "Instant"),
        ])))]
    pub tab_highlight: OrPlugin<TabHighlight>,
    /// Effects plugins bring for places in the window, each as `<plugin>/<effect>`.
    /// A place left out moves as Lume's own does.
    pub motion: MotionChoices,
}

/// The value of [`crate::accent::FORMAT_KEY`] for something a plugin brings, written
/// `<plugin>/<name>`.
pub const CONTRIBUTED_FORMAT: &str = "contributed";

/// Checks the name of something a plugin brings: `<plugin>/<name>`, the plugin's id as
/// installed and the name its manifest gives the thing.
pub fn check_contributed(text: &str) -> Result<(), String> {
    let word = |part: &str| {
        part.starts_with(|c: char| c.is_ascii_lowercase())
            && part
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    let fits = text.split_once('/').is_some_and(|(plugin, name)| {
        !plugin.is_empty() && plugin.split('.').all(word) && word(name)
    });
    if fits {
        Ok(())
    } else {
        Err(format!(
            "`{text}` is not one of Lume's choices, nor a plugin's written as <plugin>/<name>"
        ))
    }
}

/// Something a plugin brings, by its name: `<plugin>/<name>`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Contributed(pub String);

impl<'de> Deserialize<'de> for Contributed {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        check_contributed(&text).map_err(serde::de::Error::custom)?;
        Ok(Self(text))
    }
}

impl JsonSchema for Contributed {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Contributed".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "Something a plugin brings, as <plugin>/<name>",
            (crate::accent::FORMAT_KEY): CONTRIBUTED_FORMAT,
        })
    }
}

/// One of Lume's own choices, or one a plugin brings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum OrPlugin<T> {
    Own(T),
    Plugin(Contributed),
}

impl<T: Default> Default for OrPlugin<T> {
    fn default() -> Self {
        Self::Own(T::default())
    }
}

/// The effects plugins bring for the places in the window, by place. Each is
/// `<plugin>/<effect>`; a place left out, or whose plugin is not there, moves as Lume's
/// own does. Every one moves at once while the system asks for less motion.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case")]
pub struct MotionChoices {
    /// How menus open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub menu_enter: Option<Contributed>,
    /// How a notice appears.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toast_enter: Option<Contributed>,
    /// How a notice leaves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toast_exit: Option<Contributed>,
    /// How the command palette opens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub palette_enter: Option<Contributed>,
    /// How a docked panel comes in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel_enter: Option<Contributed>,
    /// How a floating pane appears.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub float_enter: Option<Contributed>,
    /// How the pill of a pane put away appears.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pill_enter: Option<Contributed>,
    /// How a status item shows that it changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_change: Option<Contributed>,
}

/// How the mark on the tab on show moves to another tab. Each moves at once when the
/// keyboard changes the tab, and when the system asks for less motion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TabHighlight {
    /// A lit pill in the tab's colour that stretches towards the tab and settles,
    /// glowing more the faster it goes.
    #[default]
    Glow,
    /// A plain pill that slides.
    Glide,
    /// A line under the tab that stretches towards the next one.
    Stretch,
    /// A plain pill that springs into place.
    Spring,
    /// No motion: the mark is simply on the other tab.
    Instant,
}

/// Which program tiles are drawn in their colours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TileColor {
    /// Every tile.
    #[default]
    Always,
    /// The tiles of the pane with the focus and of the tab it is in.
    ActivePane,
    /// The tiles of the tab on show and of its panes.
    ActiveTab,
    /// None: every tile is drawn in the theme's muted ink.
    Never,
}

/// A tile for the program a pane runs: a kind of program, drawn with a line drawing
/// on a colour of the terminal's palette, or a well-known program, drawn with its mark
/// on its colour. A shell shows the prompt unless one of the shells' marks is chosen
/// for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProgramIcon {
    /// A prompt: a shell, or a program nothing else fits.
    Terminal,
    /// A page being written: an editor or a pager.
    Editor,
    /// Git's mark: Git and the tools that work on it.
    Git,
    /// A pulse: a monitor of the system or of a process.
    Monitor,
    /// A spark: a coding agent.
    Agent,
    /// A globe: a connection to another machine.
    Remote,
    /// A box: containers and virtual machines.
    Container,
    /// A folder: a file manager.
    Files,
    /// AlmaLinux.
    Alma,
    /// Alpine Linux.
    Alpine,
    /// Ansible.
    Ansible,
    /// Arch Linux.
    Arch,
    /// Argo.
    Argo,
    /// GNU Bash.
    Bash,
    /// Bazel.
    Bazel,
    /// Google BigQuery.
    Bigquery,
    /// Bitwarden.
    Bitwarden,
    /// Bun.
    Bun,
    /// Caddy.
    Caddy,
    /// Apache Cassandra.
    Cassandra,
    /// CentOS.
    Centos,
    /// Chocolatey.
    Chocolatey,
    /// Claude.
    Claude,
    /// ClickHouse.
    Clickhouse,
    /// Cline.
    Cline,
    /// Clojure.
    Clojure,
    /// Cloudflare.
    Cloudflare,
    /// CMake.
    Cmake,
    /// Cockroach Labs.
    Cockroachdb,
    /// CodeRabbit.
    Coderabbit,
    /// Anaconda.
    Conda,
    /// Consul.
    Consul,
    /// GitHub Copilot.
    Copilot,
    /// Crystal.
    Crystal,
    /// curl.
    Curl,
    /// Cursor.
    Cursor,
    /// Dart.
    Dart,
    /// Debian.
    Debian,
    /// Deno.
    Deno,
    /// DigitalOcean.
    Digitalocean,
    /// Docker.
    Docker,
    /// .NET.
    Dotnet,
    /// DuckDB.
    Duckdb,
    /// Elasticsearch.
    Elasticsearch,
    /// Elixir.
    Elixir,
    /// GNU Emacs.
    Emacs,
    /// Erlang.
    Erlang,
    /// Fedora.
    Fedora,
    /// FFmpeg.
    Ffmpeg,
    /// Firebase.
    Firebase,
    /// fish shell.
    Fish,
    /// Flutter.
    Flutter,
    /// Fly.io.
    Fly,
    /// Forgejo.
    Forgejo,
    /// Fossil SCM.
    Fossil,
    /// Google Cloud.
    Gcloud,
    /// Google Gemini.
    Gemini,
    /// Gentoo.
    Gentoo,
    /// Gitea.
    Gitea,
    /// GitHub.
    Github,
    /// GitLab.
    Gitlab,
    /// Gleam.
    Gleam,
    /// Go.
    Go,
    /// GNU Privacy Guard.
    Gpg,
    /// Gradle.
    Gradle,
    /// Haskell.
    Haskell,
    /// Helix.
    Helix,
    /// Helm.
    Helm,
    /// Homebrew.
    Homebrew,
    /// htop.
    Htop,
    /// HTTPie.
    Httpie,
    /// Hugging Face.
    Huggingface,
    /// InfluxDB.
    Influxdb,
    /// OpenJDK.
    Java,
    /// Julia.
    Julia,
    /// Jupyter.
    Jupyter,
    /// K3s.
    K3s,
    /// Kali Linux.
    Kali,
    /// Kotlin.
    Kotlin,
    /// Kubernetes.
    Kubernetes,
    /// Lapce.
    Lapce,
    /// LM Studio.
    Lmstudio,
    /// Lua.
    Lua,
    /// Manjaro.
    Manjaro,
    /// MariaDB.
    Mariadb,
    /// Mercurial.
    Mercurial,
    /// Linux Mint.
    Mint,
    /// Mistral AI.
    Mistral,
    /// MongoDB.
    Mongodb,
    /// MySQL.
    Mysql,
    /// Neo4j.
    Neo4j,
    /// Neovim.
    Neovim,
    /// Netlify.
    Netlify,
    /// NGINX.
    Nginx,
    /// Nim.
    Nim,
    /// NixOS.
    Nix,
    /// Node.js.
    Node,
    /// Nomad.
    Nomad,
    /// npm.
    Npm,
    /// Nushell.
    Nushell,
    /// Obsidian.
    Obsidian,
    /// OCaml.
    Ocaml,
    /// Ollama.
    Ollama,
    /// 1Password.
    Onepassword,
    /// OpenCode.
    Opencode,
    /// openSUSE.
    Opensuse,
    /// OpenTofu.
    Opentofu,
    /// Packer.
    Packer,
    /// Perl.
    Perl,
    /// PHP.
    Php,
    /// PlanetScale.
    Planetscale,
    /// pnpm.
    Pnpm,
    /// Poetry.
    Poetry,
    /// PostgreSQL.
    Postgres,
    /// Prisma.
    Prisma,
    /// Pulumi.
    Pulumi,
    /// Python.
    Python,
    /// QWen.
    Qwen,
    /// R.
    R,
    /// Racket.
    Racket,
    /// Ruby on Rails.
    Rails,
    /// Railway.
    Railway,
    /// Red Hat.
    Redhat,
    /// Redis.
    Redis,
    /// Rocky Linux.
    Rocky,
    /// Ruby.
    Ruby,
    /// Rust.
    Rust,
    /// Scala.
    Scala,
    /// Slackware.
    Slackware,
    /// Snowflake.
    Snowflake,
    /// SQLite.
    Sqlite,
    /// Sublime Text.
    Sublime,
    /// Subversion.
    Subversion,
    /// Supabase.
    Supabase,
    /// SurrealDB.
    Surrealdb,
    /// Swift.
    Swift,
    /// Tailscale.
    Tailscale,
    /// Terraform.
    Terraform,
    /// tmux.
    Tmux,
    /// Turso.
    Turso,
    /// TypeScript.
    Typescript,
    /// Ubuntu.
    Ubuntu,
    /// uv.
    Uv,
    /// Vagrant.
    Vagrant,
    /// Vault.
    Vault,
    /// Vercel.
    Vercel,
    /// Vim.
    Vim,
    /// Vite.
    Vite,
    /// Void Linux.
    Void,
    /// VSCodium.
    Vscodium,
    /// WireGuard.
    Wireguard,
    /// Wireshark.
    Wireshark,
    /// Yarn.
    Yarn,
    /// Zed Industries.
    Zed,
    /// Zig.
    Zig,
    /// Zsh.
    Zsh,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: None,
            theme_light: "lume-light".to_owned(),
            theme_dark: "lume-dark".to_owned(),
            accent: None,
            card_radius: None,
            control_radius: None,
            gap: None,
            pane_header: PaneHeader::Always,
            dim_unfocused: None,
            background_image: None,
            background_blur: None,
            background_opacity: None,
            tokens: BTreeMap::new(),
            program_icons: BTreeMap::new(),
            tile_color: TileColor::Always,
            tab_highlight: OrPlugin::Own(TabHighlight::Glow),
            motion: MotionChoices::default(),
        }
    }
}

/// The fonts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case")]
pub struct FontConfig {
    /// The font terminals are drawn in.
    #[schemars(extend("x-lume-settings" = Shown::select("appearance", "Type", 20, "Terminal font").list("fonts")))]
    pub family: String,
    /// The fonts tried, in order, for a character the first has no glyph for, or when
    /// it is not installed.
    pub fallback: Vec<String>,
    /// The size of terminal text, in pixels.
    #[schemars(range(min = FONT_SIZE_MIN, max = FONT_SIZE_MAX))]
    #[schemars(extend("x-lume-settings" = Shown::number("appearance", "Type", 30, "Terminal text size", SETTINGS_STEP_LENGTH)
        .pixels()))]
    pub size: f64,
    /// The height of a line of terminal text, as a multiple of the font's size.
    #[schemars(range(min = LINE_HEIGHT_MIN, max = LINE_HEIGHT_MAX))]
    #[schemars(extend("x-lume-settings" = Shown::number("appearance", "Type", 40, "Line height", SETTINGS_STEP_LINE_HEIGHT)
        .help("The height of a line of terminal text, as a multiple of its size")))]
    pub line_height: f64,
    /// The weight of ordinary text, as fonts number weights: 400 is regular.
    #[schemars(range(min = 1, max = 1000))]
    pub weight: u16,
    /// The weight of bold text.
    #[schemars(range(min = 1, max = 1000))]
    pub bold_weight: u16,
    /// The font of everything that is not a terminal: tabs, headers, menus. `Inter` comes
    /// with the application; `system-ui` is the system's own; any other is one that is
    /// installed.
    #[schemars(extend("x-lume-settings" = Shown::select("appearance", "Type", 5, "Interface font")
        .options(&[("Inter", "Inter"), ("system-ui", "The system's font")])
        .list("interface-fonts")
        .help("The font of tabs, menus, settings and the views of plugins")))]
    pub ui_family: String,
    /// The size of that text, in pixels.
    #[schemars(range(min = UI_FONT_SIZE_MIN, max = UI_FONT_SIZE_MAX))]
    #[schemars(extend("x-lume-settings" = Shown::number("appearance", "Type", 10, "Interface text size", SETTINGS_STEP_LENGTH)
        .pixels()))]
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
            ui_family: "Inter".to_owned(),
            ui_size: 12.0,
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

/// What the cursor leaves behind when it jumps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CursorTrail {
    /// Nothing: the cursor is simply in its new place.
    #[default]
    None,
    /// The cursor smears towards its new place, its leading edge first.
    Warp,
    /// A band from the old place shrinks into the new one.
    Sweep,
    /// A glowing comet flies to the new place, its tail following.
    Comet,
}

/// What the cursor gives off when it changes shape or lands in another pane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CursorPulse {
    /// Nothing.
    #[default]
    None,
    /// A disc that grows from the cursor and fades.
    Burst,
    /// A ring that grows from the cursor and fades.
    Ripple,
    /// A box that grows from the cursor's shape and fades.
    BoxBurst,
    /// An outline that grows from the cursor's shape and fades.
    BoxRipple,
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
    /// The program a new terminal runs when no profile says otherwise. Empty, it is the
    /// user's own shell.
    pub shell: String,
    /// The arguments that program is started with.
    pub shell_args: Vec<String>,
    /// How many lines that have scrolled off the top are kept.
    #[schemars(range(min = 0, max = SCROLLBACK_LINES_MAX))]
    #[schemars(extend("x-lume-settings" = Shown::number("terminal", "History", 10, "Scrollback", SETTINGS_STEP_LINES)
        .help("Older lines are dropped past this limit. New terminals use it")
        .lines()))]
    pub scrollback_lines: u32,
    /// Keep only the newest lines in each window and fetch older ones from the session
    /// host when the terminal is scrolled back to them or searched. Off, every window
    /// keeps all of a terminal's scrollback itself, which costs memory twice over.
    #[schemars(extend("x-lume-settings" = Shown::switch("terminal", "History", 20, "Fetch older lines when needed")
        .help("Windows keep only the newest lines and fetch older ones as you scroll back or search")))]
    pub history_paging: bool,
    /// How many lines a window keeps for a terminal while older ones are fetched when
    /// needed.
    #[schemars(range(min = WINDOW_HISTORY_LINES_MIN, max = WINDOW_HISTORY_LINES_MAX))]
    #[schemars(extend("x-lume-settings" = Shown::number("terminal", "History", 30, "Lines kept in the window", SETTINGS_STEP_LINES)
        .help("The newest lines a window holds itself when older ones are fetched as needed")
        .lines()
        .when_on("terminal.history-paging")))]
    pub window_history_lines: u32,
    /// How long, in seconds, a window keeps the older lines it fetched once the terminal
    /// is back at its newest output.
    #[schemars(range(min = HISTORY_RELEASE_SECONDS_MIN, max = HISTORY_RELEASE_SECONDS_MAX))]
    #[schemars(extend("x-lume-settings" = Shown::number("terminal", "History", 40, "Let go of older lines after", SETTINGS_STEP_SECONDS)
        .help("How long fetched lines stay once you are back at the newest output")
        .seconds()
        .when_on("terminal.history-paging")))]
    pub history_release_seconds: u32,
    /// Let a pane that is out of sight, in a tab not on show or a float put down, let go
    /// of its window's copy of the screen, and get it back from the session host when it
    /// is shown again. Off, every pane keeps its copy all the time.
    #[schemars(extend("x-lume-settings" = Shown::switch("terminal", "History", 50, "Let go of hidden panes")
        .help("Panes in hidden tabs let go of their copy of the screen and get it back from the session host when shown")))]
    pub hidden_release: bool,
    /// How long, in seconds, a pane stays out of sight before it lets go of its copy of
    /// the screen.
    #[schemars(range(min = HIDDEN_RELEASE_SECONDS_MIN, max = HIDDEN_RELEASE_SECONDS_MAX))]
    #[schemars(extend("x-lume-settings" = Shown::number("terminal", "History", 60, "Let go of hidden panes after", SETTINGS_STEP_SECONDS)
        .help("How long a pane stays out of sight before it lets go of its screen")
        .seconds()
        .when_on("terminal.hidden-release")))]
    pub hidden_release_seconds: u32,
    /// The cursor's shape, until a program chooses another.
    #[schemars(extend("x-lume-settings" = Shown::segmented("terminal", "Cursor", 10, "Cursor style")
        .options(&[("block", "Block"), ("bar", "Bar"), ("underline", "Underline")])))]
    pub cursor_style: CursorStyle,
    /// Whether the cursor blinks, until a program says otherwise.
    #[schemars(extend("x-lume-settings" = Shown::switch("terminal", "Cursor", 20, "Blinking cursor")))]
    pub cursor_blink: bool,
    /// What the cursor leaves behind when it jumps a distance, or moves to another
    /// pane. Off while the system asks for less motion.
    #[schemars(extend("x-lume-settings" = Shown::select("terminal", "Cursor", 30, "Cursor trail")
        .help("Drawn when the cursor jumps, or moves to another pane")
        .options(&[
            ("none", "None"),
            ("warp", "Warp"),
            ("sweep", "Sweep"),
            ("comet", "Comet"),
        ])))]
    pub cursor_trail: OrPlugin<CursorTrail>,
    /// What the cursor gives off when it changes shape or moves to another pane. Off
    /// while the system asks for less motion.
    #[schemars(extend("x-lume-settings" = Shown::select("terminal", "Cursor", 40, "Cursor pulse")
        .help("Drawn when the cursor changes shape, or moves to another pane")
        .options(&[
            ("none", "None"),
            ("burst", "Burst"),
            ("ripple", "Ripple"),
            ("box-burst", "Box burst"),
            ("box-ripple", "Box ripple"),
        ])))]
    pub cursor_pulse: OrPlugin<CursorPulse>,
    /// How long the cursor's trail and pulse last, as a share of each effect's own
    /// length: 1 is as designed, 2 twice as long.
    #[schemars(range(min = CURSOR_EFFECT_LENGTH_MIN, max = CURSOR_EFFECT_LENGTH_MAX))]
    #[schemars(extend("x-lume-settings" = Shown::number("terminal", "Cursor", 50, "Effect length", SETTINGS_STEP_EFFECT_LENGTH)
        .help("How long the trail and the pulse last, against their own length")
        .percent()))]
    pub cursor_effect_length: f64,
    /// The space between a pane's edge and its text, in pixels. Left out, the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = PANE_LENGTH_MAX))]
    #[schemars(extend("x-lume-settings" = Shown::number("terminal", "Buffer", 20, "Padding", SETTINGS_STEP_LENGTH)
        .help("The space between a pane's edge and its text")
        .pixels()
        .unset("Theme's")
        .token("space.padding")))]
    pub padding: Option<f64>,
    /// What happens to a pane when its program ends.
    #[schemars(extend("x-lume-settings" = Shown::select("terminal", "Closing", 10, "When a program ends")
        .options(&[
            ("clean", "Close the pane if it succeeded"),
            ("always", "Always close the pane"),
            ("never", "Keep the pane open"),
        ])))]
    pub close_on_exit: CloseOnExit,
    /// Whether closing a pane asks first.
    #[schemars(extend("x-lume-settings" = Shown::select("terminal", "Closing", 20, "Confirm before closing a pane")
        .help("Ask before closing a pane whose program is still running")
        .options(&CONFIRM_CHOICES)))]
    pub confirm_close: Confirm,
    /// Copy text to the clipboard as soon as it is selected with the pointer.
    #[schemars(extend("x-lume-settings" = Shown::switch("terminal", "Clipboard", 10, "Copy on select")
        .help("Text selected with the pointer is copied at once, with no key pressed")))]
    pub copy_on_select: bool,
    /// Show a desktop notification when a command that ran for a while ends in a window
    /// that is not in use. It needs the shell to report its commands.
    #[schemars(extend("x-lume-settings" = Shown::switch("notifications", "Notify me when", 20, "Notify when a long command ends")
        .help("A desktop notification when a command ends in a window you are not using")))]
    pub notify_long_command: bool,
    /// How long a command runs, in seconds, before its end is notified.
    #[schemars(range(min = LONG_COMMAND_SECONDS_MIN, max = LONG_COMMAND_SECONDS_MAX))]
    #[schemars(extend("x-lume-settings" = Shown::number("notifications", "Notify me when", 30, "Long command", SETTINGS_STEP_SECONDS)
        .help("How long a command runs before its end is notified")
        .seconds()))]
    pub long_command_seconds: u32,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            shell: String::new(),
            shell_args: Vec::new(),
            scrollback_lines: 10_000,
            history_paging: false,
            window_history_lines: WINDOW_HISTORY_LINES_DEFAULT,
            history_release_seconds: HISTORY_RELEASE_SECONDS_DEFAULT,
            hidden_release: false,
            hidden_release_seconds: HIDDEN_RELEASE_SECONDS_DEFAULT,
            cursor_style: CursorStyle::Block,
            cursor_blink: true,
            cursor_trail: OrPlugin::Own(CursorTrail::None),
            cursor_pulse: OrPlugin::Own(CursorPulse::None),
            cursor_effect_length: 1.0,
            padding: None,
            close_on_exit: CloseOnExit::Clean,
            confirm_close: Confirm::Never,
            copy_on_select: false,
            notify_long_command: false,
            long_command_seconds: LONG_COMMAND_SECONDS_DEFAULT,
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
    /// The directory of the pane the new one is opened from, where its shell says
    /// which that is; otherwise as `workspace`.
    #[default]
    Inherit,
    /// The workspace's folder, or the home directory if it has none.
    Workspace,
    /// The home directory.
    Home,
}

/// Where things are placed in a window.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case")]
pub struct LayoutConfig {
    /// Where the list of tabs is.
    #[schemars(extend("x-lume-settings" = Shown::segmented("appearance", "Panes", 50, "Tab list")
        .help("In the bar at the top, or in a panel at a side")
        .options(&[("top", "Top"), ("left", "Left"), ("right", "Right")])))]
    pub tab_placement: TabPlacement,
    /// The directory a new pane's program starts in.
    pub new_pane_cwd: NewPaneCwd,
    /// What happens to a tab opened from a layout file when the file is saved again:
    /// "ask", "always" (programs still running in panes the file no longer has are
    /// put away, not ended) or "never".
    #[schemars(extend("x-lume-settings" = Shown::segmented("appearance", "Panes", 55, "When a layout file changes")
        .help("For tabs opened from a file in the layouts folder. Running programs are never ended")
        .options(&[("ask", "Ask"), ("always", "Apply"), ("never", "Ignore")])))]
    pub files_reapply: FilesReapply,
}

/// What happens to a tab opened from a layout file when the file changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FilesReapply {
    /// The tab's window offers to apply the change.
    #[default]
    Ask,
    /// The change is applied at once.
    Always,
    /// The tab is left as it is.
    Never,
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
pub const NOT_IN_A_WORKSPACE: [&str; 3] = ["app", "workspace", "rex"];

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

/// The key profiles are written under, in any file.
pub const PROFILES_KEY: &str = "profile";

/// Where the Rex servers are written: `[[rex.host]]` in `config.toml`.
pub const REX_KEY: &str = "rex";
pub const REX_HOSTS_KEY: &str = "host";

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
        // The pages of the settings window, which the settings shown there name.
        object.insert(
            PAGES_KEY.to_owned(),
            serde_json::to_value(pages()).unwrap_or_default(),
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
