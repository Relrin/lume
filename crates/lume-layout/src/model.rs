//! The layout tree: a workspace's windows, and in each window its dock areas, its tabs
//! and the panes that float over them.
//!
//! A tab is a tree. Its leaves are panes; its inner nodes are splits, which divide their
//! space among their children by ratio. Nothing here is measured in pixels: a page lays
//! the tree out in whatever space its window has.

use lume_proto::{BackendId, Launcher, SessionId};
use serde::{Deserialize, Serialize};

use crate::ids::{GroupId, PaneId, PanelId, SplitId, TabId, WindowId, WorkspaceId};
use crate::tuning::{DOCK_SIZE_DEFAULT, UNITS};

/// A named arrangement of windows that can be saved and brought back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub windows: Vec<Window>,
    /// The window the user last worked in.
    pub focused_window: Option<WindowId>,
    /// The workspace's scratch terminal, if it has been opened.
    pub scratch: Option<Scratch>,
    /// The number the next pane, tab, split, group or window is given. It only goes
    /// up, so no number names two things in a workspace's life.
    pub next_id: u32,
    /// The kept groups that were closed, the most recently closed last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shelf: Vec<Shelved>,
}

/// The scratch terminal: one floating pane a workspace keeps at hand. It is shown over
/// whichever window asks for it and put away again without ending its program.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scratch {
    pub pane: PaneId,
    /// The pane and where it last floated, while it is put away. While it is shown it
    /// is among the floating panes of a window instead.
    pub parked: Option<Floating>,
}

/// One window of a workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub id: WindowId,
    /// Where the window is on the desktop, once that is known.
    pub bounds: Option<Bounds>,
    pub dock: DockAreas,
    /// The tabs in the middle of the window.
    pub centre: TabGroup,
    /// The panes floating over the tabs, from the one at the back to the one in front.
    pub floating: Vec<Floating>,
    /// The pane that has the keyboard: one of the active tab, or a floating one.
    pub focus: Option<PaneId>,
    /// The panes that had the keyboard before, most recent first, beginning with the
    /// one that has it now.
    pub history: Vec<PaneId>,
}

/// A window's place and size on the desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    /// Where the window's top left corner is, in the desktop's own pixels. On a
    /// desktop whose screens have different scales only those name one place.
    pub x: i32,
    pub y: i32,
    /// The size of what the window shows, in logical pixels: the window looks as large
    /// on a screen of another scale.
    pub width: u32,
    pub height: u32,
    /// The position and size above are the ones the window returns to when it is no
    /// longer maximised or full screen.
    pub maximised: bool,
    pub fullscreen: bool,
}

/// One of the four edges of a window, where panels are docked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Area {
    Top,
    Left,
    Right,
    Bottom,
}

impl Area {
    pub const ALL: [Area; 4] = [Area::Top, Area::Left, Area::Right, Area::Bottom];
}

/// The panels at each edge of a window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DockAreas {
    pub top: DockArea,
    pub left: DockArea,
    pub right: DockArea,
    pub bottom: DockArea,
}

impl DockAreas {
    pub fn area(&self, area: Area) -> &DockArea {
        match area {
            Area::Top => &self.top,
            Area::Left => &self.left,
            Area::Right => &self.right,
            Area::Bottom => &self.bottom,
        }
    }

    pub fn area_mut(&mut self, area: Area) -> &mut DockArea {
        match area {
            Area::Top => &mut self.top,
            Area::Left => &mut self.left,
            Area::Right => &mut self.right,
            Area::Bottom => &mut self.bottom,
        }
    }
}

impl Default for DockAreas {
    /// The areas of a new window: the top bar with its three slots, and nothing docked
    /// at the other edges.
    fn default() -> Self {
        let slot = |slot: Slot| Panel {
            id: slot.panel_id(),
            content: PanelContent::Slot { slot },
        };
        Self {
            top: DockArea {
                panels: vec![
                    slot(Slot::TopbarLeft),
                    slot(Slot::TopbarCenter),
                    slot(Slot::TopbarRight),
                ],
                ..DockArea::default()
            },
            left: DockArea::default(),
            right: DockArea::default(),
            bottom: DockArea::default(),
        }
    }
}

/// The panels docked at one edge of a window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DockArea {
    pub panels: Vec<Panel>,
    /// How far the area reaches into the window, in logical pixels: its width at the
    /// left and right, its height at the bottom. The top bar's height is the theme's.
    pub size: u16,
    /// Whether the area is folded away.
    pub collapsed: bool,
}

