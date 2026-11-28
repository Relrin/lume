//! What an applied intent changed, and the layout as a whole.
//!
//! A page keeps a copy of the layout and brings it up to date from changes. Changes
//! that happen many times a second while something is dragged are small: a split's
//! ratios, a floating pane's rectangle. A change to the structure of a tab carries the
//! tab's whole tree, which leaves a page nothing to get wrong.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::{GroupId, PaneId, SplitId, TabId, WindowId};
use crate::model::{
    Area, Bounds, DockArea, Floating, Group, Node, Pane, PaneContent, Scratch, Shelved, Tab,
    TabColor, UnitRect, Window, Workspace,
};

/// One difference between a layout before an intent and after it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LayoutChange {
    /// A window was opened. It is given whole.
    WindowOpened { window: Window },
    /// Where a window is on the desktop changed.
    WindowBounds { window: WindowId, bounds: Bounds },
    /// A tab was added to a window. Its place among the window's tabs is given by the
    /// `Tabs` change that follows.
    TabAdded {
        window: WindowId,
        tab: Tab,
        index: u32,
    },
    /// A tab left its window.
    TabRemoved { tab: TabId },
    /// The order of a window's tabs, or which of them is shown, changed.
    Tabs {
        window: WindowId,
        order: Vec<TabId>,
        active: Option<TabId>,
    },
    /// A tab was named, or lost its name.
    TabTitle { tab: TabId, title: Option<String> },
    /// A tab's colour, its lock, whether it is muted, or the group it is in changed.
    TabState {
        tab: TabId,
        color: Option<TabColor>,
        locked: bool,
        /// Left out while the tab is not muted.
        #[serde(default, skip_serializing_if = "crate::model::is_false")]
        muted: bool,
        group: Option<GroupId>,
        /// The layout file the tab was opened from. Left out while there is none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin: Option<String>,
        /// The open workspace the tab belongs to. Left out for the one whose file holds
        /// the layout.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
    },
    /// The groups of a window's tabs changed: which there are, their order, or how
    /// one looks.
    Groups {
        window: WindowId,
        groups: Vec<Group>,
    },
    /// The splits and panes of a tab changed, or which pane fills it.
    TabTree {
        tab: TabId,
        root: Node,
        zoomed: Option<PaneId>,
    },
    /// The shares of a split's children changed.
    Ratios { split: SplitId, ratios: Vec<u32> },
    /// What a pane shows, or its name, changed.
    Pane { pane: Pane },
    /// The panes floating over a window changed: which there are, their order, or
    /// which of them are put away.
    Floating {
        window: WindowId,
        floating: Vec<Floating>,
    },
    /// A floating pane was moved or resized.
    FloatRect { pane: PaneId, rect: UnitRect },
    /// The panels of a dock area changed, or whether it is folded away.
    Dock {
        window: WindowId,
        area: Area,
        dock: DockArea,
    },
    /// How far a dock area reaches into its window changed.
    DockSize {
        window: WindowId,
        area: Area,
        size: u16,
    },
    /// Which pane of a window has the keyboard changed.
    Focus {
        window: WindowId,
        pane: Option<PaneId>,
    },
    /// A window was closed.
    WindowClosed { window: WindowId },
    /// The scratch terminal was made, shown, put away or closed.
    Scratch { scratch: Option<Scratch> },
    /// Which window the user is working in changed.
    WindowFocused { window: Option<WindowId> },
    /// The workspace was given another name.
    WorkspaceName { name: String },
    /// The groups that were put away changed.
    Shelf { shelf: Vec<Shelved> },
    /// A pane is gone from the workspace. What it showed is given, so whoever kept
    /// something for the pane, such as its session, can let go of it. The changes
    /// before this one have already taken the pane out of wherever it was.
    PaneClosed { pane: PaneId, content: PaneContent },
}

/// A whole layout, and where it stands in the sequence of its changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayoutSnapshot {
    /// Counts how many times the layout was replaced as a whole. Revisions are counted
    /// anew in each epoch.
    pub epoch: u32,
    /// Counts the intents that changed the layout in this epoch.
    pub revision: u64,
    pub workspace: Workspace,
}

/// What an authority tells those who follow its layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutEvent {
    /// An intent changed the layout. The changes take a copy at `revision - 1` of this
    /// epoch to `revision`.
    Changed {
        epoch: u32,
        revision: u64,
        changes: Vec<LayoutChange>,
    },
    /// The layout as a whole: it was replaced, or the follower missed changes.
    Reset(Box<LayoutSnapshot>),
}

/// What applying an intent gave.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Applied {
    pub epoch: u32,
    /// The layout's revision once the intent was applied. An intent that changed
    /// nothing leaves the revision as it was.
    pub revision: u64,
    pub created: Created,
}

/// What an intent made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Created {
    pub pane: Option<PaneId>,
    pub tab: Option<TabId>,
    pub window: Option<WindowId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<GroupId>,
}
