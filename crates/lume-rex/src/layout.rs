//! A server's view of a session, as a workspace of this application.
//!
//! A Rex session has windows, each a tab. A window has layers: the first holds its
//! layout, a tree of splits with a block at each leaf, and the others float over it. A
//! workspace says the same in the application's own terms, and [`translate`] is the one
//! place that turns the one into the other:
//!
//! - The session is a workspace with one window, numbered 1.
//! - Each of the session's windows is a tab of it, in the server's order.
//! - A layout is the tab's tree. A server's split has two sides; splits of one
//!   direction directly inside each other are joined into one split of several
//!   children, which is the one form a tree has here.
//! - A terminal block is a terminal pane. Any other block holds its place and says
//!   what it is.
//! - The layers that float over the window on show are the floating panes. The
//!   application's floating panes belong to a window and a server's to a tab, so those
//!   of the other tabs are not shown until their tab is.
//!
//! A view is not trusted. One that is not a layout (an id used twice, a node that is
//! neither a block nor a split) is refused, and so is one with more in it than the
//! application shows. Everything else is brought into bounds: shares, rectangles,
//! names, and a focus or zoom that names nothing.

use std::collections::{HashMap, HashSet};

use lume_layout::tuning::{
    FLOAT_MIN, FLOATING_MAX, SHARE_MIN, SPLIT_CHILDREN_MAX, TAB_PANES_MAX, TABS_MAX,
    TITLE_MAX_CHARS, TREE_DEPTH_MAX, WORKSPACE_NAME_MAX_CHARS,
};
use lume_layout::{
    Axis, Floating, Minimised, Node, Pane, PaneContent, PaneId, Split, SplitId, Tab, TabId,
    TerminalPane, UNITS, UnitRect, Window, WindowId, Workspace, WorkspaceId,
};
use lume_proto::{BackendId, SessionId};

use crate::api::{View, ViewBlock, ViewLayer, ViewNode, ViewWindow};
use crate::error::RexError;
use crate::tuning::SPLIT_DEPTH_MAX;

/// The number of the one window a server's layout is shown in. It is the number the
/// application gives its own first window, so the window on show is kept.
pub const WINDOW: u32 = 1;

/// The kind of layer that holds a window's layout.
const TILED: &str = "tiled";
/// The state of a layer that is put away.
const MINIMIZED: &str = "minimized";
/// How a server writes the direction of a split whose sides are one above the other.
/// Any other direction is side by side.
const VERTICAL: &str = "vertical";

/// The numbers the application knows a server's windows, splits and blocks by.
///
/// A thing keeps its number for as long as views name it. Numbers only go up, so none
/// names two things.
#[derive(Debug)]
pub struct LayoutIds {
    by_name: HashMap<String, u32>,
    last: u32,
}

impl Default for LayoutIds {
    fn default() -> Self {
        Self {
            by_name: HashMap::new(),
            last: WINDOW,
        }
    }
}

impl LayoutIds {
    /// The number of something a view named, if the last view shown named it.
    pub fn known(&self, name: &str) -> Option<u32> {
        self.by_name.get(name).copied()
    }

    /// The number the next new thing is given.
    pub fn next(&self) -> u32 {
        self.last.saturating_add(1)
    }

    fn of(&mut self, name: &str, added: &mut Vec<String>) -> Result<u32, RexError> {
        if let Some(number) = self.by_name.get(name) {
            return Ok(*number);
        }
        // The last number is kept free: a workspace's next id has to be above every
        // id in it.
        if self.last >= u32::MAX - 1 {
            return Err(RexError::Protocol(
                "a server's layout has used up every number there is".to_owned(),
            ));
        }
        self.last += 1;
        self.by_name.insert(name.to_owned(), self.last);
        added.push(name.to_owned());
        Ok(self.last)
    }
}

