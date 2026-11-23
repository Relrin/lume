//! A server's view as a workspace: what each part of a view becomes, that the numbers
//! given to its parts stay, and that no view yields a workspace the application could
//! not have made itself.

mod common;

use std::collections::HashMap;

use common::Recorded;
use lume_layout::tuning::{
    FLOATING_MAX, SHARE_MIN, SPLIT_CHILDREN_MAX, TAB_PANES_MAX, TABS_MAX, TREE_DEPTH_MAX,
};
use lume_layout::{
    Axis, Minimised, Node, PaneContent, PaneId, UNITS, Window, WindowId, Workspace, WorkspaceId,
    is_normalised, mirror, validate,
};
use lume_proto::{BackendId, SessionId};
use lume_rex::layout::{Joint, LayoutIds, Shown, WINDOW, translate};
use lume_rex::tuning::SPLIT_DEPTH_MAX;
use lume_rex::{RexError, View};
use serde_json::{Value, json};

const TERMINAL: &str = "com.superlogical.terminal";

fn backend() -> BackendId {
    BackendId("rex".to_owned())
}

/// Numbers for sessions, one for each block, as a backend gives them.
#[derive(Default)]
struct Sessions(HashMap<String, SessionId>);

impl Sessions {
    fn of(&mut self, block: &str) -> SessionId {
        let next = self.0.len() as u64 + 1;
        *self
            .0
            .entry(block.to_owned())
            .or_insert_with(|| SessionId::new(next).unwrap())
    }
}

/// Everything a view is turned into a workspace with.
#[derive(Default)]
struct Shower {
    ids: LayoutIds,
    sessions: Sessions,
}

impl Shower {
    fn show(&mut self, view: &Value) -> Result<Shown, RexError> {
        let view: View = serde_json::from_value(view.clone()).unwrap();
        let sessions = &mut self.sessions;
        translate(&view, &mut self.ids, &backend(), &mut |block| {
            sessions.of(block)
        })
    }

    /// A view that is shown, as a workspace that is sound and in its proper form.
    fn sound(&mut self, view: &Value) -> Shown {
        let shown = self.show(view).unwrap();
        if let Err(wrong) = validate(&shown.workspace) {
            panic!("the workspace is not sound: {wrong:?}");
        }
        assert!(is_normalised(&shown.workspace));
        shown
    }
}

fn sound(view: &Value) -> Shown {
    Shower::default().sound(view)
}

fn refused(view: &Value) -> String {
    let mut shower = Shower::default();
    let before = shower.ids.next();
    let error = shower.show(view).unwrap_err();
    assert_eq!(shower.ids.next(), before, "a refused view kept numbers");
    assert!(matches!(error, RexError::Protocol(_)), "{error:?}");
    error.to_string()
}

fn leaf(block: &str) -> Value {
    json!({"rect": [0, 0, 1, 1], "block_id": block})
}

fn leaf_at(block: &str, rect: [f64; 4]) -> Value {
    json!({"rect": rect, "block_id": block})
}

fn split(id: &str, direction: &str, ratio: f64, before: Value, after: Value) -> Value {
    json!({"rect": [0, 0, 1, 1], "split": {
        "split_id": id, "direction": direction, "ratio": ratio, "before": before, "after": after,
    }})
}

fn across(id: &str, ratio: f64, before: Value, after: Value) -> Value {
    split(id, "horizontal", ratio, before, after)
}

fn down(id: &str, ratio: f64, before: Value, after: Value) -> Value {
    split(id, "vertical", ratio, before, after)
}

/// The blocks a layout names, in the order they are laid out.
fn blocks_in(node: &Value, found: &mut Vec<String>) {
    if let Some(block) = node["block_id"].as_str() {
        found.push(block.to_owned());
    }
    if node["split"].is_object() {
        blocks_in(&node["split"]["before"], found);
        blocks_in(&node["split"]["after"], found);
    }
}

fn terminal(block: &str) -> Value {
    json!({
        "block_id": block, "label": "shell", "creator_name": TERMINAL,
        "flavor": "com.superlogical.terminal.shell",
    })
}

