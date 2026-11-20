//! Tabs given colours, locked, gathered into groups, moved a step at a time, merged,
//! and groups folded, moved, closed, put away and opened again.

mod common;

use common::*;
use lume_layout::{
    GroupIcon, GroupId, Intent, IntentError, LayoutChange, PaneContent, Shelved, TabColor, TabId,
    TabTarget, WindowId, Workspace, group_add, group_remove, group_step, tab_step, tabs_beside,
};

fn order(workspace: &Workspace, window: WindowId) -> Vec<TabId> {
    let centre = &workspace.window(window).unwrap().centre;
    centre.tabs.iter().map(|tab| tab.id).collect()
}

fn active(workspace: &Workspace, window: WindowId) -> Option<TabId> {
    workspace.window(window).unwrap().centre.active
}

fn group_of(workspace: &Workspace, tab: TabId) -> Option<GroupId> {
    workspace.tab(tab).unwrap().group
}

fn groups(workspace: &Workspace, window: WindowId) -> Vec<GroupId> {
    let centre = &workspace.window(window).unwrap().centre;
    centre.groups.iter().map(|group| group.id).collect()
}

fn show(workspace: &mut Workspace, tab: TabId) {
    must(
        workspace,
        Intent::TabActivate {
            target: TabTarget::Id { tab },
        },
    );
}

fn new_group(workspace: &mut Workspace, tab: TabId) -> GroupId {
    must(workspace, Intent::GroupNew { tab }).1.group.unwrap()
}

/// A window with four tabs, `t1` to `t4` here, the last on show.
fn four_tabs() -> (Workspace, WindowId, [TabId; 4]) {
    let (mut workspace, window, first, _) = one_pane();
    let (second, _) = open_tab(&mut workspace, window);
    let (third, _) = open_tab(&mut workspace, window);
    let (fourth, _) = open_tab(&mut workspace, window);
    (workspace, window, [first, second, third, fourth])
}

fn step(workspace: &mut Workspace, window: WindowId, tab: TabId, forward: bool) {
    let intent = tab_step(workspace.window(window).unwrap(), tab, forward).unwrap();
    must(workspace, intent);
}

#[test]
fn a_tab_is_given_a_colour_and_locked() {
    let (mut workspace, _, tab, _) = one_pane();
    let (changes, _) = must(
        &mut workspace,
        Intent::TabColor {
            tab,
            color: Some(TabColor::Teal),
        },
    );
    assert_eq!(
        changes,
        vec![LayoutChange::TabState {
            tab,
            color: Some(TabColor::Teal),
            locked: false,
            muted: false,
            group: None,
        }]
    );
    must(&mut workspace, Intent::TabLock { tab, on: true });
    let found = workspace.tab(tab).unwrap();
    assert_eq!((found.color, found.locked), (Some(TabColor::Teal), true));

    assert_eq!(
        refused(&mut workspace, Intent::TabClose { tab }),
        IntentError::Locked(tab)
    );
    must(&mut workspace, Intent::TabColor { tab, color: None });
    assert_eq!(workspace.tab(tab).unwrap().color, None);
}

#[test]
fn closing_the_tabs_beside_one_passes_over_those_that_are_locked() {
    let (mut workspace, window, [first, second, third, fourth]) = four_tabs();
    must(
        &mut workspace,
        Intent::TabLock {
            tab: first,
            on: true,
        },
    );
    let left = tabs_beside(workspace.window(window).unwrap(), third, false);
    assert_eq!(left, vec![first, second]);
    must(&mut workspace, Intent::TabsClose { tabs: left });
    assert_eq!(order(&workspace, window), vec![first, third, fourth]);

    show(&mut workspace, fourth);
    let right = tabs_beside(workspace.window(window).unwrap(), first, true);
    must(&mut workspace, Intent::TabsClose { tabs: right });
    assert_eq!(order(&workspace, window), vec![first]);
    assert_eq!(active(&workspace, window), Some(first));
}