impl Default for DockArea {
    fn default() -> Self {
        Self {
            panels: Vec::new(),
            size: DOCK_SIZE_DEFAULT,
            collapsed: false,
        }
    }
}

/// Something docked at an edge of a window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Panel {
    pub id: PanelId,
    pub content: PanelContent,
}

/// What a panel shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanelContent {
    /// One of the top bar's three places for status items.
    Slot { slot: Slot },
    /// The list of the window's tabs.
    TabList {},
    /// A view a plugin provides.
    PluginView { plugin: String, view: String },
}

/// A place in the top bar that status items are shown in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Slot {
    #[serde(rename = "topbar.left")]
    TopbarLeft,
    #[serde(rename = "topbar.center")]
    TopbarCenter,
    #[serde(rename = "topbar.right")]
    TopbarRight,
}

impl Slot {
    /// The id of the panel that holds this slot in every window.
    pub fn panel_id(self) -> PanelId {
        let name = match self {
            Slot::TopbarLeft => "lume.topbar.left",
            Slot::TopbarCenter => "lume.topbar.center",
            Slot::TopbarRight => "lume.topbar.right",
        };
        PanelId::new(name).expect("the slots' panel ids are well formed")
    }
}

/// The tabs of a window.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabGroup {
    pub tabs: Vec<Tab>,
    /// The tab on show. There is one whenever there are tabs.
    pub active: Option<TabId>,
    /// The groups the tabs are gathered in, in the order their first tabs come. The
    /// tabs of a group stand together, and a group has at least one tab.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Group>,
}

/// One tab: a tree of splits and panes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tab {
    pub id: TabId,
    /// The name the user gave the tab. Without one, a page names the tab after what
    /// runs in it.
    pub title: Option<String>,
    pub root: Node,
    /// The pane that fills the tab for now, hiding the others.
    pub zoomed: Option<PaneId>,
    /// The pane of this tab that last had the keyboard, which gets it again when the
    /// tab is shown.
    pub last_focus: Option<PaneId>,
    /// The colour the user gave the tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<TabColor>,
    /// A locked tab is not closed by asking to close it, nor by closing the tabs
    /// beside it or its group. Its panes still close on their own.
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
    /// The group of the window's tabs this tab is in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<GroupId>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// The colours a tab or a group can be given. A page draws each from its theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabColor {
    Red,
    Orange,
    Yellow,
    Green,
    Teal,
    Blue,
    Purple,
    Pink,
}

impl TabColor {
    /// The order a window's new groups take their colours in, each the first one no
    /// other group of the window has.
    pub const FOR_GROUPS: [TabColor; 8] = [
        TabColor::Blue,
        TabColor::Pink,
        TabColor::Yellow,
        TabColor::Purple,
        TabColor::Teal,
        TabColor::Red,
        TabColor::Green,
        TabColor::Orange,
    ];
}

/// The pictures a group's label can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupIcon {
    Bug,
    Flame,
    Rocket,
    Flask,
    Wrench,
    Layers,
    Server,
    Book,
}

/// Tabs of a window gathered under one name and colour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub id: GroupId,
    /// The name the user gave the group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub color: TabColor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<GroupIcon>,
    /// A folded group shows only its label. The tab on show is never in one.
    #[serde(default, skip_serializing_if = "is_false")]
    pub collapsed: bool,
    /// A kept group is put away when it is closed, to be opened again later, instead
    /// of being forgotten.
    #[serde(default, skip_serializing_if = "is_false")]
    pub kept: bool,
}

/// A kept group that was closed, with its tabs as they were. Its panes have no
/// sessions: they are started anew when the group is opened again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shelved {
    pub group: Group,
    pub tabs: Vec<Tab>,
}

/// A node of a tab's tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Node {
    Split(Split),
    Pane(Pane),
}

/// The direction a split lays its children out in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    /// Side by side, from left to right.
    Row,
    /// One above the other, from top to bottom.
    Column,
}

/// A node that divides its space among two or more children.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Split {
    pub id: SplitId,
    pub axis: Axis,
    /// Each child's share of the split, in [`UNITS`]. As many as there are children,
    /// adding up to exactly [`UNITS`].
    pub ratios: Vec<u32>,
    pub children: Vec<Node>,
}

