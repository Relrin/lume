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

/// The most lines of scrollback a terminal may be asked to keep.
pub const SCROLLBACK_LINES_MAX: u32 = 200_000;

/// The smallest and largest font size, in CSS pixels.
pub const FONT_SIZE_MIN: f64 = 6.0;
pub const FONT_SIZE_MAX: f64 = 72.0;

/// The longest name a workspace may have, in characters.
pub const WORKSPACE_NAME_MAX: usize = 64;

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
