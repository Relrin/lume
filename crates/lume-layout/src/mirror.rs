//! Showing a layout that a server owns.
//!
//! A server that owns a layout knows its tabs, splits and panes. It knows nothing of
//! this application's windows on the desktop, nor of the dock areas and panels around
//! their tabs. A mirror is the workspace this application shows for such a layout:
//! the server's tabs and panes, inside windows that keep what is this application's
//! own. Each time the server's layout changes the mirror is made again from it.
//!
//! Intents follow the same division. Those about what is this application's own are
//! applied to the mirror here; every other is the server's to apply, and comes back as
//! a change to what it serves.

use lume_proto::BackendId;

use crate::intent::Intent;
use crate::model::{Window, Workspace};
use crate::validate::{Violation, validate};

impl Intent {
    /// Whether the intent is about what belongs to this application alone, in whoever's
    /// layout: where a window is on the desktop, which window the user works in, and
    /// the dock areas and their panels.
    pub fn is_window_local(&self) -> bool {
        matches!(
            self,
            Intent::PanelMove { .. }
                | Intent::PanelToggle { .. }
                | Intent::PanelResize { .. }
                | Intent::PanelSet { .. }
                | Intent::PanelState { .. }
                | Intent::WindowBounds { .. }
                | Intent::WindowFocus { .. }
        )
    }
}

/// What a window keeps when the layout inside it is replaced by a server's.
fn keep_local(window: &mut Window, from: &Window) {
    window.bounds = from.bounds;
    window.dock = from.dock.clone();
}