#[test]
fn a_group_takes_the_first_colour_no_other_group_has_and_gathers_its_tabs() {
    let (mut workspace, window, [first, second, third, fourth]) = four_tabs();
    let blue = new_group(&mut workspace, second);
    let pink = new_group(&mut workspace, third);
    let colors: Vec<TabColor> = workspace
        .window(window)
        .unwrap()
        .centre
        .groups
        .iter()
        .map(|group| group.color)
        .collect();
    assert_eq!(colors, vec![TabColor::Blue, TabColor::Pink]);

    // Added to the first group, the last tab stands after its last tab.
    let add = group_add(workspace.window(window).unwrap(), fourth, blue).unwrap();
    must(&mut workspace, add);
    assert_eq!(
        order(&workspace, window),
        vec![first, second, fourth, third]
    );
    assert_eq!(group_of(&workspace, fourth), Some(blue));
    assert_eq!(groups(&workspace, window), vec![blue, pink]);

    // A tab put elsewhere in a group's name is brought among its tabs.
    must(
        &mut workspace,
        Intent::TabPlace {
            tab: first,
            index: 3,
            group: Some(blue),
        },
    );
    assert_eq!(
        order(&workspace, window),
        vec![second, fourth, first, third]
    );

    // Taken out, it stands just after the group.
    let remove = group_remove(workspace.window(window).unwrap(), second).unwrap();
    must(&mut workspace, remove);
    assert_eq!(
        order(&workspace, window),
        vec![fourth, first, second, third]
    );
    assert_eq!(group_of(&workspace, second), None);

    // A group whose last tab leaves is gone.
    must(&mut workspace, Intent::TabClose { tab: third });
    assert_eq!(groups(&workspace, window), vec![blue]);
    assert_eq!(
        refused(&mut workspace, Intent::GroupUngroup { group: pink }),
        IntentError::NoSuchGroup(pink)
    );
}

#[test]
fn a_tab_moved_between_two_tabs_of_a_group_joins_it() {
    let (mut workspace, window, [first, second, third, fourth]) = four_tabs();
    let group = new_group(&mut workspace, second);
    let add = group_add(workspace.window(window).unwrap(), third, group).unwrap();
    must(&mut workspace, add);

    must(
        &mut workspace,
        Intent::TabMove {
            tab: fourth,
            index: 2,
        },
    );
    assert_eq!(
        order(&workspace, window),
        vec![first, second, fourth, third]
    );
    assert_eq!(group_of(&workspace, fourth), Some(group));

    // At the group's edge it stays in it; beyond, it leaves.
    must(
        &mut workspace,
        Intent::TabMove {
            tab: fourth,
            index: 1,
        },
    );
    assert_eq!(group_of(&workspace, fourth), Some(group));
    must(
        &mut workspace,
        Intent::TabMove {
            tab: fourth,
            index: 0,
        },
    );
    assert_eq!(
        order(&workspace, window),
        vec![fourth, first, second, third]
    );
    assert_eq!(group_of(&workspace, fourth), None);

    // Next to the outer edge of a group that is not its own, it stays out.
    must(
        &mut workspace,
        Intent::TabMove {
            tab: fourth,
            index: 3,
        },
    );
    assert_eq!(
        order(&workspace, window),
        vec![first, second, third, fourth]
    );
    assert_eq!(group_of(&workspace, fourth), None);
}

