//! Applying an intent to a workspace.
//!
//! [`apply`] is a plain function of the workspace, the intent and, for an intent that
//! makes a terminal pane, the session that was started for it. It does no I/O and
//! draws on nothing else, so the same intents applied to the same workspace always
//! give the same workspace, and it can be tested without anything running.
//!
//! The intent is carried out on a copy. If it is refused, the copy is dropped and the
//! workspace is untouched. If it is accepted, the copy is brought to its proper form,
//! checked against the limits, compared with the original to find what changed, and
//! then takes the original's place.

use std::collections::HashMap;

use lume_proto::{BackendId, SessionId};

use crate::change::{Created, LayoutChange};
use crate::diff::diff;
use crate::error::IntentError;
use crate::geometry::{neighbour, tab_rects};
use crate::ids::{GroupId, PaneId, SplitId, TabId, WindowId};
use crate::intent::{Dir, FocusTarget, Intent, NewContent, Place, SpawnRequest, TabTarget};
use crate::model::{
    Area, Axis, Floating, Group, Location, Minimised, Node, Pane, PaneContent, Panel, PanelContent,
    Scratch, Shelved, Split, Tab, TabColor, TerminalPane, UnitRect, Window, Workspace,
};
use crate::normalise::{contain, fit, mend, release, tidy_trees};
use crate::persist::valid_name;
use crate::tuning::{
    DOCK_PANELS_MAX, DOCK_SIZE_MAX, DOCK_SIZE_MIN, FLOAT_CASCADE, FLOAT_DEFAULT, FLOATING_MAX,
    ICON_MAX_CHARS, RATIO_TOLERANCE, SHARE_MIN, SHELF_MAX, SPLIT_CHILDREN_MAX, TAB_PANES_MAX,
    TABS_MAX, TITLE_MAX_CHARS, TREE_DEPTH_MAX, UNITS, WINDOWS_MAX,
};

/// A session that was started for a pane an intent is about to make.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Started {
    /// The backend the session runs on.
    pub backend: BackendId,
    pub session: SessionId,
    /// The directory the session was started in, once whoever started it had chosen.
    pub cwd: Option<String>,
}

/// The session an intent needs started before it is applied, if it needs one: the
/// intents that make a terminal pane in a window that is already there, and those that
/// show a session that runs already, wherever it goes.
///
/// A window that is only being opened has no page yet to say how large its pane will
/// be, so its pane is made without a session, which is started once the page is there.
/// A session that runs already needs no size: the new window shows it as it is.
pub fn needs<'a>(workspace: &Workspace, intent: &'a Intent) -> Option<&'a SpawnRequest> {
    match intent {
        Intent::WindowNew {
            content: NewContent::Terminal(request),
            ..
        } if request.session.is_some() => Some(request),
        Intent::Split {
            content: NewContent::Terminal(request),
            ..
        }
        | Intent::TabNew {
            content: NewContent::Terminal(request),
            ..
        } => Some(request),
        Intent::ScratchShow {
            content: Some(NewContent::Terminal(request)),
            ..
        } if workspace.scratch.is_none() => Some(request),
        _ => None,
    }
}

/// Whether an intent would be accepted, without applying it. An intent that needs a
/// session is checked before the session is started, so that a refused intent starts
/// nothing.
pub fn check(workspace: &Workspace, intent: &Intent) -> Result<(), IntentError> {
    carry_out(workspace, intent.clone(), None).map(drop)
}

/// Applies an intent. On success the workspace has changed and the changes are
/// returned, with the ids of what the intent made; an intent may be accepted and
/// change nothing. On failure the workspace is as it was.
///
/// `started` is the session of the terminal pane the intent makes. Without one the
/// pane is made with no session yet.
pub fn apply(
    workspace: &mut Workspace,
    intent: Intent,
    started: Option<Started>,
) -> Result<(Vec<LayoutChange>, Created), IntentError> {
    let (next, created) = carry_out(workspace, intent, started)?;
    let changes = diff(workspace, &next);
    *workspace = next;
    Ok((changes, created))
}

/// Carries an intent out on a copy of a workspace and returns the copy in its proper
/// form.
fn carry_out(
    before: &Workspace,
    intent: Intent,
    started: Option<Started>,
) -> Result<(Workspace, Created), IntentError> {
    let mut workspace = before.clone();
    let mut started = started;
    let created = change(&mut workspace, intent, &mut started)?;
    if started.is_some() {
        return Err(IntentError::SessionUnused);
    }
    tidy_trees(&mut workspace);
    end_stale_zoom(before, &mut workspace);
    mend(&mut workspace);
    within_limits(&workspace)?;
    Ok((workspace, created))
}

/// A zoom ends when the tree of its tab changes: the pane that filled the tab is no
/// longer among the panes it was zoomed over.
fn end_stale_zoom(before: &Workspace, after: &mut Workspace) {
    for window in &mut after.windows {
        for tab in &mut window.centre.tabs {
            let Some(old) = before.tab(tab.id) else {
                continue;
            };
            if old.zoomed.is_some() && old.zoomed == tab.zoomed && !old.root.same_shape(&tab.root) {
                tab.zoomed = None;
            }
        }
    }
}

/// Refuses a workspace that has grown past what a workspace may hold.
fn within_limits(workspace: &Workspace) -> Result<(), IntentError> {
    if workspace.windows.len() > WINDOWS_MAX {
        return Err(IntentError::TooMany("windows"));
    }
    for window in &workspace.windows {
        if window.centre.tabs.len() > TABS_MAX {
            return Err(IntentError::TooMany("tabs"));
        }
        if window.floating.len() > FLOATING_MAX {
            return Err(IntentError::TooMany("floating panes"));
        }
        if Area::ALL
            .iter()
            .any(|area| window.dock.area(*area).panels.len() > DOCK_PANELS_MAX)
        {
            return Err(IntentError::TooMany("panels in a dock area"));
        }
        for tab in &window.centre.tabs {
            if tab.pane_ids().len() > TAB_PANES_MAX {
                return Err(IntentError::TooMany("panes in a tab"));
            }
            if tab.root.depth() > TREE_DEPTH_MAX {
                return Err(IntentError::TooMany("splits inside one another"));
            }
            for split in tab.root.splits() {
                if split.children.len() > SPLIT_CHILDREN_MAX {
                    return Err(IntentError::TooMany("panes side by side"));
                }
                if split.ratios.iter().any(|ratio| *ratio < SHARE_MIN) {
                    return Err(IntentError::TooSmall);
                }
            }
        }
    }
    Ok(())
}

