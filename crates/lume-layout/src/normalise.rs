//! Bringing a layout to its one proper form.
//!
//! After an intent has changed a layout, the same arrangement could be written in
//! several ways: a split with one child is its child, and a row inside a row is one
//! row. Each arrangement is kept in one form, so two layouts that look the same are
//! equal. What refers to panes (the focus, its history, a zoom) is put right at the
//! same time, since the pane it named may be gone.

use std::collections::HashSet;

use crate::ids::{GroupId, PaneId};
use crate::model::{Node, Split, TabGroup, UnitRect, Window, Workspace};
use crate::tuning::{FLOAT_MIN, FOCUS_HISTORY_MAX, SHARE_MIN, UNITS};

/// Makes ratios add up to a whole exactly, by giving the largest what is missing or
/// taking from it what is too much.
pub(crate) fn settle(ratios: &mut [u32]) {
    let Some(largest) = (0..ratios.len()).max_by_key(|index| ratios[*index]) else {
        return;
    };
    let sum: u64 = ratios.iter().map(|ratio| u64::from(*ratio)).sum();
    let whole = u64::from(UNITS);
    if sum < whole {
        ratios[largest] = ratios[largest].saturating_add((whole - sum) as u32);
    } else {
        ratios[largest] = ratios[largest].saturating_sub((sum - whole) as u32);
    }
}

/// The shares of a split's children once the split itself has only `share` of its
/// parent: each child's share of the parent.
fn within(share: u32, ratios: &[u32]) -> Vec<u32> {
    let mut scaled: Vec<u32> = ratios
        .iter()
        .map(|ratio| (u64::from(share) * u64::from(*ratio) / u64::from(UNITS)) as u32)
        .collect();
    // What rounding lost goes to the largest, so the children still add up to `share`.
    let sum: u32 = scaled.iter().sum();
    if let Some(largest) = (0..scaled.len()).max_by_key(|index| scaled[*index]) {
        scaled[largest] += share.saturating_sub(sum);
    }
    scaled
}

/// Takes one child's share away and gives it to the others in proportion to theirs.
pub(crate) fn release(ratios: &mut Vec<u32>, index: usize) {
    if index >= ratios.len() {
        return;
    }
    ratios.remove(index);
    let left: u64 = ratios.iter().map(|ratio| u64::from(*ratio)).sum();
    let alike = UNITS / ratios.len().max(1) as u32;
    for ratio in ratios.iter_mut() {
        let grown = (u64::from(*ratio) * u64::from(UNITS)).checked_div(left);
        // With nothing left to go by, the others share alike.
        *ratio = grown.map_or(alike, |grown| grown as u32);
    }
    settle(ratios);
}

/// The ratios nearest to those asked for that a split may have: none below the
/// smallest share, adding up to a whole.
pub(crate) fn fit(asked: &[u32]) -> Vec<u32> {
    let mut ratios: Vec<u32> = asked.iter().map(|ratio| (*ratio).max(SHARE_MIN)).collect();
    // Raising the small ones may have made the sum too large. The largest give way,
    // each no further than the smallest share.
    loop {
        let sum: u64 = ratios.iter().map(|ratio| u64::from(*ratio)).sum();
        let Some(excess) = sum
            .checked_sub(u64::from(UNITS))
            .filter(|excess| *excess > 0)
        else {
            break;
        };
        let Some(largest) = (0..ratios.len()).max_by_key(|index| ratios[*index]) else {
            break;
        };
        let room = ratios[largest] - SHARE_MIN;
        if room == 0 {
            // More children than fit at the smallest share. A split never has so many.
            break;
        }
        ratios[largest] -= room.min(excess as u32);
    }
    settle(&mut ratios);
    ratios
}

/// A tree in its proper form: no split with fewer than two children, and no split
/// directly inside a split of the same direction.
pub(crate) fn tidy(node: Node) -> Node {
    let split = match node {
        Node::Pane(_) => return node,
        Node::Split(split) => split,
    };
    let Split {
        id,
        axis,
        ratios,
        children,
    } = split;
    let mut merged_ratios = Vec::with_capacity(ratios.len());
    let mut merged_children = Vec::with_capacity(children.len());
    for (index, child) in children.into_iter().enumerate() {
        let share = ratios.get(index).copied().unwrap_or(0);
        match tidy(child) {
            // A row in a row is one row: the inner children join the outer ones, each
            // with its share of the whole.
            Node::Split(inner) if inner.axis == axis => {
                merged_ratios.extend(within(share, &inner.ratios));
                merged_children.extend(inner.children);
            }
            other => {
                merged_ratios.push(share);
                merged_children.push(other);
            }
        }
    }
    if merged_children.len() == 1 {
        return merged_children.remove(0);
    }
    settle(&mut merged_ratios);
    Node::Split(Split {
        id,
        axis,
        ratios: merged_ratios,
        children: merged_children,
    })
}

