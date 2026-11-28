//! Tabs made from layout files: put into a window whole, and given a new shape when the
//! file changes, the panes the file still names keeping their sessions.

mod common;

use common::*;
use lume_layout::{
    Axis, Intent, IntentError, LayoutChange, Node, Pane, PaneContent, PaneId, Split, SplitId, Tab,
    TabColor, TabId, TerminalPane, UNITS, Workspace, reshape_leaves,
};
use lume_proto::SessionId;

/// A pane as a layout file describes it: numbered by the file, with a key.
fn leaf(number: u32, key: &str) -> Node {
    Node::Pane(Pane {
        id: PaneId::new(number),
        title: None,
        icon: None,
        content: PaneContent::Terminal(TerminalPane {
            cwd: Some(format!("/work/{key}")),
            ..TerminalPane::default()
        }),
        key: Some(key.to_owned()),
    })
}

fn row(number: u32, ratios: &[u32], children: Vec<Node>) -> Node {
    Node::Split(Split {
        id: SplitId::new(number),
        axis: Axis::Row,
        ratios: ratios.to_vec(),
        children,
    })
}

fn made(root: Node, origin: &str) -> Tab {
    Tab {
        id: TabId::new(1),
        title: Some("code".to_owned()),
        root,
        zoomed: None,
        last_focus: None,
        color: Some(TabColor::Blue),
        locked: false,
        muted: false,
        group: None,
        origin: Some(origin.to_owned()),
        workspace: None,
    }
}

fn keys(workspace: &Workspace, tab: TabId) -> Vec<String> {
    let tab = workspace.tab(tab).unwrap();
    tab.root
        .panes()
        .into_iter()
        .map(|pane| pane.key.clone().unwrap_or_default())
        .collect()
}

fn session_of(workspace: &Workspace, pane: PaneId) -> Option<SessionId> {
    match &workspace.pane(pane)?.content {
        PaneContent::Terminal(terminal) => terminal.session,
        _ => None,
    }
}

/// Gives a pane a running session, as starting it would.
fn start(workspace: &mut Workspace, pane: PaneId, number: u64) {
    if let PaneContent::Terminal(terminal) = &mut workspace.pane_mut(pane).unwrap().content {
        terminal.session = Some(SessionId::new(number).unwrap());
    }
}

fn grafted() -> (Workspace, TabId) {
    let (mut workspace, window, _, _) = one_pane();
    let wanted = made(
        row(
            10,
            &[UNITS * 6 / 10, UNITS * 4 / 10],
            vec![leaf(11, "editor"), leaf(12, "check")],
        ),
        "dev",
    );
    let (_, created) = must(
        &mut workspace,
        Intent::TabGraft {
            window,
            tab: Box::new(wanted),
            index: None,
        },
    );
    (workspace, created.tab.unwrap())
}

#[test]
fn a_tab_from_a_file_is_put_in_numbered_anew_and_shown() {
    let (workspace, tab) = grafted();
    let window = &workspace.windows[0];
    assert_eq!(window.centre.tabs.len(), 2);
    assert_eq!(window.centre.active, Some(tab));
    let found = workspace.tab(tab).unwrap();
    assert_eq!(found.origin.as_deref(), Some("dev"));
    assert_eq!(found.color, Some(TabColor::Blue));
    assert_eq!(keys(&workspace, tab), ["editor", "check"]);
    // The file's numbers are not the workspace's.
    assert!(!found.pane_ids().contains(&PaneId::new(11)));
    assert!(
        found
            .pane_ids()
            .iter()
            .all(|pane| session_of(&workspace, *pane).is_none())
    );
}

#[test]
fn a_tab_from_a_file_can_go_where_it_is_told_and_not_past_the_end() {
    let (mut workspace, window, first, _) = one_pane();
    let (_, created) = must(
        &mut workspace,
        Intent::TabGraft {
            window,
            tab: Box::new(made(leaf(5, "only"), "one")),
            index: Some(0),
        },
    );
    let order: Vec<TabId> = workspace.windows[0]
        .centre
        .tabs
        .iter()
        .map(|tab| tab.id)
        .collect();
    assert_eq!(order, [created.tab.unwrap(), first]);

    let error = refused(
        &mut workspace,
        Intent::TabGraft {
            window,
            tab: Box::new(made(leaf(5, "only"), "one")),
            index: Some(9),
        },
    );
    assert_eq!(error, IntentError::BadIndex { index: 9, len: 2 });
}

