//! Recorded sequences of intents with what each one changed.
//!
//! `tests/layout/vectors/` at the root of the repository holds, for each sequence, the
//! workspace it starts from, every intent with the changes it reported or the reason
//! it was refused, and the workspace it ends with. The files pin how layouts, intents
//! and changes are written as JSON, and anything else that keeps a copy of a layout
//! from changes is tested against the same files.
//!
//! Run with `UPDATE_GOLDEN=1` to rewrite them after a deliberate change.

mod common;

use std::fs;
use std::path::PathBuf;

use common::*;
use lume_layout::{
    Area, Bounds, Dir, FocusTarget, GroupIcon, GroupId, Intent, PaneId, PanelContent, PanelId,
    Place, Projection, SplitId, TabColor, TabId, TabTarget, UNITS, UnitRect, WindowId, Workspace,
    apply, needs,
};
use serde_json::{Value, json};

/// Applies the intents in order and describes what happened as JSON.
fn record(name: &str, intents: Vec<Intent>) -> Value {
    let mut workspace = empty();
    let start = workspace.clone();
    let mut steps = Vec::new();
    for (turn, intent) in intents.into_iter().enumerate() {
        let before = workspace.clone();
        // An intent that needs a session is given one, as an authority would.
        let session = needs(&workspace, &intent).map(|_| started(turn as u64 + 1));
        let step = match apply(&mut workspace, intent.clone(), session) {
            Ok((changes, created)) => {
                check_applied(&before, &workspace, &changes);
                json!({ "intent": intent, "changes": changes, "created": created })
            }
            Err(error) => {
                assert_eq!(workspace, before);
                json!({ "intent": intent, "refused": error.code() })
            }
        };
        steps.push(step);
    }
    json!({
        "name": name,
        "start": start,
        "steps": steps,
        "end": workspace,
    })
}

/// Compares a recorded sequence with its file, or writes the file when asked to.
fn golden(name: &str, intents: Vec<Intent>) {
    let recorded = record(name, intents);
    let mut text = serde_json::to_string_pretty(&recorded).unwrap();
    text.push('\n');

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("layout")
        .join("vectors");
    let path = dir.join(format!("{name}.json"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, &text).unwrap();
    }

    let hint = "run with UPDATE_GOLDEN=1 if the layout's JSON changed on purpose";
    let kept =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}; {hint}", path.display()));
    assert_eq!(kept.replace("\r\n", "\n"), text, "{name}.json; {hint}");

    // What is in the file is enough to follow the layout: a copy of the start that is
    // given each step's changes ends as the file says.
    let kept: Value = serde_json::from_str(&kept).unwrap();
    let start: Workspace = serde_json::from_value(kept["start"].clone()).unwrap();
    let end: Workspace = serde_json::from_value(kept["end"].clone()).unwrap();
    let mut follower = Projection::of(&start);
    for step in kept["steps"].as_array().unwrap() {
        let Some(changes) = step.get("changes") else {
            continue;
        };
        let changes: Vec<lume_layout::LayoutChange> =
            serde_json::from_value(changes.clone()).unwrap();
        for change in &changes {
            follower.apply(change);
        }
    }
    assert_eq!(follower, Projection::of(&end), "{name}.json");
}

// The ids below are the ones the sequences make: numbers are given out in order, so a
// sequence always makes the same ids.

fn pane(number: u32) -> PaneId {
    PaneId::new(number)
}

fn window(number: u32) -> WindowId {
    WindowId::new(number)
}

fn open() -> Intent {
    Intent::WindowNew {
        content: terminal(),
        bounds: None,
    }
}

fn split_of(pane: PaneId, dir: Dir, ratio: u32) -> Intent {
    Intent::Split {
        pane,
        dir,
        ratio,
        content: terminal(),
    }
}

fn focus_on(pane: PaneId) -> Intent {
    Intent::Focus {
        target: FocusTarget::Pane { pane },
    }
}