/// A name as it is kept: without control characters or surrounding space, no longer
/// than a name may be, and absent when nothing is left.
fn name(title: Option<String>) -> Option<String> {
    let title: String = title?
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    let title: String = title.trim().chars().take(TITLE_MAX_CHARS).collect();
    let title = title.trim_end();
    (!title.is_empty()).then(|| title.to_owned())
}

/// The name of a picture as it is kept: lower-case letters, digits and `.`, `_`, `:`
/// and `-`, no longer than such a name may be, and absent when it is anything else.
fn picture(icon: Option<String>) -> Option<String> {
    let icon = icon?;
    let plain = !icon.is_empty()
        && icon.len() <= ICON_MAX_CHARS
        && icon.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '_' | ':' | '-')
        });
    plain.then_some(icon)
}

/// Makes the pane an intent asked for.
fn new_pane(workspace: &mut Workspace, content: NewContent, started: &mut Option<Started>) -> Pane {
    let content = match content {
        NewContent::Terminal(request) => {
            let started = started.take();
            let (session, backend, cwd) = match started {
                Some(started) => (Some(started.session), Some(started.backend), started.cwd),
                None => (None, None, None),
            };
            PaneContent::Terminal(TerminalPane {
                backend: backend.or(request.backend),
                session,
                profile: request.profile,
                launcher: request.launcher,
                cwd: cwd.or(request.cwd),
                env: request.env,
            })
        }
        NewContent::PluginView {
            plugin,
            view,
            state,
        } => PaneContent::PluginView {
            plugin,
            view,
            state,
        },
    };
    Pane {
        id: PaneId::new(workspace.fresh()),
        title: None,
        icon: None,
        content,
    }
}

fn window_index(workspace: &Workspace, window: WindowId) -> Result<usize, IntentError> {
    workspace
        .window_index(window)
        .ok_or(IntentError::NoSuchWindow(window))
}

/// Where a pane is, for an intent that is about a pane on show. The scratch terminal,
/// while it is put away, is not on show.
fn shown(workspace: &Workspace, pane: PaneId) -> Result<Location, IntentError> {
    match workspace.locate(pane) {
        Some(Location::Parked) | None => Err(IntentError::NoSuchPane(pane)),
        Some(location) => Ok(location),
    }
}

/// The window and the tab of a pane, for an intent that is about a pane in a tab.
fn tiled(workspace: &Workspace, pane: PaneId) -> Result<(usize, usize), IntentError> {
    match shown(workspace, pane)? {
        Location::Tab { window, tab } => Ok((window, tab)),
        _ => Err(IntentError::Floating(pane)),
    }
}

/// Gives a pane of a window the keyboard: its tab is shown, a zoom over another pane
/// of the tab ends, and a floating pane comes to the front.
fn focus(window: &mut Window, pane: PaneId) {
    if let Some(index) = window.tab_index_of(pane) {
        let tab = &mut window.centre.tabs[index];
        if tab.zoomed.is_some_and(|zoomed| zoomed != pane) {
            tab.zoomed = None;
        }
        window.centre.active = Some(tab.id);
    } else if let Some(index) = window.floating_index(pane) {
        // The keyboard cannot be with a pane that is put away: it is brought back.
        let mut floating = window.floating.remove(index);
        floating.minimised = None;
        window.floating.push(floating);
    }
    window.focus = Some(pane);
}

/// Shows a tab and gives the keyboard to the pane of it that last had it.
fn show_tab(window: &mut Window, index: usize) {
    let tab = &window.centre.tabs[index];
    let pane = tab
        .last_focus
        .filter(|pane| tab.contains(*pane))
        .unwrap_or_else(|| tab.first_pane());
    focus(window, pane);
}

/// Takes a tab out of a window. If it was the one shown, the tab after it is shown,
/// or the one before it when it was the last; a tab in a folded group is passed over
/// while there is another.
fn remove_tab(window: &mut Window, index: usize) -> Tab {
    let tab = window.centre.tabs.remove(index);
    if window.centre.active == Some(tab.id) {
        let centre = &window.centre;
        let tabs = &centre.tabs;
        let open = |found: &&Tab| !centre.folded(found);
        window.centre.active = tabs[index.min(tabs.len())..]
            .iter()
            .find(open)
            .or_else(|| tabs[..index.min(tabs.len())].iter().rev().find(open))
            .or_else(|| tabs.get(index))
            .or_else(|| tabs.last())
            .map(|neighbour| neighbour.id);
    }
    tab
}

/// The group a tab put at `index` among `tabs`, the window's tabs without it, is in.
/// Between two tabs of a group it joins that group. Beside a tab of its own group, or
/// when it was the only tab of its group, it stays in its own. Anywhere else it is in
/// none.
fn group_at(tabs: &[Tab], index: usize, own: Option<GroupId>) -> Option<GroupId> {
    let before = index
        .checked_sub(1)
        .and_then(|before| tabs.get(before))
        .and_then(|tab| tab.group);
    let after = tabs.get(index).and_then(|tab| tab.group);
    if before.is_some() && before == after {
        return before;
    }
    let own = own?;
    let alone = tabs.iter().all(|tab| tab.group != Some(own));
    (alone || before == Some(own) || after == Some(own)).then_some(own)
}

fn group_window(workspace: &Workspace, group: GroupId) -> Result<usize, IntentError> {
    workspace
        .locate_group(group)
        .ok_or(IntentError::NoSuchGroup(group))
}

fn tab_index(workspace: &Workspace, tab: TabId) -> Result<(usize, usize), IntentError> {
    workspace.locate_tab(tab).ok_or(IntentError::NoSuchTab(tab))
}

