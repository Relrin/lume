//! Intents: the only way a layout changes.
//!
//! Whoever wants the layout to be different (a page, a key, the command line, the
//! application itself) says what it wants as an intent, and the layout's authority
//! decides. An intent names what it is about by id, so it means the same thing whoever
//! sends it and whenever it arrives.

use lume_proto::{BackendId, Launcher, SessionId, TermSize};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use crate::ids::{GroupId, PaneId, PanelId, SplitId, TabId, WindowId};
use crate::model::{Area, Bounds, GroupIcon, PanelContent, Shelved, TabColor, UnitRect};

/// A direction on the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    /// Gives a pane the picture it is known by, or takes it away: the pane then shows
    /// the picture of what runs in it again.
    PaneIcon {
        pane: PaneId,
        #[serde(default)]
        icon: Option<String>,
    },
    /// Opens a tab with one pane. Without an index it goes after the last tab; in a
    /// group, it goes after the group's last tab and the index is not looked at.
    TabNew {
        window: WindowId,
        content: NewContent,
        #[serde(default)]
        index: Option<u32>,
        /// Whether the new tab is shown at once. A window's first tab always is.
        #[serde(default = "yes")]
        activate: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        group: Option<GroupId>,
    },
    /// Removes a tab and every pane in it. A locked tab is not removed.
    TabClose { tab: TabId },
    /// Removes several tabs of a window at once, passing over those that are locked.
    TabsClose { tabs: Vec<TabId> },
    /// Moves a tab to another place among its window's tabs. Put between two tabs of a
    /// group, it joins the group; put at the edge of its own group, it stays in it;
    /// anywhere else it is in no group.
    TabMove { tab: TabId, index: u32 },
    /// Moves a tab to a place among its window's tabs and puts it in a group, or in
    /// none. The tab ends up among the group's tabs whatever the place.
    TabPlace {
        tab: TabId,
        index: u32,
        #[serde(default)]
        group: Option<GroupId>,
    },
    /// Gives a tab a colour, or takes it away.
    TabColor {
        tab: TabId,
        #[serde(default)]
        color: Option<TabColor>,
    },
    /// Locks a tab against being closed, or unlocks it.
    TabLock { tab: TabId, on: bool },
    /// Mutes a tab, so that what runs in it does not ask for attention, or lets it
    /// ask again.
    TabMute { tab: TabId, on: bool },
    /// Moves the panes of a tab into another tab of its window, beside that tab's
    /// panes, and closes the tab.
    TabMerge { tab: TabId, into: TabId },
    /// Names a tab, or takes its name away.
    TabRename {
        tab: TabId,
        #[serde(default)]
        title: Option<String>,
    },
    /// Shows a tab. A folded group it is in is opened.
    TabActivate { target: TabTarget },
    /// Gathers a tab into a new group of its own, taking it out of any other.
    GroupNew { tab: TabId },
    /// Changes what a group is called and how it looks, folds or opens it, or sets
    /// whether it is kept or muted. What is left out stays as it is; a name or an icon given as
    /// null is taken away. Folding the group that holds the tab on show first shows
    /// the nearest tab outside it.
    GroupSet {
        group: GroupId,
        #[serde(
            default,
            deserialize_with = "given",
            skip_serializing_if = "Option::is_none"
        )]
        #[schemars(with = "Option<String>")]
        name: Option<Option<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<TabColor>,
        #[serde(
            default,
            deserialize_with = "given",
            skip_serializing_if = "Option::is_none"
        )]
        #[schemars(with = "Option<GroupIcon>")]
        icon: Option<Option<GroupIcon>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        collapsed: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kept: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        muted: Option<bool>,
    },
    /// Moves a group's tabs together to another place among the window's tabs:
    /// `index` is where its first tab goes among the tabs that are not in it. A group
    /// is not put inside another.
    GroupMove { group: GroupId, index: u32 },
    /// Ends a group. Its tabs stay where they are.
    GroupUngroup { group: GroupId },
    /// Closes a group's tabs. Locked tabs stay, out of the group. A kept group is put
    /// away with its tabs, to be opened again.
    GroupClose { group: GroupId },
    /// Opens a group that was put away again, after the last tab of a window.
    GroupReopen { group: GroupId, window: WindowId },
    /// Forgets a group that was put away.
    GroupForget { group: GroupId },
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
    /// Keeps what a plugin's view in a pane asked to find again.
    ViewState {
        pane: PaneId,
        state: serde_json::Value,
    },
    /// Keeps what a plugin's view in a panel asked to find again.
    PanelState {
        window: WindowId,
        panel: PanelId,
        state: serde_json::Value,
    },
    /// Records where a window is on the desktop.
    WindowBounds { window: WindowId, bounds: Bounds },
    /// Records which window the user is working in.
    WindowFocus { window: WindowId },
    /// Gives the workspace another name. The file it is kept in is the application's
    /// to rename.
    WorkspaceRename { name: String },
    /// Takes a group out of its window with all its tabs, locked or not, as one that
    /// moves to another workspace.
    GroupTake { group: GroupId },
    /// Puts a group from another workspace after the last tab of a window. Its parts
    /// are numbered anew here and its panes start without sessions.
    GroupGraft {
        window: WindowId,
        group: Box<Shelved>,
    },
}

fn yes() -> bool {
    true
}

/// A field that is there, even as null, as opposed to left out.
fn given<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

impl Intent {
    /// Whether only the application may apply this intent.
    pub fn is_core_only(&self) -> bool {
        matches!(
            self,
            Intent::SessionSet { .. }
                | Intent::PanelSet { .. }
                | Intent::ViewState { .. }
                | Intent::PanelState { .. }
                | Intent::WindowBounds { .. }
                | Intent::WindowFocus { .. }
                | Intent::WorkspaceRename { .. }
                | Intent::GroupTake { .. }
                | Intent::GroupGraft { .. }
        )
    }

    /// What the pane this intent makes is to show, for the intents that make one.
    pub fn new_content_mut(&mut self) -> Option<&mut NewContent> {
        match self {
            Intent::Split { content, .. }
            | Intent::TabNew { content, .. }
            | Intent::WindowNew { content, .. } => Some(content),
            Intent::ScratchShow { content, .. } => content.as_mut(),
            _ => None,
        }
    }
}

/// What a new pane shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    /// A session that is already running, for the pane to show instead of starting
    /// one: a session no pane shows any more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionId>,
}

/// Which pane is to have the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