/// A server's split that was joined into a split of several children: which of the
/// children are on its first side and which on its second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Joint {
    /// The first child of its first side.
    pub from: usize,
    /// The first child of its second side.
    pub mid: usize,
    /// One past the last child of its second side.
    pub to: usize,
}

/// The layer a floating pane is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    pub layer_id: String,
    /// Whether the pane is the only one in the layer. A layer is moved, sized and put
    /// away as a whole.
    pub alone: bool,
}

/// What the server calls the things of a workspace made from its view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Names {
    /// The session the view is of.
    pub session: String,
    /// The block each pane shows. A pane that only holds the place of an empty layout
    /// has none.
    pub panes: HashMap<PaneId, String>,
    /// The window each tab is.
    pub tabs: HashMap<TabId, String>,
    /// The server's splits each split was joined from, each after the splits inside it.
    pub splits: HashMap<SplitId, Vec<Joint>>,
    /// The layer of each floating pane.
    pub layers: HashMap<PaneId, Layer>,
}

/// A view as a workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub workspace: Workspace,
    pub names: Names,
}

/// Turns a view into a workspace.
///
/// `ids` gives the things of the view their numbers and keeps them from one view to
/// the next. `session_of` gives the number of a terminal block's session. A view that
/// is refused leaves `ids` as it was.
pub fn translate(
    view: &View,
    ids: &mut LayoutIds,
    backend: &BackendId,
    session_of: &mut dyn FnMut(&str) -> SessionId,
) -> Result<Shown, RexError> {
    let before = ids.last;
    let mut maker = Maker {
        ids,
        backend,
        session_of,
        used: HashSet::new(),
        added: Vec::new(),
        names: Names::default(),
    };
    let made = maker.workspace(view);
    let Maker {
        ids,
        used,
        added,
        names,
        ..
    } = maker;
    match made {
        Ok(mut workspace) => {
            ids.by_name.retain(|_, number| used.contains(number));
            workspace.next_id = ids.next();
            Ok(Shown { workspace, names })
        }
        Err(error) => {
            for name in added {
                ids.by_name.remove(&name);
            }
            ids.last = before;
            Err(error)
        }
    }
}

fn unsound(what: &str) -> RexError {
    RexError::Protocol(format!("a server sent a layout in which {what}"))
}

fn too_much(what: &str, most: usize) -> RexError {
    RexError::Protocol(format!(
        "a server's layout has more {what} than can be shown (at most {most})"
    ))
}

/// A part of a split that is being joined: its share of the whole split, the node, and
/// how deep in the server's tree the node is.
type Part<'v> = (f64, &'v ViewNode, usize);

struct Maker<'a> {
    ids: &'a mut LayoutIds,
    backend: &'a BackendId,
    session_of: &'a mut dyn FnMut(&str) -> SessionId,
    /// The numbers this view has used, each for one thing.
    used: HashSet<u32>,
    /// The names that were given a number for this view.
    added: Vec<String>,
    names: Names,
}