/// Leaves the panes under a node without sessions.
fn forget_sessions(node: &mut Node) {
    match node {
        Node::Pane(pane) => {
            if let PaneContent::Terminal(terminal) = &mut pane.content {
                terminal.session = None;
            }
        }
        Node::Split(split) => split.children.iter_mut().for_each(forget_sessions),
    }
}

/// Gives the splits and panes under a node new numbers from a workspace's counter,
/// noting what each pane was numbered before.
fn renumber(node: &mut Node, workspace: &mut Workspace, panes: &mut HashMap<PaneId, PaneId>) {
    match node {
        Node::Pane(pane) => {
            let id = PaneId::new(workspace.fresh());
            panes.insert(pane.id, id);
            pane.id = id;
        }
        Node::Split(split) => {
            split.id = SplitId::new(workspace.fresh());
            for child in &mut split.children {
                renumber(child, workspace, panes);
            }
        }
    }
}

/// Puts tabs after the last tab of a window, in a group, and shows the first of them.
fn put_group(window: &mut Window, group: Group, tabs: Vec<Tab>) {
    let first = window.centre.tabs.len();
    let id = group.id;
    window.centre.groups.push(group);
    window.centre.tabs.extend(tabs.into_iter().map(|mut tab| {
        tab.group = Some(id);
        tab
    }));
    if first < window.centre.tabs.len() {
        show_tab(window, first);
    }
}

/// Removes the tabs at these indices of a window, from the last to the first, so that
/// the tab shown afterwards is the one beside where they were.
fn remove_tabs(window: &mut Window, mut indices: Vec<usize>) -> Vec<Tab> {
    indices.sort_unstable();
    let mut removed: Vec<Tab> = indices
        .into_iter()
        .rev()
        .map(|index| remove_tab(window, index))
        .collect();
    removed.reverse();
    removed
}

/// Takes a pane out of the split it is a child of, somewhere under `node`, and gives
/// its share to its siblings.
fn take_from_split(node: &mut Node, pane: PaneId) -> Option<Pane> {
    let Node::Split(split) = node else {
        return None;
    };
    let direct = split
        .children
        .iter()
        .position(|child| matches!(child, Node::Pane(found) if found.id == pane));
    if let Some(index) = direct {
        let Node::Pane(taken) = split.children.remove(index) else {
            return None;
        };
        release(&mut split.ratios, index);
        return Some(taken);
    }
    split
        .children
        .iter_mut()
        .find_map(|child| take_from_split(child, pane))
}

/// Takes a pane out of wherever in a window it is. A tab that held nothing else goes
/// with it.
fn take_pane(workspace: &mut Workspace, pane: PaneId) -> Result<(usize, Pane), IntentError> {
    let gone = IntentError::NoSuchPane(pane);
    match shown(workspace, pane)? {
        Location::Floating { window, index } => Ok((
            window,
            workspace.windows[window].floating.remove(index).pane,
        )),
        Location::Tab { window, tab } => {
            let alone = matches!(
                &workspace.windows[window].centre.tabs[tab].root,
                Node::Pane(only) if only.id == pane
            );
            let taken = if alone {
                match remove_tab(&mut workspace.windows[window], tab).root {
                    Node::Pane(only) => Some(only),
                    Node::Split(_) => None,
                }
            } else {
                take_from_split(&mut workspace.windows[window].centre.tabs[tab].root, pane)
            };
            Ok((window, taken.ok_or(gone)?))
        }
        Location::Parked => Err(gone),
    }
}

/// Puts a pane on one side of the pane `target` under `node`, dividing the target's
/// space so that the new pane has `ratio` of it. The pane is taken from `pane` when
/// the target is found; whether it was is returned.
fn put_beside(
    node: &mut Node,
    target: PaneId,
    edge: Dir,
    ratio: u32,
    id: SplitId,
    pane: &mut Option<Pane>,
) -> bool {
    match node {
        Node::Pane(found) if found.id == target => {
            let Some(new) = pane.take() else {
                return false;
            };
            let axis = match edge {
                Dir::Left | Dir::Right => Axis::Row,
                Dir::Up | Dir::Down => Axis::Column,
            };
            let empty = Split {
                id,
                axis,
                ratios: Vec::new(),
                children: Vec::new(),
            };
            let existing = std::mem::replace(node, Node::Split(empty));
            if let Node::Split(split) = node {
                let new = Node::Pane(new);
                match edge {
                    Dir::Left | Dir::Up => {
                        split.children = vec![new, existing];
                        split.ratios = vec![ratio, UNITS - ratio];
                    }
                    Dir::Right | Dir::Down => {
                        split.children = vec![existing, new];
                        split.ratios = vec![UNITS - ratio, ratio];
                    }
                }
            }
            true
        }
        Node::Pane(_) => false,
        Node::Split(split) => split
            .children
            .iter_mut()
            .any(|child| put_beside(child, target, edge, ratio, id, pane)),
    }
}

/// Puts a pane beside a pane of a tab in the same window.
fn place_beside(
    workspace: &mut Workspace,
    window: usize,
    target: PaneId,
    edge: Dir,
    ratio: u32,
    pane: Pane,
) -> Result<(), IntentError> {
    let (target_window, tab) = tiled(workspace, target)?;
    if target_window != window {
        return Err(IntentError::CrossWindow);
    }
    let id = SplitId::new(workspace.fresh());
    let root = &mut workspace.windows[window].centre.tabs[tab].root;
    if put_beside(root, target, edge, ratio, id, &mut Some(pane)) {
        Ok(())
    } else {
        Err(IntentError::NoSuchPane(target))
    }
}

/// Opens a tab holding one pane. Without an index it goes after the last tab.
fn open_tab(
    workspace: &mut Workspace,
    window: usize,
    pane: Pane,
    index: Option<u32>,
    activate: bool,
) -> Result<TabId, IntentError> {
    let len = workspace.windows[window].centre.tabs.len();
    let at = match index {
        Some(index) if index as usize > len => return Err(IntentError::BadIndex { index, len }),
        Some(index) => index as usize,
        None => len,
    };
    let id = TabId::new(workspace.fresh());
    let pane_id = pane.id;
    let window = &mut workspace.windows[window];
    window.centre.tabs.insert(
        at,
        Tab {
            id,
            title: None,
            root: Node::Pane(pane),
            zoomed: None,
            last_focus: Some(pane_id),
            color: None,
            locked: false,
            muted: false,
            group: None,
        },
    );
    if activate || window.centre.active.is_none() {
        focus(window, pane_id);
    }
    Ok(id)
}

