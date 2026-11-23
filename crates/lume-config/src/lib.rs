//! Lume's configuration: the settings, the themes and the keys.
//!
//! The settings are data. They are TOML files the user writes: `config.toml`, a file
//! for each workspace that may override it, and themes. Nothing in them can run code.
//! The types in [`schema`] say what can be set; a JSON Schema for editors, the check a
//! file is held to and the commented file a new user is given are all made from them.
//!
//! Reading is forgiving in a particular way. A key that is not known is reported with
//! its line and ignored, so a file written for a newer version still loads. A value
//! that is not allowed is reported and the layer below shows through. Only a file that
//! is not TOML at all is unusable, and then the configuration that was in use stays in
//! use.
//!
//! From the settings come three things the rest of the application uses:
//!
//! - the [`Config`] itself, with the [`Source`] of every value;
//! - every design token's value ([`ResolvedTokens`]): a theme, with the settings that
//!   override it laid over;
//! - the keys in effect ([`Keymap`]): a platform's defaults with the user's bindings
//!   laid over.
//!
//! [`ConfigStore`] keeps the configuration in use and [`Watcher`] reads the files again
//! when they change. Nothing here needs the application to be running: the command
//! line uses the same code to check a file or list the keys.

pub mod accent;
pub mod error;
pub mod generate;
pub mod import;
pub mod keymap;
pub mod keys;
pub mod layers;
pub mod paths;
pub mod profiles;
pub mod rex;
pub mod schema;
pub mod settings;
pub mod source;
pub mod store;
pub mod suggest;
#[cfg(test)]
mod testing;
pub mod theme;
pub mod tokens;
pub mod tuning;
pub mod watch;
pub mod write;

pub use error::ConfigError;
pub use import::{Imported, import_ghostty, theme_id};
pub use keymap::{
    Binding, Dropped, Keymap, PluginBinding, Removed, default_bindings, resolve_keymap,
    resolve_keymap_with,
};
pub use keys::{Chord, KeySequence, WhenTerm, parse_when, write_when};
pub use layers::{
    LayerInput, LayerKind, Layered, check_value, plugin_setting_problems, resolve_layers,
};
pub use paths::{Dirs, Platform, check_workspace_name, host_log_file, log_file};
pub use profiles::{
    LauncherKind, Profile, ProfileEntry, ProfileOrigin, ShellIntegration, merge_profiles,
};
pub use rex::{RexConfig, RexHostEntry};
pub use schema::{
    CloseOnExit, Config, ProgramIcon, TileColor, WindowCorners, WorkspaceSettings, config_schema,
    default_config_text,
};
pub use source::{Problem, ProblemKind, Source};
pub use store::{ConfigStore, Resolved, check_setting};
pub use theme::{
    Theme, Themes, default_tokens, light_tokens, parse_theme, resolve_tokens, theme_schema,
};
pub use tokens::{Color, ResolvedTokens, TOKENS, TokenDef, TokenKind, TokenValue};
pub use watch::{WatchEvent, Watcher};

/// The colour scheme of the operating system, or of a theme.
pub use lume_proto::ColorScheme;