#[test]
fn a_new_shape_keeps_the_panes_the_file_still_names_and_their_sessions() {
    let (mut workspace, tab) = grafted();
    let [editor, check] = workspace.tab(tab).unwrap().pane_ids()[..] else {
        panic!("two panes")
    };
    start(&mut workspace, editor, 7);
    start(&mut workspace, check, 8);

    // The file now has the check first, a log beside it, and no editor.
    let wanted = made(
        row(
            20,
            &[UNITS / 2, UNITS / 2],
            vec![leaf(21, "check"), leaf(22, "logs")],
        ),
        "dev",
    );
    assert_eq!(
        reshape_leaves(workspace.tab(tab).unwrap(), &wanted),
        [editor]
    );
    let (changes, _) = must(
        &mut workspace,
        Intent::TabReshape {
            tab,
            wanted: Box::new(wanted),
            put_away: Vec::new(),
        },
    );
    assert!(
        changes
            .iter()
            .any(|change| matches!(change, LayoutChange::TabTree { .. }))
    );
    assert_eq!(keys(&workspace, tab), ["check", "logs"]);
    let panes = workspace.tab(tab).unwrap().pane_ids();
    assert_eq!(panes[0], check);
    assert_eq!(session_of(&workspace, check), SessionId::new(8));
    assert_eq!(session_of(&workspace, panes[1]), None);
    // The editor had no place, and was not asked to be kept: it closed.
    assert!(workspace.pane(editor).is_none());
}

#[test]
fn a_pane_put_away_by_a_new_shape_keeps_running_out_of_sight() {
    let (mut workspace, tab) = grafted();
    let [editor, check] = workspace.tab(tab).unwrap().pane_ids()[..] else {
        panic!("two panes")
    };
    start(&mut workspace, check, 8);
    let wanted = made(leaf(30, "editor"), "dev");
    must(
        &mut workspace,
        Intent::TabReshape {
            tab,
            wanted: Box::new(wanted),
            put_away: vec![check],
        },
    );
    assert_eq!(workspace.tab(tab).unwrap().pane_ids(), [editor]);
    let window = &workspace.windows[0];
    let floating = window
        .floating
        .iter()
        .find(|float| float.pane.id == check)
        .unwrap();
    assert!(floating.minimised.is_some());
    assert_eq!(session_of(&workspace, check), SessionId::new(8));
}

#[test]
fn only_the_tabs_own_panes_can_be_put_away() {
    let (mut workspace, tab) = grafted();
    let elsewhere = workspace.windows[0].centre.tabs[0].first_pane();
    let error = refused(
        &mut workspace,
        Intent::TabReshape {
            tab,
            wanted: Box::new(made(leaf(30, "editor"), "dev")),
            put_away: vec![elsewhere],
        },
    );
    assert_eq!(error, IntentError::NoSuchPane(elsewhere));
}

#[test]
fn a_tab_from_a_file_says_where_it_came_from_in_its_state() {
    let (mut workspace, tab) = grafted();
    let mut wanted = made(
        row(
            10,
            &[UNITS / 2, UNITS / 2],
            vec![leaf(11, "editor"), leaf(12, "check")],
        ),
        "dev",
    );
    wanted.origin = None;
    wanted.color = None;
    let (changes, _) = must(
        &mut workspace,
        Intent::TabReshape {
            tab,
            wanted: Box::new(wanted),
            put_away: Vec::new(),
        },
    );
    assert!(changes.iter().any(|change| matches!(
        change,
        LayoutChange::TabState {
            origin: None,
            color: None,
            ..
        }
    )));
    assert_eq!(workspace.tab(tab).unwrap().origin, None);
}

#[test]
fn a_tab_is_tied_to_a_file_and_let_go_of_it() {
    let (mut workspace, _, tab, pane) = one_pane();
    must(
        &mut workspace,
        Intent::TabOrigin {
            tab,
            origin: Some("saved#1".to_owned()),
            keys: vec![(pane, "shell".to_owned())],
        },
    );
    let found = workspace.tab(tab).unwrap();
    assert_eq!(found.origin.as_deref(), Some("saved#1"));
    assert_eq!(found.root.panes()[0].key.as_deref(), Some("shell"));

    must(
        &mut workspace,
        Intent::TabOrigin {
            tab,
            origin: None,
            keys: Vec::new(),
        },
    );
    assert_eq!(workspace.tab(tab).unwrap().origin, None);
}

#[test]
fn a_kept_pane_runs_what_the_file_now_says_when_it_next_starts() {
    let (mut workspace, tab) = grafted();
    let check = workspace.tab(tab).unwrap().pane_ids()[1];
    start(&mut workspace, check, 8);
    let Node::Pane(mut moved) = leaf(40, "check") else {
        unreachable!()
    };
    if let PaneContent::Terminal(terminal) = &mut moved.content {
        terminal.cwd = Some("/elsewhere".to_owned());
    }
    must(
        &mut workspace,
        Intent::TabReshape {
            tab,
            wanted: Box::new(made(Node::Pane(moved), "dev")),
            put_away: Vec::new(),
        },
    );
    let PaneContent::Terminal(terminal) = &workspace.pane(check).unwrap().content else {
        panic!("a terminal")
    };
    assert_eq!(terminal.cwd.as_deref(), Some("/elsewhere"));
    assert_eq!(terminal.session, SessionId::new(8));
}