/// A layer whose blocks are all terminals.
fn layer(id: &str, kind: &str, layout: Value) -> Value {
    let mut blocks = Vec::new();
    blocks_in(&layout, &mut blocks);
    let blocks: Vec<Value> = blocks.iter().map(|block| terminal(block)).collect();
    json!({"layer_id": id, "kind": kind, "bounds": [0, 0, 1, 1], "layout": layout, "blocks": blocks})
}

fn window(id: &str, layers: Vec<Value>) -> Value {
    json!({"window_id": id, "label": "", "layers": layers})
}

/// A window with a layout and nothing floating.
fn tiled(id: &str, layout: Value) -> Value {
    window(id, vec![layer(&format!("layer:{id}"), "tiled", layout)])
}

fn view(windows: Vec<Value>) -> Value {
    let active = windows.first().map(|window| window["window_id"].clone());
    json!({
        "revision": 1, "session_id": "session:s", "label": "work",
        "active_window_id": active, "windows": windows,
    })
}

fn only_window(workspace: &Workspace) -> &Window {
    assert_eq!(workspace.windows.len(), 1);
    assert_eq!(workspace.windows[0].id, WindowId::new(WINDOW));
    assert_eq!(workspace.focused_window, Some(WindowId::new(WINDOW)));
    &workspace.windows[0]
}

fn pane_of(shown: &Shown, block: &str) -> PaneId {
    *shown
        .names
        .panes
        .iter()
        .find(|(_, name)| *name == block)
        .unwrap_or_else(|| panic!("{block} is no pane"))
        .0
}

/// A row of blocks, each split off the one before.
fn row(count: usize) -> Value {
    let mut node = leaf("block:0");
    for index in 1..count {
        node = across(
            &format!("split:{index}"),
            0.5,
            node,
            leaf(&format!("block:{index}")),
        );
    }
    node
}

#[test]
fn the_views_a_real_server_sent_are_shown() {
    let recorded = Recorded::load();
    let mut shower = Shower::default();

    let one: Value = recorded.result("session.attach", 0)["view"].clone();
    let shown = shower.sound(&one);
    let window = only_window(&shown.workspace);
    assert_eq!(shown.workspace.name, "lume-fixture");
    assert_eq!(window.centre.tabs.len(), 1);
    let tab = &window.centre.tabs[0];
    assert_eq!(window.centre.active, Some(tab.id));
    // A window and a block a person has not named have no name here either.
    assert_eq!(tab.title, None);
    let Node::Pane(first) = &tab.root else {
        panic!("one block is one pane");
    };
    assert_eq!(first.title, None);
    assert_eq!(window.focus, Some(first.id));
    assert_eq!(tab.last_focus, Some(first.id));
    let PaneContent::Terminal(terminal) = &first.content else {
        panic!("a terminal block is a terminal pane");
    };
    assert_eq!(terminal.backend, Some(backend()));
    let session = terminal.session.unwrap();

    // The same session after a split: the first block is the same pane showing the
    // same session.
    let two = recorded.result("session.view", 0);
    let shown = shower.sound(&two);
    let window = only_window(&shown.workspace);
    let tab = &window.centre.tabs[0];
    let Node::Split(split) = &tab.root else {
        panic!("two blocks side by side are a split");
    };
    assert_eq!(split.axis, Axis::Row);
    assert_eq!(split.ratios, vec![UNITS / 2, UNITS / 2]);
    assert_eq!(tab.pane_ids()[0], first.id);
    assert_eq!(shown.workspace.sessions()[0], (first.id, session));
    assert_eq!(shown.workspace.sessions().len(), 2);
    assert_eq!(
        shown.names.splits[&split.id],
        vec![Joint {
            from: 0,
            mid: 1,
            to: 2
        }]
    );
    assert_eq!(shown.names.session, two["session_id"].as_str().unwrap());
    assert_eq!(shown.names.tabs[&tab.id], two["windows"][0]["window_id"]);
}