/// Where a pane floats when nothing says where: in the middle, each further one a
/// little down and to the right of the one before.
fn default_float(window: &Window) -> UnitRect {
    let origin = (UNITS - FLOAT_DEFAULT) / 2;
    let step = FLOAT_CASCADE * (window.floating.len() as u32 % 8);
    contain(UnitRect {
        x: origin + step,
        y: origin + step,
        w: FLOAT_DEFAULT,
        h: FLOAT_DEFAULT,
    })
}

fn rect_or_refuse(rect: UnitRect) -> Result<UnitRect, IntentError> {
    if rect.w == 0 || rect.h == 0 {
        return Err(IntentError::BadRect);
    }
    Ok(contain(rect))
}

/// Puts a pane that is in no tab into one: where it is told, or beside the pane of
/// the tab on show that last had the keyboard, or in a tab of its own when the window
/// has none.
fn dock(
    workspace: &mut Workspace,
    window: usize,
    pane: Pane,
    to: Option<Place>,
) -> Result<Created, IntentError> {
    let pane_id = pane.id;
    let mut created = Created::default();
    match to {
        Some(Place::Beside { pane: target, edge }) => {
            if target == pane_id {
                return Err(IntentError::SamePane(pane_id));
            }
            place_beside(workspace, window, target, edge, UNITS / 2, pane)?;
        }
        Some(Place::NewTab {
            window: asked,
            index,
        }) => {
            if window_index(workspace, asked)? != window {
                return Err(IntentError::CrossWindow);
            }
            created.tab = Some(open_tab(workspace, window, pane, index, true)?);
        }
        None => {
            let here = &workspace.windows[window];
            let beside = here.centre.active_tab().map(|tab| {
                here.history
                    .iter()
                    .copied()
                    .find(|seen| tab.contains(*seen))
                    .unwrap_or_else(|| tab.first_pane())
            });
            match beside {
                Some(target) => {
                    place_beside(workspace, window, target, Dir::Right, UNITS / 2, pane)?;
                }
                None => created.tab = Some(open_tab(workspace, window, pane, None, true)?),
            }
        }
    }
    focus(&mut workspace.windows[window], pane_id);
    Ok(created)
}

/// Gives a floating pane of a window the keyboard, bringing it back first if it is
/// put away: to float where it floated, or among the tabs again if that is where it
/// was taken from.
fn bring_back(
    workspace: &mut Workspace,
    window: usize,
    pane: PaneId,
) -> Result<Created, IntentError> {
    let floating = &mut workspace.windows[window].floating;
    let index = floating
        .iter()
        .position(|floating| floating.pane.id == pane)
        .ok_or(IntentError::NoSuchPane(pane))?;
    if floating[index].minimised == Some(Minimised::Tab) {
        let taken = floating.remove(index).pane;
        return dock(workspace, window, taken, None);
    }
    focus(&mut workspace.windows[window], pane);
    Ok(Created::default())
}

