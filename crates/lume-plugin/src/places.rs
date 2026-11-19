//! The places in the application's windows that plugins can add to.
//!
//! Each menu, bar and dock area a plugin can put something in has a name here. Inside
//! one, the application's own items and sections have names of their own (`lume:zoom`),
//! so a plugin can put its things right before or after them. The application's own
//! nodes are data, listed below in the order they are shown; the window draws each
//! with code of its own, found by the node's key.

use std::fmt::Write as _;

/// The menus: the menu bar of the application's menu (`menu.app`, whose items are the
/// menus Lume, View and Help and any a plugin adds beside them), each of those menus,
/// the "New tab with" menu, a tab's menu, a pane's menu, and the menus status items
/// open (`menu.status`).
pub const MENUS: [&str; 8] = [
    "menu.app",
    "menu.app.lume",
    "menu.app.view",
    "menu.app.help",
    "menu.new-tab",
    "menu.tab",
    "menu.pane",
    "menu.status",
];

/// Where status items go: the three parts of the top bar, and a pane's header.
pub const STATUS_SLOTS: [&str; 4] = [
    "topbar.left",
    "topbar.center",
    "topbar.right",
    "pane.header",
];

/// The kinds of status item.
pub const STATUS_KINDS: [&str; 5] = ["text", "icon", "meter", "sparkline", "badge"];

/// The dock areas a panel can open in.
pub const DOCK_AREAS: [&str; 3] = ["left", "right", "bottom"];

/// The kinds of panel: a list of rows the plugin gives, drawn by the application.
pub const PANEL_KINDS: [&str; 1] = ["list"];

/// The places a plugin's motion can play in: how the mark of the tab on show moves,
/// and how menus, notices, the palette, panels, floating panes, the pills of panes put
/// away and status items that change come in or go.
pub const MOTION_SLOTS: [&str; 9] = [
    "tab.highlight",
    "menu.enter",
    "toast.enter",
    "toast.exit",
    "palette.enter",
    "panel.enter",
    "float.enter",
    "pill.enter",
    "status.change",
];

/// The kinds of cursor effect a plugin's shader can draw: a trail when the cursor
/// jumps, or a pulse when it changes shape or lands in another pane.
pub const CURSOR_EFFECT_KINDS: [&str; 2] = ["trail", "pulse"];

/// What one of the application's own nodes is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// Items a line sets apart.
    Section,
    /// One row, drawn by the window.
    Item,
    /// A row that opens a menu of its own.
    Submenu,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Section => "section",
            Self::Item => "item",
            Self::Submenu => "submenu",
        }
    }
}

/// One of the application's own nodes in a menu: its key, what it is, and what it
/// holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Builtin {
    /// What a manifest calls it after `lume:`.
    pub key: &'static str,
    pub kind: NodeKind,
    pub children: &'static [Builtin],
    /// For a submenu whose items are a menu of their own, that menu.
    pub opens: Option<&'static str>,
}

const fn item(key: &'static str) -> Builtin {
    Builtin {
        key,
        kind: NodeKind::Item,
        children: &[],
        opens: None,
    }
}

const fn submenu(key: &'static str, opens: Option<&'static str>) -> Builtin {
    Builtin {
        key,
        kind: NodeKind::Submenu,
        children: &[],
        opens,
    }
}

const fn section(key: &'static str, children: &'static [Builtin]) -> Builtin {
    Builtin {
        key,
        kind: NodeKind::Section,
        children,
        opens: None,
    }
}

/// How far apart the orders of the application's sections are, and of its items, so
/// a plugin's `order` can fall between two of them.
pub const SECTION_STEP: i32 = 100;
pub const ITEM_STEP: i32 = 10;

const APP: &[Builtin] = &[
    submenu("lume", Some("menu.app.lume")),
    submenu("view", Some("menu.app.view")),
    submenu("help", Some("menu.app.help")),
];

const APP_LUME: &[Builtin] = &[
    section(
        "new",
        &[
            item("new-window"),
            item("new-tab"),
            submenu("new-tab-with", Some("menu.new-tab")),
            submenu("groups", None),
        ],
    ),
    section(
        "workspaces",
        &[item("new-workspace"), item("open-workspace")],
    ),
    section("preferences", &[item("settings")]),
    section("sessions", &[item("detached")]),
    section("leave", &[item("quit"), item("quit-end-sessions")]),
];

