//! The configuration's tuning values. Each is defined here and nowhere else.

use std::time::Duration;

/// How long the configuration directory must stay quiet after a change before the
/// files are read again. An editor saves a file in several steps (truncate, write,
/// rename), and a read in the middle of them sees half a file.
pub const CONFIG_DEBOUNCE: Duration = Duration::from_millis(75);

/// How many themes a theme may inherit from, one after another.
pub const THEME_INHERIT_MAX: usize = 4;

/// How many edits a misspelt key may be away from a known one to be suggested.
pub const SUGGEST_MAX_DISTANCE: usize = 2;

/// The most lines of scrollback a terminal may be asked to keep. Every terminal of a
/// window keeps its lines in one memory that only grows, so the limit is what a window
/// full of terminals can hold.
pub const SCROLLBACK_LINES_MAX: u32 = 100_000;

/// How many lines of scrollback a window keeps for a terminal it shows when older
/// lines are fetched from the session host as they are needed, unless the settings say
/// otherwise; and the fewest and most it may be told to keep.
pub const WINDOW_HISTORY_LINES_DEFAULT: u32 = 1_000;
pub const WINDOW_HISTORY_LINES_MIN: u32 = 100;
pub const WINDOW_HISTORY_LINES_MAX: u32 = SCROLLBACK_LINES_MAX;

/// How long a window keeps older lines it fetched after its terminal is back at its
/// newest output, in seconds, unless the settings say otherwise; and the shortest and
/// longest it may be told to.
pub const HISTORY_RELEASE_SECONDS_DEFAULT: u32 = 60;
pub const HISTORY_RELEASE_SECONDS_MIN: u32 = 5;
pub const HISTORY_RELEASE_SECONDS_MAX: u32 = 86_400;

/// How long the session host stays with no window and no running session before it
/// exits, in seconds, unless the settings say otherwise; and the longest it may be told
/// to. Zero keeps it running.
pub const HOST_IDLE_EXIT_SECONDS_DEFAULT: u32 = 60;
pub const HOST_IDLE_EXIT_SECONDS_MAX: u32 = 86_400;

/// The smallest and largest size of terminal text, in CSS pixels.
pub const FONT_SIZE_MIN: f64 = 1.0;
pub const FONT_SIZE_MAX: f64 = 72.0;

/// The smallest and largest size of the text of everything that is not a terminal, in
/// CSS pixels.
pub const UI_FONT_SIZE_MIN: f64 = 1.0;
pub const UI_FONT_SIZE_MAX: f64 = 32.0;

/// The smallest and largest height of a line of terminal text, as a multiple of its
/// size. Below 1 a line is shorter than its letters, which are cut to it.
pub const LINE_HEIGHT_MIN: f64 = 0.1;
pub const LINE_HEIGHT_MAX: f64 = 3.0;

/// The largest radius of a corner, and the largest space between panes or between a
/// pane's edge and its text, in pixels.
pub const PANE_LENGTH_MAX: f64 = 64.0;

/// The most the picture behind the panes may be blurred, in pixels.
pub const BACKGROUND_BLUR_MAX: f64 = 50.0;

/// How far the arrow keys move a typed number in the settings window, in what the
/// number is shown in: pixels, a multiple of the text's size, hundredths, lines and
/// seconds.
pub const SETTINGS_STEP_LENGTH: f64 = 1.0;
pub const SETTINGS_STEP_LINE_HEIGHT: f64 = 0.1;
pub const SETTINGS_STEP_PERCENT: f64 = 1.0;
pub const SETTINGS_STEP_LINES: f64 = 1000.0;
pub const SETTINGS_STEP_SECONDS: f64 = 1.0;

/// The shortest a command may be said to run before its end is worth a notification.
/// Anything shorter would notify about commands that are typed and done.
pub const LONG_COMMAND_SECONDS_MIN: u32 = 5;

/// How long a command runs, in seconds, before its end is worth a notification, unless
/// the settings say otherwise.
pub const LONG_COMMAND_SECONDS_DEFAULT: u32 = 10;

/// The longest a command may be said to run before a notification is worth it: a day.
pub const LONG_COMMAND_SECONDS_MAX: u32 = 86_400;