/// Brings every tab's tree to its proper form.
pub(crate) fn tidy_trees(workspace: &mut Workspace) {
    for window in &mut workspace.windows {
        for tab in &mut window.centre.tabs {
            // A pane stands in for the tree while it is taken out and put back.
            let root = std::mem::replace(&mut tab.root, Node::Split(placeholder()));
            tab.root = tidy(root);
        }
    }
}

fn placeholder() -> Split {
    Split {
        id: crate::ids::SplitId::new(0),
        axis: crate::model::Axis::Row,
        ratios: Vec::new(),
        children: Vec::new(),
    }
}

/// A floating pane's rectangle brought inside the whole, and no smaller than a
/// floating pane may be.
pub(crate) fn contain(rect: UnitRect) -> UnitRect {
    let w = rect.w.clamp(FLOAT_MIN, UNITS);
    let h = rect.h.clamp(FLOAT_MIN, UNITS);
    UnitRect {
        x: rect.x.min(UNITS - w),
        y: rect.y.min(UNITS - h),
        w,
        h,
    }
}

/// Keeps a window's groups in their one form: every tab's group is one of the
/// window's, the tabs of a group stand together where its first tab is, a group with
/// no tabs is gone, and the groups are listed in the order their tabs come.
fn mend_groups(centre: &mut TabGroup) {
    let known: HashSet<GroupId> = centre.groups.iter().map(|group| group.id).collect();
    for tab in &mut centre.tabs {
        if tab.group.is_some_and(|group| !known.contains(&group)) {
            tab.group = None;
        }
    }
    let of: Vec<Option<GroupId>> = centre.tabs.iter().map(|tab| tab.group).collect();
    let mut slots: Vec<Option<crate::model::Tab>> = std::mem::take(&mut centre.tabs)
        .into_iter()
        .map(Some)
        .collect();
    let mut order = Vec::new();
    for index in 0..slots.len() {
        let Some(tab) = slots[index].take() else {
            continue;
        };
        centre.tabs.push(tab);
        if let Some(group) = of[index] {
            order.push(group);
            for later in index + 1..slots.len() {
                if of[later] == Some(group)
                    && let Some(member) = slots[later].take()
                {
                    centre.tabs.push(member);
                }
            }
        }
    }
    let groups = std::mem::take(&mut centre.groups);
    centre.groups = order
        .into_iter()
        .filter_map(|id| groups.iter().find(|group| group.id == id).cloned())
        .collect();
}

/// Puts right everything in a window that refers to its tabs and panes.
fn mend_window(window: &mut Window) {
    mend_groups(&mut window.centre);
    for tab in &mut window.centre.tabs {
        if tab.zoomed.is_some_and(|pane| !tab.contains(pane)) {
            tab.zoomed = None;
        }
        if tab.last_focus.is_some_and(|pane| !tab.contains(pane)) {
            tab.last_focus = None;
        }
    }

    let shown = window
        .centre
        .active
        .is_some_and(|tab| window.centre.index_of(tab).is_some());
    if !shown {
        window.centre.active = window.centre.tabs.first().map(|tab| tab.id);
    }
    // The tab on show is never folded away: its group opens.
    if let Some(group) = window.centre.active_tab().and_then(|tab| tab.group)
        && let Some(group) = window.centre.group_mut(group)
    {
        group.collapsed = false;
    }

    for floating in &mut window.floating {
        floating.rect = contain(floating.rect);
    }

    // The history keeps only panes the window still has, each once.
    let present: HashSet<PaneId> = window.panes().into_iter().map(|pane| pane.id).collect();
    let mut seen = HashSet::new();
    window
        .history
        .retain(|pane| present.contains(pane) && seen.insert(*pane));

    // The keyboard belongs to a pane of the tab on show or to a floating pane. If the
    // pane that had it is gone or out of sight, it goes to the one of those that had it
    // most recently, then to where the tab's focus last was, then to the tab's first
    // pane, then to the floating pane in front.
    let candidates = window.focusable();
    let kept = window.focus.filter(|pane| candidates.contains(pane));
    let focus = kept
        .or_else(|| {
            window
                .history
                .iter()
                .copied()
                .find(|pane| candidates.contains(pane))
        })
        .or_else(|| {
            let tab = window.centre.active_tab()?;
            Some(tab.last_focus.unwrap_or_else(|| tab.first_pane()))
        })
        .or_else(|| {
            window
                .shown_floating()
                .last()
                .map(|floating| floating.pane.id)
        });
    window.focus = focus;

    if let Some(pane) = focus {
        window.history.retain(|seen| *seen != pane);
        window.history.insert(0, pane);
        if let Some(tab) = window.centre.active_tab_mut()
            && tab.contains(pane)
        {
            tab.last_focus = Some(pane);
            // A zoomed pane hides the others; the keyboard cannot be with a hidden one.
            if tab.zoomed.is_some_and(|zoomed| zoomed != pane) {
                tab.zoomed = None;
            }
        }
    }
    window.history.truncate(FOCUS_HISTORY_MAX);
}

