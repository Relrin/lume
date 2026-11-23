//! What the application asks of a server's layout, against a pretend server: each
//! intent is the call a server takes it as, the answer waits for the view the call
//! made, and what a server cannot do or will not do is said as such.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::views::{across, down, layer, leaf, leaf_at, view, window};
use common::{Listener, SESSION, Served, Tell, World, client, serve, streams, until};
use lume_layout::{
    Dir, FocusTarget, Intent, LayoutAuthority, LayoutError, LayoutSnapshot, NewContent,
    PaneContent, PaneId, Place, SpawnRequest, SplitId, UNITS, WindowId,
};
use lume_proto::{Launcher, TermSize};
use lume_rex::tuning::APPLY_SETTLE;
use lume_rex::{Link, RexBackend};
use lume_session::SessionBackend;
use serde_json::{Value, json};

const SHELL: &str = "com.superlogical.terminal.shell";

/// a | b | (c over d), with f floating over them.
fn layout_of_four() -> Value {
    across(
        "split:1",
        0.5,
        leaf("block:a"),
        across(
            "split:2",
            0.5,
            leaf("block:b"),
            down("split:3", 0.5, leaf("block:c"), leaf("block:d")),
        ),
    )
}

fn four() -> Value {
    view(vec![window(
        "window:1",
        vec![
            layer("layer:t", "tiled", layout_of_four()),
            layer(
                "layer:f",
                "floating",
                leaf_at("block:f", [0.25, 0.25, 0.5, 0.5]),
            ),
        ],
    )])
}

/// A server whose one session has this view, and that answers what changes a layout.
fn world_of(mut view: Value) -> World {
    let mut world = World::recorded().changing();
    view["session_id"] = json!(SESSION);
    world.views.insert(SESSION.to_owned(), view);
    world
}

/// A backend showing the session of a pretend server.
struct Showing {
    backend: Arc<RexBackend>,
    served: Served,
    layout: Arc<dyn LayoutAuthority>,
    _listener: Listener,
}

impl Showing {
    async fn of(world: World) -> Self {
        let (dialer, mut listener) = streams();
        let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
        let served = serve(listener.accept().await, world);
        backend.ready().await.unwrap();
        backend.show_session(SESSION).await.unwrap();
        let layout = backend.layout().unwrap();
        Self {
            backend,
            served,
            layout,
            _listener: listener,
        }
    }

    async fn view(&self) -> LayoutSnapshot {
        self.layout.view().await.unwrap()
    }

    /// The panes of the first tab in the order they are laid out, then the floating
    /// ones.
    async fn panes(&self) -> Vec<PaneId> {
        let snapshot = self.view().await;
        let window = &snapshot.workspace.windows[0];
        let mut panes = window.centre.tabs[0].pane_ids();
        panes.extend(window.floating.iter().map(|floating| floating.pane.id));
        panes
    }

    /// The split at the top of the first tab.
    async fn top(&self) -> SplitId {
        self.view().await.workspace.windows[0].centre.tabs[0]
            .root
            .splits()[0]
            .id
    }

    /// What a method was last called with.
    fn last(&self, method: &str) -> Value {
        self.served
            .asked(method)
            .pop()
            .unwrap_or_else(|| panic!("{method} was not called"))
    }

    /// The calls that change a layout, in order.
    fn changes(&self) -> Vec<String> {
        self.served
            .called_of(&["block.close", "session."])
            .into_iter()
            .filter(|method| {
                !["session.attach", "session.attach_all", "session.view"].contains(&method.as_str())
            })
            .collect()
    }
}

fn terminal() -> NewContent {
    NewContent::Terminal(SpawnRequest::default())
}

fn window_id() -> WindowId {
    WindowId::new(1)
}