/// A leaf of a tab's tree, or a floating pane: one place that shows one thing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pane {
    pub id: PaneId,
    /// The name the user gave the pane.
    pub title: Option<String>,
    pub content: PaneContent,
}

/// What a pane shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaneContent {
    Terminal(TerminalPane),
    /// A view a plugin provides, with whatever the view asked to have kept for it.
    PluginView {
        plugin: String,
        view: String,
        state: serde_json::Value,
    },
}

/// A pane that shows a terminal session, and what is needed to start the session again.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalPane {
    /// The backend the session runs on. Without one, it is the application's default.
    pub backend: Option<BackendId>,
    /// The running session. A pane has none before its session is started, after its
    /// program has ended, and when it has just been read from a file.
    pub session: Option<SessionId>,
    /// The profile the session was started from.
    pub profile: Option<String>,
    /// What the session runs. Without one, it is the profile's or the default shell.
    pub launcher: Option<Launcher>,
    /// The directory the session was started in.
    pub cwd: Option<String>,
    /// Variables the session was started with, on top of the environment.
    pub env: Vec<(String, String)>,
}

/// A pane that floats over a window's tabs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Floating {
    pub pane: Pane,
    pub rect: UnitRect,
    /// Set while the pane is put away. It keeps its place, and its program keeps
    /// running; a page shows it as a small marker to bring it back by. A pane that is
    /// put away cannot have the keyboard.
    #[serde(default)]
    pub minimised: Option<Minimised>,
}

/// Where a pane that is put away goes when it is brought back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Minimised {
    /// It floated, and floats again where it was.
    Floating,
    /// It was in a tab, and goes back among the tabs.
    Tab,
}

/// A rectangle within a whole that is [`UNITS`] wide and [`UNITS`] high.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl UnitRect {
    /// The whole.
    pub const FULL: UnitRect = UnitRect {
        x: 0,
        y: 0,
        w: UNITS,
        h: UNITS,
    };

    /// The coordinate just past the rectangle's right edge.
    pub fn right(self) -> u32 {
        self.x.saturating_add(self.w)
    }

    /// The coordinate just past the rectangle's bottom edge.
    pub fn bottom(self) -> u32 {
        self.y.saturating_add(self.h)
    }
}

/// Where in a workspace a pane is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Location {
    /// In the tree of a tab: the indices of the window and of the tab in it.
    Tab { window: usize, tab: usize },
    /// Floating over a window: the indices of the window and of the pane among its
    /// floating panes.
    Floating { window: usize, index: usize },
    /// The scratch terminal, put away.
    Parked,
}

impl Node {
    /// The ids of the panes under this node, in the order they are laid out.
    pub fn pane_ids(&self) -> Vec<PaneId> {
        self.panes().into_iter().map(|pane| pane.id).collect()
    }

    /// The panes under this node, in the order they are laid out.
    pub fn panes(&self) -> Vec<&Pane> {
        let mut found = Vec::new();
        self.collect_panes(&mut found);
        found
    }

    fn collect_panes<'a>(&'a self, found: &mut Vec<&'a Pane>) {
        match self {
            Node::Pane(pane) => found.push(pane),
            Node::Split(split) => {
                for child in &split.children {
                    child.collect_panes(found);
                }
            }
        }
    }

    /// The splits under this node, each before the splits inside it.
    pub fn splits(&self) -> Vec<&Split> {
        let mut found = Vec::new();
        self.collect_splits(&mut found);
        found
    }

    fn collect_splits<'a>(&'a self, found: &mut Vec<&'a Split>) {
        if let Node::Split(split) = self {
            found.push(split);
            for child in &split.children {
                child.collect_splits(found);
            }
        }
    }

    pub fn pane(&self, id: PaneId) -> Option<&Pane> {
        match self {
            Node::Pane(pane) => (pane.id == id).then_some(pane),
            Node::Split(split) => split.children.iter().find_map(|child| child.pane(id)),
        }
    }

    pub fn pane_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        match self {
            Node::Pane(pane) => (pane.id == id).then_some(pane),
            Node::Split(split) => split
                .children
                .iter_mut()
                .find_map(|child| child.pane_mut(id)),
        }
    }

    pub fn split(&self, id: SplitId) -> Option<&Split> {
        match self {
            Node::Pane(_) => None,
            Node::Split(split) if split.id == id => Some(split),
            Node::Split(split) => split.children.iter().find_map(|child| child.split(id)),
        }
    }

    pub fn split_mut(&mut self, id: SplitId) -> Option<&mut Split> {
        let Node::Split(split) = self else {
            return None;
        };
        if split.id == id {
            return Some(split);
        }
        split
            .children
            .iter_mut()
            .find_map(|child| child.split_mut(id))
    }

    /// How many splits lie between this node and its deepest pane.
    pub fn depth(&self) -> usize {
        match self {
            Node::Pane(_) => 0,
            Node::Split(split) => 1 + split.children.iter().map(Node::depth).max().unwrap_or(0),
        }
    }

    /// Whether two trees have the same splits and panes in the same places, whatever
    /// the splits' ratios and the panes' contents.
    pub fn same_shape(&self, other: &Node) -> bool {
        match (self, other) {
            (Node::Pane(a), Node::Pane(b)) => a.id == b.id,
            (Node::Split(a), Node::Split(b)) => {
                a.id == b.id
                    && a.axis == b.axis
                    && a.children.len() == b.children.len()
                    && a.children
                        .iter()
                        .zip(&b.children)
                        .all(|(a, b)| a.same_shape(b))
            }
            _ => false,
        }
    }
}