/// Carries out an intent, leaving the tidying to the caller.
fn change(
    workspace: &mut Workspace,
    intent: Intent,
    started: &mut Option<Started>,
) -> Result<Created, IntentError> {
    let mut created = Created::default();
    match intent {
        Intent::Split {
            pane,
            dir,
            ratio,
            content,
        } => {
            let (window, _) = tiled(workspace, pane)?;
            if !(SHARE_MIN..=UNITS - SHARE_MIN).contains(&ratio) {
                return Err(IntentError::BadRatios(
                    "the new pane's share is outside what a pane may have",
                ));
            }
            let new = new_pane(workspace, content, started);
            let id = new.id;
            place_beside(workspace, window, pane, dir, ratio, new)?;
            focus(&mut workspace.windows[window], id);
            created.pane = Some(id);
        }
        Intent::Close { pane } => {
            if workspace.locate(pane) == Some(Location::Parked) {
                workspace.scratch = None;
            } else {
                take_pane(workspace, pane)?;
            }
        }
        Intent::Focus { target } => match target {
            FocusTarget::Pane { pane } => match shown(workspace, pane)? {
                Location::Tab { window, .. } => focus(&mut workspace.windows[window], pane),
                // Asking for a pane that is put away brings it back.
                Location::Floating { window, .. } => {
                    created = bring_back(workspace, window, pane)?;
                }
                Location::Parked => return Err(IntentError::NoSuchPane(pane)),
            },
            FocusTarget::Next { window } | FocusTarget::Previous { window } => {
                let forward = matches!(target, FocusTarget::Next { .. });
                let window = window_index(workspace, window)?;
                let window = &mut workspace.windows[window];
                let order = window.focusable();
                if !order.is_empty() {
                    let at = window
                        .focus
                        .and_then(|pane| order.iter().position(|other| *other == pane));
                    let next = match (at, forward) {
                        (Some(at), true) => (at + 1) % order.len(),
                        (Some(at), false) => (at + order.len() - 1) % order.len(),
                        (None, true) => 0,
                        (None, false) => order.len() - 1,
                    };
                    focus(window, order[next]);
                }
            }
            FocusTarget::Back { window } => {
                let window = window_index(workspace, window)?;
                let window = &mut workspace.windows[window];
                // A pane that is put away is brought back by asking for it, not by
                // going back to it.
                let candidates = window.focusable();
                let before = window
                    .history
                    .iter()
                    .copied()
                    .find(|pane| Some(*pane) != window.focus && candidates.contains(pane));
                if let Some(pane) = before {
                    focus(window, pane);
                }
            }
        },
        Intent::FocusDir { window, dir } => {
            let window = window_index(workspace, window)?;
            let window = &mut workspace.windows[window];
            if let Some(from) = window.focus {
                // A floating pane's neighbours are the other floating panes; a pane in
                // a tab has those of its tab.
                let rects = if window.floating_index(from).is_some() {
                    window
                        .shown_floating()
                        .map(|floating| (floating.pane.id, floating.rect))
                        .collect()
                } else {
                    window
                        .centre
                        .active_tab()
                        .map(|tab| tab_rects(&tab.root))
                        .unwrap_or_default()
                };
                if let Some(to) = neighbour(&rects, from, dir, &window.history) {
                    focus(window, to);
                }
            }
        }
        Intent::Move { pane, to } => {
            let from = match shown(workspace, pane)? {
                Location::Tab { window, .. } | Location::Floating { window, .. } => window,
                Location::Parked => return Err(IntentError::NoSuchPane(pane)),
            };
            // Checked before anything is taken out, so that the pane is not taken out
            // of the very tab it is to be put beside itself in.
            match to {
                Place::Beside { pane: target, .. } => {
                    if target == pane {
                        return Err(IntentError::SamePane(pane));
                    }
                    if tiled(workspace, target)?.0 != from {
                        return Err(IntentError::CrossWindow);
                    }
                }
                Place::NewTab { window, .. } => {
                    if window_index(workspace, window)? != from {
                        return Err(IntentError::CrossWindow);
                    }
                }
            }
            let (window, taken) = take_pane(workspace, pane)?;
            created = dock(workspace, window, taken, Some(to))?;
        }
        Intent::Swap { a, b } => {
            if a == b {
                return Err(IntentError::SamePane(a));
            }
            let (window_a, tab_a) = tiled(workspace, a)?;
            let (window_b, tab_b) = tiled(workspace, b)?;
            if window_a != window_b {
                return Err(IntentError::CrossWindow);
            }
            let tabs = &mut workspace.windows[window_a].centre.tabs;
            let first = tabs[tab_a]
                .root
                .pane(a)
                .cloned()
                .ok_or(IntentError::NoSuchPane(a))?;
            let second = tabs[tab_b]
                .root
                .pane(b)
                .cloned()
                .ok_or(IntentError::NoSuchPane(b))?;
            exchange(&mut tabs[tab_a].root, &first, &second);
            if tab_b != tab_a {
                exchange(&mut tabs[tab_b].root, &first, &second);
            }
        }
        Intent::Resize { split, ratios } => {
            let (window, tab) = workspace
                .locate_split(split)
                .ok_or(IntentError::NoSuchSplit(split))?;
            let found = workspace.windows[window].centre.tabs[tab]
                .root
                .split_mut(split)
                .ok_or(IntentError::NoSuchSplit(split))?;
            if ratios.len() != found.children.len() {
                return Err(IntentError::BadRatios(
                    "there is not one ratio for each child",
                ));
            }
            let sum: u64 = ratios.iter().map(|ratio| u64::from(*ratio)).sum();
            if sum.abs_diff(u64::from(UNITS)) > u64::from(RATIO_TOLERANCE) {
                return Err(IntentError::BadRatios("they do not add up to a whole"));
            }
            found.ratios = fit(&ratios);
        }
        Intent::Zoom { pane, on } => {
            let (window, tab) = tiled(workspace, pane)?;
            let window = &mut workspace.windows[window];
            if on {
                window.centre.tabs[tab].zoomed = Some(pane);
                focus(window, pane);
            } else if window.centre.tabs[tab].zoomed == Some(pane) {
                window.centre.tabs[tab].zoomed = None;
            }
        }
        Intent::Float { pane, rect } => {
            tiled(workspace, pane)?;
            let rect = rect.map(rect_or_refuse).transpose()?;
            let (window, taken) = take_pane(workspace, pane)?;
            let window = &mut workspace.windows[window];
            let rect = rect.unwrap_or_else(|| default_float(window));
            window.floating.push(Floating {
                pane: taken,
                rect,
                minimised: None,
            });
            focus(window, pane);
        }
        Intent::Minimise { pane, on } => match (shown(workspace, pane)?, on) {
            (Location::Floating { window, index }, true) => {
                // One that is put away already keeps where it came from.
                workspace.windows[window].floating[index]
                    .minimised
                    .get_or_insert(Minimised::Floating);
            }
            (Location::Floating { window, index }, false) => {
                if workspace.windows[window].floating[index]
                    .minimised
                    .is_some()
                {
                    created = bring_back(workspace, window, pane)?;
                }
            }
            (Location::Tab { .. }, true) => {
                let (window, taken) = take_pane(workspace, pane)?;
                let window = &mut workspace.windows[window];
                let rect = default_float(window);
                window.floating.push(Floating {
                    pane: taken,
                    rect,
                    minimised: Some(Minimised::Tab),
                });
            }
            // A pane in a tab is on show already.
            (Location::Tab { .. }, false) => {}
            (Location::Parked, _) => return Err(IntentError::NoSuchPane(pane)),
        },
        Intent::FloatMove { pane, rect } => {
            let Location::Floating { window, index } = shown(workspace, pane)? else {
                return Err(IntentError::NotFloating(pane));
            };
            workspace.windows[window].floating[index].rect = rect_or_refuse(rect)?;
        }
        Intent::Dock { pane, to } => {
            let Location::Floating { window, index } = shown(workspace, pane)? else {
                return Err(IntentError::NotFloating(pane));
            };
            let taken = workspace.windows[window].floating.remove(index).pane;
            created = dock(workspace, window, taken, to)?;
        }
        Intent::PaneRename { pane, title } => {
            workspace
                .pane_mut(pane)
                .ok_or(IntentError::NoSuchPane(pane))?
                .title = name(title);
        }
        Intent::PaneIcon { pane, icon } => {
            workspace
                .pane_mut(pane)
                .ok_or(IntentError::NoSuchPane(pane))?
                .icon = picture(icon).map(String::into_boxed_str);
        }
        Intent::TabNew {
            window,
            content,
            index,
            activate,
            group,
        } => {
            let window = window_index(workspace, window)?;
            let index = match group {
                Some(group) => {
                    let centre = &workspace.windows[window].centre;
                    if centre.group(group).is_none() {
                        return Err(IntentError::NoSuchGroup(group));
                    }
                    centre.members(group).last().map(|last| *last as u32 + 1)
                }
                None => index,
            };
            let pane = new_pane(workspace, content, started);
            created.pane = Some(pane.id);
            let tab = open_tab(workspace, window, pane, index, activate)?;
            created.tab = Some(tab);
            if let Some(found) = workspace.windows[window].centre.tab_mut(tab) {
                found.group = group;
            }
        }
        Intent::TabClose { tab } => {
            let (window, index) = tab_index(workspace, tab)?;
            if workspace.windows[window].centre.tabs[index].locked {
                return Err(IntentError::Locked(tab));
            }
            remove_tab(&mut workspace.windows[window], index);
        }
        Intent::TabsClose { tabs } => {
            let mut by_window: Vec<(usize, Vec<usize>)> = Vec::new();
            for tab in tabs {
                let (window, index) = tab_index(workspace, tab)?;
                if workspace.windows[window].centre.tabs[index].locked {
                    continue;
                }
                match by_window.iter_mut().find(|(found, _)| *found == window) {
                    Some((_, indices)) if !indices.contains(&index) => indices.push(index),
                    Some(_) => {}
                    None => by_window.push((window, vec![index])),
                }
            }
            for (window, indices) in by_window {
                remove_tabs(&mut workspace.windows[window], indices);
            }
        }
        Intent::TabMove { tab, index } => {
            let (window, from) = tab_index(workspace, tab)?;
            let tabs = &mut workspace.windows[window].centre.tabs;
            if index as usize >= tabs.len() {
                return Err(IntentError::BadIndex {
                    index,
                    len: tabs.len(),
                });
            }
            let mut moved = tabs.remove(from);
            moved.group = group_at(tabs, index as usize, moved.group);
            tabs.insert(index as usize, moved);
        }
        Intent::TabPlace { tab, index, group } => {
            let (window, from) = tab_index(workspace, tab)?;
            let centre = &mut workspace.windows[window].centre;
            if let Some(group) = group
                && centre.group(group).is_none()
            {
                return Err(IntentError::NoSuchGroup(group));
            }
            if index as usize >= centre.tabs.len() {
                return Err(IntentError::BadIndex {
                    index,
                    len: centre.tabs.len(),
                });
            }
            let mut moved = centre.tabs.remove(from);
            moved.group = group;
            centre.tabs.insert(index as usize, moved);
        }
        Intent::TabColor { tab, color } => {
            let (window, index) = tab_index(workspace, tab)?;
            workspace.windows[window].centre.tabs[index].color = color;
        }
        Intent::TabLock { tab, on } => {
            let (window, index) = tab_index(workspace, tab)?;
            workspace.windows[window].centre.tabs[index].locked = on;
        }
        Intent::TabMute { tab, on } => {
            let (window, index) = tab_index(workspace, tab)?;
            workspace.windows[window].centre.tabs[index].muted = on;
        }
        Intent::TabMerge { tab, into } => {
            if tab == into {
                return Err(IntentError::SameTab(tab));
            }
            let (window, from) = tab_index(workspace, tab)?;
            if tab_index(workspace, into)?.0 != window {
                return Err(IntentError::CrossWindow);
            }
            let id = SplitId::new(workspace.fresh());
            let window = &mut workspace.windows[window];
            let moved = remove_tab(window, from);
            let pane = moved
                .last_focus
                .filter(|pane| moved.contains(*pane))
                .unwrap_or_else(|| moved.first_pane());
            let target = window
                .centre
                .tab_mut(into)
                .ok_or(IntentError::NoSuchTab(into))?;
            let existing = std::mem::replace(&mut target.root, Node::Split(placeholder()));
            target.root = Node::Split(Split {
                id,
                axis: Axis::Row,
                ratios: vec![UNITS / 2, UNITS - UNITS / 2],
                children: vec![existing, moved.root],
            });
            target.zoomed = None;
            focus(window, pane);
        }
        Intent::TabRename { tab, title } => {
            let (window, index) = workspace
                .locate_tab(tab)
                .ok_or(IntentError::NoSuchTab(tab))?;
            workspace.windows[window].centre.tabs[index].title = name(title);
        }
        Intent::TabActivate { target } => {
            let (window, index) = match target {
                TabTarget::Id { tab } => workspace
                    .locate_tab(tab)
                    .ok_or(IntentError::NoSuchTab(tab))?,
                TabTarget::Index { window, index } => {
                    let window = window_index(workspace, window)?;
                    let len = workspace.windows[window].centre.tabs.len();
                    if index as usize >= len {
                        return Err(IntentError::BadIndex { index, len });
                    }
                    (window, index as usize)
                }
                TabTarget::Next { window } | TabTarget::Previous { window } => {
                    let forward = matches!(target, TabTarget::Next { .. });
                    let window = window_index(workspace, window)?;
                    let centre = &workspace.windows[window].centre;
                    let len = centre.tabs.len();
                    let Some(at) = centre.active.and_then(|tab| centre.index_of(tab)) else {
                        // A window with no tabs has none to go to.
                        return Ok(created);
                    };
                    let next = if forward {
                        (at + 1) % len
                    } else {
                        (at + len - 1) % len
                    };
                    (window, next)
                }
            };
            show_tab(&mut workspace.windows[window], index);
        }
        Intent::GroupNew { tab } => {
            let (window, index) = tab_index(workspace, tab)?;
            let id = GroupId::new(workspace.fresh());
            let centre = &mut workspace.windows[window].centre;
            let color = TabColor::FOR_GROUPS
                .into_iter()
                .find(|color| centre.groups.iter().all(|group| group.color != *color))
                .unwrap_or(TabColor::FOR_GROUPS[0]);
            centre.groups.push(Group {
                id,
                name: None,
                color,
                icon: None,
                collapsed: false,
                kept: false,
                muted: false,
            });
            centre.tabs[index].group = Some(id);
            created.group = Some(id);
        }
        Intent::GroupSet {
            group,
            name: new_name,
            color,
            icon,
            collapsed,
            kept,
            muted,
        } => {
            let window = group_window(workspace, group)?;
            let window = &mut workspace.windows[window];
            if collapsed == Some(true) {
                let centre = &window.centre;
                let members = centre.members(group);
                let active_inside = centre
                    .active_tab()
                    .is_some_and(|tab| tab.group == Some(group));
                if active_inside
                    && let (Some(first), Some(last)) = (members.first(), members.last())
                {
                    let open = |index: &usize| {
                        let tab = &centre.tabs[*index];
                        tab.group != Some(group) && !centre.folded(tab)
                    };
                    let nearest = (last + 1..centre.tabs.len())
                        .find(open)
                        .or_else(|| (0..*first).rev().find(open))
                        .ok_or(IntentError::NothingElseToShow)?;
                    show_tab(window, nearest);
                }
            }
            let found = window
                .centre
                .group_mut(group)
                .ok_or(IntentError::NoSuchGroup(group))?;
            if let Some(new_name) = new_name {
                found.name = name(new_name);
            }
            if let Some(color) = color {
                found.color = color;
            }
            if let Some(icon) = icon {
                found.icon = icon;
            }
            if let Some(collapsed) = collapsed {
                found.collapsed = collapsed;
            }
            if let Some(kept) = kept {
                found.kept = kept;
            }
            if let Some(muted) = muted {
                found.muted = muted;
            }
        }
        Intent::GroupMove { group, index } => {
            let window = group_window(workspace, group)?;
            let centre = &mut workspace.windows[window].centre;
            let (block, mut rest): (Vec<Tab>, Vec<Tab>) = std::mem::take(&mut centre.tabs)
                .into_iter()
                .partition(|tab| tab.group == Some(group));
            let at = index as usize;
            let inside = at > 0
                && rest
                    .get(at - 1)
                    .and_then(|tab| tab.group)
                    .is_some_and(|before| rest.get(at).and_then(|tab| tab.group) == Some(before));
            if at > rest.len() || inside {
                let len = rest.len();
                // Put back as it was: the copy is dropped on refusal, but say why.
                return Err(if inside {
                    IntentError::InsideGroup
                } else {
                    IntentError::BadIndex { index, len }
                });
            }
            rest.splice(at..at, block);
            centre.tabs = rest;
        }
        Intent::GroupUngroup { group } => {
            let window = group_window(workspace, group)?;
            let centre = &mut workspace.windows[window].centre;
            centre.groups.retain(|found| found.id != group);
            for tab in &mut centre.tabs {
                if tab.group == Some(group) {
                    tab.group = None;
                }
            }
        }
        Intent::GroupClose { group } => {
            let window = group_window(workspace, group)?;
            let here = &mut workspace.windows[window];
            let mut found = here
                .centre
                .group(group)
                .cloned()
                .ok_or(IntentError::NoSuchGroup(group))?;
            here.centre.groups.retain(|other| other.id != group);
            let mut closing = Vec::new();
            for (index, tab) in here.centre.tabs.iter_mut().enumerate() {
                if tab.group != Some(group) {
                    continue;
                }
                if tab.locked {
                    tab.group = None;
                } else {
                    closing.push(index);
                }
            }
            let mut tabs = remove_tabs(here, closing);
            if found.kept && !tabs.is_empty() {
                for tab in &mut tabs {
                    forget_sessions(&mut tab.root);
                    tab.group = Some(group);
                }
                found.collapsed = false;
                workspace.shelf.push(Shelved { group: found, tabs });
                // The oldest is forgotten once more are kept than a workspace keeps.
                let over = workspace.shelf.len().saturating_sub(SHELF_MAX);
                workspace.shelf.drain(..over);
            }
        }
        Intent::GroupReopen { group, window } => {
            let window = window_index(workspace, window)?;
            let at = workspace
                .shelf
                .iter()
                .position(|shelved| shelved.group.id == group)
                .ok_or(IntentError::NoSuchGroup(group))?;
            let Shelved { group, tabs } = workspace.shelf.remove(at);
            put_group(&mut workspace.windows[window], group, tabs);
        }
        Intent::GroupForget { group } => {
            let at = workspace
                .shelf
                .iter()
                .position(|shelved| shelved.group.id == group)
                .ok_or(IntentError::NoSuchGroup(group))?;
            workspace.shelf.remove(at);
        }
        Intent::PanelMove {
            window,
            panel,
            area,
            index,
        } => {
            let id = window;
            let window = window_index(workspace, window)?;
            let dock = &mut workspace.windows[window].dock;
            let from = Area::ALL
                .into_iter()
                .find_map(|area| {
                    let at = dock
                        .area(area)
                        .panels
                        .iter()
                        .position(|found| found.id == panel)?;
                    Some((area, at))
                })
                .ok_or(IntentError::NoSuchPanel { window: id, panel })?;
            // The top bar and its slots stay as they are.
            if from.0 == Area::Top || area == Area::Top {
                return Err(IntentError::Fixed);
            }
            let moved = dock.area_mut(from.0).panels.remove(from.1);
            let panels = &mut dock.area_mut(area).panels;
            if index as usize > panels.len() {
                return Err(IntentError::BadIndex {
                    index,
                    len: panels.len(),
                });
            }
            panels.insert(index as usize, moved);
        }
        Intent::PanelToggle { window, area, on } => {
            let window = window_index(workspace, window)?;
            if area == Area::Top {
                return Err(IntentError::Fixed);
            }
            let dock = workspace.windows[window].dock.area_mut(area);
            dock.collapsed = !on.unwrap_or(dock.collapsed);
        }
        Intent::PanelResize { window, area, size } => {
            let window = window_index(workspace, window)?;
            if area == Area::Top {
                return Err(IntentError::Fixed);
            }
            workspace.windows[window].dock.area_mut(area).size =
                size.clamp(DOCK_SIZE_MIN, DOCK_SIZE_MAX);
        }
        Intent::ScratchShow { window, content } => {
            let window = window_index(workspace, window)?;
            let (floating, pane) = match workspace.scratch.take() {
                Some(scratch) => {
                    let pane = scratch.pane;
                    let floating = match scratch.parked {
                        Some(parked) => parked,
                        None => {
                            let Some(Location::Floating { window, index }) = workspace.locate(pane)
                            else {
                                return Err(IntentError::NoSuchPane(pane));
                            };
                            workspace.windows[window].floating.remove(index)
                        }
                    };
                    (floating, pane)
                }
                None => {
                    let content = content.ok_or(IntentError::ContentRequired)?;
                    let pane = new_pane(workspace, content, started);
                    let id = pane.id;
                    created.pane = Some(id);
                    let rect = default_float(&workspace.windows[window]);
                    (
                        Floating {
                            pane,
                            rect,
                            minimised: None,
                        },
                        id,
                    )
                }
            };
            workspace.scratch = Some(Scratch { pane, parked: None });
            let window = &mut workspace.windows[window];
            window.floating.push(floating);
            focus(window, pane);
        }
        Intent::ScratchHide {} => {
            if let Some(scratch) = &workspace.scratch
                && let Some(Location::Floating { window, index }) = workspace.locate(scratch.pane)
            {
                let parked = workspace.windows[window].floating.remove(index);
                if let Some(scratch) = &mut workspace.scratch {
                    scratch.parked = Some(parked);
                }
            }
        }
        Intent::WindowNew { content, bounds } => {
            let id = WindowId::new(workspace.fresh());
            let mut window = Window::new(id);
            window.bounds = bounds;
            workspace.windows.push(window);
            let index = workspace.windows.len() - 1;
            let pane = new_pane(workspace, content, started);
            created.pane = Some(pane.id);
            created.tab = Some(open_tab(workspace, index, pane, None, true)?);
            created.window = Some(id);
            workspace.focused_window = Some(id);
        }
        Intent::WindowClose { window } => {
            let index = window_index(workspace, window)?;
            workspace.windows.remove(index);
        }
        Intent::SessionSet {
            pane,
            session,
            backend,
            cwd,
        } => {
            let found = workspace
                .pane_mut(pane)
                .ok_or(IntentError::NoSuchPane(pane))?;
            let PaneContent::Terminal(terminal) = &mut found.content else {
                return Err(IntentError::NotTerminal(pane));
            };
            terminal.session = session;
            if backend.is_some() {
                terminal.backend = backend;
            }
            if cwd.is_some() {
                terminal.cwd = cwd;
            }
        }
        Intent::PanelSet {
            window,
            panel,
            content,
            area,
            index,
        } => {
            let window = window_index(workspace, window)?;
            // A slot belongs in the top bar, and nothing else does.
            let slot = matches!(content, PanelContent::Slot { .. });
            if area.is_some_and(|area| slot != (area == Area::Top)) {
                return Err(IntentError::Fixed);
            }
            let dock = &mut workspace.windows[window].dock;
            for area in Area::ALL {
                dock.area_mut(area).panels.retain(|found| found.id != panel);
            }
            if let Some(area) = area {
                let panels = &mut dock.area_mut(area).panels;
                let at = (index as usize).min(panels.len());
                panels.insert(at, Panel { id: panel, content });
            }
        }
        Intent::ViewState { pane, state } => {
            let found = workspace
                .pane_mut(pane)
                .ok_or(IntentError::NoSuchPane(pane))?;
            // A pane that shows a terminal keeps nothing for a view.
            if let PaneContent::PluginView { state: kept, .. } = &mut found.content {
                *kept = state;
            }
        }
        Intent::PanelState {
            window,
            panel,
            state,
        } => {
            let window = window_index(workspace, window)?;
            let dock = &mut workspace.windows[window].dock;
            for area in Area::ALL {
                for found in &mut dock.area_mut(area).panels {
                    if found.id != panel {
                        continue;
                    }
                    if let PanelContent::PluginView { state: kept, .. } = &mut found.content {
                        *kept = state.clone();
                    }
                }
            }
        }
        Intent::WindowBounds { window, bounds } => {
            let window = window_index(workspace, window)?;
            workspace.windows[window].bounds = Some(bounds);
        }
        Intent::WindowFocus { window } => {
            window_index(workspace, window)?;
            workspace.focused_window = Some(window);
        }
        Intent::WorkspaceRename { name } => {
            if !valid_name(&name) {
                return Err(IntentError::BadName(name));
            }
            workspace.name = name;
        }
        Intent::GroupTake { group } => {
            let window = group_window(workspace, group)?;
            let here = &mut workspace.windows[window];
            here.centre.groups.retain(|other| other.id != group);
            let members = here.centre.members(group);
            remove_tabs(here, members);
        }
        Intent::GroupGraft { window, group } => {
            let window = window_index(workspace, window)?;
            let Shelved { mut group, tabs } = *group;
            group.id = GroupId::new(workspace.fresh());
            group.collapsed = false;
            let mut grafted = Vec::with_capacity(tabs.len());
            for mut tab in tabs {
                tab.id = TabId::new(workspace.fresh());
                let mut panes = HashMap::new();
                renumber(&mut tab.root, workspace, &mut panes);
                forget_sessions(&mut tab.root);
                tab.zoomed = tab.zoomed.and_then(|pane| panes.get(&pane).copied());
                tab.last_focus = tab.last_focus.and_then(|pane| panes.get(&pane).copied());
                grafted.push(tab);
            }
            created.group = Some(group.id);
            put_group(&mut workspace.windows[window], group, grafted);
        }
    }
    Ok(created)
}

fn placeholder() -> Split {
    Split {
        id: SplitId::new(0),
        axis: Axis::Row,
        ratios: Vec::new(),
        children: Vec::new(),
    }
}

/// Puts each of two panes where the other is, wherever under `node` either is. Each
/// place is visited once, so a pane put in a place is not taken for the one that was
/// there.
fn exchange(node: &mut Node, a: &Pane, b: &Pane) {
    match node {
        Node::Pane(pane) if pane.id == a.id => *pane = b.clone(),
        Node::Pane(pane) if pane.id == b.id => *pane = a.clone(),
        Node::Pane(_) => {}
        Node::Split(split) => {
            for child in &mut split.children {
                exchange(child, a, b);
            }
        }
    }
}
