//! The changes that take one workspace to another.
//!
//! An intent is applied to a copy of the workspace, and what it changed is found by
//! comparing the copy with the original. The changes are therefore complete by
//! construction: whatever an intent touches, directly or through the tidying that
//! follows it, is in the list.

use std::collections::HashSet;

use crate::change::LayoutChange;
use crate::ids::{PaneId, TabId};
use crate::model::{Area, Minimised, Tab, Window, Workspace};

/// The changes that take `before` to `after`, in an order in which each can be
/// applied to a copy of `before`.
pub(crate) fn diff(before: &Workspace, after: &Workspace) -> Vec<LayoutChange> {
    let mut changes = Vec::new();

    // Windows that are new come first, whole: nothing more is said about them.
    for window in &after.windows {
        if before.window(window.id).is_none() {
            changes.push(LayoutChange::WindowOpened {
                window: window.clone(),
            });
        }
    }

    // Tabs leave before tabs arrive, so that a tab is never in two places.
    let kept: Vec<(&Window, &Window)> = before
        .windows
        .iter()
        .filter_map(|old| Some((old, after.window(old.id)?)))
        .collect();
    for (old, new) in &kept {
        for tab in &old.centre.tabs {
            if new.centre.index_of(tab.id).is_none() {
                changes.push(LayoutChange::TabRemoved { tab: tab.id });
            }
        }
    }
    for (old, new) in &kept {
        diff_window(old, new, &mut changes);
    }

    for window in &before.windows {
        if after.window(window.id).is_none() {
            changes.push(LayoutChange::WindowClosed { window: window.id });
        }
    }
    if before.scratch != after.scratch {
        changes.push(LayoutChange::Scratch {
            scratch: after.scratch.clone(),
        });
    }
    if before.focused_window != after.focused_window {
        changes.push(LayoutChange::WindowFocused {
            window: after.focused_window,
        });
    }
    if before.name != after.name {
        changes.push(LayoutChange::WorkspaceName {
            name: after.name.clone(),
        });
    }
    if before.shelf != after.shelf {
        changes.push(LayoutChange::Shelf {
            shelf: after.shelf.clone(),
        });
    }

    // Last, the panes that are gone, for whoever kept something for them.
    let left: HashSet<PaneId> = after.panes().into_iter().map(|pane| pane.id).collect();
    for pane in before.panes() {
        if !left.contains(&pane.id) {
            changes.push(LayoutChange::PaneClosed {
                pane: pane.id,
                content: pane.content.clone(),
            });
        }
    }
    changes
}

fn diff_window(old: &Window, new: &Window, changes: &mut Vec<LayoutChange>) {
    let window = new.id;
    if let Some(bounds) = new.bounds
        && old.bounds != new.bounds
    {
        changes.push(LayoutChange::WindowBounds { window, bounds });
    }

    for area in Area::ALL {
        let (was, is) = (old.dock.area(area), new.dock.area(area));
        if was.panels != is.panels || was.collapsed != is.collapsed {
            changes.push(LayoutChange::Dock {
                window,
                area,
                dock: is.clone(),
            });
        } else if was.size != is.size {
            changes.push(LayoutChange::DockSize {
                window,
                area,
                size: is.size,
            });
        }
    }

    for (index, tab) in new.centre.tabs.iter().enumerate() {
        match old.centre.tab(tab.id) {
            None => changes.push(LayoutChange::TabAdded {
                window,
                tab: tab.clone(),
                index: index as u32,
            }),
            Some(was) => diff_tab(was, tab, changes),
        }
    }
    if old.centre.groups != new.centre.groups {
        changes.push(LayoutChange::Groups {
            window,
            groups: new.centre.groups.clone(),
        });
    }
    let order =
        |window: &Window| -> Vec<TabId> { window.centre.tabs.iter().map(|tab| tab.id).collect() };
    if order(old) != order(new) || old.centre.active != new.centre.active {
        changes.push(LayoutChange::Tabs {
            window,
            order: order(new),
            active: new.centre.active,
        });
    }

    let ids = |window: &Window| -> Vec<(PaneId, Option<Minimised>)> {
        window
            .floating
            .iter()
            .map(|floating| (floating.pane.id, floating.minimised))
            .collect()
    };
    if ids(old) != ids(new) {
        changes.push(LayoutChange::Floating {
            window,
            floating: new.floating.clone(),
        });
    } else {
        for (was, is) in old.floating.iter().zip(&new.floating) {
            if was.pane != is.pane {
                changes.push(LayoutChange::Pane {
                    pane: is.pane.clone(),
                });
            }
            if was.rect != is.rect {
                changes.push(LayoutChange::FloatRect {
                    pane: is.pane.id,
                    rect: is.rect,
                });
            }
        }
    }

    if old.focus != new.focus {
        changes.push(LayoutChange::Focus {
            window,
            pane: new.focus,
        });
    }
}

fn diff_tab(old: &Tab, new: &Tab, changes: &mut Vec<LayoutChange>) {
    if old.title != new.title {
        changes.push(LayoutChange::TabTitle {
            tab: new.id,
            title: new.title.clone(),
        });
    }
    if (old.color, old.locked, old.group) != (new.color, new.locked, new.group) {
        changes.push(LayoutChange::TabState {
            tab: new.id,
            color: new.color,
            locked: new.locked,
            group: new.group,
        });
    }
    if old.zoomed != new.zoomed || !old.root.same_shape(&new.root) {
        changes.push(LayoutChange::TabTree {
            tab: new.id,
            root: new.root.clone(),
            zoomed: new.zoomed,
        });
        return;
    }
    // The same splits and panes in the same places: only what is in them can differ.
    for (was, is) in old.root.splits().into_iter().zip(new.root.splits()) {
        if was.ratios != is.ratios {
            changes.push(LayoutChange::Ratios {
                split: is.id,
                ratios: is.ratios.clone(),
            });
        }
    }
    for (was, is) in old.root.panes().into_iter().zip(new.root.panes()) {
        if was != is {
            changes.push(LayoutChange::Pane { pane: is.clone() });
        }
    }
}