const APP_VIEW: &[Builtin] = &[
    section("splits", &[item("split-right"), item("split-down")]),
    section(
        "zoom",
        &[item("zoom-in"), item("zoom-out"), item("zoom-reset")],
    ),
    section("panels", &[item("panel-list")]),
];

const APP_HELP: &[Builtin] = &[
    section("about", &[item("whats-new")]),
    section("support", &[item("report-issue"), item("show-logs")]),
    section("host", &[item("restart-host")]),
];

const NEW_TAB: &[Builtin] = &[
    section("default", &[item("default-terminal")]),
    section("profiles", &[item("profile-list")]),
    section("more", &[item("more-profiles")]),
];

const TAB: &[Builtin] = &[
    section("name", &[item("rename"), item("color")]),
    section(
        "group",
        &[
            item("new-group"),
            item("to-group"),
            item("remove-from-group"),
        ],
    ),
    section(
        "position",
        &[
            item("new-tab"),
            item("move-left"),
            item("move-right"),
            item("move-into"),
        ],
    ),
    section(
        "closing",
        &[
            item("lock"),
            item("close-left"),
            item("close-right"),
            item("close"),
        ],
    ),
];

const PANE: &[Builtin] = &[section(
    "show",
    &[item("show-in-new-tab"), item("show-in-new-window")],
)];

/// The application's own nodes at the top of a menu, in the order they are shown.
pub fn skeleton(menu: &str) -> &'static [Builtin] {
    match menu {
        "menu.app" => APP,
        "menu.app.lume" => APP_LUME,
        "menu.app.view" => APP_VIEW,
        "menu.app.help" => APP_HELP,
        "menu.new-tab" => NEW_TAB,
        "menu.tab" => TAB,
        "menu.pane" => PANE,
        _ => &[],
    }
}

/// Where one of the application's nodes is in a menu: at the top, or in a section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    pub node: &'static Builtin,
    /// The section it is in, if it is not at the top.
    pub section: Option<&'static str>,
}

/// One of the application's nodes in a menu, by its key.
pub fn find(menu: &str, key: &str) -> Option<Found> {
    for node in skeleton(menu) {
        if node.key == key {
            return Some(Found {
                node,
                section: None,
            });
        }
        if let Some(child) = node.children.iter().find(|child| child.key == key) {
            return Some(Found {
                node: child,
                section: Some(node.key),
            });
        }
    }
    None
}

/// The menu one of the application's submenus opens, for a plugin's item that names
/// it as its parent.
pub fn opened_by(menu: &str, key: &str) -> Option<&'static str> {
    find(menu, key).and_then(|found| found.node.opens)
}