#[tokio::test]
async fn a_split_is_asked_of_the_server_and_answered_with_the_pane_it_made() {
    let mut world = world_of(four());
    // What the server makes of it: e below b.
    let mut after = four();
    after["windows"][0]["layers"][0] = layer(
        "layer:t",
        "tiled",
        across(
            "split:1",
            0.5,
            leaf("block:a"),
            across(
                "split:2",
                0.5,
                down("split:9", 0.75, leaf("block:b"), leaf("block:e")),
                down("split:3", 0.5, leaf("block:c"), leaf("block:d")),
            ),
        ),
    );
    world.then.push_back(after);
    world.extra.insert(
        "session.new_split".to_owned(),
        json!({ "anchor_id": "block:b", "block_ids": ["block:e"], "split_ids": ["split:9"] }),
    );
    let showing = Showing::of(world).await;
    let panes = showing.panes().await;
    let before = showing.view().await;

    let applied = showing
        .layout
        .apply(Intent::Split {
            pane: panes[1],
            dir: Dir::Down,
            ratio: UNITS / 4,
            content: NewContent::Terminal(SpawnRequest {
                cwd: Some("/src".to_owned()),
                size: Some(TermSize::new(100, 30).unwrap()),
                // A profile is the application's own; the server starts its own shell.
                profile: Some("PowerShell".to_owned()),
                ..SpawnRequest::default()
            }),
        })
        .await
        .unwrap();
    // The new pane is to have a quarter, below: the first side has the rest.
    assert_eq!(
        showing.last("session.new_split"),
        json!({
            "session_id": SESSION, "anchor_id": "block:b",
            "direction": "vertical", "side": "after", "ratio": 0.75, "focus": true,
            "layout": {"block": {
                "flavor": SHELL,
                "options": {"cwd": "/src", "size": {"columns": 100, "rows": 30}},
            }},
        })
    );

    // The answer is of the layout with the pane in it.
    let now = showing.view().await;
    assert_eq!((applied.epoch, applied.revision), (now.epoch, now.revision));
    assert_eq!(now.revision, before.revision + 1);
    let made = applied.created.pane.expect("the new pane's number");
    assert!(!panes.contains(&made));
    assert_eq!(showing.panes().await[2], made);
    let pane = now.workspace.pane(made).unwrap();
    assert!(matches!(pane.content, PaneContent::Terminal(_)));
    assert_eq!(applied.created.tab, None);

    // To the left: the new pane is the first side, and a program of the server's own.
    showing
        .layout
        .apply(Intent::Split {
            pane: panes[0],
            dir: Dir::Left,
            ratio: UNITS / 4,
            content: NewContent::Terminal(SpawnRequest {
                launcher: Some(Launcher::Shell {
                    program: "htop".to_owned(),
                    args: vec!["-d".to_owned(), "5".to_owned()],
                    login: false,
                }),
                ..SpawnRequest::default()
            }),
        })
        .await
        .unwrap();
    let asked = showing.last("session.new_split");
    assert_eq!(
        (&asked["direction"], &asked["side"], &asked["ratio"]),
        (&json!("horizontal"), &json!("before"), &json!(0.25))
    );
    assert_eq!(
        asked["layout"]["block"]["options"],
        json!({ "shell": "none", "command": ["htop", "-d", "5"], "login": false })
    );

    // Nothing but a flavour when nothing is asked for, and a share within bounds.
    showing
        .layout
        .apply(Intent::Split {
            pane: panes[0],
            dir: Dir::Right,
            ratio: 1,
            content: terminal(),
        })
        .await
        .unwrap();
    let asked = showing.last("session.new_split");
    assert_eq!(asked["layout"], json!({"block": {"flavor": SHELL}}));
    assert_eq!(asked["ratio"], json!(0.95));
}

