//! Whether a workspace is one this crate could have made.
//!
//! A workspace that was read from a file, or that reached this process from elsewhere,
//! is checked before anything is done with it: everything that applies intents relies
//! on what is checked here.

use std::collections::HashSet;
use std::fmt;

use crate::ids::{GroupId, PaneId};
use crate::model::{Area, Node, PanelContent, Tab, Window, Workspace};
use crate::normalise::contain;
use crate::tuning::{
    DOCK_PANELS_MAX, DOCK_SIZE_MAX, DOCK_SIZE_MIN, FLOATING_MAX, FOCUS_HISTORY_MAX, SHARE_MIN,
    SHELF_MAX, SPLIT_CHILDREN_MAX, TAB_PANES_MAX, TABS_MAX, TITLE_MAX_CHARS, TREE_DEPTH_MAX, UNITS,
    WINDOWS_MAX,
};

/// One thing that is wrong with a workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation(pub String);

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Collects what is wrong.
struct Report {
    found: Vec<Violation>,
}

impl Report {
    fn check(&mut self, holds: bool, problem: impl FnOnce() -> String) {
        if !holds {
            self.found.push(Violation(problem()));
        }
    }
}

/// Checks that a workspace is sound: ids are used once and below the workspace's
/// counter, every split's ratios fit its children, nothing is larger than it may be,
/// and everything that refers to a window, tab or pane refers to one that is there.
pub fn validate(workspace: &Workspace) -> Result<(), Vec<Violation>> {
    let mut report = Report { found: Vec::new() };
    let mut numbers = HashSet::new();
    let mut claim = |report: &mut Report, number: u32, what: String| {
        report.check(number < workspace.next_id, || {
            format!("{what} is not below the workspace's next id")
        });
        report.check(numbers.insert(number), || {
            format!("{what} uses a number twice")
        });
    };

    report.check(workspace.windows.len() <= WINDOWS_MAX, || {
        "the workspace has too many windows".to_owned()
    });
    for window in &workspace.windows {
        claim(
            &mut report,
            window.id.get(),
            format!("window {}", window.id),
        );
        for tab in &window.centre.tabs {
            claim(&mut report, tab.id.get(), format!("tab {}", tab.id));
            for split in tab.root.splits() {
                claim(&mut report, split.id.get(), format!("split {}", split.id));
            }
        }
        for group in &window.centre.groups {
            claim(&mut report, group.id.get(), format!("group {}", group.id));
        }
        for pane in window.panes() {
            claim(&mut report, pane.id.get(), format!("pane {}", pane.id));
        }
        check_window(&mut report, window);
    }
    report.check(workspace.shelf.len() <= SHELF_MAX, || {
        "the workspace keeps too many groups put away".to_owned()
    });
    for shelved in &workspace.shelf {
        let group = shelved.group.id;
        claim(&mut report, group.get(), format!("group {group}"));
        report.check(!shelved.tabs.is_empty(), || {
            format!("group {group} was put away with no tabs")
        });
        for tab in &shelved.tabs {
            claim(&mut report, tab.id.get(), format!("tab {}", tab.id));
            for split in tab.root.splits() {
                claim(&mut report, split.id.get(), format!("split {}", split.id));
            }
            for pane in tab.root.panes() {
                claim(&mut report, pane.id.get(), format!("pane {}", pane.id));
            }
            check_tab(&mut report, tab);
        }
    }
    if let Some(parked) = workspace.parked() {
        claim(
            &mut report,
            parked.pane.id.get(),
            format!("pane {}", parked.pane.id),
        );
        report.check(contain(parked.rect) == parked.rect, || {
            "the scratch terminal's rectangle is outside what a floating pane may have".to_owned()
        });
    }

    if let Some(scratch) = &workspace.scratch {
        let shown = workspace
            .windows
            .iter()
            .any(|window| window.floating_index(scratch.pane).is_some());
        let parked = scratch
            .parked
            .as_ref()
            .is_some_and(|floating| floating.pane.id == scratch.pane);
        report.check(shown != parked, || {
            "the scratch terminal is neither floating over a window nor put away".to_owned()
        });
    }

    match workspace.focused_window {
        Some(window) => report.check(workspace.window(window).is_some(), || {
            format!("the focused window {window} is not there")
        }),
        None => report.check(workspace.windows.is_empty(), || {
            "no window is focused though there are windows".to_owned()
        }),
    }

    if report.found.is_empty() {
        Ok(())
    } else {
        Err(report.found)
    }
}

