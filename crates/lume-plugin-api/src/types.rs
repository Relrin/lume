//! What a plugin's code sends with each call, and what it is given back.
//!
//! Each type here is read from what the code sent, strictly: a field Lume does not
//! know is an error, so a mistake shows at once rather than as something that is
//! quietly ignored.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Nothing: what a call that takes no arguments is sent.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Nothing {}

/// What a status item shows. Each field left out keeps what the item showed before;
/// the manifest says what kind of item it is.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct StatusValue {
    /// The words of a text or badge item, or the amount written out beside a meter or
    /// a sparkline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// What the item stands for, read out and shown beside a meter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// How full a meter is, from 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// A sparkline's values, from the oldest to the newest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
    /// What the pointer is told when it rests on the item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    /// The rows of the card shown when the pointer rests on the item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<Vec<CardRow>>,
    /// Whether the item is out of sight for now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
}

/// What a status item's colour says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Neutral,
    Info,
    Success,
    Warning,
    Danger,
}

/// One row of a status item's card: a name and what it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CardRow {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusSet {
    /// The status item, by the name the manifest gives it.
    pub id: String,
    pub value: StatusValue,
    /// The pane whose header shows it, for an item in a pane's header.
    #[serde(default)]
    pub pane: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusClear {
    pub id: String,
    #[serde(default)]
    pub pane: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandExecute {
    /// The command's id: one of the application's, or one of the plugin's own by its
    /// name.
    pub id: String,
    #[serde(default)]
    pub args: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KvKey {
    pub key: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KvSet {
    pub key: String,
    pub value: Value,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SettingsGet {
    /// One setting, by its key; all of the plugin's if left out.
    #[serde(default)]
    pub key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextSet {
    /// The name of a condition the plugin's keys and menus can be shown under, as
    /// `plugin:<id>.<key>`.
    pub key: String,
    pub value: bool,
}

/// What the plugin can hear about.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Topic {
    /// What sessions report: titles, directories, commands that start and end.
    Session,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Subscribe {
    pub topic: Topic,
}

/// Something a session reported, as a plugin hears it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionReport {
    /// The session.
    pub session: String,
    /// The pane that shows it, if one does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
    /// The kind of report: `title`, `cwd`, `bell`, `command_started`,
    /// `command_finished`, `program_status`, `foreground`, `activity`,
    /// `disconnected`, `state` or `exit`.
    pub kind: String,
    /// What was reported, as the session reported it.
    pub details: Value,
}

/// Why a call failed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

/// The kinds of failure a call can have.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The plugin has no permission for it.
    Denied,
    /// What was sent is not what the call takes.
    Invalid,
    /// What was asked for is not there.
    NotFound,
    /// The plugin is making more calls than it may at once.
    Busy,
    /// It could not be done.
    Failed,
    /// The plugin was stopped before it was done.
    Stopped,
    /// The command was not handled by the plugin's code.
    Unhandled,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn denied(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Denied, message)
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Invalid, message)
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Failed, message)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}
