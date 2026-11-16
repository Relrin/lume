//! A copy of a layout, kept up to date from changes.
//!
//! This is what a page does with the changes it is sent, written down once so that it
//! can be tested against the authority: a projection that is given every change ends
//! up with the workspace the authority has.
//!
//! A projection holds what a view of the layout needs. What only the authority uses to
//! decide (the history of the focus, where each tab's focus last was, the next id) is
//! left out of it.

use crate::change::LayoutChange;
use crate::model::{Tab, Window, Workspace};

/// A follower's copy of a workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Projection {
    workspace: Workspace,
}

fn view_of_tab(tab: &mut Tab) {
    tab.last_focus = None;
}

fn view_of_window(window: &mut Window) {
    window.history.clear();
    window.centre.tabs.iter_mut().for_each(view_of_tab);
}

impl Projection {
    /// A projection of a workspace as it is now.
    pub fn of(workspace: &Workspace) -> Self {
        let mut workspace = workspace.clone();
        workspace.next_id = 0;
        workspace.windows.iter_mut().for_each(view_of_window);
        Self { workspace }
    }

    /// The workspace as the projection has it.
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// Brings the projection up to date with one change. A change about something the
    /// projection does not have is ignored: a follower that has missed changes notices
    /// by their numbering and asks for the whole layout.
    pub fn apply(&mut self, change: &LayoutChange) {
        let workspace = &mut self.workspace;
        match change {
            LayoutChange::WindowOpened { window } => {
                let mut window = window.clone();
                view_of_window(&mut window);
                workspace.windows.retain(|other| other.id != window.id);
                workspace.windows.push(window);
            }
            LayoutChange::WindowBounds { window, bounds } => {
                if let Some(window) = workspace.window_mut(*window) {
                    window.bounds = Some(*bounds);
                }
            }
            LayoutChange::TabAdded { window, tab, index } => {
                if let Some(window) = workspace.window_mut(*window) {
                    let mut tab = tab.clone();
                    view_of_tab(&mut tab);
                    let tabs = &mut window.centre.tabs;
                    tabs.insert((*index as usize).min(tabs.len()), tab);
                }
            }
            LayoutChange::TabRemoved { tab } => {
                for window in &mut workspace.windows {
                    window.centre.tabs.retain(|other| other.id != *tab);
                }
            }
            LayoutChange::Tabs {
                window,
                order,
                active,
            } => {
                if let Some(window) = workspace.window_mut(*window) {
                    let place = |tab: &Tab| {
                        order
                            .iter()
                            .position(|id| *id == tab.id)
                            .unwrap_or(usize::MAX)
                    };
                    window.centre.tabs.sort_by_key(place);
                    window.centre.active = *active;
                }
            }
            LayoutChange::TabTitle { tab, title } => {
                if let Some(tab) = tab_mut(workspace, *tab) {
                    tab.title = title.clone();
                }
            }
            LayoutChange::TabTree { tab, root, zoomed } => {
                if let Some(tab) = tab_mut(workspace, *tab) {
                    tab.root = root.clone();
                    tab.zoomed = *zoomed;
                }
            }
            LayoutChange::Ratios { split, ratios } => {
                let found = workspace
                    .windows
                    .iter_mut()
                    .flat_map(|window| &mut window.centre.tabs)
                    .find_map(|tab| tab.root.split_mut(*split));
                if let Some(found) = found {
                    found.ratios = ratios.clone();
                }
            }
            LayoutChange::Pane { pane } => {
                if let Some(found) = workspace.pane_mut(pane.id) {
                    *found = pane.clone();
                }
            }
            LayoutChange::Floating { window, floating } => {
                if let Some(window) = workspace.window_mut(*window) {
                    window.floating = floating.clone();
                }
            }
            LayoutChange::FloatRect { pane, rect } => {
                let found = workspace
                    .windows
                    .iter_mut()
                    .flat_map(|window| &mut window.floating)
                    .find(|floating| floating.pane.id == *pane);
                if let Some(found) = found {
                    found.rect = *rect;
                }
            }
            LayoutChange::Dock { window, area, dock } => {
                if let Some(window) = workspace.window_mut(*window) {
                    *window.dock.area_mut(*area) = dock.clone();
                }
            }
            LayoutChange::DockSize { window, area, size } => {
                if let Some(window) = workspace.window_mut(*window) {
                    window.dock.area_mut(*area).size = *size;
                }
            }
            LayoutChange::Focus { window, pane } => {
                if let Some(window) = workspace.window_mut(*window) {
                    window.focus = *pane;
                }
            }
            LayoutChange::WindowClosed { window } => {
                workspace.windows.retain(|other| other.id != *window);
            }
            LayoutChange::Scratch { scratch } => workspace.scratch = scratch.clone(),
            LayoutChange::WindowFocused { window } => workspace.focused_window = *window,
            LayoutChange::WorkspaceName { name } => workspace.name.clone_from(name),
            // The changes before it have already taken the pane out.
            LayoutChange::PaneClosed { .. } => {}
        }
    }
}

fn tab_mut(workspace: &mut Workspace, tab: crate::ids::TabId) -> Option<&mut Tab> {
    workspace
        .windows
        .iter_mut()
        .find_map(|window| window.centre.tab_mut(tab))
}