#[test]
fn splits_of_one_direction_are_joined_and_others_are_kept() {
    // a | b | (c over d), as a server nests it: a | (b | (c over d)).
    let layout = across(
        "split:1",
        0.5,
        leaf("block:a"),
        across(
            "split:2",
            0.5,
            leaf("block:b"),
            down("split:3", 0.3, leaf("block:c"), leaf("block:d")),
        ),
    );
    let shown = sound(&view(vec![tiled("window:1", layout)]));
    let tab = &only_window(&shown.workspace).centre.tabs[0];
    let Node::Split(top) = &tab.root else {
        panic!("a split");
    };
    assert_eq!(top.axis, Axis::Row);
    assert_eq!(top.ratios, vec![UNITS / 2, UNITS / 4, UNITS / 4]);
    assert_eq!(top.children.len(), 3);
    let Node::Split(inner) = &top.children[2] else {
        panic!("the split of the other direction is kept");
    };
    assert_eq!(inner.axis, Axis::Column);
    assert_eq!(inner.ratios, vec![UNITS / 10 * 3, UNITS / 10 * 7]);
    let order: Vec<&str> = tab
        .pane_ids()
        .iter()
        .map(|pane| shown.names.panes[pane].as_str())
        .collect();
    assert_eq!(order, ["block:a", "block:b", "block:c", "block:d"]);

    // The split is named after the server's topmost, and knows the server's splits it
    // was made of: the inner one divides the second and third child, the outer one the
    // first from the rest.
    let mut shower = Shower::default();
    let again = shower.sound(&view(vec![tiled(
        "window:1",
        across("split:1", 0.5, leaf("block:a"), leaf("block:b")),
    )]));
    let Node::Split(alone) = &only_window(&again.workspace).centre.tabs[0].root else {
        panic!("a split");
    };
    assert_eq!(shower.ids.known("split:1"), Some(alone.id.get()));
    assert_eq!(
        shown.names.splits[&top.id],
        vec![
            Joint {
                from: 1,
                mid: 2,
                to: 3
            },
            Joint {
                from: 0,
                mid: 1,
                to: 3
            },
        ]
    );
    assert_eq!(
        shown.names.splits[&inner.id],
        vec![Joint {
            from: 0,
            mid: 1,
            to: 2
        }]
    );

    // Nested on the first side instead: (a | b) | c.
    let layout = across(
        "split:1",
        0.8,
        across("split:2", 0.25, leaf("block:a"), leaf("block:b")),
        leaf("block:c"),
    );
    let shown = sound(&view(vec![tiled("window:1", layout)]));
    let Node::Split(top) = &only_window(&shown.workspace).centre.tabs[0].root else {
        panic!("a split");
    };
    assert_eq!(top.ratios, vec![UNITS / 5, UNITS / 5 * 3, UNITS / 5]);
    assert_eq!(
        shown.names.splits[&top.id],
        vec![
            Joint {
                from: 0,
                mid: 1,
                to: 2
            },
            Joint {
                from: 0,
                mid: 2,
                to: 3
            },
        ]
    );
}

#[test]
fn shares_are_brought_into_what_a_split_may_have() {
    for ratio in [0.0, 1.0, -5.0, 7.5, 1e-9, 1e308, 0.999_999] {
        let layout = across(
            "split:1",
            ratio,
            leaf("block:a"),
            down("split:2", 1.0 - ratio, leaf("block:b"), leaf("block:c")),
        );
        let shown = sound(&view(vec![tiled("window:1", layout)]));
        let tab = &only_window(&shown.workspace).centre.tabs[0];
        for split in tab.root.splits() {
            assert!(split.ratios.iter().all(|share| *share >= SHARE_MIN));
        }
    }
    // A row of as many as a split may have, each half of what was left.
    let shown = sound(&view(vec![tiled("window:1", row(SPLIT_CHILDREN_MAX))]));
    let Node::Split(top) = &only_window(&shown.workspace).centre.tabs[0].root else {
        panic!("a split");
    };
    assert_eq!(top.ratios, vec![SHARE_MIN; SPLIT_CHILDREN_MAX]);
}

