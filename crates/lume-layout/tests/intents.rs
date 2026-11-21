//! Each intent does what it says, and each reason for refusing one is given when it
//! applies. Every intent applied here is also checked for what must hold after any
//! intent; see `common`.

mod common;

use common::*;
use lume_layout::tuning::{DOCK_SIZE_MAX, DOCK_SIZE_MIN, FLOATING_MAX, SHARE_MIN};
use lume_layout::{
    Area, Axis, Bounds, Dir, FocusTarget, Intent, IntentError, LayoutChange, Minimised, NewContent,
    Node, PaneContent, PaneId, PanelContent, PanelId, Place, Slot, SpawnRequest, SplitId, TabId,
    TabTarget, UNITS, UnitRect, WindowId, Workspace, apply, needs,
};
use lume_proto::{BackendId, SessionId};

/// The split at the root of a tab: its direction, its ratios and the panes that are
/// its children.
fn root_split(workspace: &Workspace, tab: TabId) -> (Axis, Vec<u32>, Vec<PaneId>) {
    let Node::Split(split) = &workspace.tab(tab).unwrap().root else {
        panic!("the tab's root is a pane");
    };
    let children = split
        .children
        .iter()
        .map(|child| match child {
            Node::Pane(pane) => pane.id,
            Node::Split(inner) => panic!("split {} is inside the root", inner.id),
        })
        .collect();
    (split.axis, split.ratios.clone(), children)
}

fn focus_pane(workspace: &mut Workspace, pane: PaneId) {
    let target = FocusTarget::Pane { pane };
    must(workspace, Intent::Focus { target });
}

fn terminal_of(workspace: &Workspace, pane: PaneId) -> lume_layout::TerminalPane {
    match &workspace.pane(pane).unwrap().content {
        PaneContent::Terminal(terminal) => terminal.clone(),
        PaneContent::PluginView { .. } => panic!("{pane} is not a terminal"),
    }
}

const NOWHERE: PaneId = PaneId::new(999);

// Splitting.

#[test]
fn splitting_puts_the_new_pane_on_the_side_asked_for_and_focuses_it() {
    let sides = [
        (Dir::Right, Axis::Row, false),
        (Dir::Left, Axis::Row, true),
        (Dir::Down, Axis::Column, false),
        (Dir::Up, Axis::Column, true),
    ];
    for (dir, axis, new_first) in sides {
        let (mut workspace, window, tab, first) = one_pane();
        let new = split(&mut workspace, first, dir);
        let expected = if new_first {
            vec![new, first]
        } else {
            vec![first, new]
        };
        assert_eq!(
            root_split(&workspace, tab),
            (axis, vec![UNITS / 2, UNITS / 2], expected),
            "{dir:?}"
        );
        assert_eq!(focus(&workspace, window), Some(new));
    }
}

#[test]
fn splitting_the_same_way_again_joins_the_row() {
    let (mut workspace, _, tab, a) = one_pane();
    let b = split(&mut workspace, a, Dir::Right);
    let c = split(&mut workspace, b, Dir::Right);
    assert_eq!(
        root_split(&workspace, tab),
        (Axis::Row, vec![500_000, 250_000, 250_000], vec![a, b, c])
    );
    assert_eq!(workspace.tab(tab).unwrap().root.splits().len(), 1);
}

#[test]
fn the_new_pane_is_given_the_share_asked_for() {
    let (mut workspace, _, tab, a) = one_pane();
    let intent = Intent::Split {
        pane: a,
        dir: Dir::Left,
        ratio: 250_000,
        content: plugin_view(),
    };
    let new = must(&mut workspace, intent).1.pane.unwrap();
    assert_eq!(
        root_split(&workspace, tab),
        (Axis::Row, vec![250_000, 750_000], vec![new, a])
    );
    assert!(matches!(
        workspace.pane(new).unwrap().content,
        PaneContent::PluginView { .. }
    ));
}

#[test]
fn a_pane_cannot_be_split_below_the_smallest_share() {
    let (mut workspace, _, tab, a) = one_pane();
    // A half, a quarter, an eighth and a sixteenth are large enough; a thirty-second
    // is not.
    for _ in 0..4 {
        split(&mut workspace, a, Dir::Right);
    }
    let intent = Intent::Split {
        pane: a,
        dir: Dir::Right,
        ratio: UNITS / 2,
        content: terminal(),
    };
    assert_eq!(refused(&mut workspace, intent), IntentError::TooSmall);
    assert_eq!(panes_of(&workspace, tab).len(), 5);
}

#[test]
fn a_share_outside_what_a_pane_may_have_is_refused() {
    let (mut workspace, _, _, a) = one_pane();
    for ratio in [0, SHARE_MIN - 1, UNITS - SHARE_MIN + 1, UNITS, u32::MAX] {
        let intent = Intent::Split {
            pane: a,
            dir: Dir::Down,
            ratio,
            content: terminal(),
        };
        let error = refused(&mut workspace, intent);
        assert!(matches!(error, IntentError::BadRatios(_)), "{ratio}");
        assert_eq!(error.code(), "invalid_intent");
    }
}

#[test]
fn an_intent_about_a_pane_that_is_not_there_is_refused() {
    let (mut workspace, _, _, a) = one_pane();
    let intents = [
        Intent::Close { pane: NOWHERE },
        Intent::Zoom {
            pane: NOWHERE,
            on: true,
        },
        Intent::Focus {
            target: FocusTarget::Pane { pane: NOWHERE },
        },
        Intent::Swap { a, b: NOWHERE },
        Intent::PaneRename {
            pane: NOWHERE,
            title: None,
        },
        Intent::PaneIcon {
            pane: NOWHERE,
            icon: None,
        },
        Intent::Float {
            pane: NOWHERE,
            rect: None,
        },
    ];
    for intent in intents {
        let error = refused(&mut workspace, intent);
        assert_eq!(error, IntentError::NoSuchPane(NOWHERE));
        assert_eq!(error.code(), "not_found");
    }
}

// Closing.

