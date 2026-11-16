//! Whatever intents are applied in whatever order, the layout stays sound.
//!
//! Sequences of intents are made at random. Most of them name things that are there,
//! chosen by position so that they still mean something as the layout changes; some
//! name things that are not, or ask for what is refused. After every intent the checks
//! in `common` must hold: the workspace is sound and in its proper form, a follower
//! that was given the changes has the same workspace, every pane that went is reported
//! gone once, and numbers are never used twice. A refused intent changes nothing.

mod common;

use common::*;
use lume_layout::{
    Area, Bounds, Created, Dir, FocusTarget, Intent, IntentError, LayoutChange, Node, PaneId,
    PanelContent, PanelId, Place, SplitId, TabId, TabTarget, UNITS, UnitRect, WindowId, Workspace,
    apply, needs, tab_rects,
};
use lume_proto::SessionId;
use proptest::prelude::*;

/// An intent before it is known what is in the workspace: what it names is chosen by
/// position when its turn comes.
#[derive(Clone, Debug)]
enum Op {
    Split {
        pane: usize,
        dir: usize,
        ratio: u32,
        view: bool,
    },
    Close {
        pane: usize,
    },
    Focus {
        pane: usize,
    },
    FocusStep {
        window: usize,
        how: usize,
    },
    FocusDir {
        window: usize,
        dir: usize,
    },
    MoveBeside {
        pane: usize,
        target: usize,
        edge: usize,
    },
    MoveToTab {
        pane: usize,
        index: Option<u32>,
    },
    Swap {
        a: usize,
        b: usize,
    },
    Resize {
        split: usize,
        weights: Vec<u32>,
        exact: bool,
    },
    Zoom {
        pane: usize,
        on: bool,
    },
    Float {
        pane: usize,
        rect: Option<[u32; 4]>,
    },
    FloatMove {
        pane: usize,
        rect: [u32; 4],
    },
    Dock {
        pane: usize,
        beside: Option<usize>,
    },
    PaneRename {
        pane: usize,
        named: bool,
    },
    TabNew {
        window: usize,
        index: Option<u32>,
        activate: bool,
    },
    TabClose {
        tab: usize,
    },
    TabMove {
        tab: usize,
        index: u32,
    },
    TabRename {
        tab: usize,
        named: bool,
    },
    TabActivate {
        window: usize,
        how: usize,
        index: u32,
    },
    PanelSet {
        window: usize,
        area: Option<usize>,
        index: u32,
    },
    PanelMove {
        window: usize,
        area: usize,
        index: u32,
    },
    PanelToggle {
        window: usize,
        area: usize,
        on: Option<bool>,
    },
    PanelResize {
        window: usize,
        area: usize,
        size: u16,
    },
    ScratchShow {
        window: usize,
    },
    ScratchHide,
    WindowNew,
    WindowClose {
        window: usize,
    },
    SessionSet {
        pane: usize,
        session: Option<u64>,
    },
    WindowBounds {
        window: usize,
        x: i32,
    },
    WindowFocus {
        window: usize,
    },
    WorkspaceRename {
        name: usize,
    },
    Minimise {
        pane: usize,
        on: bool,
    },
}

