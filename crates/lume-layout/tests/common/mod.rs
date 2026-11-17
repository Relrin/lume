//! What the layout's tests share: small workspaces, and applying an intent with every
//! check that must hold after any intent.

#![allow(dead_code)]

use std::collections::HashSet;

use lume_layout::{
    Created, Dir, Intent, IntentError, LayoutChange, NewContent, PaneId, Projection, SpawnRequest,
    Started, TabId, UNITS, WindowId, Workspace, WorkspaceId, apply, check, is_normalised, validate,
};
use lume_proto::{BackendId, SessionId};

/// A workspace with nothing in it.
pub fn empty() -> Workspace {
    Workspace::new(WorkspaceId::new(0xa1).unwrap(), "test")
}

/// A terminal started however the application starts terminals.
pub fn terminal() -> NewContent {
    NewContent::Terminal(SpawnRequest::default())
}

/// A view of some plugin.
pub fn plugin_view() -> NewContent {
    NewContent::PluginView {
        plugin: "lume.notes".to_owned(),
        view: "page".to_owned(),
        state: serde_json::json!({ "page": 3 }),
    }
}

/// A session with this number, started on the local backend.
pub fn started(number: u64) -> Started {
    Started {
        backend: BackendId("local".to_owned()),
        session: SessionId::new(number).unwrap(),
        cwd: Some("/work".to_owned()),
    }
}

/// The ids of everything in a workspace that has a number.
fn numbers(workspace: &Workspace) -> HashSet<u32> {
    let mut numbers = HashSet::new();
    for window in &workspace.windows {
        numbers.insert(window.id.get());
        for tab in &window.centre.tabs {
            numbers.insert(tab.id.get());
            numbers.extend(tab.root.splits().into_iter().map(|split| split.id.get()));
        }
    }
    numbers.extend(workspace.panes().into_iter().map(|pane| pane.id.get()));
    // What is put away keeps its numbers.
    for shelved in &workspace.shelf {
        for tab in &shelved.tabs {
            numbers.insert(tab.id.get());
            numbers.extend(tab.root.splits().into_iter().map(|split| split.id.get()));
            numbers.extend(tab.root.pane_ids().into_iter().map(|pane| pane.get()));
        }
    }
    numbers
}

/// Checks everything that must hold of a workspace after an intent was applied to
/// `before` and reported `changes`.
pub fn check_applied(before: &Workspace, after: &Workspace, changes: &[LayoutChange]) {
    if let Err(violations) = validate(after) {
        panic!("the workspace is not sound: {violations:?}\n{after:#?}");
    }
    assert!(is_normalised(after), "a tree is not in its proper form");

    // A follower that is given the changes has what the authority has.
    let mut follower = Projection::of(before);
    for change in changes {
        follower.apply(change);
    }
    assert_eq!(
        follower,
        Projection::of(after),
        "a follower of these changes does not end up with the workspace: {changes:#?}"
    );

    // Each pane that is gone is reported gone, once, and no other is.
    let left: HashSet<PaneId> = after.panes().into_iter().map(|pane| pane.id).collect();
    let mut gone: Vec<PaneId> = before
        .panes()
        .into_iter()
        .map(|pane| pane.id)
        .filter(|pane| !left.contains(pane))
        .collect();
    let mut reported: Vec<PaneId> = changes
        .iter()
        .filter_map(|change| match change {
            LayoutChange::PaneClosed { pane, .. } => Some(*pane),
            _ => None,
        })
        .collect();
    gone.sort();
    reported.sort();
    assert_eq!(reported, gone, "the panes reported closed");

    // Numbers only go up: what is new was numbered from the counter.
    assert!(after.next_id >= before.next_id);
    let known = numbers(before);
    for number in numbers(after) {
        assert!(
            known.contains(&number) || (before.next_id..after.next_id).contains(&number),
            "number {number} was not taken from the counter"
        );
    }

    // What changed nothing reports nothing, and the other way round.
    assert_eq!(
        changes.is_empty(),
        Projection::of(before) == Projection::of(after)
    );
}

/// Applies an intent that must be accepted, with every check, and returns what it
/// reported.
pub fn must(workspace: &mut Workspace, intent: Intent) -> (Vec<LayoutChange>, Created) {
    must_with(workspace, intent, None)
}

/// As [`must`], for an intent whose new pane has a session.
pub fn must_with(
    workspace: &mut Workspace,
    intent: Intent,
    started: Option<Started>,
) -> (Vec<LayoutChange>, Created) {
    let before = workspace.clone();
    assert_eq!(check(&before, &intent), Ok(()), "{intent:?}");
    let (changes, created) = match apply(workspace, intent.clone(), started) {
        Ok(applied) => applied,
        Err(error) => panic!("{intent:?} was refused: {error}"),
    };
    check_applied(&before, workspace, &changes);
    (changes, created)
}

/// Applies an intent that must be refused, checks that nothing changed, and returns
/// why it was refused.
pub fn refused(workspace: &mut Workspace, intent: Intent) -> IntentError {
    let before = workspace.clone();
    let checked = check(&before, &intent);
    let error = match apply(workspace, intent.clone(), None) {
        Ok(_) => panic!("{intent:?} was accepted"),
        Err(error) => error,
    };
    assert_eq!(*workspace, before, "a refused intent changed the workspace");
    assert_eq!(
        checked,
        Err(error.clone()),
        "the check and the apply disagree"
    );
    error
}

/// Opens a window with one terminal pane and returns the ids of the window, its tab
/// and the pane.
pub fn open_window(workspace: &mut Workspace) -> (WindowId, TabId, PaneId) {
    let (_, created) = must(
        workspace,
        Intent::WindowNew {
            content: terminal(),
            bounds: None,
        },
    );
    (
        created.window.unwrap(),
        created.tab.unwrap(),
        created.pane.unwrap(),
    )
}

/// A workspace with one window holding one pane.
pub fn one_pane() -> (Workspace, WindowId, TabId, PaneId) {
    let mut workspace = empty();
    let (window, tab, pane) = open_window(&mut workspace);
    (workspace, window, tab, pane)
}

/// Splits a pane in half and returns the new pane.
pub fn split(workspace: &mut Workspace, pane: PaneId, dir: Dir) -> PaneId {
    let intent = Intent::Split {
        pane,
        dir,
        ratio: UNITS / 2,
        content: terminal(),
    };
    must(workspace, intent).1.pane.unwrap()
}

/// Opens another tab in a window and returns the tab and its pane.
pub fn open_tab(workspace: &mut Workspace, window: WindowId) -> (TabId, PaneId) {
    let intent = Intent::TabNew {
        window,
        content: terminal(),
        index: None,
        activate: true,
        group: None,
    };
    let (_, created) = must(workspace, intent);
    (created.tab.unwrap(), created.pane.unwrap())
}

/// The pane of a window that has the keyboard.
pub fn focus(workspace: &Workspace, window: WindowId) -> Option<PaneId> {
    workspace.window(window).unwrap().focus
}

/// The ids of a tab's panes, in the order they are laid out.
pub fn panes_of(workspace: &Workspace, tab: TabId) -> Vec<PaneId> {
    workspace.tab(tab).unwrap().pane_ids()
}