#[test]
fn closing_a_pane_gives_its_share_to_its_neighbours() {
    let (mut workspace, _, tab, a) = one_pane();
    let b = split(&mut workspace, a, Dir::Right);
    let c = split(&mut workspace, b, Dir::Right);
    let (changes, _) = must(&mut workspace, Intent::Close { pane: b });
    // A half and a quarter stay two to one.
    assert_eq!(
        root_split(&workspace, tab),
        (Axis::Row, vec![666_667, 333_333], vec![a, c])
    );
    assert!(changes.contains(&LayoutChange::PaneClosed {
        pane: b,
        content: PaneContent::Terminal(Default::default()),
    }));
}

#[test]
fn closing_the_last_pane_of_a_tab_closes_the_tab() {
    let (mut workspace, window, first_tab, a) = one_pane();
    let (second_tab, b) = open_tab(&mut workspace, window);

    let (changes, _) = must(&mut workspace, Intent::Close { pane: b });
    assert!(changes.contains(&LayoutChange::TabRemoved { tab: second_tab }));
    let centre = &workspace.window(window).unwrap().centre;
    assert_eq!(centre.tabs.len(), 1);
    assert_eq!(centre.active, Some(first_tab));
    assert_eq!(focus(&workspace, window), Some(a));

    must(&mut workspace, Intent::Close { pane: a });
    let window = workspace.window(window).unwrap();
    assert!(window.centre.tabs.is_empty());
    assert_eq!(window.centre.active, None);
    assert_eq!(window.focus, None);
}

#[test]
fn closing_the_focused_pane_gives_the_keyboard_to_the_one_that_had_it_before() {
    let (mut workspace, window, _, a) = one_pane();
    let b = split(&mut workspace, a, Dir::Right);
    let c = split(&mut workspace, b, Dir::Down);
    focus_pane(&mut workspace, a);
    focus_pane(&mut workspace, c);

    must(&mut workspace, Intent::Close { pane: c });
    assert_eq!(focus(&workspace, window), Some(a));
    must(&mut workspace, Intent::Close { pane: a });
    assert_eq!(focus(&workspace, window), Some(b));
}

// The keyboard.

#[test]
fn focusing_a_pane_in_a_tab_that_is_not_shown_shows_the_tab() {
    let (mut workspace, window, first_tab, a) = one_pane();
    let (second_tab, _) = open_tab(&mut workspace, window);
    assert_eq!(
        workspace.window(window).unwrap().centre.active,
        Some(second_tab)
    );

    focus_pane(&mut workspace, a);
    assert_eq!(
        workspace.window(window).unwrap().centre.active,
        Some(first_tab)
    );
}

#[test]
fn the_keyboard_goes_through_the_panes_in_order_and_back_to_the_one_before() {
    let (mut workspace, window, _, a) = one_pane();
    let b = split(&mut workspace, a, Dir::Right);
    let c = split(&mut workspace, b, Dir::Right);
    let step = |workspace: &mut Workspace, target| {
        must(workspace, Intent::Focus { target });
        focus(workspace, window).unwrap()
    };

    // From the last, the next is the first again.
    assert_eq!(step(&mut workspace, FocusTarget::Next { window }), a);
    assert_eq!(step(&mut workspace, FocusTarget::Next { window }), b);
    assert_eq!(step(&mut workspace, FocusTarget::Previous { window }), a);
    assert_eq!(step(&mut workspace, FocusTarget::Previous { window }), c);

    focus_pane(&mut workspace, a);
    assert_eq!(step(&mut workspace, FocusTarget::Back { window }), c);
    assert_eq!(step(&mut workspace, FocusTarget::Back { window }), a);
}

#[test]
fn the_keyboard_moves_to_the_pane_next_door() {
    let (mut workspace, window, _, a) = one_pane();
    let b = split(&mut workspace, a, Dir::Right);
    let c = split(&mut workspace, b, Dir::Down);
    let go = |workspace: &mut Workspace, dir| must(workspace, Intent::FocusDir { window, dir }).0;

    go(&mut workspace, Dir::Up);
    assert_eq!(focus(&workspace, window), Some(b));
    go(&mut workspace, Dir::Left);
    assert_eq!(focus(&workspace, window), Some(a));
    // Nothing lies further left: nothing changes, and that is not an error.
    assert_eq!(go(&mut workspace, Dir::Left), vec![]);
    // Both `b` and `c` are to the right of `a`; `b` had the keyboard more recently.
    go(&mut workspace, Dir::Right);
    assert_eq!(focus(&workspace, window), Some(b));
    go(&mut workspace, Dir::Down);
    assert_eq!(focus(&workspace, window), Some(c));
}

// Moving panes.

#[test]
fn a_pane_is_moved_beside_another() {
    let (mut workspace, window, tab, a) = one_pane();
    let b = split(&mut workspace, a, Dir::Right);
    let to = Place::Beside {
        pane: b,
        edge: Dir::Down,
    };
    must(&mut workspace, Intent::Move { pane: a, to });
    assert_eq!(
        root_split(&workspace, tab),
        (Axis::Column, vec![UNITS / 2, UNITS / 2], vec![b, a])
    );
    assert_eq!(focus(&workspace, window), Some(a));
}

#[test]
fn a_pane_is_moved_to_a_tab_of_its_own() {
    let (mut workspace, window, tab, a) = one_pane();
    let b = split(&mut workspace, a, Dir::Right);
    let to = Place::NewTab {
        window,
        index: Some(0),
    };
    let (_, created) = must(&mut workspace, Intent::Move { pane: b, to });
    let new_tab = created.tab.unwrap();

    let centre = &workspace.window(window).unwrap().centre;
    assert_eq!(centre.tabs[0].id, new_tab);
    assert_eq!(centre.active, Some(new_tab));
    assert_eq!(panes_of(&workspace, new_tab), vec![b]);
    assert_eq!(panes_of(&workspace, tab), vec![a]);
}

#[test]
fn a_pane_stays_in_its_window_and_is_not_put_beside_itself() {
    let (mut workspace, window, _, a) = one_pane();
    let (other_window, _, far) = open_window(&mut workspace);

    let beside_itself = Place::Beside {
        pane: a,
        edge: Dir::Left,
    };
    let error = refused(
        &mut workspace,
        Intent::Move {
            pane: a,
            to: beside_itself,
        },
    );
    assert_eq!(error, IntentError::SamePane(a));

    let places = [
        Place::Beside {
            pane: far,
            edge: Dir::Left,
        },
        Place::NewTab {
            window: other_window,
            index: None,
        },
    ];
    for to in places {
        let error = refused(&mut workspace, Intent::Move { pane: a, to });
        assert_eq!(error, IntentError::CrossWindow);
        assert_eq!(error.code(), "unsupported");
    }
    let error = refused(&mut workspace, Intent::Swap { a, b: far });
    assert_eq!(error, IntentError::CrossWindow);
    assert_eq!(workspace.window_of(a), Some(window));
}