impl Maker<'_> {
    /// The number of a thing the view names, which nothing else in the view may have.
    fn claim(&mut self, name: &str, what: &str) -> Result<u32, RexError> {
        let number = self.ids.of(name, &mut self.added)?;
        if self.used.insert(number) {
            Ok(number)
        } else {
            Err(unsound(&format!("{what} has an id that is used twice")))
        }
    }

    fn workspace(&mut self, view: &View) -> Result<Workspace, RexError> {
        let session = view.session_id.as_deref().unwrap_or_default();
        self.names.session = session.to_owned();
        if view.windows.len() > TABS_MAX {
            return Err(too_much("windows", TABS_MAX));
        }

        let active = view
            .active_window_id
            .as_deref()
            .and_then(|id| {
                view.windows
                    .iter()
                    .position(|window| window.window_id.as_deref() == Some(id))
            })
            .or(if view.windows.is_empty() {
                None
            } else {
                Some(0)
            });

        let mut window = Window::new(WindowId::new(WINDOW));
        let mut wanted = None;
        for (index, served) in view.windows.iter().enumerate() {
            let on_show = active == Some(index);
            let blocks = blocks_of(served);
            let tab = self.tab(served, &blocks)?;
            if on_show {
                window.centre.active = Some(tab.id);
                window.floating = self.floating(served, &blocks)?;
                wanted = served
                    .focused_block_id
                    .as_deref()
                    .and_then(|block| self.ids.known(block))
                    .map(PaneId::new);
            }
            window.centre.tabs.push(tab);
        }

        // The keyboard is where the server says, if that is somewhere it can be here.
        let focusable = window.focusable();
        window.focus = wanted
            .filter(|pane| focusable.contains(pane))
            .or_else(|| focusable.first().copied());
        window.history = window.focus.into_iter().collect();
        if let Some(focus) = window.focus
            && let Some(tab) = window.centre.active_tab_mut()
            && tab.contains(focus)
            && tab.zoomed.is_some_and(|zoomed| zoomed != focus)
        {
            // A server ends a zoom when the focus moves to another pane of the tab.
            tab.zoomed = None;
        }

        let mut workspace = Workspace::new(workspace_id(session), workspace_name(&view.label));
        workspace.focused_window = Some(window.id);
        workspace.windows.push(window);
        Ok(workspace)
    }

    fn tab(
        &mut self,
        served: &ViewWindow,
        blocks: &HashMap<&str, &ViewBlock>,
    ) -> Result<Tab, RexError> {
        let Some(window_id) = served.window_id.as_deref() else {
            return Err(unsound("a window has no id"));
        };
        let id = TabId::new(self.claim(window_id, "a window")?);
        self.names.tabs.insert(id, window_id.to_owned());

        let root = match tiled(served).and_then(|layer| layer.layout.as_ref()) {
            Some(layout) => self.tree(layout, blocks, 0)?,
            // Nothing is tiled: every block of the window floats. A tab has a tree, so
            // a pane that shows nothing holds its place.
            None => Node::Pane(Pane {
                id: PaneId::new(self.claim(&format!("empty:{window_id}"), "a window")?),
                title: None,
                icon: None,
                content: PaneContent::Placeholder {
                    label: String::new(),
                    kind: String::new(),
                },
            }),
        };
        if root.pane_ids().len() > TAB_PANES_MAX {
            return Err(too_much("blocks in a window", TAB_PANES_MAX));
        }
        if root.depth() > TREE_DEPTH_MAX {
            return Err(too_much("splits inside each other", TREE_DEPTH_MAX));
        }

        let within = |block: &Option<String>| {
            block
                .as_deref()
                .and_then(|block| self.ids.known(block))
                .map(PaneId::new)
                .filter(|pane| root.pane(*pane).is_some())
        };
        let zoomed = within(&served.zoomed_block_id);
        let last_focus = within(&served.focused_block_id);
        let default = format!("Window {}", tail(window_id));
        Ok(Tab {
            id,
            title: named(&served.label).filter(|title| *title != default),
            root,
            zoomed,
            last_focus,
            color: None,
            locked: false,
            muted: false,
            group: None,
        })
    }

    fn tree(
        &mut self,
        node: &ViewNode,
        blocks: &HashMap<&str, &ViewBlock>,
        depth: usize,
    ) -> Result<Node, RexError> {
        let Some(split) = &node.split else {
            return match node.block_id.as_deref() {
                Some(block) => Ok(Node::Pane(self.pane(block, blocks)?)),
                None => Err(unsound("a part of a layout is neither a block nor a split")),
            };
        };
        let Some(split_id) = split.split_id.as_deref() else {
            return Err(unsound("a split has no id"));
        };
        let id = SplitId::new(self.claim(split_id, "a split")?);
        let axis = axis_of(&split.direction);

        let mut parts: Vec<Part<'_>> = Vec::new();
        let mut joints = Vec::new();
        join(node, axis, 1.0, depth, &mut parts, &mut joints)?;

        let weights: Vec<f64> = parts.iter().map(|(weight, _, _)| *weight).collect();
        let mut children = Vec::with_capacity(parts.len());
        for (_, part, depth) in parts {
            children.push(self.tree(part, blocks, depth)?);
        }
        self.names.splits.insert(id, joints);
        Ok(Node::Split(Split {
            id,
            axis,
            ratios: fit(&weights),
            children,
        }))
    }

    fn pane(
        &mut self,
        block_id: &str,
        blocks: &HashMap<&str, &ViewBlock>,
    ) -> Result<Pane, RexError> {
        let id = PaneId::new(self.claim(block_id, "a block")?);
        self.names.panes.insert(id, block_id.to_owned());
        let (title, content) = match blocks.get(block_id) {
            Some(block) if block.is_terminal() => {
                let short = block.flavor.rsplit('.').next().unwrap_or_default();
                let default = format!("Block {}", tail(block_id));
                (
                    // A block starts out named after its kind, which is no name a
                    // person gave it.
                    named(&block.label).filter(|title| title != short && *title != default),
                    PaneContent::Terminal(TerminalPane {
                        backend: Some(self.backend.clone()),
                        session: Some((self.session_of)(block_id)),
                        ..TerminalPane::default()
                    }),
                )
            }
            Some(block) => (
                None,
                PaneContent::Placeholder {
                    label: named(&block.label).unwrap_or_default(),
                    kind: named(&block.creator_name).unwrap_or_default(),
                },
            ),
            // The layout names a block the layer does not list.
            None => (
                None,
                PaneContent::Placeholder {
                    label: String::new(),
                    kind: String::new(),
                },
            ),
        };
        Ok(Pane {
            id,
            title,
            icon: None,
            content,
        })
    }

    /// The blocks that float over a window, from the back to the front.
    fn floating(
        &mut self,
        served: &ViewWindow,
        blocks: &HashMap<&str, &ViewBlock>,
    ) -> Result<Vec<Floating>, RexError> {
        let holds_layout = tiled_index(served);
        let mut floating = Vec::new();
        for (index, layer) in served.layers.iter().enumerate() {
            if Some(index) == holds_layout {
                continue;
            }
            let Some(layout) = &layer.layout else {
                continue;
            };
            let mut leaves = Vec::new();
            leaves_of(layout, 0, &mut leaves)?;
            let minimised =
                (layer.state.as_deref() == Some(MINIMIZED)).then_some(Minimised::Floating);
            for (block, rect) in &leaves {
                if floating.len() == FLOATING_MAX {
                    return Err(too_much("floating blocks in a window", FLOATING_MAX));
                }
                let pane = self.pane(block, blocks)?;
                if let Some(layer_id) = &layer.layer_id {
                    self.names.layers.insert(
                        pane.id,
                        Layer {
                            layer_id: layer_id.clone(),
                            alone: leaves.len() == 1,
                        },
                    );
                }
                floating.push(Floating {
                    pane,
                    rect: unit_rect(rect),
                    minimised,
                });
            }
        }
        Ok(floating)
    }
}