#[test]
fn a_tab_passes_the_edge_of_a_group_in_two_steps() {
    let (mut workspace, window, [first, second, third, fourth]) = four_tabs();
    let group = new_group(&mut workspace, second);
    let add = group_add(workspace.window(window).unwrap(), third, group).unwrap();
    must(&mut workspace, add);

    // From the right: into the group where it stands, then along it.
    step(&mut workspace, window, fourth, false);
    assert_eq!(
        order(&workspace, window),
        vec![first, second, third, fourth]
    );
    assert_eq!(group_of(&workspace, fourth), Some(group));
    step(&mut workspace, window, fourth, false);
    assert_eq!(
        order(&workspace, window),
        vec![first, second, fourth, third]
    );
    step(&mut workspace, window, fourth, false);
    assert_eq!(
        order(&workspace, window),
        vec![first, fourth, second, third]
    );
    // At the left edge: out of the group where it stands, then past the first tab.
    step(&mut workspace, window, fourth, false);
    assert_eq!(
        order(&workspace, window),
        vec![first, fourth, second, third]
    );
    assert_eq!(group_of(&workspace, fourth), None);
    step(&mut workspace, window, fourth, false);
    assert_eq!(
        order(&workspace, window),
        vec![fourth, first, second, third]
    );
    assert!(tab_step(workspace.window(window).unwrap(), fourth, false).is_none());

    // A group of one moves as a group.
    let alone = new_group(&mut workspace, fourth);
    step(&mut workspace, window, fourth, true);
    assert_eq!(
        order(&workspace, window),
        vec![first, fourth, second, third]
    );
    assert_eq!(group_of(&workspace, fourth), Some(alone));
}

#[test]
fn a_group_moves_past_a_tab_or_a_whole_group_and_never_into_one() {
    let (mut workspace, window, [first, second, third, fourth]) = four_tabs();
    let a = new_group(&mut workspace, first);
    let add = group_add(workspace.window(window).unwrap(), second, a).unwrap();
    must(&mut workspace, add);
    let b = new_group(&mut workspace, third);
    let add = group_add(workspace.window(window).unwrap(), fourth, b).unwrap();
    must(&mut workspace, add);

    let mv = group_step(workspace.window(window).unwrap(), a, true).unwrap();
    must(&mut workspace, mv);
    assert_eq!(
        order(&workspace, window),
        vec![third, fourth, first, second]
    );
    assert_eq!(groups(&workspace, window), vec![b, a]);
    assert!(group_step(workspace.window(window).unwrap(), a, true).is_none());

    assert_eq!(
        refused(&mut workspace, Intent::GroupMove { group: a, index: 1 }),
        IntentError::InsideGroup
    );
    assert_eq!(
        refused(&mut workspace, Intent::GroupMove { group: a, index: 3 }),
        IntentError::BadIndex { index: 3, len: 2 }
    );
    must(&mut workspace, Intent::GroupMove { group: a, index: 0 });
    assert_eq!(
        order(&workspace, window),
        vec![first, second, third, fourth]
    );
}

#[test]
fn the_tab_on_show_is_never_folded_away() {
    let (mut workspace, window, [first, second, third, fourth]) = four_tabs();
    let group = new_group(&mut workspace, second);
    let add = group_add(workspace.window(window).unwrap(), third, group).unwrap();
    must(&mut workspace, add);
    show(&mut workspace, third);

    // Folding the group on show shows the nearest tab outside it, after it first.
    must(
        &mut workspace,
        Intent::GroupSet {
            group,
            name: None,
            color: None,
            icon: None,
            collapsed: Some(true),
            kept: None,
            muted: None,
        },
    );
    assert_eq!(active(&workspace, window), Some(fourth));
    assert!(workspace.window(window).unwrap().centre.groups[0].collapsed);

    // Closing the tab on show passes over the folded tabs.
    must(&mut workspace, Intent::TabClose { tab: fourth });
    assert_eq!(active(&workspace, window), Some(first));

    // Showing a folded tab opens its group.
    show(&mut workspace, second);
    assert!(!workspace.window(window).unwrap().centre.groups[0].collapsed);

    // With every other tab gone there is nothing to show instead.
    must(&mut workspace, Intent::TabClose { tab: first });
    let fold = Intent::GroupSet {
        group,
        name: None,
        color: None,
        icon: None,
        collapsed: Some(true),
        kept: None,
        muted: None,
    };
    assert_eq!(
        refused(&mut workspace, fold),
        IntentError::NothingElseToShow
    );
}

