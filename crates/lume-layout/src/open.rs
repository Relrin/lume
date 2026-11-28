//! Several workspaces open in one layout.
//!
//! The layout on show is held by one workspace's file: the one the application started
//! with. Other workspaces can be opened beside it. Their tabs join the tab strip of a
//! window, marked as theirs ([`Tab::workspace`]), and the rest of each such workspace
//! stays as its file has it. When the layout is written, the tabs of each workspace go
//! back to that workspace's own file.

use std::collections::{HashMap, HashSet};

use crate::ids::{GroupId, TabId, WindowId};
use crate::model::{Group, Tab, Workspace};
use crate::normalise::mend;

/// The tabs of every window of a workspace, in one list from the first window to the
/// last, with the groups they are in. This is what is put in a window of the layout
/// when the workspace is opened beside another.
pub fn all_tabs(workspace: &Workspace) -> (Vec<Tab>, Vec<Group>) {
    let mut tabs = Vec::new();
    let mut groups = Vec::new();
    for window in &workspace.windows {
        tabs.extend(window.centre.tabs.iter().cloned());
        groups.extend(window.centre.groups.iter().cloned());
    }
    (tabs, groups)
}

/// Takes the tabs that belong to a workspace out of a layout, with their groups: those
/// marked with `belongs`, or, without it, those of the workspace whose file holds the
/// layout. The layout keeps the rest, in its proper form.
pub fn split_off(workspace: &mut Workspace, belongs: Option<&str>) -> (Vec<Tab>, Vec<Group>) {
    let mut tabs = Vec::new();
    let mut groups = Vec::new();
    for window in &mut workspace.windows {
        let (theirs, ours): (Vec<Tab>, Vec<Tab>) = std::mem::take(&mut window.centre.tabs)
            .into_iter()
            .partition(|tab| tab.workspace.as_deref() == belongs);
        let used: HashSet<GroupId> = theirs.iter().filter_map(|tab| tab.group).collect();
        let (moved, kept): (Vec<Group>, Vec<Group>) = std::mem::take(&mut window.centre.groups)
            .into_iter()
            .partition(|group| used.contains(&group.id));
        window.centre.tabs = ours;
        window.centre.groups = kept;
        tabs.extend(theirs);
        groups.extend(moved);
    }
    mend(workspace);
    (tabs, groups)
}

/// Gives a workspace's file its tabs back: they take the place of the tabs its windows
/// had when it was read, all in its first window, numbered anew from its own counter
/// and no longer marked as another's. The panes keep their sessions. `active` is the
/// tab, by the id it has in the layout, to show. Windows other than the first that are
/// left with nothing in them are dropped.
pub fn put_back(rest: &mut Workspace, tabs: Vec<Tab>, groups: Vec<Group>, active: Option<TabId>) {
    for window in &mut rest.windows {
        window.centre.tabs.clear();
        window.centre.groups.clear();
        window.centre.active = None;
    }
    let first = rest.windows.first().map(|window| window.id);
    rest.windows
        .retain(|window| Some(window.id) == first || !window.floating.is_empty());
    let Some(window) = rest.windows.first_mut().map(|window| window.id) else {
        return;
    };

    let mut renamed: HashMap<GroupId, GroupId> = HashMap::new();
    let mut numbered = Vec::with_capacity(groups.len());
    for mut group in groups {
        let id = GroupId::new(rest.fresh());
        renamed.insert(group.id, id);
        group.id = id;
        numbered.push(group);
    }
    let mut shown = None;
    let mut placed = Vec::with_capacity(tabs.len());
    for tab in tabs {
        let was = tab.id;
        let mut tab = crate::apply::renumbered(rest, tab);
        tab.group = tab.group.and_then(|group| renamed.get(&group).copied());
        tab.workspace = None;
        if Some(was) == active {
            shown = Some(tab.id);
        }
        placed.push(tab);
    }
    if let Some(window) = rest.window_mut(window) {
        window.centre.active = shown.or_else(|| placed.first().map(|tab| tab.id));
        window.centre.tabs = placed;
        window.centre.groups = numbered;
    }
    mend(rest);
}

