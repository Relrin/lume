//! What the application asks of a layout, as what is asked of a server.
//!
//! An intent names panes, tabs and splits by the application's numbers and means them
//! as the copy of the server's layout shows them. [`plan`] turns one into the calls
//! that do the same on the server, naming blocks and windows as the server does. It
//! asks nothing itself: what it gives is carried out by the authority.
//!
//! What a server has no way to do is refused here, before anything is asked.

use lume_layout::tuning::SHARE_MIN;
use lume_layout::{
    Axis, Dir, FocusTarget, Intent, IntentError, LayoutError, NewContent, Node, PaneId, Place,
    SpawnRequest, UNITS, Workspace,
};
use lume_proto::Launcher;
use serde_json::{Map, Value, json};

use crate::layout::Names;

/// The flavour of block that is a terminal running a shell or a command.
const SHELL_FLAVOR: &str = "com.superlogical.terminal.shell";

/// How far a split's first side's share may differ from the one asked for and still
/// be left alone: less than a server and the application round between them.
const SAME_SHARE: f64 = 2.0 / UNITS as f64;

/// One call to a server.
#[derive(Debug, PartialEq)]
pub(crate) struct Call {
    pub method: &'static str,
    pub args: Value,
}

/// What an intent makes that the application is told the number of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Makes {
    Nothing,
    /// A pane: the first block the last call says it made.
    Pane,
}

/// The calls that carry out an intent, in order.
#[derive(Debug, PartialEq)]
pub(crate) struct Plan {
    pub calls: Vec<Call>,
    pub makes: Makes,
}

impl Plan {
    fn one(method: &'static str, args: Value) -> Self {
        Self {
            calls: vec![Call { method, args }],
            makes: Makes::Nothing,
        }
    }
}

fn unsupported(what: &str) -> LayoutError {
    LayoutError::Unsupported(format!("a Rex server's layout cannot {what}"))
}

/// The names an intent's numbers are looked up in.
struct Asked<'a> {
    workspace: &'a Workspace,
    names: &'a Names,
}

impl Asked<'_> {
    /// The call's arguments, with the session they are about.
    fn of_session(&self, mut args: Value) -> Value {
        args["session_id"] = json!(self.names.session);
        args
    }

    /// The block a pane shows.
    fn block(&self, pane: PaneId) -> Result<&str, LayoutError> {
        match self.names.panes.get(&pane) {
            Some(block) => Ok(block),
            None if self.workspace.pane(pane).is_some() => Err(unsupported(
                "change a pane that only holds the place of a window with nothing tiled",
            )),
            None => Err(IntentError::NoSuchPane(pane).into()),
        }
    }

    /// The block of a pane that is in a tab's tree.
    fn tiled(&self, pane: PaneId) -> Result<&str, LayoutError> {
        let block = self.block(pane)?;
        if self.names.layers.contains_key(&pane) {
            return Err(IntentError::Floating(pane).into());
        }
        Ok(block)
    }
}

/// The direction of a server's split that puts something at an edge, and the side of
/// the split it is then on.
fn towards(edge: Dir) -> (&'static str, &'static str) {
    match edge {
        Dir::Left => ("horizontal", "before"),
        Dir::Right => ("horizontal", "after"),
        Dir::Up => ("vertical", "before"),
        Dir::Down => ("vertical", "after"),
    }
}

fn direction_of(axis: Axis) -> &'static str {
    match axis {
        Axis::Row => "horizontal",
        Axis::Column => "vertical",
    }
}

/// A share of a split as a server takes it: a fraction that is neither nothing nor
/// the whole.
fn fraction(units: f64) -> f64 {
    let least = f64::from(SHARE_MIN) / f64::from(UNITS);
    (units / f64::from(UNITS)).clamp(least, 1.0 - least)
}

/// What a server is asked to start for a new terminal pane.
fn new_block(request: &SpawnRequest) -> Result<Value, LayoutError> {
    if request.session.is_some() {
        return Err(unsupported(
            "show a session that is already running in a new pane",
        ));
    }
    let mut options = Map::new();
    match &request.launcher {
        None => {}
        // The program is one on the server's machine, started as it is.
        Some(Launcher::Shell {
            program,
            args,
            login,
        }) => {
            let mut command = vec![program.clone()];
            command.extend(args.iter().cloned());
            options.insert("shell".to_owned(), json!("none"));
            options.insert("command".to_owned(), json!(command));
            options.insert("login".to_owned(), json!(login));
        }
        Some(_) => {
            return Err(unsupported(
                "start anything but a program of the server's own machine",
            ));
        }
    }
    if let Some(cwd) = request.cwd.as_deref().filter(|cwd| !cwd.is_empty()) {
        options.insert("cwd".to_owned(), json!(cwd));
    }
    if let Some(size) = request.size {
        options.insert(
            "size".to_owned(),
            json!({ "columns": size.cols(), "rows": size.rows() }),
        );
    }
    let mut block = json!({ "flavor": SHELL_FLAVOR });
    if !options.is_empty() {
        block["options"] = Value::Object(options);
    }
    Ok(json!({ "block": block }))
}

/// The layout for the content of a new pane.
pub(crate) fn new_layout(content: &NewContent) -> Result<Value, LayoutError> {
    match content {
        NewContent::Terminal(request) => new_block(request),
        NewContent::PluginView { .. } => Err(unsupported("hold a plugin's view")),
    }
}

/// The first and the last pane under a node, as they are laid out.
fn ends(node: &Node) -> Option<(PaneId, PaneId)> {
    let panes = node.pane_ids();
    Some((*panes.first()?, *panes.last()?))
}