#[test]
fn splits_and_tabs() {
    // The window is w1, its first pane p2 in tab t3.
    let w = window(1);
    golden(
        "splits-and-tabs",
        vec![
            open(),
            split_of(pane(2), Dir::Right, UNITS / 2),
            // p4 beside p2 in split s5; then p6 below p4 in split s7.
            split_of(pane(4), Dir::Down, 300_000),
            Intent::Resize {
                split: SplitId::new(5),
                ratios: vec![350_000, 650_000],
            },
            Intent::FocusDir {
                window: w,
                dir: Dir::Left,
            },
            Intent::Zoom {
                pane: pane(2),
                on: true,
            },
            // The keyboard goes to a pane the zoom hides, which ends the zoom.
            focus_on(pane(6)),
            Intent::Swap {
                a: pane(2),
                b: pane(6),
            },
            Intent::Move {
                pane: pane(4),
                to: Place::Beside {
                    pane: pane(2),
                    edge: Dir::Left,
                },
            },
            Intent::PaneRename {
                pane: pane(4),
                title: Some("logs".to_owned()),
            },
            Intent::PaneIcon {
                pane: pane(4),
                icon: Some("agent".to_owned()),
            },
            Intent::TabNew {
                window: w,
                content: plugin_view(),
                index: Some(0),
                activate: false,
                group: None,
            },
            Intent::TabRename {
                tab: TabId::new(10),
                title: Some("notes".to_owned()),
            },
            Intent::TabActivate {
                target: TabTarget::Next { window: w },
            },
            Intent::TabMove {
                tab: TabId::new(10),
                index: 1,
            },
            Intent::Close { pane: pane(6) },
            // Refused: a pane that is gone, and a share no pane may have.
            Intent::Close { pane: pane(6) },
            split_of(pane(2), Dir::Up, 10),
            Intent::TabClose {
                tab: TabId::new(10),
            },
            Intent::Close { pane: pane(4) },
            Intent::Close { pane: pane(2) },
        ],
    );
}

#[test]
fn floating_and_scratch() {
    let w = window(1);
    let show = Intent::ScratchShow {
        window: w,
        content: Some(terminal()),
    };
    golden(
        "floating-and-scratch",
        vec![
            open(),
            split_of(pane(2), Dir::Right, UNITS / 2),
            Intent::Float {
                pane: pane(4),
                rect: None,
            },
            Intent::FloatMove {
                pane: pane(4),
                rect: UnitRect {
                    x: 850_000,
                    y: 100_000,
                    w: 300_000,
                    h: 50_000,
                },
            },
            // The scratch terminal is pane p6.
            show.clone(),
            focus_on(pane(4)),
            Intent::ScratchHide {},
            Intent::ScratchHide {},
            show.clone(),
            Intent::SessionSet {
                pane: pane(6),
                session: None,
                backend: None,
                cwd: None,
            },
            Intent::Dock {
                pane: pane(4),
                to: Some(Place::Beside {
                    pane: pane(2),
                    edge: Dir::Down,
                }),
            },
            // Refused: it floats no longer.
            Intent::Dock {
                pane: pane(4),
                to: None,
            },
            Intent::Move {
                pane: pane(6),
                to: Place::NewTab {
                    window: w,
                    index: None,
                },
            },
            show,
            // The new scratch terminal is pane p9.
            Intent::Close { pane: pane(9) },
        ],
    );
}

#[test]
fn panels_and_windows() {
    let w = window(1);
    let tab_list = PanelId::new("lume.tabs").unwrap();
    let bounds = Bounds {
        x: 120,
        y: 80,
        width: 1280,
        height: 800,
        maximised: false,
        fullscreen: false,
    };
    golden(
        "panels-and-windows",
        vec![
            open(),
            Intent::PanelSet {
                window: w,
                panel: tab_list.clone(),
                content: PanelContent::TabList {},
                area: Some(Area::Left),
                index: 0,
            },
            Intent::PanelResize {
                window: w,
                area: Area::Left,
                size: 320,
            },
            Intent::PanelMove {
                window: w,
                panel: tab_list.clone(),
                area: Area::Right,
                index: 0,
            },
            Intent::PanelToggle {
                window: w,
                area: Area::Right,
                on: None,
            },
            // Refused: the top bar stays as it is.
            Intent::PanelMove {
                window: w,
                panel: tab_list.clone(),
                area: Area::Top,
                index: 0,
            },
            Intent::WindowBounds { window: w, bounds },
            // A second window, w4, with pane p5 in tab t6.
            Intent::WindowNew {
                content: terminal(),
                bounds: Some(Bounds {
                    x: 400,
                    maximised: true,
                    ..bounds
                }),
            },
            Intent::WindowFocus { window: w },
            // Refused: a pane stays in its window.
            Intent::Swap {
                a: pane(2),
                b: pane(5),
            },
            Intent::PanelSet {
                window: w,
                panel: tab_list,
                content: PanelContent::TabList {},
                area: None,
                index: 0,
            },
            Intent::WindowClose { window: w },
            Intent::WindowClose { window: window(4) },
        ],
    );
}

