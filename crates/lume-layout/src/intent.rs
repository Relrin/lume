//! Intents: the only way a layout changes.
//!
//! Whoever wants the layout to be different (a page, a key, the command line, the
//! application itself) says what it wants as an intent, and the layout's authority
//! decides. An intent names what it is about by id, so it means the same thing whoever
//! sends it and whenever it arrives.

use lume_proto::{BackendId, Launcher, SessionId, TermSize};
use serde::{Deserialize, Serialize};

use crate::ids::{PaneId, PanelId, SplitId, TabId, WindowId};
use crate::model::{Area, Bounds, PanelContent, UnitRect};

/// A direction on the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::Left, Dir::Right, Dir::Up, Dir::Down];
}

/// A change asked of a layout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Intent {
    /// Divides a pane's space and puts a new pane on one side of it. `ratio` is the new
    /// pane's share of the space, in units.
    Split {
        pane: PaneId,
        dir: Dir,
        ratio: u32,
        content: NewContent,
    },
    /// Removes a pane. A tab that loses its last pane goes with it.
    Close { pane: PaneId },
    /// Gives a pane the keyboard.
    Focus { target: FocusTarget },
    /// Gives the keyboard to the pane next to the focused one in a direction. Nothing
    /// changes when there is none.
    FocusDir { window: WindowId, dir: Dir },
    /// Takes a pane from where it is and puts it somewhere else in its window.
    Move { pane: PaneId, to: Place },
    /// Exchanges the places of two panes.
    Swap { a: PaneId, b: PaneId },
    /// Sets the shares of a split's children. Shares that are too small are raised to
    /// the smallest allowed and the others reduced to make room.
    Resize { split: SplitId, ratios: Vec<u32> },
    /// Lets a pane fill its tab, or ends that.
    Zoom { pane: PaneId, on: bool },
    /// Takes a pane out of its tab and floats it over the window.
    Float {
        pane: PaneId,
        #[serde(default)]
        rect: Option<UnitRect>,
    },
    /// Moves or resizes a floating pane.
    FloatMove { pane: PaneId, rect: UnitRect },
    /// Puts a pane away for now, or brings it back. A pane that is put away floats
    /// out of sight: one that was in a tab is taken out of it first. Brought back, it
    /// has the keyboard, and is where it came from: floating where it floated, or
    /// among the tabs again, beside the pane that last had the keyboard.
    Minimise { pane: PaneId, on: bool },
    /// Puts a floating pane back among the tabs: where it is told, or beside the pane
    /// that last had the keyboard.
    Dock {
        pane: PaneId,
        #[serde(default)]
        to: Option<Place>,
    },
    /// Names a pane, or takes its name away.
    PaneRename {
        pane: PaneId,
        #[serde(default)]
        title: Option<String>,
    },
    /// Opens a tab with one pane. Without an index it goes after the last tab.
    TabNew {
        window: WindowId,
        content: NewContent,
        #[serde(default)]
        index: Option<u32>,
        /// Whether the new tab is shown at once. A window's first tab always is.
        #[serde(default = "yes")]
        activate: bool,
    },
    /// Removes a tab and every pane in it.
    TabClose { tab: TabId },
    /// Moves a tab to another place among its window's tabs.
    TabMove { tab: TabId, index: u32 },
    /// Names a tab, or takes its name away.
    TabRename {
        tab: TabId,
        #[serde(default)]
        title: Option<String>,
    },
    /// Shows a tab.
    TabActivate { target: TabTarget },
    /// Moves a panel to a place in a dock area of its window.
    PanelMove {
        window: WindowId,
        panel: PanelId,
        area: Area,
        index: u32,
    },
    /// Folds a dock area away or opens it. Without `on` it does whichever it is not.
    PanelToggle {
        window: WindowId,
        area: Area,
        #[serde(default)]
        on: Option<bool>,
    },
    /// Sets how far a dock area reaches into its window.
    PanelResize {
        window: WindowId,
        area: Area,
        size: u16,
    },
    /// Shows the workspace's scratch terminal over a window. The first time, it is
    /// made from `content`.
    ScratchShow {
        window: WindowId,
        #[serde(default)]
        content: Option<NewContent>,
    },
    /// Puts the scratch terminal away. Its program keeps running.
    ScratchHide {},
    /// Opens a window with one tab.
    WindowNew {
        content: NewContent,
        #[serde(default)]
        bounds: Option<Bounds>,
    },
    /// Closes a window and everything in it.
    WindowClose { window: WindowId },

    // The intents below record what the application alone knows. An authority refuses
    // them from anyone else.
    /// Records the session a terminal pane shows, or that it shows none any more. A
    /// backend or a directory given with it replaces the pane's.
    SessionSet {
        pane: PaneId,
        session: Option<SessionId>,
        #[serde(default)]
        backend: Option<BackendId>,
        #[serde(default)]
        cwd: Option<String>,
    },
    /// Puts a panel in a dock area, replacing any panel of the same id, or removes it
    /// when no area is given.
    PanelSet {
        window: WindowId,
        panel: PanelId,
        content: PanelContent,
        area: Option<Area>,
        index: u32,
    },
    /// Records where a window is on the desktop.
    WindowBounds { window: WindowId, bounds: Bounds },
    /// Records which window the user is working in.
    WindowFocus { window: WindowId },
    /// Gives the workspace another name. The file it is kept in is the application's
    /// to rename.
    WorkspaceRename { name: String },
}

fn yes() -> bool {
    true
}

impl Intent {
    /// Whether only the application may apply this intent.
    pub fn is_core_only(&self) -> bool {
        matches!(
            self,
            Intent::SessionSet { .. }
                | Intent::PanelSet { .. }
                | Intent::WindowBounds { .. }
                | Intent::WindowFocus { .. }
                | Intent::WorkspaceRename { .. }
        )
    }
}

/// What a new pane shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NewContent {
    /// A terminal, and how to start its session.
    Terminal(SpawnRequest),
    /// A view a plugin provides.
    PluginView {
        plugin: String,
        view: String,
        #[serde(default)]
        state: serde_json::Value,
    },
}

/// How the session of a new terminal pane is to be started. Everything may be left out:
/// the application then starts its default shell where new panes start.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnRequest {
    #[serde(default)]
    pub backend: Option<BackendId>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub launcher: Option<Launcher>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: Vec<(String, String)>,
    /// The grid the pane will have, if whoever asks can tell. A session started at the
    /// size its pane turns out to have draws its first screen once, not twice.
    #[serde(default)]
    pub size: Option<TermSize>,
}

/// Which pane is to have the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FocusTarget {
    /// This pane. If it is in a tab that is not shown, the tab is shown.
    Pane { pane: PaneId },
    /// The pane after the focused one, in the order the window lays its panes out.
    Next { window: WindowId },
    /// The pane before the focused one.
    Previous { window: WindowId },
    /// The pane that had the keyboard before the focused one.
    Back { window: WindowId },
}

/// Which tab is to be shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TabTarget {
    Id {
        tab: TabId,
    },
    /// The tab at this place among the window's tabs, counted from nought.
    Index {
        window: WindowId,
        index: u32,
    },
    /// The tab after the one shown, or the first after the last.
    Next {
        window: WindowId,
    },
    /// The tab before the one shown, or the last before the first.
    Previous {
        window: WindowId,
    },
}

/// Where a pane is to go.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Place {
    /// On one side of another pane, sharing that pane's space half and half.
    Beside { pane: PaneId, edge: Dir },
    /// In a new tab of its window. Without an index the tab goes after the last.
    NewTab {
        window: WindowId,
        #[serde(default)]
        index: Option<u32>,
    },
}