fn ops() -> impl Strategy<Value = Vec<Op>> {
    let position = 0usize..9;
    let rect = [
        0u32..1_200_000,
        0u32..1_200_000,
        0u32..900_000,
        0u32..900_000,
    ];
    let op = prop_oneof![
        12 => (position.clone(), 0usize..4, 0u32..UNITS, any::<bool>())
            .prop_map(|(pane, dir, ratio, view)| Op::Split { pane, dir, ratio, view }),
        6 => position.clone().prop_map(|pane| Op::Close { pane }),
        6 => position.clone().prop_map(|pane| Op::Focus { pane }),
        3 => (position.clone(), 0usize..3)
            .prop_map(|(window, how)| Op::FocusStep { window, how }),
        6 => (position.clone(), 0usize..4)
            .prop_map(|(window, dir)| Op::FocusDir { window, dir }),
        5 => (position.clone(), position.clone(), 0usize..4)
            .prop_map(|(pane, target, edge)| Op::MoveBeside { pane, target, edge }),
        2 => (position.clone(), proptest::option::of(0u32..4))
            .prop_map(|(pane, index)| Op::MoveToTab { pane, index }),
        4 => (position.clone(), position.clone()).prop_map(|(a, b)| Op::Swap { a, b }),
        6 => (
            position.clone(),
            proptest::collection::vec(0u32..12, 1..6),
            any::<bool>()
        )
            .prop_map(|(split, weights, exact)| Op::Resize { split, weights, exact }),
        3 => (position.clone(), any::<bool>()).prop_map(|(pane, on)| Op::Zoom { pane, on }),
        4 => (position.clone(), proptest::option::of(rect.clone()))
            .prop_map(|(pane, rect)| Op::Float { pane, rect }),
        3 => (position.clone(), rect).prop_map(|(pane, rect)| Op::FloatMove { pane, rect }),
        4 => (position.clone(), proptest::option::of(position.clone()))
            .prop_map(|(pane, beside)| Op::Dock { pane, beside }),
        1 => (position.clone(), any::<bool>())
            .prop_map(|(pane, named)| Op::PaneRename { pane, named }),
        5 => (
            position.clone(),
            proptest::option::of(0u32..4),
            any::<bool>()
        )
            .prop_map(|(window, index, activate)| Op::TabNew { window, index, activate }),
        2 => position.clone().prop_map(|tab| Op::TabClose { tab }),
        4 => (position.clone(), 0u32..4).prop_map(|(tab, index)| Op::TabMove { tab, index }),
        1 => (position.clone(), any::<bool>())
            .prop_map(|(tab, named)| Op::TabRename { tab, named }),
        4 => (position.clone(), 0usize..4, 0u32..4)
            .prop_map(|(window, how, index)| Op::TabActivate { window, how, index }),
        4 => (position.clone(), proptest::option::of(0usize..4), 0u32..3)
            .prop_map(|(window, area, index)| Op::PanelSet { window, area, index }),
        5 => (position.clone(), 0usize..4, 0u32..2)
            .prop_map(|(window, area, index)| Op::PanelMove { window, area, index }),
        1 => (
            position.clone(),
            0usize..4,
            proptest::option::of(any::<bool>())
        )
            .prop_map(|(window, area, on)| Op::PanelToggle { window, area, on }),
        1 => (position.clone(), 0usize..4, any::<u16>())
            .prop_map(|(window, area, size)| Op::PanelResize { window, area, size }),
        3 => position.clone().prop_map(|window| Op::ScratchShow { window }),
        2 => Just(Op::ScratchHide),
        2 => Just(Op::WindowNew),
        1 => position.clone().prop_map(|window| Op::WindowClose { window }),
        3 => (position.clone(), proptest::option::of(1u64..50))
            .prop_map(|(pane, session)| Op::SessionSet { pane, session }),
        1 => (position.clone(), -100i32..100)
            .prop_map(|(window, x)| Op::WindowBounds { window, x }),
        1 => position.clone().prop_map(|window| Op::WindowFocus { window }),
        1 => position.clone().prop_map(|name| Op::WorkspaceRename { name }),
        4 => (position, any::<bool>()).prop_map(|(pane, on)| Op::Minimise { pane, on }),
    ];
    proptest::collection::vec(op, 1..70)
}

/// The item at a position, counted round and round. One position in each round names
/// nothing that is there, so that intents about what is missing are tried too.
fn pick<T: Copy>(items: &[T], position: usize, missing: T) -> T {
    let round = items.len() + 1;
    items.get(position % round).copied().unwrap_or(missing)
}

const NO_PANE: PaneId = PaneId::new(900_001);
const NO_TAB: TabId = TabId::new(900_002);
const NO_SPLIT: SplitId = SplitId::new(900_003);
const NO_WINDOW: WindowId = WindowId::new(900_004);

fn panes(workspace: &Workspace) -> Vec<PaneId> {
    workspace.panes().into_iter().map(|pane| pane.id).collect()
}

fn floating(workspace: &Workspace) -> Vec<PaneId> {
    workspace
        .windows
        .iter()
        .flat_map(|window| &window.floating)
        .map(|floating| floating.pane.id)
        .collect()
}

fn windows(workspace: &Workspace) -> Vec<WindowId> {
    workspace.windows.iter().map(|window| window.id).collect()
}

fn tabs(workspace: &Workspace) -> Vec<TabId> {
    workspace
        .windows
        .iter()
        .flat_map(|window| &window.centre.tabs)
        .map(|tab| tab.id)
        .collect()
}