#[test]
fn things_keep_their_numbers_from_view_to_view() {
    let mut shower = Shower::default();
    let first = shower.sound(&view(vec![
        tiled(
            "window:1",
            across("split:1", 0.5, leaf("block:a"), leaf("block:b")),
        ),
        tiled("window:2", leaf("block:c")),
    ]));
    let (a, b, c) = (
        pane_of(&first, "block:a"),
        pane_of(&first, "block:b"),
        pane_of(&first, "block:c"),
    );
    let tabs: Vec<_> = only_window(&first.workspace)
        .centre
        .tabs
        .iter()
        .map(|tab| tab.id)
        .collect();
    let next = first.workspace.next_id;

    // The windows change places, a block goes and another comes.
    let second = shower.sound(&view(vec![
        tiled("window:2", leaf("block:c")),
        tiled(
            "window:1",
            across("split:1", 0.5, leaf("block:a"), leaf("block:d")),
        ),
    ]));
    assert_eq!(pane_of(&second, "block:a"), a);
    assert_eq!(pane_of(&second, "block:c"), c);
    let d = pane_of(&second, "block:d");
    assert!(d.get() >= next, "a new block took a number that was used");
    assert_ne!(d, b);
    let now: Vec<_> = only_window(&second.workspace)
        .centre
        .tabs
        .iter()
        .map(|tab| tab.id)
        .collect();
    assert_eq!(now, [tabs[1], tabs[0]]);
    assert!(second.workspace.next_id > d.get());

    // What is gone is forgotten, and a number is not given again.
    assert_eq!(shower.ids.known("block:b"), None);
    let third = shower.sound(&view(vec![tiled("window:2", leaf("block:b"))]));
    assert!(pane_of(&third, "block:b").get() >= second.workspace.next_id);

    // A view that is refused changes nothing.
    let before = shower.ids.next();
    shower
        .show(&view(vec![
            tiled("window:9", leaf("block:x")),
            tiled("window:9", leaf("block:y")),
        ]))
        .unwrap_err();
    assert_eq!(shower.ids.next(), before);
    assert_eq!(shower.ids.known("block:x"), None);
    assert!(shower.ids.known("block:b").is_some());
}

#[test]
fn names_zoom_focus_and_the_tab_on_show_are_the_servers() {
    let mut one = tiled(
        "window:01a2-ea8b4645",
        across("split:1", 0.5, leaf("block:a"), leaf("block:b")),
    );
    one["label"] = json!("Window ea8b4645");
    one["focused_block_id"] = json!("block:b");
    one["layers"][0]["blocks"][1]["label"] = json!("  logs\u{7} ");
    let mut two = tiled(
        "window:2",
        down("split:2", 0.5, leaf("block:c"), leaf("block:d")),
    );
    two["label"] = json!("build");
    two["focused_block_id"] = json!("block:d");
    two["zoomed_block_id"] = json!("block:d");
    let mut served = view(vec![one, two]);
    served["active_window_id"] = json!("window:2");

    let shown = sound(&served);
    let window = only_window(&shown.workspace);
    let (first, second) = (&window.centre.tabs[0], &window.centre.tabs[1]);
    assert_eq!(first.title, None);
    assert_eq!(second.title.as_deref(), Some("build"));
    assert_eq!(window.centre.active, Some(second.id));
    let (b, d) = (pane_of(&shown, "block:b"), pane_of(&shown, "block:d"));
    assert_eq!(first.last_focus, Some(b));
    assert_eq!(first.zoomed, None);
    assert_eq!(second.zoomed, Some(d));
    assert_eq!(window.focus, Some(d));
    assert_eq!(window.history, vec![d]);
    assert_eq!(
        shown.workspace.pane(b).unwrap().title.as_deref(),
        Some("logs")
    );
    assert_eq!(shown.workspace.pane(d).unwrap().title, None);

    // A focus, a zoom and a window on show that name nothing, or a zoom that hides
    // the focus: the workspace is sound all the same.
    served["active_window_id"] = json!("window:none");
    served["windows"][0]["focused_block_id"] = json!("block:none");
    served["windows"][0]["zoomed_block_id"] = json!("block:d");
    let shown = sound(&served);
    let window = only_window(&shown.workspace);
    assert_eq!(window.centre.active, Some(window.centre.tabs[0].id));
    assert_eq!(window.focus, Some(pane_of(&shown, "block:a")));
    assert_eq!(window.centre.tabs[0].zoomed, None);

    served["windows"][0]["focused_block_id"] = json!("block:a");
    served["windows"][0]["zoomed_block_id"] = json!("block:b");
    let shown = sound(&served);
    let window = only_window(&shown.workspace);
    assert_eq!(window.focus, Some(pane_of(&shown, "block:a")));
    assert_eq!(window.centre.tabs[0].zoomed, None);
}