#[test]
fn two_panes_exchange_places() {
    let (mut workspace, window, tab, a) = one_pane();
    let b = split(&mut workspace, a, Dir::Right);
    let c = split(&mut workspace, b, Dir::Right);
    must(&mut workspace, Intent::Swap { a, b: c });
    assert_eq!(panes_of(&workspace, tab), vec![c, b, a]);

    // Across tabs of one window too.
    let (other_tab, d) = open_tab(&mut workspace, window);
    must(&mut workspace, Intent::Swap { a: b, b: d });
    assert_eq!(panes_of(&workspace, tab), vec![c, d, a]);
    assert_eq!(panes_of(&workspace, other_tab), vec![b]);
    // The tab on show is still on show, and the keyboard is with a pane of it.
    assert_eq!(focus(&workspace, window), Some(b));

    assert_eq!(
        refused(&mut workspace, Intent::Swap { a, b: a }),
        IntentError::SamePane(a)
    );
}

// Sizes.

#[test]
fn a_split_is_given_new_ratios() {
    let (mut workspace, _, tab, a) = one_pane();
    let b = split(&mut workspace, a, Dir::Right);
    let Node::Split(root) = &workspace.tab(tab).unwrap().root else {
        panic!("the tab's root is a pane");
    };
    let split_id = root.id;
    let resize = |ratios: &[u32]| Intent::Resize {
        split: split_id,
        ratios: ratios.to_vec(),
    };

    let (changes, _) = must(&mut workspace, resize(&[300_000, 700_000]));
    assert_eq!(
        changes,
        vec![LayoutChange::Ratios {
            split: split_id,
            ratios: vec![300_000, 700_000],
        }]
    );

    // A child is never made smaller than the smallest share; the other makes room.
    must(&mut workspace, resize(&[10, 999_990]));
    assert_eq!(
        root_split(&workspace, tab),
        (Axis::Row, vec![SHARE_MIN, UNITS - SHARE_MIN], vec![a, b])
    );
    // Ratios that are a little off a whole are rounding, and are brought to a whole.
    must(&mut workspace, resize(&[333_333, 666_600]));
    assert_eq!(root_split(&workspace, tab).1, vec![333_333, 666_667]);

    for ratios in [&[UNITS][..], &[300_000, 300_000], &[500_000, 500_000, 0]] {
        let error = refused(&mut workspace, resize(ratios));
        assert!(matches!(error, IntentError::BadRatios(_)), "{ratios:?}");
    }
    let unknown = Intent::Resize {
        split: SplitId::new(999),
        ratios: vec![UNITS],
    };
    assert_eq!(
        refused(&mut workspace, unknown),
        IntentError::NoSuchSplit(SplitId::new(999))
    );
}

#[test]
fn a_zoomed_pane_fills_its_tab_until_the_tab_changes() {
    let (mut workspace, window, tab, a) = one_pane();
    let b = split(&mut workspace, a, Dir::Right);
    let zoomed = |workspace: &Workspace| workspace.tab(tab).unwrap().zoomed;
    let zoom = |pane, on| Intent::Zoom { pane, on };

    must(&mut workspace, zoom(a, true));
    assert_eq!(zoomed(&workspace), Some(a));
    assert_eq!(focus(&workspace, window), Some(a));

    // Splitting changes the tab, which ends the zoom.
    split(&mut workspace, a, Dir::Down);
    assert_eq!(zoomed(&workspace), None);

    // So does giving the keyboard to a pane the zoom hides.
    must(&mut workspace, zoom(a, true));
    focus_pane(&mut workspace, b);
    assert_eq!(zoomed(&workspace), None);

    must(&mut workspace, zoom(a, true));
    // Ending the zoom of a pane that is not zoomed changes nothing.
    assert_eq!(must(&mut workspace, zoom(b, false)).0, vec![]);
    must(&mut workspace, zoom(a, false));
    assert_eq!(zoomed(&workspace), None);
}

// Floating.

#[test]
fn a_pane_floats_over_its_window_and_is_docked_again() {
    let (mut workspace, window, tab, a) = one_pane();
    let b = split(&mut workspace, a, Dir::Right);

    must(
        &mut workspace,
        Intent::Float {
            pane: b,
            rect: None,
        },
    );
    let floating = &workspace.window(window).unwrap().floating;
    assert_eq!(floating.len(), 1);
    assert_eq!(floating[0].pane.id, b);
    assert_eq!(
        floating[0].rect,
        UnitRect {
            x: 200_000,
            y: 200_000,
            w: 600_000,
            h: 600_000
        }
    );
    assert_eq!(panes_of(&workspace, tab), vec![a]);
    assert_eq!(focus(&workspace, window), Some(b));

    // Without a place, it goes back beside the pane that last had the keyboard.
    must(&mut workspace, Intent::Dock { pane: b, to: None });
    assert!(workspace.window(window).unwrap().floating.is_empty());
    assert_eq!(
        root_split(&workspace, tab),
        (Axis::Row, vec![UNITS / 2, UNITS / 2], vec![a, b])
    );
}

#[test]
fn the_only_pane_of_a_window_can_float() {
    let (mut workspace, window, _, a) = one_pane();
    must(
        &mut workspace,
        Intent::Float {
            pane: a,
            rect: None,
        },
    );
    let floated = workspace.window(window).unwrap();
    assert!(floated.centre.tabs.is_empty());
    assert_eq!(floated.focus, Some(a));

    // With no tab to go back to, it gets one of its own.
    let (_, created) = must(&mut workspace, Intent::Dock { pane: a, to: None });
    assert_eq!(panes_of(&workspace, created.tab.unwrap()), vec![a]);
}