impl Tab {
    /// The ids of the tab's panes, in the order they are laid out.
    pub fn pane_ids(&self) -> Vec<PaneId> {
        self.root.pane_ids()
    }

    pub fn contains(&self, pane: PaneId) -> bool {
        self.root.pane(pane).is_some()
    }

    /// The tab's first pane: the one at its top left.
    pub fn first_pane(&self) -> PaneId {
        let mut node = &self.root;
        loop {
            match node {
                Node::Pane(pane) => return pane.id,
                // A split has children; one without is not a tree `validate` passes.
                Node::Split(split) => match split.children.first() {
                    Some(child) => node = child,
                    None => return PaneId::new(0),
                },
            }
        }
    }
}

impl TabGroup {
    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    pub fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    pub fn index_of(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    /// The tab on show.
    pub fn active_tab(&self) -> Option<&Tab> {
        self.tab(self.active?)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tab_mut(self.active?)
    }

    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.groups.iter().find(|group| group.id == id)
    }

    pub fn group_mut(&mut self, id: GroupId) -> Option<&mut Group> {
        self.groups.iter_mut().find(|group| group.id == id)
    }

    /// The indices of a group's tabs, in order.
    pub fn members(&self, id: GroupId) -> Vec<usize> {
        self.tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| tab.group == Some(id))
            .map(|(index, _)| index)
            .collect()
    }

    /// Whether a tab is hidden in a folded group.
    pub fn folded(&self, tab: &Tab) -> bool {
        tab.group
            .and_then(|group| self.group(group))
            .is_some_and(|group| group.collapsed)
    }
}

impl Window {
    /// A window with the standard top bar and nothing in it.
    pub fn new(id: WindowId) -> Self {
        Self {
            id,
            bounds: None,
            dock: DockAreas::default(),
            centre: TabGroup::default(),
            floating: Vec::new(),
            focus: None,
            history: Vec::new(),
        }
    }

    /// Every pane of the window: those of its tabs, tab by tab, then the floating ones
    /// from back to front.
    pub fn panes(&self) -> Vec<&Pane> {
        let mut found = Vec::new();
        for tab in &self.centre.tabs {
            tab.root.collect_panes(&mut found);
        }
        found.extend(self.floating.iter().map(|floating| &floating.pane));
        found
    }

    pub fn contains(&self, pane: PaneId) -> bool {
        self.tab_index_of(pane).is_some() || self.floating_index(pane).is_some()
    }

    /// The index of the tab that holds a pane.
    pub fn tab_index_of(&self, pane: PaneId) -> Option<usize> {
        self.centre.tabs.iter().position(|tab| tab.contains(pane))
    }

    /// The index of a pane among the floating ones.
    pub fn floating_index(&self, pane: PaneId) -> Option<usize> {
        self.floating
            .iter()
            .position(|floating| floating.pane.id == pane)
    }

    /// The panes the keyboard may be given to: those of the tab on show, in the order
    /// they are laid out, then the floating ones that are not put away, from back to
    /// front.
    pub fn focusable(&self) -> Vec<PaneId> {
        let mut panes = self
            .centre
            .active_tab()
            .map(Tab::pane_ids)
            .unwrap_or_default();
        panes.extend(self.shown_floating().map(|floating| floating.pane.id));
        panes
    }