#[test]
fn floating_layers_are_the_floating_panes_of_the_tab_on_show() {
    let mut put_away = layer(
        "layer:m",
        "floating",
        leaf_at("block:m", [0.5, 0.5, 0.25, 0.25]),
    );
    put_away["state"] = json!("minimized");
    let one = window(
        "window:1",
        vec![
            layer("layer:t", "tiled", leaf("block:a")),
            layer(
                "layer:f",
                "floating",
                leaf_at("block:f", [0.25, 0.125, 0.5, 0.5]),
            ),
            // A kind this client does not know floats, and a layer may hold a layout.
            layer(
                "layer:g",
                "popover",
                across(
                    "split:g",
                    0.5,
                    leaf_at("block:g", [0.0, 0.0, 0.25, 1.0]),
                    leaf_at("block:h", [0.25, 0.0, 0.25, 1.0]),
                ),
            ),
            put_away,
        ],
    );
    let two = window(
        "window:2",
        vec![
            layer("layer:t2", "tiled", leaf("block:b")),
            layer("layer:f2", "floating", leaf("block:z")),
        ],
    );
    let mut served = view(vec![one, two]);
    served["windows"][0]["focused_block_id"] = json!("block:f");

    let shown = sound(&served);
    let window = only_window(&shown.workspace);
    let floating: Vec<&str> = window
        .floating
        .iter()
        .map(|floating| shown.names.panes[&floating.pane.id].as_str())
        .collect();
    assert_eq!(floating, ["block:f", "block:g", "block:h", "block:m"]);
    let f = &window.floating[0];
    assert_eq!(
        (f.rect.x, f.rect.y, f.rect.w, f.rect.h),
        (UNITS / 4, UNITS / 8, UNITS / 2, UNITS / 2)
    );
    assert_eq!(f.minimised, None);
    assert_eq!(window.floating[3].minimised, Some(Minimised::Floating));
    assert_eq!(window.focus, Some(f.pane.id));
    assert!(shown.names.layers[&f.pane.id].alone);
    assert_eq!(shown.names.layers[&f.pane.id].layer_id, "layer:f");
    let g = &shown.names.layers[&window.floating[1].pane.id];
    assert_eq!((g.layer_id.as_str(), g.alone), ("layer:g", false));
    // What floats over the other tab is not shown, and is no pane.
    assert!(!shown.names.panes.values().any(|block| block == "block:z"));

    // A pane that is put away cannot have the keyboard.
    served["windows"][0]["focused_block_id"] = json!("block:m");
    let shown = sound(&served);
    assert_eq!(
        only_window(&shown.workspace).focus,
        Some(pane_of(&shown, "block:a"))
    );

    // The other tab on show: its floating pane, and none of the first tab's.
    served["active_window_id"] = json!("window:2");
    let shown = sound(&served);
    let window = only_window(&shown.workspace);
    assert_eq!(window.floating.len(), 1);
    assert_eq!(shown.names.panes[&window.floating[0].pane.id], "block:z");
}