/// Where the layer that holds a window's layout is among its layers.
fn tiled_index(window: &ViewWindow) -> Option<usize> {
    window.layers.iter().position(|layer| layer.kind == TILED)
}

fn tiled(window: &ViewWindow) -> Option<&ViewLayer> {
    tiled_index(window).map(|index| &window.layers[index])
}

/// What a window's layers say of its blocks, by block.
fn blocks_of(window: &ViewWindow) -> HashMap<&str, &ViewBlock> {
    window
        .layers
        .iter()
        .flat_map(|layer| &layer.blocks)
        .filter_map(|block| Some((block.block_id.as_deref()?, block)))
        .collect()
}

fn axis_of(direction: &str) -> Axis {
    if direction == VERTICAL {
        Axis::Column
    } else {
        Axis::Row
    }
}

fn too_deep() -> RexError {
    unsound("splits nest deeper than any server nests them")
}

/// Gathers the parts of a split and of the splits of the same direction directly
/// inside it, in the order they are laid out, each with its share of the whole.
fn join<'v>(
    node: &'v ViewNode,
    axis: Axis,
    weight: f64,
    depth: usize,
    parts: &mut Vec<Part<'v>>,
    joints: &mut Vec<Joint>,
) -> Result<(), RexError> {
    if depth > SPLIT_DEPTH_MAX {
        return Err(too_deep());
    }
    match &node.split {
        Some(split) if axis_of(&split.direction) == axis => {
            let first = if split.ratio.is_finite() {
                split.ratio.clamp(0.0, 1.0)
            } else {
                0.5
            };
            let from = parts.len();
            join(
                &split.before,
                axis,
                weight * first,
                depth + 1,
                parts,
                joints,
            )?;
            let mid = parts.len();
            join(
                &split.after,
                axis,
                weight * (1.0 - first),
                depth + 1,
                parts,
                joints,
            )?;
            joints.push(Joint {
                from,
                mid,
                to: parts.len(),
            });
        }
        _ => {
            if parts.len() == SPLIT_CHILDREN_MAX {
                return Err(too_much("blocks in a row", SPLIT_CHILDREN_MAX));
            }
            parts.push((weight, node, depth));
        }
    }
    Ok(())
}

