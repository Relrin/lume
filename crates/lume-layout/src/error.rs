//! Why an intent was refused, and why an authority could not carry one out.

use crate::ids::{PaneId, PanelId, SplitId, TabId, WindowId};

/// Why an intent was refused. A refused intent changes nothing.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IntentError {
    #[error("there is no pane {0}")]
    NoSuchPane(PaneId),
    #[error("there is no tab {0}")]
    NoSuchTab(TabId),
    #[error("there is no split {0}")]
    NoSuchSplit(SplitId),
    #[error("there is no window {0}")]
    NoSuchWindow(WindowId),
    #[error("window {window} has no panel {panel}")]
    NoSuchPanel { window: WindowId, panel: PanelId },
    #[error("the ratios do not fit the split: {0}")]
    BadRatios(&'static str),
    #[error("the pane would be smaller than a pane may be")]
    TooSmall,
    #[error("there would be more {0} than there may be")]
    TooMany(&'static str),
    #[error("an intent about two panes was given pane {0} twice")]
    SamePane(PaneId),
    #[error("panes and tabs cannot be moved from one window to another")]
    CrossWindow,
    #[error("pane {0} floats, and this can only be done to a pane in a tab")]
    Floating(PaneId),
    #[error("pane {0} does not float")]
    NotFloating(PaneId),
    #[error("pane {0} is not a terminal")]
    NotTerminal(PaneId),
    #[error("this panel stays where it is")]
    Fixed,
    #[error("there is no scratch terminal yet, and nothing was given to make one from")]
    ContentRequired,
    #[error("a floating pane's rectangle must have a width and a height")]
    BadRect,
    #[error("there is no place {index} among {len}")]
    BadIndex { index: u32, len: usize },
    #[error("`{0}` cannot be a workspace's name")]
    BadName(String),
    #[error("only the application may apply this intent")]
    CoreOnly,
    #[error("a session was started for an intent that makes no terminal pane")]
    SessionUnused,
}

impl IntentError {
    /// A short, stable name for the kind of refusal, for whoever reads it by program.
    pub fn code(&self) -> &'static str {
        match self {
            IntentError::NoSuchPane(_)
            | IntentError::NoSuchTab(_)
            | IntentError::NoSuchSplit(_)
            | IntentError::NoSuchWindow(_)
            | IntentError::NoSuchPanel { .. } => "not_found",
            IntentError::TooSmall => "too_small",
            IntentError::TooMany(_) => "limit_exceeded",
            IntentError::CrossWindow => "unsupported",
            IntentError::CoreOnly => "forbidden",
            IntentError::BadRatios(_)
            | IntentError::SamePane(_)
            | IntentError::Floating(_)
            | IntentError::NotFloating(_)
            | IntentError::NotTerminal(_)
            | IntentError::Fixed
            | IntentError::ContentRequired
            | IntentError::BadRect
            | IntentError::BadName(_)
            | IntentError::BadIndex { .. }
            | IntentError::SessionUnused => "invalid_intent",
        }
    }
}

/// Why a session for a new pane could not be started.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct SpawnFailure {
    /// A short, stable name for the kind of failure.
    pub code: String,
    pub message: String,
}

/// Why an authority did not carry out what it was asked.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    /// The intent was refused.
    #[error(transparent)]
    Intent(#[from] IntentError),
    /// The intent needed a session, and the session could not be started. No pane was
    /// made.
    #[error("the session for the new pane could not be started: {0}")]
    Spawn(SpawnFailure),
    /// The authority can no longer be reached.
    #[error("the layout can no longer be reached")]
    Closed,
}

impl LayoutError {
    /// A short, stable name for the kind of failure.
    pub fn code(&self) -> &str {
        match self {
            LayoutError::Intent(error) => error.code(),
            LayoutError::Spawn(failure) => &failure.code,
            LayoutError::Closed => "disconnected",
        }
    }
}