#[test]
fn a_group_is_named_coloured_and_given_a_picture() {
    let (mut workspace, _, tab, _) = one_pane();
    let group = new_group(&mut workspace, tab);
    let set = |name, icon| Intent::GroupSet {
        group,
        name,
        color: Some(TabColor::Green),
        icon,
        collapsed: None,
        kept: Some(true),
        muted: None,
    };
    must(
        &mut workspace,
        set(
            Some(Some("  Billing\t".to_owned())),
            Some(Some(GroupIcon::Bug)),
        ),
    );
    let found = workspace.windows[0].centre.groups[0].clone();
    assert_eq!(found.name.as_deref(), Some("Billing"));
    assert_eq!(
        (found.color, found.icon, found.kept),
        (TabColor::Green, Some(GroupIcon::Bug), true)
    );

    // Left out, they stay; given as null, they go.
    must(&mut workspace, set(None, None));
    assert_eq!(
        workspace.windows[0].centre.groups[0].name.as_deref(),
        Some("Billing")
    );
    must(&mut workspace, set(Some(None), Some(None)));
    let found = &workspace.windows[0].centre.groups[0];
    assert_eq!((found.name.clone(), found.icon), (None, None));

    // The JSON tells left out from null.
    let left_out: Intent =
        serde_json::from_str(&format!(r#"{{"type":"group_set","group":"{group}"}}"#)).unwrap();
    let cleared: Intent = serde_json::from_str(&format!(
        r#"{{"type":"group_set","group":"{group}","name":null,"icon":null}}"#
    ))
    .unwrap();
    assert!(matches!(
        left_out,
        Intent::GroupSet {
            name: None,
            icon: None,
            ..
        }
    ));
    assert!(matches!(
        cleared,
        Intent::GroupSet {
            name: Some(None),
            icon: Some(None),
            ..
        }
    ));
}

#[test]
fn a_new_tab_in_a_group_goes_after_its_last_tab() {
    let (mut workspace, window, [first, second, third, _]) = four_tabs();
    let group = new_group(&mut workspace, first);
    let add = group_add(workspace.window(window).unwrap(), second, group).unwrap();
    must(&mut workspace, add);
    let (_, created) = must(
        &mut workspace,
        Intent::TabNew {
            window,
            content: terminal(),
            index: None,
            activate: true,
            group: Some(group),
        },
    );
    let new = created.tab.unwrap();
    assert_eq!(
        &order(&workspace, window)[..4],
        &[first, second, new, third]
    );
    assert_eq!(group_of(&workspace, new), Some(group));
}

#[test]
fn ending_a_group_leaves_its_tabs_and_closing_one_keeps_its_locked_tabs() {
    let (mut workspace, window, [first, second, third, fourth]) = four_tabs();
    let group = new_group(&mut workspace, second);
    let add = group_add(workspace.window(window).unwrap(), third, group).unwrap();
    must(&mut workspace, add);
    must(&mut workspace, Intent::GroupUngroup { group });
    assert_eq!(
        order(&workspace, window),
        vec![first, second, third, fourth]
    );
    assert!(groups(&workspace, window).is_empty());

    let group = new_group(&mut workspace, second);
    let add = group_add(workspace.window(window).unwrap(), third, group).unwrap();
    must(&mut workspace, add);
    must(
        &mut workspace,
        Intent::TabLock {
            tab: third,
            on: true,
        },
    );
    show(&mut workspace, second);
    must(&mut workspace, Intent::GroupClose { group });
    assert_eq!(order(&workspace, window), vec![first, third, fourth]);
    assert_eq!(group_of(&workspace, third), None);
    assert_eq!(active(&workspace, window), Some(third));
    assert!(
        workspace.shelf.is_empty(),
        "a group that is not kept is forgotten"
    );
}

#[test]
fn a_kept_group_is_put_away_and_opened_again_with_new_sessions() {
    let (mut workspace, window, first, _) = one_pane();
    let tab_with = |workspace: &mut Workspace, session| {
        let intent = Intent::TabNew {
            window,
            content: terminal(),
            index: None,
            activate: true,
            group: None,
        };
        must_with(workspace, intent, Some(started(session)))
            .1
            .tab
            .unwrap()
    };
    let second = tab_with(&mut workspace, 7);
    let third = tab_with(&mut workspace, 8);
    let group = new_group(&mut workspace, second);
    let add = group_add(workspace.window(window).unwrap(), third, group).unwrap();
    must(&mut workspace, add);
    must(
        &mut workspace,
        Intent::GroupSet {
            group,
            name: Some(Some("Infra".to_owned())),
            color: None,
            icon: None,
            collapsed: None,
            kept: Some(true),
            muted: None,
        },
    );

    let (changes, _) = must(&mut workspace, Intent::GroupClose { group });
    assert_eq!(order(&workspace, window), vec![first]);
    let closed = changes
        .iter()
        .filter(|change| matches!(change, LayoutChange::PaneClosed { .. }))
        .count();
    assert_eq!(closed, 2, "their sessions end");
    let [
        Shelved {
            group: put_away,
            tabs,
        },
    ] = workspace.shelf.as_slice()
    else {
        panic!("one group is put away: {:?}", workspace.shelf);
    };
    assert_eq!(put_away.name.as_deref(), Some("Infra"));
    assert_eq!(
        tabs.iter().map(|tab| tab.id).collect::<Vec<_>>(),
        vec![second, third]
    );
    for tab in tabs {
        for pane in tab.root.panes() {
            let PaneContent::Terminal(terminal) = &pane.content else {
                panic!("a terminal");
            };
            assert_eq!(terminal.session, None);
        }
    }

    must(&mut workspace, Intent::GroupReopen { group, window });
    assert_eq!(order(&workspace, window), vec![first, second, third]);
    assert_eq!(active(&workspace, window), Some(second));
    assert_eq!(groups(&workspace, window), vec![group]);
    assert!(workspace.shelf.is_empty());

    must(&mut workspace, Intent::GroupClose { group });
    must(&mut workspace, Intent::GroupForget { group });
    assert!(workspace.shelf.is_empty());
    assert_eq!(
        refused(&mut workspace, Intent::GroupReopen { group, window }),
        IntentError::NoSuchGroup(group)
    );
}

#[test]
fn a_tab_is_merged_into_another_beside_its_panes() {
    let (mut workspace, window, [first, second, _, _]) = four_tabs();
    let moved = panes_of(&workspace, second);
    let kept = panes_of(&workspace, first);
    must(
        &mut workspace,
        Intent::TabMerge {
            tab: second,
            into: first,
        },
    );
    assert!(workspace.tab(second).is_none());
    assert_eq!(panes_of(&workspace, first), [kept, moved.clone()].concat());
    assert_eq!(active(&workspace, window), Some(first));
    assert_eq!(focus(&workspace, window), moved.first().copied());
    assert_eq!(
        refused(
            &mut workspace,
            Intent::TabMerge {
                tab: first,
                into: first
            }
        ),
        IntentError::SameTab(first)
    );
}

#[test]
fn a_group_moves_to_another_workspace_numbered_anew() {
    let (mut workspace, window, [_, second, third, _]) = four_tabs();
    let group = new_group(&mut workspace, second);
    let add = group_add(workspace.window(window).unwrap(), third, group).unwrap();
    must(&mut workspace, add);
    must(
        &mut workspace,
        Intent::TabLock {
            tab: third,
            on: true,
        },
    );
    let taken = Shelved {
        group: workspace.window(window).unwrap().centre.groups[0].clone(),
        tabs: vec![
            workspace.tab(second).unwrap().clone(),
            workspace.tab(third).unwrap().clone(),
        ],
    };

    let take = Intent::GroupTake { group };
    assert!(take.is_core_only());
    must(&mut workspace, take);
    assert_eq!(order(&workspace, window).len(), 2, "locked tabs go too");

    let (mut other, there, _, _) = one_pane();
    // The other workspace's counter is behind: what is grafted is numbered anew.
    let (_, created) = must(
        &mut other,
        Intent::GroupGraft {
            window: there,
            group: Box::new(taken),
        },
    );
    let grafted = created.group.unwrap();
    let tabs = order(&other, there);
    assert_eq!(tabs.len(), 3);
    assert_eq!(groups(&other, there), vec![grafted]);
    assert!(!other.tab(tabs[1]).unwrap().locked);
    assert!(other.tab(tabs[2]).unwrap().locked);
}
