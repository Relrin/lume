//! The intents behind moving a tab or a group one place along its window's tabs, and
//! behind putting a tab in a group or taking it out.
//!
//! A tab at the edge of a group takes two steps to pass it: the first takes it out of
//! its group, or into the group beside it, where it stands; the second moves it. A
//! group moves past the whole of its neighbour, which is a tab or another group.

use crate::ids::{GroupId, TabId};
use crate::intent::Intent;
use crate::model::{TabGroup, Window};

/// The intent that moves a tab one place left (`forward` false) or right. None when it
/// has nowhere to go.
pub fn tab_step(window: &Window, tab: TabId, forward: bool) -> Option<Intent> {
    let centre = &window.centre;
    let at = centre.index_of(tab)?;
    let next = if forward {
        at.checked_add(1).filter(|next| *next < centre.tabs.len())
    } else {
        at.checked_sub(1)
    };
    let place = |index: usize, group: Option<GroupId>| Intent::TabPlace {
        tab,
        index: index as u32,
        group,
    };
    match centre.tabs[at].group {
        Some(group) => {
            let members = centre.members(group);
            if members.len() == 1 {
                // A group of one moves as a group.
                return group_step(window, group, forward);
            }
            let edge = if forward {
                members.last() == Some(&at)
            } else {
                members.first() == Some(&at)
            };
            if edge {
                Some(place(at, None))
            } else {
                next.map(|next| place(next, Some(group)))
            }
        }
        None => {
            let next = next?;
            match centre.tabs[next].group {
                Some(beside) => Some(place(at, Some(beside))),
                None => Some(place(next, None)),
            }
        }
    }
}

/// The tabs and groups of a window's strip in order, each as the indices of its tabs.
fn units(centre: &TabGroup) -> Vec<(Option<GroupId>, Vec<usize>)> {
    let mut units: Vec<(Option<GroupId>, Vec<usize>)> = Vec::new();
    for (index, tab) in centre.tabs.iter().enumerate() {
        match units.last_mut() {
            Some((Some(group), members)) if tab.group == Some(*group) => members.push(index),
            _ => units.push((tab.group, vec![index])),
        }
    }
    units
}

/// The intent that moves a group past its neighbour on one side. None when it is at
/// that end already.
pub fn group_step(window: &Window, group: GroupId, forward: bool) -> Option<Intent> {
    let units = units(&window.centre);
    let at = units.iter().position(|(id, _)| *id == Some(group))?;
    let start = units[at].1.first().copied()?;
    let index = if forward {
        start + units.get(at + 1)?.1.len()
    } else {
        start.checked_sub(units[at.checked_sub(1)?].1.len())?
    };
    Some(Intent::GroupMove {
        group,
        index: index as u32,
    })
}

/// The intent that puts a tab after the last tab of a group.
pub fn group_add(window: &Window, tab: TabId, group: GroupId) -> Option<Intent> {
    let centre = &window.centre;
    let at = centre.index_of(tab)?;
    let last = centre
        .members(group)
        .into_iter()
        .rev()
        .find(|member| *member != at)?;
    let index = if at < last { last } else { last + 1 };
    Some(Intent::TabPlace {
        tab,
        index: index as u32,
        group: Some(group),
    })
}

/// The intent that takes a tab out of its group, to stand just after it.
pub fn group_remove(window: &Window, tab: TabId) -> Option<Intent> {
    let centre = &window.centre;
    let group = centre.tab(tab)?.group?;
    let last = *centre.members(group).last()?;
    Some(Intent::TabPlace {
        tab,
        index: last as u32,
        group: None,
    })
}

/// The tabs on one side of a tab: those before it, or after it.
pub fn tabs_beside(window: &Window, tab: TabId, after: bool) -> Vec<TabId> {
    let tabs = &window.centre.tabs;
    let Some(at) = window.centre.index_of(tab) else {
        return Vec::new();
    };
    let side = if after { &tabs[at + 1..] } else { &tabs[..at] };
    side.iter().map(|tab| tab.id).collect()
}