/// The places as TypeScript, for the window: the keys of the application's own nodes
/// in each menu, which its renderers are held to by a test, and the other names.
pub fn typescript() -> String {
    let mut ts = String::from(
        "/**\n * The places in a window that plugins can add to, and the application's own nodes in\n * each menu, in the order they are shown.\n *\n * Generated by `cargo xtask api` from the places of the plugin crate. Do not edit:\n * change the places and generate again.\n */\n\n",
    );
    ts.push_str("/** What one of the application's own nodes is. */\n");
    ts.push_str("export type BuiltinKind = \"section\" | \"item\" | \"submenu\";\n\n");
    ts.push_str("/** One of the application's own nodes. */\n");
    ts.push_str("export interface BuiltinNode {\n  readonly key: string;\n  readonly kind: BuiltinKind;\n  readonly children: readonly BuiltinNode[];\n  /** The menu a submenu opens, where it is a menu of its own. */\n  readonly opens?: string;\n}\n\n");
    let list = |name: &str, doc: &str, values: &[&str], ts: &mut String| {
        let _ = writeln!(ts, "/** {doc} */");
        let _ = writeln!(ts, "export const {name} = [");
        for value in values {
            let _ = writeln!(ts, "  \"{value}\",");
        }
        let _ = writeln!(ts, "] as const;\n");
    };
    list("MENUS", "The menus plugins can add to.", &MENUS, &mut ts);
    list(
        "STATUS_SLOTS",
        "Where status items go.",
        &STATUS_SLOTS,
        &mut ts,
    );
    list(
        "MOTION_SLOTS",
        "The places a plugin's motion can play in.",
        &MOTION_SLOTS,
        &mut ts,
    );
    list(
        "DOCK_AREAS",
        "The dock areas a plugin's panel can open in.",
        &DOCK_AREAS,
        &mut ts,
    );
    fn write_node(node: &Builtin, indent: usize, ts: &mut String) {
        let pad = " ".repeat(indent);
        let _ = write!(
            ts,
            "{pad}{{ key: \"{}\", kind: \"{}\", children: [",
            node.key,
            node.kind.as_str()
        );
        if node.children.is_empty() {
            ts.push(']');
        } else {
            ts.push('\n');
            for child in node.children {
                write_node(child, indent + 2, ts);
            }
            let _ = write!(ts, "{pad}]");
        }
        if let Some(opens) = node.opens {
            let _ = write!(ts, ", opens: \"{opens}\"");
        }
        ts.push_str(" },\n");
    }
    ts.push_str("/** The application's own nodes at the top of each menu. */\n");
    ts.push_str(
        "export const BUILTIN_PLACES: Readonly<Record<(typeof MENUS)[number], readonly BuiltinNode[]>> = {\n",
    );
    for menu in MENUS {
        let nodes = skeleton(menu);
        if nodes.is_empty() {
            let _ = writeln!(ts, "  \"{menu}\": [],");
            continue;
        }
        let _ = writeln!(ts, "  \"{menu}\": [");
        for one in nodes {
            write_node(one, 4, &mut ts);
        }
        ts.push_str("  ],\n");
    }
    ts.push_str("};\n");
    ts
}

/// Where the generated places go, from the root of the repository.
pub const TYPESCRIPT_PATH: &str = "packages/ui/src/shell/places.generated.ts";

/// The files made from the places and kept in the repository.
pub fn generated_files() -> Vec<lume_config::generate::Generated> {
    vec![lume_config::generate::Generated {
        kind: lume_config::generate::GeneratedKind::Api,
        path: TYPESCRIPT_PATH,
        text: typescript(),
    }]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn every_menu_but_the_status_menus_has_nodes_and_keys_are_not_given_twice() {
        for menu in MENUS {
            let nodes = skeleton(menu);
            assert_eq!(nodes.is_empty(), menu == "menu.status", "{menu}");
            let mut keys = BTreeSet::new();
            for node in nodes {
                assert!(keys.insert(node.key), "{menu}: {}", node.key);
                for child in node.children {
                    assert!(keys.insert(child.key), "{menu}: {}", child.key);
                    assert_ne!(child.kind, NodeKind::Section, "{menu}: {}", child.key);
                }
            }
        }
    }

    #[test]
    fn a_node_is_found_with_the_section_it_is_in() {
        let zoom = find("menu.app.view", "zoom-in").unwrap();
        assert_eq!(zoom.section, Some("zoom"));
        assert_eq!(find("menu.app.view", "zoom").unwrap().section, None);
        assert_eq!(opened_by("menu.app", "view"), Some("menu.app.view"));
        assert_eq!(
            opened_by("menu.app.lume", "new-tab-with"),
            Some("menu.new-tab")
        );
        assert_eq!(opened_by("menu.app.lume", "groups"), None);
        assert!(find("menu.tab", "nowhere").is_none());
        // What a submenu opens is one of the menus.
        for menu in MENUS {
            for node in skeleton(menu) {
                for one in std::iter::once(node).chain(node.children) {
                    if let Some(opens) = one.opens {
                        assert!(MENUS.contains(&opens), "{opens}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_typescript_lists_every_node() {
        let ts = typescript();
        assert!(ts.contains(
            "{ key: \"new-tab-with\", kind: \"submenu\", children: [], opens: \"menu.new-tab\" },"
        ));
        assert!(ts.contains("\"menu.status\": [],"));
        assert!(ts.ends_with("};\n"));
    }
}