/// Has the window `next` will be shown in keep the id of the window the user works in
/// now, so that showing `next` in place of `shown` leaves that window open on the
/// desktop. The window of `next` the user last worked in, or its first, takes the id;
/// a window of `next` that already had it takes the id it gives up.
pub fn keep_window_ids(shown: &Workspace, next: &mut Workspace) {
    let Some(kept) = shown
        .focused_window
        .or_else(|| shown.windows.first().map(|window| window.id))
    else {
        return;
    };
    let Some(moving) = next
        .focused_window
        .filter(|window| next.window(*window).is_some())
        .or_else(|| next.windows.first().map(|window| window.id))
    else {
        return;
    };
    if moving == kept {
        return;
    }
    let rename = |id: WindowId| {
        if id == moving {
            kept
        } else if id == kept {
            moving
        } else {
            id
        }
    };
    for window in &mut next.windows {
        window.id = rename(window.id);
    }
    next.focused_window = next.focused_window.map(rename);
    next.next_id = next.next_id.max(kept.get() + 1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::apply;
    use crate::ids::WorkspaceId;
    use crate::intent::{Intent, NewContent, SpawnRequest};
    use crate::model::{Node, PaneContent};
    use crate::validate::validate;
    use lume_proto::SessionId;

    fn begun(name: &str, tabs: usize) -> Workspace {
        let mut workspace = Workspace::new(WorkspaceId::new(1).unwrap(), name);
        apply(
            &mut workspace,
            Intent::WindowNew {
                content: NewContent::Terminal(SpawnRequest::default()),
                bounds: None,
            },
            None,
        )
        .unwrap();
        let window = workspace.windows[0].id;
        for _ in 1..tabs {
            apply(
                &mut workspace,
                Intent::TabNew {
                    window,
                    content: NewContent::Terminal(SpawnRequest::default()),
                    index: None,
                    activate: true,
                    group: None,
                },
                None,
            )
            .unwrap();
        }
        workspace
    }

    fn with_session(workspace: &mut Workspace, session: u64) {
        let pane = workspace.windows[0].centre.tabs[0].first_pane();
        if let Some(found) = workspace.pane_mut(pane)
            && let PaneContent::Terminal(terminal) = &mut found.content
        {
            terminal.session = SessionId::new(session);
        }
    }

    fn adopt(into: &mut Workspace, other: &Workspace) {
        let (tabs, groups) = all_tabs(other);
        let window = into.windows[0].id;
        apply(
            into,
            Intent::TabsAdopt {
                window,
                workspace: Some(other.name.clone()),
                tabs,
                groups,
            },
            None,
        )
        .unwrap();
    }

    #[test]
    fn a_workspace_opened_beside_another_joins_its_strip_with_its_sessions() {
        let mut home = begun("home", 2);
        let mut other = begun("site", 2);
        with_session(&mut other, 7);
        adopt(&mut home, &other);

        validate(&home).unwrap();
        let tabs = &home.windows[0].centre.tabs;
        assert_eq!(tabs.len(), 4);
        assert_eq!(home.windows.len(), 1);
        assert!(tabs[..2].iter().all(|tab| tab.workspace.is_none()));
        assert!(
            tabs[2..]
                .iter()
                .all(|tab| tab.workspace.as_deref() == Some("site"))
        );
        // The first of its tabs is on show, and its session is still named.
        assert_eq!(home.windows[0].centre.active, Some(tabs[2].id));
        let Node::Pane(pane) = &tabs[2].root else {
            panic!("a split");
        };
        let PaneContent::Terminal(terminal) = &pane.content else {
            panic!("not a terminal");
        };
        assert_eq!(terminal.session, SessionId::new(7));
    }

    #[test]
    fn a_new_tab_belongs_to_the_workspace_of_the_tab_on_show() {
        let mut home = begun("home", 1);
        adopt(&mut home, &begun("site", 1));
        let window = home.windows[0].id;
        apply(
            &mut home,
            Intent::TabNew {
                window,
                content: NewContent::Terminal(SpawnRequest::default()),
                index: None,
                activate: true,
                group: None,
            },
            None,
        )
        .unwrap();
        let last = home.windows[0].centre.tabs.last().unwrap();
        assert_eq!(last.workspace.as_deref(), Some("site"));
    }

    #[test]
    fn the_tabs_of_a_group_belong_to_the_workspace_of_its_first() {
        let mut home = begun("home", 2);
        let first = home.windows[0].centre.tabs[0].id;
        let second = home.windows[0].centre.tabs[1].id;
        let (_, created) = apply(&mut home, Intent::GroupNew { tab: first }, None).unwrap();
        let group = created.group.unwrap();
        apply(
            &mut home,
            Intent::GroupRetag {
                group,
                workspace: Some("site".to_owned()),
            },
            None,
        )
        .unwrap();
        apply(
            &mut home,
            Intent::TabPlace {
                tab: second,
                index: 1,
                group: Some(group),
            },
            None,
        )
        .unwrap();
        assert!(
            home.windows[0]
                .centre
                .tabs
                .iter()
                .all(|tab| tab.workspace.as_deref() == Some("site"))
        );
    }

    #[test]
    fn splitting_off_and_putting_back_gives_each_file_its_own_tabs() {
        let mut home = begun("home", 2);
        let mut other = begun("site", 3);
        with_session(&mut other, 9);
        adopt(&mut home, &other);

        let mut layout = home.clone();
        let (theirs, groups) = split_off(&mut layout, Some("site"));
        assert_eq!(theirs.len(), 3);
        assert_eq!(layout.windows[0].centre.tabs.len(), 2);
        validate(&layout).unwrap();

        let active = home.windows[0].centre.active;
        put_back(&mut other, theirs, groups, active);
        validate(&other).unwrap();
        assert_eq!(other.windows[0].centre.tabs.len(), 3);
        assert!(
            other.windows[0]
                .centre
                .tabs
                .iter()
                .all(|tab| tab.workspace.is_none())
        );
        assert_eq!(other.sessions().len(), 1);
        assert_eq!(
            other.windows[0].centre.active,
            Some(other.windows[0].centre.tabs[0].id)
        );
    }

    #[test]
    fn taking_a_workspaces_tabs_leaves_the_others() {
        let mut home = begun("home", 2);
        adopt(&mut home, &begun("site", 2));
        apply(
            &mut home,
            Intent::TabsTake {
                workspace: "site".to_owned(),
            },
            None,
        )
        .unwrap();
        assert_eq!(home.windows[0].centre.tabs.len(), 2);
        assert!(
            home.windows[0]
                .centre
                .tabs
                .iter()
                .all(|tab| tab.workspace.is_none())
        );
    }

    #[test]
    fn the_window_on_show_keeps_its_id() {
        let shown = begun("home", 1);
        let mut next = begun("site", 1);
        // Another workspace's window was numbered differently.
        let kept = shown.windows[0].id;
        next.windows[0].id = WindowId::new(kept.get() + 40);
        next.focused_window = Some(next.windows[0].id);
        next.next_id = next.next_id.max(kept.get() + 41);
        keep_window_ids(&shown, &mut next);
        assert_eq!(next.windows[0].id, kept);
        assert_eq!(next.focused_window, Some(kept));
        validate(&next).unwrap();
    }
}