/// The calls that give a split the shares asked for: one for each of the server's
/// splits it was joined from whose own share changes.
fn resize(
    asked: &Asked<'_>,
    split: lume_layout::SplitId,
    ratios: &[u32],
) -> Result<Vec<Call>, LayoutError> {
    let found = asked
        .workspace
        .windows
        .iter()
        .flat_map(|window| &window.centre.tabs)
        .find_map(|tab| tab.root.split(split));
    let (Some(found), Some(joints)) = (found, asked.names.splits.get(&split)) else {
        return Err(IntentError::NoSuchSplit(split).into());
    };
    if ratios.len() != found.children.len() {
        return Err(IntentError::BadRatios("there is not one for each child").into());
    }
    if ratios.contains(&0) {
        return Err(IntentError::BadRatios("a child would have nothing").into());
    }
    let share = |of: &[u32], from: usize, to: usize| -> f64 {
        of[from..to].iter().map(|ratio| f64::from(*ratio)).sum()
    };

    let mut calls = Vec::new();
    // From the outermost of the server's splits inwards.
    for joint in joints.iter().rev() {
        if joint.from >= joint.mid || joint.mid >= joint.to || joint.to > ratios.len() {
            continue;
        }
        let first_of =
            |of: &[u32]| share(of, joint.from, joint.mid) / share(of, joint.from, joint.to);
        let wanted = first_of(ratios);
        if (wanted - first_of(&found.ratios)).abs() <= SAME_SHARE {
            continue;
        }
        // A server finds the split to move by a block on each side of it: the two
        // that meet at the border.
        let before = ends(&found.children[joint.mid - 1]).map(|(_, last)| last);
        let after = ends(&found.children[joint.mid]).map(|(first, _)| first);
        let (Some(before), Some(after)) = (before, after) else {
            continue;
        };
        calls.push(Call {
            method: "session.resize_pane",
            args: asked.of_session(json!({
                "direction": direction_of(found.axis),
                "before_block_id": asked.block(before)?,
                "after_block_id": asked.block(after)?,
                "ratio": fraction(wanted * f64::from(UNITS)),
            })),
        });
    }
    Ok(calls)
}

/// The calls that do on a server what an intent asks, in a layout shown as
/// `workspace` whose things the server calls `names`.
pub(crate) fn plan(
    intent: &Intent,
    workspace: &Workspace,
    names: &Names,
) -> Result<Plan, LayoutError> {
    let asked = Asked { workspace, names };
    let plan = match intent {
        Intent::Split {
            pane,
            dir,
            ratio,
            content,
        } => {
            let (direction, side) = towards(*dir);
            // A server is told the first side's share; the application says the new
            // pane's.
            let new = f64::from(*ratio);
            let first = if side == "before" {
                new
            } else {
                f64::from(UNITS) - new
            };
            Plan {
                calls: vec![Call {
                    method: "session.new_split",
                    args: asked.of_session(json!({
                        "anchor_id": asked.tiled(*pane)?,
                        "direction": direction,
                        "side": side,
                        "ratio": fraction(first),
                        "layout": new_layout(content)?,
                        "focus": true,
                    })),
                }],
                makes: Makes::Pane,
            }
        }
        Intent::Close { pane } => Plan::one(
            "block.close",
            asked.of_session(json!({ "block_id": asked.block(*pane)? })),
        ),
        Intent::Focus { target } => match target {
            FocusTarget::Pane { pane } => Plan::one(
                "session.focus_block",
                asked.of_session(json!({ "block_id": asked.block(*pane)? })),
            ),
            FocusTarget::Next { .. } => Plan::one(
                "session.focus_next_pane",
                asked.of_session(json!({ "wrap": true })),
            ),
            FocusTarget::Previous { .. } => Plan::one(
                "session.focus_previous_pane",
                asked.of_session(json!({ "wrap": true })),
            ),
            FocusTarget::Back { .. } => {
                return Err(unsupported(
                    "go back to the pane that had the keyboard before",
                ));
            }
        },
        Intent::FocusDir { dir, .. } => {
            let direction = match dir {
                Dir::Left => "left",
                Dir::Right => "right",
                Dir::Up => "up",
                Dir::Down => "down",
            };
            Plan::one(
                "session.focus_direction",
                asked.of_session(json!({ "direction": direction })),
            )
        }
        Intent::Swap { a, b } => {
            if a == b {
                return Err(IntentError::SamePane(*a).into());
            }
            Plan::one(
                "session.swap_blocks",
                asked.of_session(json!({
                    "block_id": asked.block(*a)?,
                    "other_block_id": asked.block(*b)?,
                })),
            )
        }
        Intent::Move { pane, to } => match to {
            Place::Beside { pane: anchor, edge } => {
                if pane == anchor {
                    return Err(IntentError::SamePane(*pane).into());
                }
                let (direction, side) = towards(*edge);
                Plan::one(
                    "session.move_block",
                    asked.of_session(json!({
                        "block_id": asked.block(*pane)?,
                        "anchor_block_id": asked.tiled(*anchor)?,
                        "direction": direction,
                        "side": side,
                    })),
                )
            }
            Place::NewTab { .. } => {
                return Err(unsupported("move a pane into a tab of its own"));
            }
        },
        Intent::Resize { split, ratios } => Plan {
            calls: resize(&asked, *split, ratios)?,
            makes: Makes::Nothing,
        },
        Intent::Zoom { pane, on } => Plan::one(
            "session.zoom_block",
            asked.of_session(json!({ "block_id": asked.tiled(*pane)?, "zoom": on })),
        ),
        Intent::PaneRename { pane, title } => Plan::one(
            "session.set_block_label",
            asked.of_session(json!({
                "block_id": asked.block(*pane)?,
                // An empty name gives the block the server's own name for it again.
                "label": title.as_deref().unwrap_or_default(),
            })),
        ),
        _ => return Err(unsupported("do this yet")),
    };
    Ok(plan)
}