#[test]
fn what_is_no_terminal_holds_its_place() {
    let mut one = tiled(
        "window:1",
        across("split:1", 0.5, leaf("block:a"), leaf("block:tunnel")),
    );
    one["layers"][0]["blocks"][1] = json!({
        "block_id": "block:tunnel", "label": "Tunnel\n", "creator_name": "com.superlogical.tunnel",
    });
    // A window with nothing tiled, and one with no layers at all.
    let two = window(
        "window:2",
        vec![
            json!({"layer_id": "layer:t", "kind": "tiled", "bounds": [0, 0, 1, 1], "layout": null, "blocks": null}),
            layer("layer:f", "floating", leaf("block:f")),
        ],
    );
    let three = json!({"window_id": "window:3", "layers": null});
    // A layout that names a block its layer does not list.
    let mut four = tiled("window:4", leaf("block:unlisted"));
    four["layers"][0]["blocks"] = json!([]);
    let mut served = view(vec![one, two, three, four]);

    let shown = sound(&served);
    let window = only_window(&shown.workspace);
    let tunnel = shown
        .workspace
        .pane(pane_of(&shown, "block:tunnel"))
        .unwrap();
    assert_eq!(
        tunnel.content,
        PaneContent::Placeholder {
            label: "Tunnel".to_owned(),
            kind: "com.superlogical.tunnel".to_owned(),
        }
    );
    for tab in &window.centre.tabs[1..] {
        let Node::Pane(pane) = &tab.root else {
            panic!("one pane holds the place");
        };
        assert!(matches!(pane.content, PaneContent::Placeholder { .. }));
    }
    // Only terminals are sessions.
    assert_eq!(shown.workspace.sessions().len(), 1);

    // The tab with nothing tiled, on show: its floating block has the keyboard.
    served["active_window_id"] = json!("window:2");
    served["windows"][1]["focused_block_id"] = json!("block:f");
    let shown = sound(&served);
    assert_eq!(
        only_window(&shown.workspace).focus,
        Some(pane_of(&shown, "block:f"))
    );
}

#[test]
fn a_session_with_no_windows_is_an_empty_window() {
    for windows in [json!([]), json!(null)] {
        let shown = sound(&json!({"revision": 0, "session_id": "session:s", "windows": windows}));
        let window = only_window(&shown.workspace);
        assert!(window.centre.tabs.is_empty());
        assert_eq!(window.centre.active, None);
        assert_eq!(window.focus, None);
        assert_eq!(shown.workspace.next_id, WINDOW + 1);
        assert_eq!(shown.workspace.name, "Rex");
    }
}

#[test]
fn a_view_that_is_no_layout_is_refused() {
    // One block in two places, in one window and across two.
    let twice = across("split:1", 0.5, leaf("block:a"), leaf("block:a"));
    assert!(refused(&view(vec![tiled("window:1", twice)])).contains("used twice"));
    refused(&view(vec![
        tiled("window:1", leaf("block:a")),
        tiled("window:2", leaf("block:a")),
    ]));
    // In the layout and floating over it.
    refused(&view(vec![window(
        "window:1",
        vec![
            layer("layer:t", "tiled", leaf("block:a")),
            layer("layer:f", "floating", leaf("block:a")),
        ],
    )]));
    // Two windows, two splits, or a window and a block by one name.
    refused(&view(vec![
        tiled("window:1", leaf("block:a")),
        tiled("window:1", leaf("block:b")),
    ]));
    refused(&view(vec![tiled(
        "window:1",
        across(
            "split:1",
            0.5,
            leaf("block:a"),
            down("split:1", 0.5, leaf("block:b"), leaf("block:c")),
        ),
    )]));
    refused(&view(vec![tiled("same", leaf("same"))]));

    // Things without a name, and a part that is nothing.
    refused(&view(vec![json!({"window_id": null, "layers": []})]));
    let mut nameless = across("split:1", 0.5, leaf("block:a"), leaf("block:b"));
    nameless["split"]["split_id"] = json!(null);
    refused(&view(vec![tiled("window:1", nameless)]));
    let hollow = across(
        "split:1",
        0.5,
        leaf("block:a"),
        json!({"rect": [0, 0, 1, 1]}),
    );
    assert!(refused(&view(vec![tiled("window:1", hollow)])).contains("neither"));
}