#[test]
fn tab_groups() {
    let w = window(1);
    let tab = TabId::new;
    let group = GroupId::new(10);
    let set = |name: Option<&str>, collapsed: Option<bool>| Intent::GroupSet {
        group,
        name: name.map(|name| Some(name.to_owned())),
        color: None,
        icon: name.map(|_| Some(GroupIcon::Bug)),
        collapsed,
        kept: name.map(|_| true),
        muted: None,
    };
    let new_tab = || Intent::TabNew {
        window: w,
        content: terminal(),
        index: None,
        activate: true,
        group: None,
    };
    golden(
        "tab-groups",
        vec![
            // Tabs t3, t5, t7 and t9, and then group g10.
            open(),
            new_tab(),
            new_tab(),
            new_tab(),
            Intent::TabColor {
                tab: tab(3),
                color: Some(TabColor::Teal),
            },
            Intent::GroupNew { tab: tab(5) },
            Intent::TabPlace {
                tab: tab(7),
                index: 2,
                group: Some(group),
            },
            set(Some("Billing"), None),
            Intent::TabLock {
                tab: tab(9),
                on: true,
            },
            // Refused: it is locked.
            Intent::TabClose { tab: tab(9) },
            Intent::TabActivate {
                target: TabTarget::Id { tab: tab(5) },
            },
            // Folded, it gives way to the tab after it.
            set(None, Some(true)),
            Intent::TabMove {
                tab: tab(3),
                index: 2,
            },
            Intent::GroupMove { group, index: 2 },
            Intent::TabMerge {
                tab: tab(3),
                into: tab(9),
            },
            // Kept, it is put away and comes back.
            Intent::GroupClose { group },
            Intent::GroupReopen { group, window: w },
            Intent::GroupUngroup { group },
            Intent::TabsClose {
                tabs: vec![tab(5), tab(7), tab(9)],
            },
            Intent::TabLock {
                tab: tab(9),
                on: false,
            },
        ],
    );
}

#[test]
fn plugin_views() {
    // The window is w1 with pane p2 in tab t3; the view's pane is p4 in tab t5.
    let w = window(1);
    let panel = PanelId::new("acme.tools.changes").unwrap();
    golden(
        "plugin-views",
        vec![
            open(),
            Intent::TabNew {
                window: w,
                content: plugin_view(),
                index: None,
                activate: true,
                group: None,
            },
            Intent::ViewState {
                pane: pane(4),
                state: json!({ "file": "src/main.rs", "line": 12 }),
            },
            // What a view keeps goes with its pane wherever the pane goes.
            Intent::Float {
                pane: pane(4),
                rect: None,
            },
            Intent::Minimise {
                pane: pane(4),
                on: true,
            },
            Intent::Minimise {
                pane: pane(4),
                on: false,
            },
            Intent::Dock {
                pane: pane(4),
                to: None,
            },
            // A terminal keeps nothing for a view: nothing changes.
            Intent::ViewState {
                pane: pane(2),
                state: json!({ "x": 1 }),
            },
            // Refused: there is no such pane.
            Intent::ViewState {
                pane: pane(99),
                state: json!(null),
            },
            Intent::PanelSet {
                window: w,
                panel: panel.clone(),
                content: PanelContent::PluginView {
                    plugin: "acme.tools".to_owned(),
                    view: "changes".to_owned(),
                    state: Value::Null,
                },
                area: Some(Area::Right),
                index: 0,
            },
            Intent::PanelState {
                window: w,
                panel: panel.clone(),
                state: json!({ "tab": "history" }),
            },
            // A panel that is moved keeps what its view kept.
            Intent::PanelMove {
                window: w,
                panel,
                area: Area::Left,
                index: 0,
            },
        ],
    );
}