fn check_window(report: &mut Report, window: &Window) {
    let id = window.id;
    report.check(window.centre.tabs.len() <= TABS_MAX, || {
        format!("window {id} has too many tabs")
    });
    report.check(window.floating.len() <= FLOATING_MAX, || {
        format!("window {id} has too many floating panes")
    });
    for tab in &window.centre.tabs {
        check_tab(report, tab);
    }
    check_groups(report, window);
    match window.centre.active {
        Some(tab) => report.check(window.centre.index_of(tab).is_some(), || {
            format!("window {id} shows tab {tab}, which it does not have")
        }),
        None => report.check(window.centre.tabs.is_empty(), || {
            format!("window {id} shows none of its tabs")
        }),
    }
    for floating in &window.floating {
        report.check(contain(floating.rect) == floating.rect, || {
            format!(
                "floating pane {} has a rectangle outside what it may have",
                floating.pane.id
            )
        });
    }

    let focusable = window.focusable();
    match window.focus {
        Some(pane) => {
            report.check(focusable.contains(&pane), || {
                format!("window {id} focuses pane {pane}, which is not on show in it")
            });
            report.check(window.history.first() == Some(&pane), || {
                format!("window {id}'s focus history does not begin with its focus")
            });
            if let Some(tab) = window.centre.active_tab()
                && tab.contains(pane)
            {
                report.check(tab.zoomed.is_none_or(|zoomed| zoomed == pane), || {
                    format!("window {id} focuses a pane hidden behind a zoomed one")
                });
            }
        }
        None => report.check(focusable.is_empty(), || {
            format!("window {id} has panes on show and focuses none")
        }),
    }
    let mut seen: HashSet<PaneId> = HashSet::new();
    report.check(
        window
            .history
            .iter()
            .all(|pane| window.contains(*pane) && seen.insert(*pane)),
        || format!("window {id}'s focus history names a pane twice or one it does not have"),
    );
    report.check(window.history.len() <= FOCUS_HISTORY_MAX, || {
        format!("window {id}'s focus history is too long")
    });

    let mut panels = HashSet::new();
    for area in Area::ALL {
        let dock = window.dock.area(area);
        report.check(dock.panels.len() <= DOCK_PANELS_MAX, || {
            format!("window {id} has too many panels in one dock area")
        });
        report.check((DOCK_SIZE_MIN..=DOCK_SIZE_MAX).contains(&dock.size), || {
            format!("window {id} has a dock area of a size it may not have")
        });
        for panel in &dock.panels {
            report.check(panels.insert(&panel.id), || {
                format!("window {id} has panel {} twice", panel.id)
            });
            // The top bar holds the slots and nothing else, and the slots are nowhere
            // but in the top bar.
            let slot = matches!(panel.content, PanelContent::Slot { .. });
            report.check(slot == (area == Area::Top), || {
                format!("window {id} has panel {} in the wrong dock area", panel.id)
            });
        }
    }
}

fn check_groups(report: &mut Report, window: &Window) {
    let id = window.id;
    let centre = &window.centre;
    let mut seen: Vec<GroupId> = Vec::new();
    let mut previous: Option<GroupId> = None;
    for tab in &centre.tabs {
        if let Some(group) = tab.group {
            report.check(centre.group(group).is_some(), || {
                format!(
                    "tab {} is in group {group}, which window {id} does not have",
                    tab.id
                )
            });
            if previous != Some(group) {
                report.check(!seen.contains(&group), || {
                    format!("the tabs of group {group} do not stand together")
                });
                seen.push(group);
            }
        }
        previous = tab.group;
    }
    let listed: Vec<GroupId> = centre.groups.iter().map(|group| group.id).collect();
    report.check(listed == seen, || {
        format!("window {id}'s groups are not those of its tabs, in their order")
    });
    for group in &centre.groups {
        report.check(
            group
                .name
                .as_ref()
                .is_none_or(|name| !name.is_empty() && name.chars().count() <= TITLE_MAX_CHARS),
            || format!("group {}'s name is empty or too long", group.id),
        );
    }
    if let Some(tab) = centre.active_tab() {
        report.check(!centre.folded(tab), || {
            format!("window {id} shows tab {}, which is folded away", tab.id)
        });
    }
}

fn check_tab(report: &mut Report, tab: &Tab) {
    let id = tab.id;
    report.check(tab.pane_ids().len() <= TAB_PANES_MAX, || {
        format!("tab {id} has too many panes")
    });
    report.check(tab.root.depth() <= TREE_DEPTH_MAX, || {
        format!("tab {id}'s splits nest too deep")
    });
    for split in tab.root.splits() {
        let count = split.children.len();
        report.check((1..=SPLIT_CHILDREN_MAX).contains(&count), || {
            format!("split {} has {count} children", split.id)
        });
        report.check(split.ratios.len() == count, || {
            format!("split {} has not one ratio for each child", split.id)
        });
        let sum: u64 = split.ratios.iter().map(|ratio| u64::from(*ratio)).sum();
        report.check(sum == u64::from(UNITS), || {
            format!("split {}'s ratios do not add up to a whole", split.id)
        });
        report.check(split.ratios.iter().all(|ratio| *ratio >= SHARE_MIN), || {
            format!("split {} has a child smaller than a child may be", split.id)
        });
    }
    for (what, pane) in [("zoomed", tab.zoomed), ("last focused", tab.last_focus)] {
        if let Some(pane) = pane {
            report.check(tab.contains(pane), || {
                format!("tab {id}'s {what} pane {pane} is not in it")
            });
        }
    }
}

/// Whether every tab's tree is in its proper form: no split with fewer than two
/// children, and no split directly inside a split of the same direction.
pub fn is_normalised(workspace: &Workspace) -> bool {
    workspace
        .windows
        .iter()
        .flat_map(|window| &window.centre.tabs)
        .all(|tab| tree_is_tidy(&tab.root))
}

fn tree_is_tidy(node: &Node) -> bool {
    let Node::Split(split) = node else {
        return true;
    };
    split.children.len() >= 2
        && split.children.iter().all(|child| {
            let same_way = matches!(child, Node::Split(inner) if inner.axis == split.axis);
            !same_way && tree_is_tidy(child)
        })
}
