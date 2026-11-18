//! The places in the application's windows that plugins can add to.
//!
//! Each menu, bar and dock area a plugin can put something in has a name here. Inside
//! one, the application's own items and sections have names of their own (`lume:zoom`),
//! so a plugin can put its things right before or after them.

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