/// The longest name a workspace may have, in characters.
pub const WORKSPACE_NAME_MAX: usize = 64;

/// The longest id a profile may have, in characters.
pub const PROFILE_ID_MAX_CHARS: usize = 128;

/// How many profiles one file may define. More than this is not a list a person keeps.
pub const PROFILES_MAX: usize = 512;

/// The largest configuration or theme file that is read. A larger one is not a file
/// anybody wrote by hand.
pub const FILE_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// How many chords a key sequence may have.
pub const KEY_SEQUENCE_MAX: usize = 4;

/// How light a colour scheme's background must be for the theme made from it to be a
/// light one, from 0 (black) to 1 (white).
pub const IMPORT_LIGHT_LUMINANCE: f64 = 0.5;

/// How far towards black the window behind the panes is from a scheme's background,
/// for a dark scheme and for a light one. The panes must stand out from it.
pub const IMPORT_DARK_SURFACE_MIX: f64 = 0.35;
pub const IMPORT_LIGHT_SURFACE_MIX: f64 = 0.08;

/// How far from a scheme's background towards its foreground the borders are, and a
/// control under the pointer, and one being pressed.
pub const IMPORT_BORDER_MIX: f64 = 0.18;
pub const IMPORT_HOVER_MIX: f64 = 0.12;
pub const IMPORT_PRESSED_MIX: f64 = 0.24;

/// How far from a scheme's foreground towards its background quiet text is.
pub const IMPORT_MUTED_MIX: f64 = 0.4;

/// How far from a scheme's background towards its foreground selected text's
/// background is, for a scheme that does not say.
pub const IMPORT_SELECTION_MIX: f64 = 0.3;

/// How opaque the mark of where a dragged thing would land is, out of 255.
pub const IMPORT_DROP_PREVIEW_ALPHA: u8 = 0x40;

/// The accent colours offered beside the theme's own: the name written in the settings,
/// what it is called, and its colour on a dark theme and on a light one. The light
/// colours are deeper, so that a focus border or a selected tab still stands out
/// against a light pane.
pub const ACCENT_PRESETS: [AccentPreset; 6] = [
    AccentPreset::new("mint", "Mint", "#7fe3b8", "#147a55"),
    AccentPreset::new("blue", "Blue", "#7aa7ff", "#2f6fd0"),
    AccentPreset::new("lilac", "Lilac", "#c9a6f5", "#7a4fc0"),
    AccentPreset::new("amber", "Amber", "#f0b35a", "#a8650b"),
    AccentPreset::new("coral", "Coral", "#ef8f8f", "#c2453d"),
    AccentPreset::new("graphite", "Graphite", "#9aa3ab", "#59636c"),
];

/// One accent colour offered beside the theme's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccentPreset {
    pub name: &'static str,
    pub label: &'static str,
    pub dark: &'static str,
    pub light: &'static str,
}

impl AccentPreset {
    const fn new(
        name: &'static str,
        label: &'static str,
        dark: &'static str,
        light: &'static str,
    ) -> Self {
        Self {
            name,
            label,
            dark,
            light,
        }
    }
}

/// How far from the terminal's background towards a chosen accent the background of
/// selected text is, on a dark theme and on a light one.
pub const SELECTION_ACCENT_MIX_DARK: f64 = 0.22;
pub const SELECTION_ACCENT_MIX_LIGHT: f64 = 0.3;

/// The text drawn on a chosen accent: whichever of the two reads better on it.
pub const ACCENT_TEXT_DARK: &str = "#0f1113";
pub const ACCENT_TEXT_LIGHT: &str = "#ffffff";

/// How much shorter or longer than its own the cursor's effects may be made to last: a
/// quarter as long at the least, three times as long at the most.
pub const CURSOR_EFFECT_LENGTH_MIN: f64 = 0.25;
pub const CURSOR_EFFECT_LENGTH_MAX: f64 = 3.0;

/// How far the arrow keys move the length of the cursor's effects, in hundredths.
pub const SETTINGS_STEP_EFFECT_LENGTH: f64 = 5.0;