fn splits(workspace: &Workspace) -> Vec<(SplitId, usize)> {
    workspace
        .windows
        .iter()
        .flat_map(|window| &window.centre.tabs)
        .flat_map(|tab| tab.root.splits())
        .map(|split| (split.id, split.children.len()))
        .collect()
}

fn rect_of(parts: [u32; 4]) -> UnitRect {
    UnitRect {
        x: parts[0],
        y: parts[1],
        w: parts[2],
        h: parts[3],
    }
}

fn area_of(position: usize) -> Area {
    Area::ALL[position % Area::ALL.len()]
}

fn dir_of(position: usize) -> Dir {
    Dir::ALL[position % Dir::ALL.len()]
}

fn side_panel() -> PanelId {
    PanelId::new("lume.tabs").unwrap()
}

/// The intent an operation stands for in a workspace as it is now.
fn resolve(workspace: &Workspace, op: &Op) -> Intent {
    let pane = |position: &usize| pick(&panes(workspace), *position, NO_PANE);
    let window = |position: &usize| pick(&windows(workspace), *position, NO_WINDOW);
    let tab = |position: &usize| pick(&tabs(workspace), *position, NO_TAB);
    let name = |named: &bool| named.then(|| "a name".to_owned());
    match op {
        Op::Split {
            pane: position,
            dir,
            ratio,
            view,
        } => Intent::Split {
            pane: pane(position),
            dir: dir_of(*dir),
            ratio: *ratio,
            content: if *view { plugin_view() } else { terminal() },
        },
        Op::Close { pane: position } => Intent::Close {
            pane: pane(position),
        },
        Op::Focus { pane: position } => Intent::Focus {
            target: FocusTarget::Pane {
                pane: pane(position),
            },
        },
        Op::FocusStep {
            window: position,
            how,
        } => {
            let window = window(position);
            let target = match how {
                0 => FocusTarget::Next { window },
                1 => FocusTarget::Previous { window },
                _ => FocusTarget::Back { window },
            };
            Intent::Focus { target }
        }
        Op::FocusDir {
            window: position,
            dir,
        } => Intent::FocusDir {
            window: window(position),
            dir: dir_of(*dir),
        },
        Op::MoveBeside {
            pane: position,
            target,
            edge,
        } => Intent::Move {
            pane: pane(position),
            to: Place::Beside {
                pane: pane(target),
                edge: dir_of(*edge),
            },
        },
        Op::MoveToTab {
            pane: position,
            index,
        } => {
            let pane = pane(position);
            let window = workspace.window_of(pane).unwrap_or(NO_WINDOW);
            Intent::Move {
                pane,
                to: Place::NewTab {
                    window,
                    index: *index,
                },
            }
        }
        Op::Swap { a, b } => Intent::Swap {
            a: pane(a),
            b: pane(b),
        },
        Op::Resize {
            split: position,
            weights,
            exact,
        } => {
            let (split, children) = pick(&splits(workspace), *position, (NO_SPLIT, 2));
            // Ratios for the split's children in the proportions of the weights. When
            // they are not asked to be exact, the weights are passed as they are,
            // which is seldom what a split takes.
            let mut ratios: Vec<u32> = (0..children)
                .map(|child| weights[child % weights.len()])
                .collect();
            if *exact {
                let total: u32 = ratios.iter().sum::<u32>().max(1);
                ratios = ratios
                    .iter()
                    .map(|weight| weight * (UNITS / total))
                    .collect();
                let short = UNITS - ratios.iter().sum::<u32>().min(UNITS);
                ratios[0] += short;
            }
            Intent::Resize { split, ratios }
        }
        Op::Zoom { pane: position, on } => Intent::Zoom {
            pane: pane(position),
            on: *on,
        },
        Op::Float {
            pane: position,
            rect,
        } => Intent::Float {
            pane: pane(position),
            rect: rect.map(rect_of),
        },
        Op::FloatMove {
            pane: position,
            rect,
        } => Intent::FloatMove {
            // Mostly a pane that floats, when there is one.
            pane: pick(&floating(workspace), *position, pane(position)),
            rect: rect_of(*rect),
        },
        Op::Dock {
            pane: position,
            beside,
        } => Intent::Dock {
            pane: pick(&floating(workspace), *position, pane(position)),
            to: beside.as_ref().map(|target| Place::Beside {
                pane: pane(target),
                edge: Dir::Down,
            }),
        },
        Op::PaneRename {
            pane: position,
            named,
        } => Intent::PaneRename {
            pane: pane(position),
            title: name(named),
        },
        Op::TabNew {
            window: position,
            index,
            activate,
        } => Intent::TabNew {
            window: window(position),
            content: terminal(),
            index: *index,
            activate: *activate,
        },
        Op::TabClose { tab: position } => Intent::TabClose { tab: tab(position) },
        Op::TabMove {
            tab: position,
            index,
        } => Intent::TabMove {
            tab: tab(position),
            index: *index,
        },
        Op::TabRename {
            tab: position,
            named,
        } => Intent::TabRename {
            tab: tab(position),
            title: name(named),
        },
        Op::TabActivate {
            window: position,
            how,
            index,
        } => {
            let window = window(position);
            let target = match how {
                0 => TabTarget::Next { window },
                1 => TabTarget::Previous { window },
                2 => TabTarget::Index {
                    window,
                    index: *index,
                },
                _ => TabTarget::Id {
                    tab: tab(&(*index as usize)),
                },
            };
            Intent::TabActivate { target }
        }
        Op::PanelSet {
            window: position,
            area,
            index,
        } => Intent::PanelSet {
            window: window(position),
            panel: side_panel(),
            content: PanelContent::TabList {},
            area: area.map(area_of),
            index: *index,
        },
        Op::PanelMove {
            window: position,
            area,
            index,
        } => Intent::PanelMove {
            window: window(position),
            panel: side_panel(),
            area: area_of(*area),
            index: *index,
        },
        Op::PanelToggle {
            window: position,
            area,
            on,
        } => Intent::PanelToggle {
            window: window(position),
            area: area_of(*area),
            on: *on,
        },
        Op::PanelResize {
            window: position,
            area,
            size,
        } => Intent::PanelResize {
            window: window(position),
            area: area_of(*area),
            size: *size,
        },
        Op::ScratchShow { window: position } => Intent::ScratchShow {
            window: window(position),
            content: Some(terminal()),
        },
        Op::ScratchHide => Intent::ScratchHide {},
        Op::WindowNew => Intent::WindowNew {
            content: terminal(),
            bounds: None,
        },
        Op::WindowClose { window: position } => Intent::WindowClose {
            window: window(position),
        },
        Op::SessionSet {
            pane: position,
            session,
        } => Intent::SessionSet {
            pane: pane(position),
            session: session.and_then(SessionId::new),
            backend: None,
            cwd: None,
        },
        Op::WindowBounds {
            window: position,
            x,
        } => Intent::WindowBounds {
            window: window(position),
            bounds: Bounds {
                x: *x,
                y: 0,
                width: 800,
                height: 600,
                maximised: false,
                fullscreen: false,
            },
        },
        Op::WindowFocus { window: position } => Intent::WindowFocus {
            window: window(position),
        },
        Op::Minimise { pane: position, on } => Intent::Minimise {
            pane: pane(position),
            on: *on,
        },
        Op::WorkspaceRename { name: position } => Intent::WorkspaceRename {
            // A device's name and a path are no names a workspace may have.
            name: pick(
                &["work", "notes 2", "site.v2", "a-b_c", "con"],
                *position,
                "bad/name",
            )
            .to_owned(),
        },
    }
}