#[test]
fn a_floating_pane_is_moved_and_kept_inside_its_window() {
    let (mut workspace, window, _, a) = one_pane();
    let b = split(&mut workspace, a, Dir::Right);
    let float = |pane| Intent::Float { pane, rect: None };
    must(&mut workspace, float(b));

    let rect = UnitRect {
        x: 900_000,
        y: 0,
        w: 300_000,
        h: 300_000,
    };
    let (changes, _) = must(&mut workspace, Intent::FloatMove { pane: b, rect });
    let kept = UnitRect { x: 700_000, ..rect };
    assert_eq!(
        changes,
        vec![LayoutChange::FloatRect {
            pane: b,
            rect: kept
        }]
    );
    assert_eq!(workspace.window(window).unwrap().floating[0].rect, kept);

    let flat = UnitRect { w: 0, ..rect };
    assert_eq!(
        refused(
            &mut workspace,
            Intent::FloatMove {
                pane: b,
                rect: flat
            }
        ),
        IntentError::BadRect
    );
    assert_eq!(
        refused(&mut workspace, Intent::FloatMove { pane: a, rect }),
        IntentError::NotFloating(a)
    );
    assert_eq!(
        refused(&mut workspace, Intent::Dock { pane: a, to: None }),
        IntentError::NotFloating(a)
    );
    // What is done to a pane in a tab cannot be done to one that floats.
    assert_eq!(refused(&mut workspace, float(b)), IntentError::Floating(b));
    let split_it = Intent::Split {
        pane: b,
        dir: Dir::Right,
        ratio: UNITS / 2,
        content: terminal(),
    };
    assert_eq!(refused(&mut workspace, split_it), IntentError::Floating(b));
}

#[test]
fn the_floating_pane_with_the_keyboard_is_in_front() {
    let (mut workspace, window, _, a) = one_pane();
    let b = split(&mut workspace, a, Dir::Right);
    let c = split(&mut workspace, b, Dir::Right);
    let order = |workspace: &Workspace| -> Vec<PaneId> {
        let window = workspace.window(window).unwrap();
        window.floating.iter().map(|f| f.pane.id).collect()
    };
    must(
        &mut workspace,
        Intent::Float {
            pane: b,
            rect: None,
        },
    );
    must(
        &mut workspace,
        Intent::Float {
            pane: c,
            rect: None,
        },
    );
    assert_eq!(order(&workspace), vec![b, c]);

    focus_pane(&mut workspace, b);
    assert_eq!(order(&workspace), vec![c, b]);
    // Each further pane floats a little down and to the right of the one before.
    let rects = &workspace.window(window).unwrap().floating;
    assert_ne!(rects[0].rect, rects[1].rect);
}

#[test]
fn no_more_panes_float_than_may() {
    let (mut workspace, window, _, _) = one_pane();
    for _ in 0..FLOATING_MAX {
        let (_, pane) = open_tab(&mut workspace, window);
        must(&mut workspace, Intent::Float { pane, rect: None });
    }
    let (_, pane) = open_tab(&mut workspace, window);
    let error = refused(&mut workspace, Intent::Float { pane, rect: None });
    assert_eq!(error, IntentError::TooMany("floating panes"));
    assert_eq!(error.code(), "limit_exceeded");
}

// Names.

#[test]
fn names_are_kept_tidy() {
    let (mut workspace, _, tab, a) = one_pane();
    let rename = |title: &str| Intent::PaneRename {
        pane: a,
        title: Some(title.to_owned()),
    };
    must(&mut workspace, rename("  build\u{7} logs \n"));
    assert_eq!(
        workspace.pane(a).unwrap().title.as_deref(),
        Some("build logs")
    );
    must(&mut workspace, rename(" \t "));
    assert_eq!(workspace.pane(a).unwrap().title, None);

    let long = "é".repeat(1000);
    must(
        &mut workspace,
        Intent::TabRename {
            tab,
            title: Some(long),
        },
    );
    let kept = workspace.tab(tab).unwrap().title.clone().unwrap();
    assert_eq!(kept.chars().count(), 256);
    let (changes, _) = must(&mut workspace, Intent::TabRename { tab, title: None });
    assert_eq!(changes, vec![LayoutChange::TabTitle { tab, title: None }]);
}

#[test]
fn a_pane_is_given_a_picture_and_loses_it() {
    let (mut workspace, _, _, a) = one_pane();
    let give = |icon: Option<&str>| Intent::PaneIcon {
        pane: a,
        icon: icon.map(str::to_owned),
    };
    let (changes, _) = must(&mut workspace, give(Some("agent")));
    assert!(matches!(changes[..], [LayoutChange::Pane { .. }]));
    assert_eq!(workspace.pane(a).unwrap().icon.as_deref(), Some("agent"));

    must(&mut workspace, give(Some("image:some.plugin:mark-2")));
    assert_eq!(
        workspace.pane(a).unwrap().icon.as_deref(),
        Some("image:some.plugin:mark-2")
    );

    // A name that is not plain, or is too long, is no picture.
    for odd in ["Agent", "two words", "a/b", "", &"a".repeat(97)] {
        must(&mut workspace, give(Some("agent")));
        must(&mut workspace, give(Some(odd)));
        assert_eq!(workspace.pane(a).unwrap().icon, None, "{odd:?}");
    }

    must(&mut workspace, give(Some("agent")));
    must(&mut workspace, give(None));
    assert_eq!(workspace.pane(a).unwrap().icon, None);
    // A pane with no picture is written as it always was.
    let written = serde_json::to_value(workspace.pane(a).unwrap()).unwrap();
    assert!(written.get("icon").is_none());
}

// Tabs.