#[tokio::test]
async fn each_intent_about_panes_is_the_call_a_server_takes_it_as() {
    let showing = Showing::of(world_of(four())).await;
    let panes = showing.panes().await;
    let (a, b, c, f) = (panes[0], panes[1], panes[2], panes[4]);
    let of = |args: Value| {
        let mut args = args;
        args["session_id"] = json!(SESSION);
        args
    };

    let asked: Vec<(Intent, &str, Value)> = vec![
        (
            Intent::Close { pane: b },
            "block.close",
            json!({ "block_id": "block:b" }),
        ),
        (
            Intent::Close { pane: f },
            "block.close",
            json!({ "block_id": "block:f" }),
        ),
        (
            Intent::Focus {
                target: FocusTarget::Pane { pane: c },
            },
            "session.focus_block",
            json!({ "block_id": "block:c" }),
        ),
        (
            Intent::Focus {
                target: FocusTarget::Next {
                    window: window_id(),
                },
            },
            "session.focus_next_pane",
            json!({ "wrap": true }),
        ),
        (
            Intent::Focus {
                target: FocusTarget::Previous {
                    window: window_id(),
                },
            },
            "session.focus_previous_pane",
            json!({ "wrap": true }),
        ),
        (
            Intent::FocusDir {
                window: window_id(),
                dir: Dir::Up,
            },
            "session.focus_direction",
            json!({ "direction": "up" }),
        ),
        (
            Intent::Swap { a, b: c },
            "session.swap_blocks",
            json!({ "block_id": "block:a", "other_block_id": "block:c" }),
        ),
        (
            Intent::Move {
                pane: a,
                to: Place::Beside {
                    pane: c,
                    edge: Dir::Up,
                },
            },
            "session.move_block",
            json!({
                "block_id": "block:a", "anchor_block_id": "block:c",
                "direction": "vertical", "side": "before",
            }),
        ),
        (
            // What floats goes back among the tiles the same way.
            Intent::Move {
                pane: f,
                to: Place::Beside {
                    pane: a,
                    edge: Dir::Right,
                },
            },
            "session.move_block",
            json!({
                "block_id": "block:f", "anchor_block_id": "block:a",
                "direction": "horizontal", "side": "after",
            }),
        ),
        (
            Intent::Zoom { pane: b, on: true },
            "session.zoom_block",
            json!({ "block_id": "block:b", "zoom": true }),
        ),
        (
            Intent::Zoom { pane: b, on: false },
            "session.zoom_block",
            json!({ "block_id": "block:b", "zoom": false }),
        ),
        (
            Intent::PaneRename {
                pane: c,
                title: Some("logs".to_owned()),
            },
            "session.set_block_label",
            json!({ "block_id": "block:c", "label": "logs" }),
        ),
        (
            Intent::PaneRename {
                pane: c,
                title: None,
            },
            "session.set_block_label",
            json!({ "block_id": "block:c", "label": "" }),
        ),
    ];
    let count = asked.len();
    for (intent, method, args) in asked {
        let applied = showing.layout.apply(intent.clone()).await.unwrap();
        assert_eq!(showing.last(method), of(args), "{intent:?}");
        assert_eq!(applied.created.pane, None, "{intent:?}");
    }
    // One call each, and nothing else was asked.
    assert_eq!(showing.changes().len(), count);
}

fn near(asked: &Value, wanted: f64) -> bool {
    (asked.as_f64().unwrap() - wanted).abs() < 1e-9
}