/// What applying one intent gave.
type Outcome = Result<(Vec<LayoutChange>, Created), IntentError>;

/// Applies a sequence of operations to a workspace that starts with one pane, with
/// every check after each, and returns the workspace and what each intent gave.
fn run(ops: &[Op]) -> (Workspace, Vec<(Intent, Outcome)>) {
    let (mut workspace, ..) = one_pane();
    let mut outcomes = Vec::new();
    for (turn, op) in ops.iter().enumerate() {
        let intent = resolve(&workspace, op);
        let before = workspace.clone();
        // An intent that needs a session is given one, as an authority would.
        let session = needs(&workspace, &intent).map(|_| started(turn as u64 + 1));
        let outcome = apply(&mut workspace, intent.clone(), session);
        match &outcome {
            Ok((changes, _)) => {
                check_applied(&before, &workspace, changes);
                if let Intent::FocusDir { window, dir } = &intent {
                    check_direction(&before, &workspace, *window, *dir);
                }
            }
            Err(_) => assert_eq!(
                workspace, before,
                "{intent:?} was refused and changed things"
            ),
        }
        outcomes.push((intent, outcome));
    }
    (workspace, outcomes)
}

/// The pane the keyboard moved to lies in the direction that was asked for.
fn check_direction(before: &Workspace, after: &Workspace, window: WindowId, dir: Dir) {
    let was = before.window(window).unwrap();
    let (Some(from), Some(to)) = (was.focus, after.window(window).unwrap().focus) else {
        return;
    };
    if from == to {
        return;
    }
    let rects: Vec<(PaneId, UnitRect)> = if was.floating.iter().any(|f| f.pane.id == from) {
        was.floating.iter().map(|f| (f.pane.id, f.rect)).collect()
    } else {
        tab_rects(&was.centre.active_tab().unwrap().root)
    };
    let rect = |pane| rects.iter().find(|(id, _)| *id == pane).unwrap().1;
    let (origin, target) = (rect(from), rect(to));
    let beyond = match dir {
        Dir::Left => target.right() <= origin.x,
        Dir::Right => target.x >= origin.right(),
        Dir::Up => target.bottom() <= origin.y,
        Dir::Down => target.y >= origin.bottom(),
    };
    assert!(beyond, "{to} is not {dir:?} of {from}");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn any_sequence_of_intents_leaves_a_sound_layout_its_followers_agree_with(ops in ops()) {
        run(&ops);
    }

    #[test]
    fn the_same_intents_always_give_the_same_layout(ops in ops()) {
        let (first, first_outcomes) = run(&ops);
        let (second, second_outcomes) = run(&ops);
        prop_assert_eq!(first, second);
        prop_assert_eq!(first_outcomes, second_outcomes);
    }

    #[test]
    fn a_layout_and_what_changes_it_survive_being_written_as_json(ops in ops()) {
        let (workspace, outcomes) = run(&ops);
        let text = serde_json::to_string(&workspace).unwrap();
        prop_assert_eq!(serde_json::from_str::<Workspace>(&text).unwrap(), workspace);
        for (intent, outcome) in outcomes {
            let text = serde_json::to_string(&intent).unwrap();
            prop_assert_eq!(serde_json::from_str::<Intent>(&text).unwrap(), intent);
            if let Ok((changes, created)) = outcome {
                let text = serde_json::to_string(&changes).unwrap();
                prop_assert_eq!(
                    serde_json::from_str::<Vec<LayoutChange>>(&text).unwrap(),
                    changes
                );
                let text = serde_json::to_string(&created).unwrap();
                prop_assert_eq!(serde_json::from_str::<Created>(&text).unwrap(), created);
            }
        }
    }
}