#[test]
fn a_view_with_more_than_is_shown_is_refused() {
    let many = |count: usize| -> Vec<Value> {
        (0..count)
            .map(|index| tiled(&format!("window:{index}"), leaf(&format!("block:{index}"))))
            .collect()
    };
    sound(&view(many(TABS_MAX)));
    assert!(refused(&view(many(TABS_MAX + 1))).contains("more windows"));

    // More blocks in a row than a split has children.
    refused(&view(vec![tiled("window:1", row(SPLIT_CHILDREN_MAX + 1))]));

    // More blocks in a window: rows of eight, one above the other.
    let grid = |rows: usize| {
        let line = |at: usize| {
            let mut node = leaf(&format!("block:{at}:0"));
            for index in 1..8 {
                node = across(
                    &format!("split:{at}:{index}"),
                    0.5,
                    node,
                    leaf(&format!("block:{at}:{index}")),
                );
            }
            node
        };
        let mut node = line(0);
        for at in 1..rows {
            node = down(&format!("split:{at}"), 0.5, node, line(at));
        }
        node
    };
    sound(&view(vec![tiled("window:1", grid(TAB_PANES_MAX / 8))]));
    assert!(
        refused(&view(vec![tiled("window:1", grid(TAB_PANES_MAX / 8 + 1))]))
            .contains("blocks in a window")
    );

    // Splits inside each other, turning each time so none is joined.
    let nest = |depth: usize| {
        let mut node = leaf("block:0");
        for index in 1..=depth {
            let direction = if index % 2 == 0 {
                "horizontal"
            } else {
                "vertical"
            };
            node = split(
                &format!("split:{index}"),
                direction,
                0.5,
                node,
                leaf(&format!("block:{index}")),
            );
        }
        node
    };
    sound(&view(vec![tiled("window:1", nest(TREE_DEPTH_MAX))]));
    assert!(
        refused(&view(vec![tiled("window:1", nest(TREE_DEPTH_MAX + 1))]))
            .contains("inside each other")
    );
    // Far deeper than a server nests them: refused, not followed to the end.
    refused(&view(vec![tiled("window:1", nest(SPLIT_DEPTH_MAX + 16))]));
    refused(&view(vec![window(
        "window:1",
        vec![
            layer("layer:t", "tiled", leaf("block:t")),
            layer("layer:f", "floating", nest(SPLIT_DEPTH_MAX + 16)),
        ],
    )]));

    // More floating over one window.
    let floats = |count: usize| {
        let mut layers = vec![layer("layer:t", "tiled", leaf("block:t"))];
        layers.extend((0..count).map(|index| {
            layer(
                &format!("layer:{index}"),
                "floating",
                leaf(&format!("block:{index}")),
            )
        }));
        view(vec![window("window:1", layers)])
    };
    sound(&floats(FLOATING_MAX));
    assert!(refused(&floats(FLOATING_MAX + 1)).contains("floating"));
}

#[test]
fn the_application_takes_the_workspace_as_a_copy_of_a_servers() {
    let shown = sound(&view(vec![
        window(
            "window:1",
            vec![
                layer(
                    "layer:t",
                    "tiled",
                    across("split:1", 0.5, leaf("block:a"), leaf("block:b")),
                ),
                layer(
                    "layer:f",
                    "floating",
                    leaf_at("block:f", [0.1, 0.1, 0.5, 0.5]),
                ),
            ],
        ),
        tiled("window:2", leaf("block:c")),
    ]));

    let mut own = Workspace::new(WorkspaceId::new(1).unwrap(), "mine");
    own.windows.push(Window::new(WindowId::new(WINDOW)));
    own.focused_window = Some(WindowId::new(WINDOW));
    own.next_id = WINDOW + 1;
    validate(&own).unwrap();

    let copy = mirror(&own, shown.workspace.clone(), &backend()).unwrap();
    assert_eq!(copy.backend, Some(backend()));
    assert_eq!(copy.windows[0].id, own.windows[0].id);
    assert_eq!(copy.windows[0].centre, shown.workspace.windows[0].centre);
}