/// The blocks of a layout and where each is in its window, in the order they are laid
/// out.
fn leaves_of<'v>(
    node: &'v ViewNode,
    depth: usize,
    leaves: &mut Vec<(&'v str, [f64; 4])>,
) -> Result<(), RexError> {
    if depth > SPLIT_DEPTH_MAX {
        return Err(too_deep());
    }
    match (&node.split, node.block_id.as_deref()) {
        (Some(split), _) => {
            leaves_of(&split.before, depth + 1, leaves)?;
            leaves_of(&split.after, depth + 1, leaves)
        }
        (None, Some(block)) => {
            leaves.push((block, node.rect));
            Ok(())
        }
        (None, None) => Err(unsound("a part of a layout is neither a block nor a split")),
    }
}

/// A fraction of a whole in units of it.
fn units(fraction: f64) -> u32 {
    if !fraction.is_finite() {
        return 0;
    }
    // Between nothing and the whole, so the number fits.
    (fraction.clamp(0.0, 1.0) * f64::from(UNITS)).round() as u32
}

/// The shares of a split's children, from what each has of the whole: adding up to a
/// whole exactly, none smaller than a child may be. The largest give way to the ones
/// that are raised.
fn fit(weights: &[f64]) -> Vec<u32> {
    let total: f64 = weights.iter().sum();
    let mut shares: Vec<u32> = weights
        .iter()
        .map(|weight| {
            if total > 0.0 {
                units(weight / total)
            } else {
                0
            }
        })
        .map(|share| share.max(SHARE_MIN))
        .collect();
    loop {
        let sum: u64 = shares.iter().map(|share| u64::from(*share)).sum();
        let Some(largest) = (0..shares.len()).max_by_key(|index| shares[*index]) else {
            break;
        };
        let whole = u64::from(UNITS);
        if sum > whole {
            let room = shares[largest] - SHARE_MIN;
            if room == 0 {
                // More children than fit at the smallest share: a split never has so
                // many.
                break;
            }
            // No more than `room`, which is a share.
            shares[largest] -= u64::from(room).min(sum - whole) as u32;
        } else if sum < whole {
            // The sum is short by less than a whole.
            shares[largest] += (whole - sum) as u32;
        } else {
            break;
        }
    }
    shares
}

/// A rectangle a floating pane may have, from a block's place in its window.
fn unit_rect(rect: &[f64; 4]) -> UnitRect {
    let w = units(rect[2]).clamp(FLOAT_MIN, UNITS);
    let h = units(rect[3]).clamp(FLOAT_MIN, UNITS);
    UnitRect {
        x: units(rect[0]).min(UNITS - w),
        y: units(rect[1]).min(UNITS - h),
        w,
        h,
    }
}