/// The sequences above reach the parts of the layout they are meant to: this is a
/// check on the test, not on the layout. A generator that only ever made refused
/// intents would pass everything.
#[test]
fn the_sequences_exercise_every_kind_of_intent() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;

    let mut runner = TestRunner::deterministic();
    let mut accepted = std::collections::HashMap::<String, usize>::new();
    let mut deepest = 0;
    for _ in 0..300 {
        let ops = ops().new_tree(&mut runner).unwrap().current();
        let (workspace, outcomes) = run(&ops);
        for (intent, outcome) in outcomes {
            if outcome.is_ok_and(|(changes, _)| !changes.is_empty()) {
                let kind = serde_json::to_value(&intent).unwrap()["type"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                *accepted.entry(kind).or_default() += 1;
            }
        }
        for window in &workspace.windows {
            for tab in &window.centre.tabs {
                deepest = deepest.max(depth(&tab.root));
            }
        }
    }
    for kind in [
        "split",
        "close",
        "focus",
        "focus_dir",
        "move",
        "swap",
        "resize",
        "zoom",
        "float",
        "float_move",
        "dock",
        "pane_rename",
        "tab_new",
        "tab_close",
        "tab_move",
        "tab_rename",
        "tab_activate",
        "panel_move",
        "panel_toggle",
        "panel_resize",
        "scratch_show",
        "scratch_hide",
        "window_new",
        "window_close",
        "session_set",
        "panel_set",
        "window_bounds",
        "window_focus",
        "workspace_rename",
        "minimise",
    ] {
        assert!(
            accepted.get(kind).copied().unwrap_or(0) >= 5,
            "`{kind}` changed the layout {} times in 300 sequences",
            accepted.get(kind).copied().unwrap_or(0)
        );
    }
    assert!(deepest >= 3, "no tree nested deeper than {deepest}");
}

fn depth(node: &Node) -> usize {
    node.depth()
}