#[test]
fn tabs_are_opened_shown_moved_and_closed() {
    let (mut workspace, window, first, a) = one_pane();
    let order = |workspace: &Workspace| -> Vec<TabId> {
        let centre = &workspace.window(window).unwrap().centre;
        centre.tabs.iter().map(|tab| tab.id).collect()
    };
    let active = |workspace: &Workspace| workspace.window(window).unwrap().centre.active;
    let show = |workspace: &mut Workspace, target| {
        must(workspace, Intent::TabActivate { target });
    };

    // Opened in front without being shown: the tab on show and the keyboard stay.
    let behind = Intent::TabNew {
        window,
        content: terminal(),
        index: Some(0),
        activate: false,
        group: None,
    };
    let second = must(&mut workspace, behind).1.tab.unwrap();
    assert_eq!(order(&workspace), vec![second, first]);
    assert_eq!(active(&workspace), Some(first));
    assert_eq!(focus(&workspace, window), Some(a));

    let too_far = Intent::TabNew {
        window,
        content: terminal(),
        index: Some(5),
        activate: true,
        group: None,
    };
    assert_eq!(
        refused(&mut workspace, too_far),
        IntentError::BadIndex { index: 5, len: 2 }
    );

    show(&mut workspace, TabTarget::Index { window, index: 0 });
    assert_eq!(active(&workspace), Some(second));
    show(&mut workspace, TabTarget::Next { window });
    assert_eq!(active(&workspace), Some(first));
    // After the last comes the first, and before the first the last.
    show(&mut workspace, TabTarget::Next { window });
    assert_eq!(active(&workspace), Some(second));
    show(&mut workspace, TabTarget::Previous { window });
    assert_eq!(active(&workspace), Some(first));
    show(&mut workspace, TabTarget::Id { tab: second });
    assert_eq!(active(&workspace), Some(second));
    assert_eq!(
        refused(
            &mut workspace,
            Intent::TabActivate {
                target: TabTarget::Index { window, index: 2 }
            }
        ),
        IntentError::BadIndex { index: 2, len: 2 }
    );

    must(
        &mut workspace,
        Intent::TabMove {
            tab: first,
            index: 0,
        },
    );
    assert_eq!(order(&workspace), vec![first, second]);
    assert_eq!(
        refused(
            &mut workspace,
            Intent::TabMove {
                tab: first,
                index: 2
            }
        ),
        IntentError::BadIndex { index: 2, len: 2 }
    );

    // Closing the tab on show shows the one after it, or the one before the last.
    let (changes, _) = must(&mut workspace, Intent::TabClose { tab: second });
    assert_eq!(active(&workspace), Some(first));
    assert_eq!(
        changes
            .iter()
            .filter(|change| matches!(change, LayoutChange::PaneClosed { .. }))
            .count(),
        1
    );
    assert_eq!(
        refused(&mut workspace, Intent::TabClose { tab: second }),
        IntentError::NoSuchTab(second)
    );
}

#[test]
fn closing_the_tab_on_show_shows_the_one_after_it() {
    let (mut workspace, window, first, _) = one_pane();
    let (second, _) = open_tab(&mut workspace, window);
    let (third, _) = open_tab(&mut workspace, window);
    must(
        &mut workspace,
        Intent::TabActivate {
            target: TabTarget::Id { tab: second },
        },
    );
    must(&mut workspace, Intent::TabClose { tab: second });
    let active = workspace.window(window).unwrap().centre.active;
    assert_eq!(active, Some(third));
    must(&mut workspace, Intent::TabClose { tab: third });
    let active = workspace.window(window).unwrap().centre.active;
    assert_eq!(active, Some(first));
}

#[test]
fn showing_a_tab_gives_the_keyboard_to_the_pane_that_had_it_there() {
    let (mut workspace, window, first, a) = one_pane();
    split(&mut workspace, a, Dir::Right);
    focus_pane(&mut workspace, a);
    open_tab(&mut workspace, window);

    must(
        &mut workspace,
        Intent::TabActivate {
            target: TabTarget::Id { tab: first },
        },
    );
    assert_eq!(focus(&workspace, window), Some(a));
}

// Panels.

fn tab_list() -> PanelId {
    PanelId::new("lume.tabs").unwrap()
}

#[test]
fn a_panel_is_docked_moved_folded_and_resized() {
    let (mut workspace, window, _, _) = one_pane();
    let panels = |workspace: &Workspace, area| -> Vec<PanelId> {
        let dock = &workspace.window(window).unwrap().dock;
        dock.area(area)
            .panels
            .iter()
            .map(|p| p.id.clone())
            .collect()
    };
    let set = |area| Intent::PanelSet {
        window,
        panel: tab_list(),
        content: PanelContent::TabList {},
        area,
        index: 0,
    };

    must(&mut workspace, set(Some(Area::Left)));
    assert_eq!(panels(&workspace, Area::Left), vec![tab_list()]);

    let move_to = |area, index| Intent::PanelMove {
        window,
        panel: tab_list(),
        area,
        index,
    };
    must(&mut workspace, move_to(Area::Right, 0));
    assert_eq!(panels(&workspace, Area::Left), vec![]);
    assert_eq!(panels(&workspace, Area::Right), vec![tab_list()]);
    assert_eq!(
        refused(&mut workspace, move_to(Area::Bottom, 3)),
        IntentError::BadIndex { index: 3, len: 0 }
    );

    let folded = |workspace: &Workspace| {
        let dock = &workspace.window(window).unwrap().dock;
        dock.area(Area::Right).collapsed
    };
    let toggle = |on| Intent::PanelToggle {
        window,
        area: Area::Right,
        on,
    };
    must(&mut workspace, toggle(None));
    assert!(folded(&workspace));
    must(&mut workspace, toggle(None));
    assert!(!folded(&workspace));
    must(&mut workspace, toggle(Some(false)));
    assert!(folded(&workspace));
    assert_eq!(must(&mut workspace, toggle(Some(false))).0, vec![]);

    let resize = |size| Intent::PanelResize {
        window,
        area: Area::Right,
        size,
    };
    let (changes, _) = must(&mut workspace, resize(10));
    assert_eq!(
        changes,
        vec![LayoutChange::DockSize {
            window,
            area: Area::Right,
            size: DOCK_SIZE_MIN,
        }]
    );
    must(&mut workspace, resize(u16::MAX));
    let dock = &workspace.window(window).unwrap().dock;
    assert_eq!(dock.area(Area::Right).size, DOCK_SIZE_MAX);

    // Taken out again by being set nowhere.
    must(&mut workspace, set(None));
    assert_eq!(panels(&workspace, Area::Right), vec![]);
}