    /// The floating panes that are on show, from back to front: those that are not
    /// put away.
    pub fn shown_floating(&self) -> impl Iterator<Item = &Floating> {
        self.floating
            .iter()
            .filter(|floating| floating.minimised.is_none())
    }
}

impl Workspace {
    /// A workspace with no windows yet.
    pub fn new(id: WorkspaceId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            windows: Vec::new(),
            focused_window: None,
            scratch: None,
            next_id: 1,
            shelf: Vec::new(),
        }
    }

    /// Takes the next number for a pane, tab, split, group or window.
    pub(crate) fn fresh(&mut self) -> u32 {
        let number = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        number
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|window| window.id == id)
    }

    pub fn window_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows.iter_mut().find(|window| window.id == id)
    }

    pub(crate) fn window_index(&self, id: WindowId) -> Option<usize> {
        self.windows.iter().position(|window| window.id == id)
    }

    /// Every pane of the workspace: each window's, then the scratch terminal if it is
    /// put away.
    pub fn panes(&self) -> Vec<&Pane> {
        let mut found: Vec<&Pane> = self.windows.iter().flat_map(Window::panes).collect();
        found.extend(self.parked().map(|floating| &floating.pane));
        found
    }

    /// The scratch terminal, if it is put away.
    pub(crate) fn parked(&self) -> Option<&Floating> {
        self.scratch.as_ref()?.parked.as_ref()
    }

    pub(crate) fn locate(&self, pane: PaneId) -> Option<Location> {
        for (index, window) in self.windows.iter().enumerate() {
            if let Some(tab) = window.tab_index_of(pane) {
                return Some(Location::Tab { window: index, tab });
            }
            if let Some(floating) = window.floating_index(pane) {
                return Some(Location::Floating {
                    window: index,
                    index: floating,
                });
            }
        }
        self.parked()
            .is_some_and(|floating| floating.pane.id == pane)
            .then_some(Location::Parked)
    }

    /// A pane, wherever in the workspace it is.
    pub fn pane(&self, id: PaneId) -> Option<&Pane> {
        self.panes().into_iter().find(|pane| pane.id == id)
    }

    pub fn pane_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        match self.locate(id)? {
            Location::Tab { window, tab } => {
                self.windows[window].centre.tabs[tab].root.pane_mut(id)
            }
            Location::Floating { window, index } => {
                Some(&mut self.windows[window].floating[index].pane)
            }
            Location::Parked => self
                .scratch
                .as_mut()
                .and_then(|scratch| scratch.parked.as_mut())
                .map(|floating| &mut floating.pane),
        }
    }

    /// The window a pane is shown in. The scratch terminal is in none while it is put
    /// away.
    pub fn window_of(&self, pane: PaneId) -> Option<WindowId> {
        match self.locate(pane)? {
            Location::Tab { window, .. } | Location::Floating { window, .. } => {
                Some(self.windows[window].id)
            }
            Location::Parked => None,
        }
    }

    /// The window a group of tabs is in, by index.
    pub(crate) fn locate_group(&self, group: GroupId) -> Option<usize> {
        self.windows
            .iter()
            .position(|window| window.centre.group(group).is_some())
    }

    /// The window and the tab a tab id names.
    pub(crate) fn locate_tab(&self, tab: TabId) -> Option<(usize, usize)> {
        self.windows
            .iter()
            .enumerate()
            .find_map(|(index, window)| Some((index, window.centre.index_of(tab)?)))
    }

    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        let (window, tab) = self.locate_tab(id)?;
        Some(&self.windows[window].centre.tabs[tab])
    }

    /// The tab that holds a split.
    pub(crate) fn locate_split(&self, split: SplitId) -> Option<(usize, usize)> {
        self.windows.iter().enumerate().find_map(|(index, window)| {
            let tab = window
                .centre
                .tabs
                .iter()
                .position(|tab| tab.root.split(split).is_some())?;
            Some((index, tab))
        })
    }

    /// The sessions the workspace's terminal panes show.
    pub fn sessions(&self) -> Vec<(PaneId, SessionId)> {
        self.panes()
            .into_iter()
            .filter_map(|pane| match &pane.content {
                PaneContent::Terminal(terminal) => Some((pane.id, terminal.session?)),
                PaneContent::PluginView { .. } => None,
            })
            .collect()
    }
}