#[tokio::test]
async fn a_resize_moves_the_servers_splits_a_split_here_was_joined_from() {
    let showing = Showing::of(world_of(four())).await;
    let top = showing.top().await;
    let before = showing.view().await;
    let resize = |ratios: [u32; 3]| Intent::Resize {
        split: top,
        ratios: ratios.to_vec(),
    };
    let tenth = UNITS / 10;

    // a | b | (c over d) at 50, 25, 25. All three change: the outer split of the
    // server's first, between a and b, then the inner, between b and what is beside
    // it, each by its own first side's share.
    showing
        .layout
        .apply(resize([2 * tenth, 3 * tenth, 5 * tenth]))
        .await
        .unwrap();
    let calls = showing.served.asked("session.resize_pane");
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(calls[0]["direction"], "horizontal");
    assert_eq!(
        (&calls[0]["before_block_id"], &calls[0]["after_block_id"]),
        (&json!("block:a"), &json!("block:b"))
    );
    assert!(near(&calls[0]["ratio"], 0.2), "{calls:?}");
    // The pane at the border of a child that is itself a split: its first.
    assert_eq!(
        (&calls[1]["before_block_id"], &calls[1]["after_block_id"]),
        (&json!("block:b"), &json!("block:c"))
    );
    assert!(near(&calls[1]["ratio"], 0.375), "{calls:?}");
    assert_eq!(calls[1]["session_id"], SESSION);

    // The pretend server left the view as it was, so the split still stands at 50,
    // 25, 25. Only the inner border moves: one call.
    showing
        .layout
        .apply(resize([5 * tenth, tenth, 4 * tenth]))
        .await
        .unwrap();
    let calls = showing.served.asked("session.resize_pane");
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[2]["before_block_id"], "block:b");
    assert!(near(&calls[2]["ratio"], 0.2));

    // The shares it has already: nothing is asked.
    let applied = showing
        .layout
        .apply(resize([5 * tenth, 25 * tenth / 10, 25 * tenth / 10]))
        .await
        .unwrap();
    assert_eq!(showing.served.asked("session.resize_pane").len(), 3);
    assert_eq!(applied.created.pane, None);

    // The split of the other direction is one split of the server's.
    let column = before.workspace.windows[0].centre.tabs[0].root.splits()[1].id;
    showing
        .layout
        .apply(Intent::Resize {
            split: column,
            ratios: vec![3 * tenth, 7 * tenth],
        })
        .await
        .unwrap();
    let calls = showing.served.asked("session.resize_pane");
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[3]["direction"], "vertical");
    assert_eq!(
        (&calls[3]["before_block_id"], &calls[3]["after_block_id"]),
        (&json!("block:c"), &json!("block:d"))
    );
    assert!(near(&calls[3]["ratio"], 0.3));

    // Ratios that do not fit the split, and a split that is not there.
    for ratios in [vec![UNITS / 2, UNITS / 2], vec![0, UNITS / 2, UNITS / 2]] {
        let refused = showing
            .layout
            .apply(Intent::Resize { split: top, ratios })
            .await
            .unwrap_err();
        assert_eq!(refused.code(), "invalid_intent");
    }
    let refused = showing
        .layout
        .apply(Intent::Resize {
            split: SplitId::new(9999),
            ratios: vec![UNITS / 2, UNITS / 2],
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code(), "not_found");
    assert_eq!(showing.served.asked("session.resize_pane").len(), 4);
}

#[tokio::test]
async fn what_a_server_cannot_do_is_refused_without_asking_it() {
    // A second window with nothing tiled, whose tab has a pane that only holds its
    // place.
    let mut served = four();
    served["windows"].as_array_mut().unwrap().push(window(
        "window:2",
        vec![
            json!({"layer_id": "layer:t2", "kind": "tiled", "bounds": [0, 0, 1, 1], "layout": null, "blocks": null}),
            layer("layer:f2", "floating", leaf("block:z")),
        ],
    ));
    let showing = Showing::of(world_of(served)).await;
    let panes = showing.panes().await;
    let (a, f) = (panes[0], panes[4]);
    let holder = showing.view().await.workspace.windows[0].centre.tabs[1].first_pane();
    let gone = PaneId::new(9999);
    let split = |pane: PaneId, content: NewContent| Intent::Split {
        pane,
        dir: Dir::Right,
        ratio: UNITS / 2,
        content,
    };

    let refusals: Vec<(Intent, &str)> = vec![
        (split(gone, terminal()), "not_found"),
        (Intent::Close { pane: gone }, "not_found"),
        (Intent::Swap { a, b: gone }, "not_found"),
        (Intent::Swap { a, b: a }, "invalid_intent"),
        (
            Intent::Move {
                pane: a,
                to: Place::Beside {
                    pane: a,
                    edge: Dir::Left,
                },
            },
            "invalid_intent",
        ),
        // Beside a floating pane there is no place among the tiles.
        (split(f, terminal()), "invalid_intent"),
        (
            Intent::Move {
                pane: a,
                to: Place::Beside {
                    pane: f,
                    edge: Dir::Left,
                },
            },
            "invalid_intent",
        ),
        (Intent::Zoom { pane: f, on: true }, "invalid_intent"),
        (Intent::Close { pane: holder }, "unsupported"),
        (
            split(
                a,
                NewContent::PluginView {
                    plugin: "git-review".to_owned(),
                    view: "changes".to_owned(),
                    state: Value::Null,
                },
            ),
            "unsupported",
        ),
        (
            split(
                a,
                NewContent::Terminal(SpawnRequest {
                    launcher: Some(Launcher::Wsl {
                        distro: "Ubuntu".to_owned(),
                        user: None,
                    }),
                    ..SpawnRequest::default()
                }),
            ),
            "unsupported",
        ),
        (
            split(
                a,
                NewContent::Terminal(SpawnRequest {
                    session: lume_proto::SessionId::new(3),
                    ..SpawnRequest::default()
                }),
            ),
            "unsupported",
        ),
        (
            Intent::Focus {
                target: FocusTarget::Back {
                    window: window_id(),
                },
            },
            "unsupported",
        ),
        (
            Intent::Move {
                pane: a,
                to: Place::NewTab {
                    window: window_id(),
                    index: None,
                },
            },
            "unsupported",
        ),
        (
            Intent::PaneIcon {
                pane: a,
                icon: Some("tile:rust".to_owned()),
            },
            "unsupported",
        ),
    ];
    let before = showing.view().await;
    for (intent, code) in refusals {
        let refused = showing.layout.apply(intent.clone()).await.unwrap_err();
        assert_eq!(refused.code(), code, "{intent:?}: {refused}");
    }
    assert_eq!(showing.changes(), Vec::<String>::new());
    assert_eq!(showing.view().await, before);

    // With no session on show there is no layout to change.
    showing.backend.show_nothing();
    let refused = showing
        .layout
        .apply(Intent::Close { pane: a })
        .await
        .unwrap_err();
    assert_eq!(refused.code(), "unsupported");
}