#[test]
fn the_top_bar_and_its_slots_stay_as_they_are() {
    let (mut workspace, window, _, _) = one_pane();
    let slot = Slot::TopbarLeft.panel_id();
    let fixed = [
        Intent::PanelMove {
            window,
            panel: slot.clone(),
            area: Area::Left,
            index: 0,
        },
        Intent::PanelToggle {
            window,
            area: Area::Top,
            on: None,
        },
        Intent::PanelResize {
            window,
            area: Area::Top,
            size: 300,
        },
        // A slot belongs in the top bar, and a panel that is not a slot does not.
        Intent::PanelSet {
            window,
            panel: slot,
            content: PanelContent::Slot {
                slot: Slot::TopbarLeft,
            },
            area: Some(Area::Left),
            index: 0,
        },
        Intent::PanelSet {
            window,
            panel: tab_list(),
            content: PanelContent::TabList {},
            area: Some(Area::Top),
            index: 0,
        },
    ];
    for intent in fixed {
        assert_eq!(refused(&mut workspace, intent), IntentError::Fixed);
    }

    must(
        &mut workspace,
        Intent::PanelSet {
            window,
            panel: tab_list(),
            content: PanelContent::TabList {},
            area: Some(Area::Left),
            index: 0,
        },
    );
    let to_top = Intent::PanelMove {
        window,
        panel: tab_list(),
        area: Area::Top,
        index: 0,
    };
    assert_eq!(refused(&mut workspace, to_top), IntentError::Fixed);

    let unknown = PanelId::new("nobody.panel").unwrap();
    let missing = Intent::PanelMove {
        window,
        panel: unknown.clone(),
        area: Area::Left,
        index: 0,
    };
    assert_eq!(
        refused(&mut workspace, missing),
        IntentError::NoSuchPanel {
            window,
            panel: unknown
        }
    );
}

// The scratch terminal.

#[test]
fn the_scratch_terminal_is_shown_put_away_and_shown_again() {
    let (mut workspace, window, _, a) = one_pane();
    let show = |window, content| Intent::ScratchShow { window, content };

    assert_eq!(
        refused(&mut workspace, show(window, None)),
        IntentError::ContentRequired
    );
    let scratch = must(&mut workspace, show(window, Some(terminal())))
        .1
        .pane
        .unwrap();
    assert_eq!(workspace.scratch.as_ref().unwrap().pane, scratch);
    assert_eq!(focus(&workspace, window), Some(scratch));
    let rect = workspace.window(window).unwrap().floating[0].rect;

    // Put away, it is in no window, and the keyboard goes back where it was.
    must(&mut workspace, Intent::ScratchHide {});
    assert!(workspace.window(window).unwrap().floating.is_empty());
    assert_eq!(workspace.window_of(scratch), None);
    assert!(workspace.pane(scratch).is_some());
    assert_eq!(focus(&workspace, window), Some(a));
    assert_eq!(must(&mut workspace, Intent::ScratchHide {}).0, vec![]);
    // It cannot be given the keyboard while it is put away.
    assert_eq!(
        refused(
            &mut workspace,
            Intent::Focus {
                target: FocusTarget::Pane { pane: scratch }
            }
        ),
        IntentError::NoSuchPane(scratch)
    );

    // Shown again, it is the same pane where it was. What is given to make one from
    // is not used.
    let (_, created) = must(&mut workspace, show(window, Some(terminal())));
    assert_eq!(created.pane, None);
    let floating = &workspace.window(window).unwrap().floating;
    assert_eq!((floating[0].pane.id, floating[0].rect), (scratch, rect));

    // Asked for over another window, it moves there.
    let (other, _, _) = open_window(&mut workspace);
    must(&mut workspace, show(other, None));
    assert_eq!(workspace.window_of(scratch), Some(other));
    assert!(workspace.window(window).unwrap().floating.is_empty());
}

#[test]
fn a_workspace_has_no_scratch_terminal_once_its_pane_is_closed_or_docked() {
    let show = |window| Intent::ScratchShow {
        window,
        content: Some(terminal()),
    };

    let (mut workspace, window, _, _) = one_pane();
    let scratch = must(&mut workspace, show(window)).1.pane.unwrap();
    must(&mut workspace, Intent::Close { pane: scratch });
    assert_eq!(workspace.scratch, None);

    // Closed while put away.
    let scratch = must(&mut workspace, show(window)).1.pane.unwrap();
    must(&mut workspace, Intent::ScratchHide {});
    let (changes, _) = must(&mut workspace, Intent::Close { pane: scratch });
    assert_eq!(workspace.scratch, None);
    assert!(matches!(
        changes.last(),
        Some(LayoutChange::PaneClosed { pane, .. }) if *pane == scratch
    ));

    // Put among the tabs, the pane lives on as an ordinary pane.
    let scratch = must(&mut workspace, show(window)).1.pane.unwrap();
    must(
        &mut workspace,
        Intent::Dock {
            pane: scratch,
            to: None,
        },
    );
    assert_eq!(workspace.scratch, None);
    assert!(workspace.pane(scratch).is_some());
    // The next one is a new pane.
    let next = must(&mut workspace, show(window)).1.pane.unwrap();
    assert_ne!(next, scratch);
}

// Windows.

#[test]
fn windows_are_opened_and_closed() {
    let mut workspace = empty();
    let (first, _, a) = open_window(&mut workspace);
    let bounds = Bounds {
        x: 40,
        y: 60,
        width: 1200,
        height: 800,
        maximised: false,
        fullscreen: false,
    };
    let (changes, created) = must(
        &mut workspace,
        Intent::WindowNew {
            content: terminal(),
            bounds: Some(bounds),
        },
    );
    let second = created.window.unwrap();
    assert!(matches!(changes[0], LayoutChange::WindowOpened { .. }));
    assert_eq!(workspace.window(second).unwrap().bounds, Some(bounds));
    assert_eq!(workspace.focused_window, Some(second));

    must(&mut workspace, Intent::WindowFocus { window: first });
    assert_eq!(workspace.focused_window, Some(first));

    let (changes, _) = must(&mut workspace, Intent::WindowClose { window: first });
    assert!(changes.contains(&LayoutChange::WindowClosed { window: first }));
    assert!(changes.contains(&LayoutChange::PaneClosed {
        pane: a,
        content: PaneContent::Terminal(Default::default()),
    }));
    assert_eq!(workspace.focused_window, Some(second));

    must(&mut workspace, Intent::WindowClose { window: second });
    assert!(workspace.windows.is_empty());
    assert_eq!(workspace.focused_window, None);
    assert_eq!(
        refused(&mut workspace, Intent::WindowClose { window: second }),
        IntentError::NoSuchWindow(second)
    );
}

