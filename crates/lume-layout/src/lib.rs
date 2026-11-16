//! The layout of a workspace: its windows, their tabs, the splits and panes of each
//! tab, the panes that float, and the panels docked at a window's edges.
//!
//! The layout is a tree ([`Workspace`]) that changes in one way only: an [`Intent`] is
//! applied to it. [`apply`] is a plain function that either refuses the intent and
//! leaves the tree alone, or changes the tree, brings it back to its one proper form,
//! and says what changed as a list of [`LayoutChange`]s. A view keeps a copy of the
//! tree and follows it from those changes; [`Projection`] is that copy, written once
//! here so the two can be tested against each other.
//!
//! Something has to own the tree, order the intents and tell the views. That is a
//! [`LayoutAuthority`]. [`LocalLayout`] is the authority for workspaces this
//! application owns; a session backend that keeps its own arrangement of panes can
//! provide another.
//!
//! Nothing here knows pixels, windows of the operating system, or how a session is
//! started. Sizes are ratios, a window is an id, and a session is started by whoever
//! the authority was given as its [`Spawner`].

mod apply;
mod authority;
mod change;
mod diff;
mod error;
mod geometry;
mod ids;
mod intent;
mod model;
mod normalise;
mod persist;
mod projection;
pub mod tuning;
mod validate;

pub use apply::{Started, apply, check, needs};
pub use authority::{ChangeSource, ChangeStream, LayoutAuthority, LocalLayout, Spawner};
pub use change::{Applied, Created, LayoutChange, LayoutEvent, LayoutSnapshot};
pub use error::{IntentError, LayoutError, SpawnFailure};
pub use geometry::{neighbour, tab_rects};
pub use ids::{PaneId, PanelId, SplitId, TabId, WindowId, WorkspaceId};
pub use intent::{Dir, FocusTarget, Intent, NewContent, Place, SpawnRequest, TabTarget};
pub use model::{
    Area, Axis, Bounds, DockArea, DockAreas, Floating, Minimised, Node, Pane, PaneContent, Panel,
    PanelContent, Scratch, Slot, Split, Tab, TabGroup, TerminalPane, UnitRect, Window, Workspace,
};
pub use persist::{Loaded, Store, StoreError, WorkspaceFile, valid_name};
pub use projection::Projection;
pub use tuning::UNITS;
pub use validate::{Violation, is_normalised, validate};