#[tokio::test]
async fn what_a_server_refuses_is_said_as_the_server_said_it() {
    let mut world = world_of(four());
    world.refuse = Some((
        "conflict".to_owned(),
        409,
        "the window has as many blocks as it allows".to_owned(),
    ));
    let showing = Showing::of(world).await;
    let panes = showing.panes().await;
    let before = showing.view().await;

    let refused = showing
        .layout
        .apply(Intent::Split {
            pane: panes[0],
            dir: Dir::Right,
            ratio: UNITS / 2,
            content: terminal(),
        })
        .await
        .unwrap_err();
    assert_eq!(
        refused,
        LayoutError::Refused {
            code: "conflict".to_owned(),
            message: "the window has as many blocks as it allows".to_owned(),
        }
    );
    assert_eq!(refused.code(), "conflict");
    assert_eq!(showing.view().await, before);
}

#[tokio::test]
async fn a_change_the_server_does_not_announce_is_read_all_the_same() {
    let mut world = world_of(four());
    world.quiet = true;
    let mut after = four();
    after["windows"][0]["layers"][0]["blocks"][0]["label"] = json!("named");
    world.then.push_back(after);
    let showing = Showing::of(world).await;
    let a = showing.panes().await[0];
    let before = showing.view().await;

    let began = Instant::now();
    let applied = showing
        .layout
        .apply(Intent::PaneRename {
            pane: a,
            title: Some("named".to_owned()),
        })
        .await
        .unwrap();
    // It waited to be told, and then read the view itself.
    assert!(began.elapsed() >= APPLY_SETTLE);
    let now = showing.view().await;
    assert_eq!(applied.revision, before.revision + 1);
    assert_eq!(
        now.workspace.pane(a).unwrap().title.as_deref(),
        Some("named")
    );
}

#[tokio::test]
async fn nothing_is_asked_of_a_server_that_cannot_be_reached() {
    let showing = Showing::of(world_of(four())).await;
    let a = showing.panes().await[0];
    showing.served.tell(Tell::Drop);
    until("the connection is known to be lost", || {
        !matches!(&*showing.backend.link().borrow(), Link::Connected(_))
    })
    .await;

    let refused = tokio::time::timeout(
        Duration::from_secs(20),
        showing.layout.apply(Intent::Close { pane: a }),
    )
    .await
    .expect("the answer never came")
    .unwrap_err();
    assert_eq!(refused, LayoutError::Closed);
    assert_eq!(refused.code(), "disconnected");
}