#[test]
fn where_a_window_is_on_the_desktop_is_recorded() {
    let (mut workspace, window, _, _) = one_pane();
    let bounds = Bounds {
        x: -8,
        y: 0,
        width: 960,
        height: 640,
        maximised: true,
        fullscreen: false,
    };
    let (changes, _) = must(&mut workspace, Intent::WindowBounds { window, bounds });
    assert_eq!(changes, vec![LayoutChange::WindowBounds { window, bounds }]);
    let nowhere = WindowId::new(999);
    assert_eq!(
        refused(
            &mut workspace,
            Intent::WindowBounds {
                window: nowhere,
                bounds
            }
        ),
        IntentError::NoSuchWindow(nowhere)
    );
}

// Sessions.

#[test]
fn a_new_terminal_pane_takes_the_session_that_was_started_for_it() {
    let (mut workspace, window, _, _) = one_pane();
    let request = SpawnRequest {
        profile: Some("work".to_owned()),
        cwd: Some("/asked".to_owned()),
        env: vec![("EDITOR".to_owned(), "vi".to_owned())],
        ..SpawnRequest::default()
    };
    let intent = Intent::TabNew {
        window,
        content: NewContent::Terminal(request),
        index: None,
        activate: true,
        group: None,
    };
    let (_, created) = must_with(&mut workspace, intent, Some(started(7)));
    let terminal = terminal_of(&workspace, created.pane.unwrap());
    assert_eq!(terminal.session, SessionId::new(7));
    assert_eq!(terminal.backend, Some(BackendId("local".to_owned())));
    // The directory the session was really started in is the one kept.
    assert_eq!(terminal.cwd.as_deref(), Some("/work"));
    assert_eq!(terminal.profile.as_deref(), Some("work"));
    assert_eq!(terminal.env.len(), 1);
    assert_eq!(workspace.sessions().len(), 1);
}

#[test]
fn a_session_started_for_an_intent_that_makes_no_terminal_is_refused() {
    let (mut workspace, _, _, a) = one_pane();
    let before = workspace.clone();
    let intents = [
        Intent::Close { pane: a },
        Intent::Split {
            pane: a,
            dir: Dir::Right,
            ratio: UNITS / 2,
            content: plugin_view(),
        },
    ];
    for intent in intents {
        let error = apply(&mut workspace, intent, Some(started(1))).unwrap_err();
        assert_eq!(error, IntentError::SessionUnused);
        assert_eq!(workspace, before);
    }
}

#[test]
fn the_session_a_pane_shows_is_recorded() {
    let (mut workspace, _, _, a) = one_pane();
    let set = |pane, session: Option<u64>| Intent::SessionSet {
        pane,
        session: session.and_then(SessionId::new),
        backend: None,
        cwd: None,
    };
    must(&mut workspace, set(a, Some(5)));
    assert_eq!(terminal_of(&workspace, a).session, SessionId::new(5));

    let moved = Intent::SessionSet {
        pane: a,
        session: SessionId::new(6),
        backend: Some(BackendId("remote".to_owned())),
        cwd: Some("/srv".to_owned()),
    };
    let (changes, _) = must(&mut workspace, moved);
    assert!(matches!(changes[..], [LayoutChange::Pane { .. }]));
    let terminal = terminal_of(&workspace, a);
    assert_eq!(terminal.backend, Some(BackendId("remote".to_owned())));
    assert_eq!(terminal.cwd.as_deref(), Some("/srv"));

    // Ended: the pane has no session, and keeps what it would be started again with.
    must(&mut workspace, set(a, None));
    let terminal = terminal_of(&workspace, a);
    assert_eq!(terminal.session, None);
    assert_eq!(terminal.cwd.as_deref(), Some("/srv"));

    let view = must(
        &mut workspace,
        Intent::Split {
            pane: a,
            dir: Dir::Right,
            ratio: UNITS / 2,
            content: plugin_view(),
        },
    )
    .1
    .pane
    .unwrap();
    assert_eq!(
        refused(&mut workspace, set(view, Some(1))),
        IntentError::NotTerminal(view)
    );
}

#[test]
fn an_intent_says_whether_a_session_must_be_started_for_it() {
    let (mut workspace, window, _, a) = one_pane();
    let split_with = |content| Intent::Split {
        pane: a,
        dir: Dir::Right,
        ratio: UNITS / 2,
        content,
    };
    assert!(needs(&workspace, &split_with(terminal())).is_some());
    assert!(needs(&workspace, &split_with(plugin_view())).is_none());
    assert!(needs(&workspace, &Intent::Close { pane: a }).is_none());
    // A window that is being opened has nobody yet to say how large its pane is.
    let open = Intent::WindowNew {
        content: terminal(),
        bounds: None,
    };
    assert!(needs(&workspace, &open).is_none());
    // Unless it shows a session that runs already, which needs no size.
    let shown = Intent::WindowNew {
        content: NewContent::Terminal(SpawnRequest {
            session: Some(SessionId::new(7).unwrap()),
            ..SpawnRequest::default()
        }),
        bounds: None,
    };
    assert!(needs(&workspace, &shown).is_some());

    let show = Intent::ScratchShow {
        window,
        content: Some(terminal()),
    };
    assert!(needs(&workspace, &show).is_some());
    must(&mut workspace, show.clone());
    // The scratch terminal is there: showing it again starts nothing.
    assert!(needs(&workspace, &show).is_none());
}

#[test]
fn a_floating_pane_is_put_away_and_brought_back_where_it_was() {
    let (mut workspace, window, _, a) = one_pane();
    let b = split(&mut workspace, a, Dir::Right);
    let rect = UnitRect {
        x: 100_000,
        y: 200_000,
        w: 400_000,
        h: 300_000,
    };
    must(
        &mut workspace,
        Intent::Float {
            pane: b,
            rect: Some(rect),
        },
    );
    assert_eq!(focus(&workspace, window), Some(b));

    // Put away, it floats still, and the keyboard goes to a pane that is on show.
    let (changes, _) = must(&mut workspace, Intent::Minimise { pane: b, on: true });
    let floating = &workspace.window(window).unwrap().floating;
    assert_eq!(floating[0].minimised, Some(Minimised::Floating));
    assert_eq!(floating[0].rect, rect);
    assert_eq!(focus(&workspace, window), Some(a));
    assert!(
        changes
            .iter()
            .any(|change| matches!(change, LayoutChange::Floating { .. }))
    );
    assert!(!workspace.window(window).unwrap().focusable().contains(&b));

    // Going through the panes, or back to the one before, passes it over.
    for target in [
        FocusTarget::Next { window },
        FocusTarget::Previous { window },
        FocusTarget::Back { window },
    ] {
        must(&mut workspace, Intent::Focus { target });
        assert_eq!(focus(&workspace, window), Some(a));
    }
    // Putting it away again changes nothing.
    let (changes, _) = must(&mut workspace, Intent::Minimise { pane: b, on: true });
    assert_eq!(changes, vec![]);

    // Brought back, it is where it was, in front, with the keyboard.
    must(&mut workspace, Intent::Minimise { pane: b, on: false });
    let floating = &workspace.window(window).unwrap().floating;
    assert_eq!(floating[0].minimised, None);
    assert_eq!(floating[0].rect, rect);
    assert_eq!(focus(&workspace, window), Some(b));

    // Asking for a pane that is put away brings it back too.
    must(&mut workspace, Intent::Minimise { pane: b, on: true });
    focus_pane(&mut workspace, b);
    assert_eq!(
        workspace.window(window).unwrap().floating[0].minimised,
        None
    );
    assert_eq!(focus(&workspace, window), Some(b));
}

