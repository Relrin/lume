//! The configuration as a page is told it.
//!
//! A page asks for the configuration in use when it starts and is told it again, whole,
//! whenever it changes. What it is told is one value: the settings, where each came
//! from, what was wrong in the files, the design tokens the theme and the settings come
//! to, the keys in effect and a few facts of the platform. Its JSON is the page's to
//! read, so its TypeScript type is generated from [`ConfigView`] (see
//! [`crate::generate`]).

use std::collections::BTreeMap;

use lume_proto::ColorScheme;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::keymap::{Binding, Removed};
use crate::paths::Platform;
use crate::schema::{Config, TitleBar};
use crate::source::{Problem, Source};

/// The theme the tokens were resolved from.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ThemeChoice {
    pub id: String,
    /// Whether the theme is a light or a dark one.
    pub scheme: ColorScheme,
}

/// A layout file there is to open, as a menu offers it.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct LayoutEntry {
    /// The name of its file without `.toml`.
    pub id: String,
    /// What a person calls it.
    pub name: String,
    pub description: Option<String>,
    /// How many tabs it opens.
    pub tabs: usize,
    /// The file it is read from.
    pub file: String,
}

/// The keys in effect.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct KeymapView {
    /// The bindings, lowest first: of two that match a key equally well, the later one
    /// wins.
    pub bindings: Vec<Binding>,
    /// The bindings that were unbound or replaced, and what did it.
    pub removed: Vec<Removed>,
}

/// The configuration in use, without its revision: what is compared to tell whether
/// anything a page shows changed.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ConfigBody {
    pub platform: Platform,
    /// Who draws the window's title bar, already decided for the platform.
    pub title_bar: TitleBar,
    /// The colour scheme of the operating system.
    pub scheme: ColorScheme,
    /// The theme the tokens were resolved from.
    pub theme: ThemeChoice,
    pub config: Config,
    /// Where each setting came from, by the setting's path.
    #[schemars(with = "BTreeMap<String, Value>")]
    pub sources: BTreeMap<String, Source>,
    /// What was wrong in the files and was passed over.
    pub problems: Vec<Problem>,
    /// Every design token's value, by the token's name.
    #[schemars(extend("x-ts" = "Tokens"))]
    pub tokens: Map<String, Value>,
    pub keymap: KeymapView,
    /// The layout files there are to open, in the order of their ids.
    pub layouts: Vec<LayoutEntry>,
    /// Counts the schemas of the settings there have been: it changes when plugins add
    /// to the settings or take from them.
    pub schema_revision: u64,
}

/// The configuration in use, as `config_view` returns it and `config_changed` carries
/// it.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ConfigView {
    /// Higher than that of every view before this one.
    pub revision: u64,
    #[serde(flatten)]
    pub body: ConfigBody,
}

impl ConfigBody {
    /// The body as JSON.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}