/// Puts right everything in a workspace that refers to its windows, tabs and panes.
pub(crate) fn mend(workspace: &mut Workspace) {
    for window in &mut workspace.windows {
        mend_window(window);
    }

    let focused = workspace
        .focused_window
        .is_some_and(|window| workspace.window(window).is_some());
    if !focused {
        workspace.focused_window = workspace.windows.first().map(|window| window.id);
    }

    // The scratch terminal is one particular floating pane. Once that pane is closed,
    // or put among the tabs, the workspace has no scratch terminal.
    let gone = workspace.scratch.as_ref().is_some_and(|scratch| {
        scratch.parked.is_none()
            && !workspace
                .windows
                .iter()
                .any(|window| window.floating_index(scratch.pane).is_some())
    });
    if gone {
        workspace.scratch = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::SplitId;
    use crate::model::{Axis, Pane, PaneContent, TerminalPane};

    fn pane(number: u32) -> Node {
        Node::Pane(Pane {
            id: PaneId::new(number),
            title: None,
            icon: None,
            content: PaneContent::Terminal(TerminalPane::default()),
        })
    }

    fn split(id: u32, axis: Axis, ratios: &[u32], children: Vec<Node>) -> Node {
        Node::Split(Split {
            id: SplitId::new(id),
            axis,
            ratios: ratios.to_vec(),
            children,
        })
    }

    #[test]
    fn a_split_with_one_child_becomes_the_child() {
        let tree = split(1, Axis::Row, &[UNITS], vec![pane(1)]);
        assert_eq!(tidy(tree), pane(1));
    }

    #[test]
    fn a_row_in_a_row_becomes_one_row() {
        let tree = split(
            1,
            Axis::Row,
            &[400_000, 600_000],
            vec![
                pane(1),
                split(2, Axis::Row, &[500_000, 500_000], vec![pane(2), pane(3)]),
            ],
        );
        assert_eq!(
            tidy(tree),
            split(
                1,
                Axis::Row,
                &[400_000, 300_000, 300_000],
                vec![pane(1), pane(2), pane(3)]
            )
        );
    }

    #[test]
    fn a_column_in_a_row_is_left_alone() {
        let tree = split(
            1,
            Axis::Row,
            &[400_000, 600_000],
            vec![
                pane(1),
                split(2, Axis::Column, &[500_000, 500_000], vec![pane(2), pane(3)]),
            ],
        );
        assert_eq!(tidy(tree.clone()), tree);
    }

    #[test]
    fn merged_shares_still_add_up_to_a_whole() {
        let tree = split(
            1,
            Axis::Row,
            &[333_333, 666_667],
            vec![
                split(
                    2,
                    Axis::Row,
                    &[333_333, 333_333, 333_334],
                    vec![pane(1), pane(2), pane(3)],
                ),
                pane(4),
            ],
        );
        let Node::Split(merged) = tidy(tree) else {
            panic!("the row is still a split");
        };
        assert_eq!(merged.children.len(), 4);
        assert_eq!(merged.ratios.iter().sum::<u32>(), UNITS);
    }

    #[test]
    fn a_released_share_goes_to_the_others_in_proportion() {
        let mut ratios = vec![200_000, 300_000, 500_000];
        release(&mut ratios, 2);
        assert_eq!(ratios, vec![400_000, 600_000]);

        let mut ratios = vec![333_333, 333_333, 333_334];
        release(&mut ratios, 0);
        assert_eq!(ratios.iter().sum::<u32>(), UNITS);
        assert_eq!(ratios.len(), 2);
    }

    #[test]
    fn fitted_ratios_keep_every_child_at_least_the_smallest_share() {
        assert_eq!(fit(&[500_000, 500_000]), vec![500_000, 500_000]);
        assert_eq!(fit(&[0, UNITS]), vec![SHARE_MIN, UNITS - SHARE_MIN]);
        assert_eq!(
            fit(&[10_000, 10_000, 980_000]),
            vec![SHARE_MIN, SHARE_MIN, UNITS - 2 * SHARE_MIN]
        );
        // Off by rounding: brought to a whole.
        assert_eq!(fit(&[333_333, 333_333, 333_333]).iter().sum::<u32>(), UNITS);
        // Twenty children at the smallest share fill the split exactly.
        assert_eq!(fit(&[0; 20]), vec![SHARE_MIN; 20]);
    }

    #[test]
    fn a_floating_rectangle_is_kept_inside_and_large_enough() {
        let far = contain(UnitRect {
            x: UNITS,
            y: 900_000,
            w: 10,
            h: 400_000,
        });
        assert_eq!(
            far,
            UnitRect {
                x: UNITS - FLOAT_MIN,
                y: 600_000,
                w: FLOAT_MIN,
                h: 400_000
            }
        );
        assert_eq!(contain(UnitRect::FULL), UnitRect::FULL);
    }
}