#[test]
fn a_pane_in_a_tab_is_put_away_by_taking_it_out_of_the_tab() {
    let (mut workspace, window, tab, a) = one_pane();
    let b = split(&mut workspace, a, Dir::Down);

    must(&mut workspace, Intent::Minimise { pane: b, on: true });
    assert_eq!(panes_of(&workspace, tab), vec![a]);
    let floating = &workspace.window(window).unwrap().floating;
    assert_eq!(floating.len(), 1);
    assert_eq!(floating[0].pane.id, b);
    assert_eq!(floating[0].minimised, Some(Minimised::Tab));
    assert_eq!(focus(&workspace, window), Some(a));

    // A pane in a tab is on show: there is nothing to bring back.
    let (changes, _) = must(&mut workspace, Intent::Minimise { pane: a, on: false });
    assert_eq!(changes, vec![]);

    // The last pane on show may be put away as well, and the window is left with
    // nothing that has the keyboard until one is brought back.
    must(&mut workspace, Intent::Minimise { pane: a, on: true });
    let shown = workspace.window(window).unwrap();
    assert!(shown.centre.tabs.is_empty());
    assert_eq!(shown.focus, None);
    assert_eq!(shown.floating.len(), 2);

    // Brought back, a pane that was taken out of a tab is among the tabs again: in
    // a tab of its own when the window has none left.
    let (_, created) = must(&mut workspace, Intent::Minimise { pane: a, on: false });
    let shown = workspace.window(window).unwrap();
    assert_eq!(shown.centre.tabs.len(), 1);
    assert_eq!(created.tab, shown.centre.active);
    assert_eq!(shown.floating.len(), 1);
    assert_eq!(focus(&workspace, window), Some(a));

    // The other goes beside the pane that has the keyboard.
    must(&mut workspace, Intent::Minimise { pane: b, on: false });
    let shown = workspace.window(window).unwrap();
    assert!(shown.floating.is_empty());
    assert_eq!(shown.centre.active_tab().unwrap().pane_ids(), vec![a, b]);
    assert_eq!(focus(&workspace, window), Some(b));
    assert_eq!(
        refused(
            &mut workspace,
            Intent::Minimise {
                pane: NOWHERE,
                on: true
            }
        ),
        IntentError::NoSuchPane(NOWHERE)
    );
}

#[test]
fn a_workspace_is_given_another_name_if_the_name_can_be_a_files() {
    let (mut workspace, _, _, _) = one_pane();
    let (changes, _) = must(
        &mut workspace,
        Intent::WorkspaceRename {
            name: "site v2".to_owned(),
        },
    );
    assert_eq!(workspace.name, "site v2");
    assert_eq!(
        changes,
        vec![LayoutChange::WorkspaceName {
            name: "site v2".to_owned()
        }]
    );

    let refused = refused(
        &mut workspace,
        Intent::WorkspaceRename {
            name: "../elsewhere".to_owned(),
        },
    );
    assert_eq!(refused, IntentError::BadName("../elsewhere".to_owned()));
    assert_eq!(workspace.name, "site v2");
    assert!(
        Intent::WorkspaceRename {
            name: String::new()
        }
        .is_core_only()
    );
}

#[test]
fn the_intents_only_the_application_applies_are_told_apart() {
    let window = WindowId::new(1);
    assert!(Intent::WindowFocus { window }.is_core_only());
    assert!(
        Intent::SessionSet {
            pane: NOWHERE,
            session: None,
            backend: None,
            cwd: None
        }
        .is_core_only()
    );
    assert!(!Intent::WindowClose { window }.is_core_only());
    assert_eq!(IntentError::CoreOnly.code(), "forbidden");
}

// How intents are written.

#[test]
fn an_intent_is_read_from_json_with_what_may_be_left_out_left_out() {
    let split: Intent = serde_json::from_str(
        r#"{"type":"split","pane":"p2","dir":"right","ratio":500000,"content":{"type":"terminal"}}"#,
    )
    .unwrap();
    assert_eq!(
        split,
        Intent::Split {
            pane: PaneId::new(2),
            dir: Dir::Right,
            ratio: 500_000,
            content: terminal(),
        }
    );

    let tab: Intent =
        serde_json::from_str(r#"{"type":"tab_new","window":"w1","content":{"type":"terminal"}}"#)
            .unwrap();
    assert_eq!(
        tab,
        Intent::TabNew {
            window: WindowId::new(1),
            content: terminal(),
            index: None,
            activate: true,
            group: None,
        }
    );

    let focus: Intent =
        serde_json::from_str(r#"{"type":"focus","target":{"type":"pane","pane":"p7"}}"#).unwrap();
    assert_eq!(
        focus,
        Intent::Focus {
            target: FocusTarget::Pane {
                pane: PaneId::new(7)
            }
        }
    );

    for wrong in [
        r#"{"type":"close","pane":"p2","force":true}"#,
        r#"{"type":"close","pane":2}"#,
        r#"{"type":"close","pane":"t2"}"#,
        r#"{"type":"explode"}"#,
        r#"{"type":"split","pane":"p2","dir":"right","ratio":500000,"content":{"type":"terminal","shell":"sh"}}"#,
    ] {
        assert!(serde_json::from_str::<Intent>(wrong).is_err(), "{wrong}");
    }
}