/// The workspace to show for the layout a server serves.
///
/// `served` is the server's layout as its authority gives it, `backend` whose it is,
/// and `shown` the workspace on show now: the mirror made before, or the workspace the
/// mirror takes the place of. A served window that has the id of a window on show keeps
/// that window's place on the desktop and its dock areas. Where none has, the first
/// served window takes the dock areas of the window the user was working in, so the
/// panels around the tabs stay as they were.
///
/// What a server serves is not trusted to be a sound layout: one that is not is
/// refused, with what is wrong with it.
pub fn mirror(
    shown: &Workspace,
    mut served: Workspace,
    backend: &BackendId,
) -> Result<Workspace, Vec<Violation>> {
    served.backend = Some(backend.clone());
    // A scratch terminal and put-away groups are this application's, and a server's
    // layout has neither.
    served.scratch = None;
    served.shelf.clear();
    if served
        .focused_window
        .is_none_or(|window| served.window(window).is_none())
    {
        served.focused_window = served.windows.first().map(|window| window.id);
    }

    let worked_in = shown
        .focused_window
        .and_then(|window| shown.window(window))
        .or_else(|| shown.windows.first());
    let mut matched = false;
    for window in &mut served.windows {
        if let Some(same) = shown.window(window.id) {
            keep_local(window, same);
            matched = true;
        }
    }
    if !matched && let (Some(first), Some(from)) = (served.windows.first_mut(), worked_in) {
        first.dock = from.dock.clone();
    }

    validate(&served)?;
    Ok(served)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{WindowId, WorkspaceId};
    use crate::model::{Area, Bounds, Scratch};

    fn backend() -> BackendId {
        BackendId("server".to_owned())
    }

    fn workspace(name: &str, window: u32) -> Workspace {
        let mut workspace = Workspace::new(WorkspaceId::new(1).unwrap(), name);
        workspace.windows.push(Window::new(WindowId::new(window)));
        workspace.next_id = window + 1;
        workspace
    }

    fn bounds() -> Bounds {
        Bounds {
            x: 10,
            y: 20,
            width: 800,
            height: 600,
            maximised: false,
            fullscreen: false,
        }
    }

    #[test]
    fn a_served_window_keeps_the_place_and_panels_of_the_window_it_replaces() {
        let mut shown = workspace("mine", 1);
        shown.windows[0].bounds = Some(bounds());
        shown.windows[0].dock.area_mut(Area::Left).collapsed = true;

        let made = mirror(&shown, workspace("theirs", 1), &backend()).unwrap();

        assert_eq!(made.name, "theirs");
        assert_eq!(made.backend, Some(backend()));
        assert_eq!(made.windows[0].bounds, Some(bounds()));
        assert_eq!(made.windows[0].dock, shown.windows[0].dock);
        assert_eq!(made.focused_window, Some(made.windows[0].id));
    }

    #[test]
    fn a_window_with_another_id_takes_the_panels_and_not_the_place() {
        let mut shown = workspace("mine", 1);
        shown.windows[0].bounds = Some(bounds());
        shown.windows[0].dock.area_mut(Area::Left).collapsed = true;

        let made = mirror(&shown, workspace("theirs", 7), &backend()).unwrap();

        assert_eq!(made.windows[0].bounds, None);
        assert_eq!(made.windows[0].dock, shown.windows[0].dock);
    }

    #[test]
    fn what_is_this_applications_own_is_not_taken_from_a_server() {
        let shown = workspace("mine", 1);
        let mut served = workspace("theirs", 1);
        served.scratch = Some(Scratch {
            pane: crate::ids::PaneId::new(1),
            parked: None,
        });
        served.focused_window = Some(WindowId::new(99));

        let made = mirror(&shown, served, &backend()).unwrap();

        assert_eq!(made.scratch, None);
        assert_eq!(made.focused_window, Some(made.windows[0].id));
    }

    #[test]
    fn a_served_layout_that_is_not_sound_is_refused() {
        let shown = workspace("mine", 1);
        let mut served = workspace("theirs", 1);
        // Two windows with one id.
        served.windows.push(Window::new(WindowId::new(1)));

        assert!(mirror(&shown, served, &backend()).is_err());
    }

    struct NoSessions;

    #[async_trait::async_trait]
    impl crate::Spawner for NoSessions {
        async fn spawn(
            &self,
            _request: &crate::SpawnRequest,
        ) -> Result<crate::Started, crate::SpawnFailure> {
            unreachable!("nothing here makes a pane")
        }

        async fn discard(&self, _started: &crate::Started) {}
    }

    #[tokio::test]
    async fn a_copy_follows_its_owner_by_what_differs() {
        use crate::{LayoutAuthority, LayoutChange, LayoutEvent, LocalLayout};

        let shown = workspace("mine", 1);
        let made = mirror(&shown, workspace("theirs", 1), &backend()).unwrap();
        let layout = LocalLayout::new(made, 1, std::sync::Arc::new(NoSessions));
        let mut changes = layout.changes();

        let after = layout
            .update(|shown| mirror(shown, workspace("renamed", 1), &backend()).ok())
            .unwrap();
        assert_eq!((after.epoch, after.revision), (1, 1));
        assert_eq!(
            changes.next().await,
            Some(LayoutEvent::Changed {
                epoch: 1,
                revision: 1,
                changes: vec![LayoutChange::WorkspaceName {
                    name: "renamed".to_owned()
                }],
            })
        );

        // The same again changes nothing, and a refusal leaves everything as it was.
        let same = layout
            .update(|shown| mirror(shown, workspace("renamed", 1), &backend()).ok())
            .unwrap();
        assert_eq!(same.revision, 1);
        assert!(layout.update(|_| None).is_none());
        assert_eq!(layout.workspace().name, "renamed");
    }

    #[test]
    fn only_what_is_about_windows_and_panels_is_local() {
        let window = WindowId::new(1);
        assert!(Intent::WindowFocus { window }.is_window_local());
        assert!(
            Intent::PanelToggle {
                window,
                area: Area::Left,
                on: None
            }
            .is_window_local()
        );
        assert!(!Intent::WindowClose { window }.is_window_local());
        assert!(!Intent::ScratchHide {}.is_window_local());
    }
}