/// A name as it may be shown: without control characters, trimmed, no longer than a
/// name may be. Nothing if nothing is left.
fn named(text: &str) -> Option<String> {
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    let name: String = clean.trim().chars().take(TITLE_MAX_CHARS).collect();
    let name = name.trim_end();
    (!name.is_empty()).then(|| name.to_owned())
}

fn workspace_name(label: &str) -> String {
    named(label)
        .map(|name| {
            let name: String = name.chars().take(WORKSPACE_NAME_MAX_CHARS).collect();
            name.trim_end().to_owned()
        })
        .unwrap_or_else(|| "Rex".to_owned())
}

/// The end of an id, which a server names a thing after until a person names it.
fn tail(id: &str) -> &str {
    let start = id.char_indices().rev().nth(7).map_or(0, |(index, _)| index);
    &id[start..]
}

/// A number for a session that is the same each time the session is shown.
fn workspace_id(session: &str) -> WorkspaceId {
    // FNV-1a.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in session.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    WorkspaceId::new(hash)
        .or(WorkspaceId::new(1))
        .expect("one is not zero")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shares_add_up_to_a_whole_and_none_is_too_small() {
        for weights in [
            vec![0.5, 0.5],
            vec![1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0],
            vec![0.999, 0.001],
            vec![0.0, 0.0, 0.0],
            vec![1e-300, 1.0, 1e-300, 1e-300],
            vec![0.05; 20],
            vec![1.0; 7],
        ] {
            let shares = fit(&weights);
            assert_eq!(shares.len(), weights.len());
            assert_eq!(shares.iter().sum::<u32>(), UNITS, "{weights:?}");
            assert!(
                shares.iter().all(|share| *share >= SHARE_MIN),
                "{weights:?}"
            );
        }
        assert_eq!(fit(&[0.25, 0.75]), vec![UNITS / 4, UNITS / 4 * 3]);
        assert_eq!(fit(&[0.999, 0.001]), vec![UNITS - SHARE_MIN, SHARE_MIN]);
    }

    #[test]
    fn a_floating_rectangle_is_kept_inside_its_window() {
        assert_eq!(
            unit_rect(&[0.25, 0.5, 0.5, 0.25]),
            UnitRect {
                x: UNITS / 4,
                y: UNITS / 2,
                w: UNITS / 2,
                h: UNITS / 4
            }
        );
        let far = unit_rect(&[0.99, -3.0, 0.0, 9.0]);
        assert_eq!((far.w, far.h), (FLOAT_MIN, UNITS));
        assert_eq!((far.x, far.y), (UNITS - FLOAT_MIN, 0));
        let odd = unit_rect(&[f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.5]);
        assert_eq!((odd.x, odd.y, odd.w), (0, 0, FLOAT_MIN));
    }

    #[test]
    fn names_are_cleaned_and_cut() {
        assert_eq!(named("  build\u{7}\n "), Some("build".to_owned()));
        assert_eq!(named(" \t\u{1b} "), None);
        assert_eq!(
            named(&"é".repeat(400)).unwrap().chars().count(),
            TITLE_MAX_CHARS
        );
        assert_eq!(workspace_name(""), "Rex");
        assert_eq!(
            workspace_name(&"w".repeat(100)).len(),
            WORKSPACE_NAME_MAX_CHARS
        );
        assert_eq!(tail("window:0123-ea8b4645"), "ea8b4645");
        assert_eq!(tail("abc"), "abc");
        assert_eq!(tail("ééééééééé"), "éééééééé");
    }

    #[test]
    fn a_session_has_the_same_number_each_time() {
        assert_eq!(workspace_id("session:a"), workspace_id("session:a"));
        assert_ne!(workspace_id("session:a"), workspace_id("session:b"));
    }
}
