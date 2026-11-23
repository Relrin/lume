//! What the application asks of a layout, as what is asked of a server.
//!
//! An intent names panes, tabs and splits by the application's numbers and means them
//! as the copy of the server's layout shows them. [`plan`] turns one into the calls
//! that do the same on the server, naming blocks and windows as the server does. It
//! asks nothing itself: what it gives is carried out by the authority.
//!
//! What a server has no way to do is refused here, before anything is asked.

use lume_layout::tuning::{FLOAT_DEFAULT, SHARE_MIN};
use lume_layout::{
    Axis, Dir, FocusTarget, Intent, IntentError, LayoutError, NewContent, Node, PaneId, Place,
    SpawnRequest, TabGroup, TabId, TabTarget, UNITS, UnitRect, Workspace,
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
    /// An argument that is something the call before this one made: the argument's
    /// name, and the name the call before answers with it under.
    pub takes: Option<(&'static str, &'static str)>,
}

impl Call {
    fn new(method: &'static str, args: Value) -> Self {
        Self {
            method,
            args,
            takes: None,
        }
    }
}

/// What an intent makes that the application is told the number of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Makes {
    Nothing,
    /// A pane: the first block the first call says it made.
    Pane,
    /// A tab and the pane in it: the window and the first block the first call says
    /// it made.
    Tab,
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
            calls: vec![Call::new(method, args)],
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

/// The layout of the first window of a new session: one terminal that runs the shell
/// the server starts when it is told nothing else.
pub(crate) fn shell_layout() -> Value {
    json!({ "block": { "flavor": SHELL_FLAVOR } })
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
        calls.push(Call::new(
            "session.resize_pane",
            asked.of_session(json!({
                "direction": direction_of(found.axis),
                "before_block_id": asked.block(before)?,
                "after_block_id": asked.block(after)?,
                "ratio": fraction(wanted * f64::from(UNITS)),
            })),
        ));
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
                calls: vec![Call::new(
                    "session.new_split",
                    asked.of_session(json!({
                        "anchor_id": asked.tiled(*pane)?,
                        "direction": direction,
                        "side": side,
                        "ratio": fraction(first),
                        "layout": new_layout(content)?,
                        "focus": true,
                    })),
                )],
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
        Intent::TabNew {
            content,
            index,
            activate,
            group,
            ..
        } => {
            if group.is_some() {
                return Err(unsupported("gather tabs in groups"));
            }
            let mut calls = vec![Call::new(
                "session.new_window",
                asked.of_session(json!({ "layout": new_layout(content)?, "focus": activate })),
            )];
            // A server puts a new window after the last. Anywhere else it is moved to.
            let tabs = &asked.centre()?.tabs;
            if let Some(after) = index.and_then(|index| tabs.get(index as usize)) {
                calls.push(Call {
                    method: "session.move_window",
                    args: asked.of_session(json!({ "before_window_id": asked.window(after.id)? })),
                    takes: Some(("window_id", "window_id")),
                });
            }
            Plan {
                calls,
                makes: Makes::Tab,
            }
        }
        Intent::TabClose { tab } => Plan::one(
            "session.close_window",
            asked.of_session(json!({ "window_id": asked.window(*tab)? })),
        ),
        Intent::TabsClose { tabs } => {
            let mut calls = Vec::with_capacity(tabs.len());
            for tab in tabs {
                calls.push(Call::new(
                    "session.close_window",
                    asked.of_session(json!({ "window_id": asked.window(*tab)? })),
                ));
            }
            Plan {
                calls,
                makes: Makes::Nothing,
            }
        }
        Intent::TabMove { tab, index } => asked.place(*tab, *index)?,
        Intent::TabPlace { tab, index, group } => {
            if group.is_some() {
                return Err(unsupported("gather tabs in groups"));
            }
            asked.place(*tab, *index)?
        }
        Intent::TabRename { tab, title } => Plan::one(
            "session.set_window_label",
            asked.of_session(json!({
                "window_id": asked.window(*tab)?,
                // An empty name gives the window the server's own name for it again.
                "label": title.as_deref().unwrap_or_default(),
            })),
        ),
        Intent::TabActivate { target } => {
            let centre = asked.centre()?;
            let shown = centre.active.and_then(|tab| centre.index_of(tab));
            let count = centre.tabs.len();
            let tab = match target {
                TabTarget::Id { tab } => *tab,
                TabTarget::Index { index, .. } => match centre.tabs.get(*index as usize) {
                    Some(tab) => tab.id,
                    None => {
                        return Err(IntentError::BadIndex {
                            index: *index,
                            len: count,
                        }
                        .into());
                    }
                },
                // The one after the tab on show, or the one before, around the ends.
                TabTarget::Next { .. } | TabTarget::Previous { .. } => {
                    let Some(shown) = shown else {
                        return Err(unsupported("show a tab where there are none"));
                    };
                    let step = if matches!(target, TabTarget::Next { .. }) {
                        1
                    } else {
                        count - 1
                    };
                    centre.tabs[(shown + step) % count].id
                }
            };
            Plan::one(
                "session.focus_window",
                asked.of_session(json!({ "window_id": asked.window(tab)? })),
            )
        }
        Intent::Float { pane, rect } => match asked.names.layers.get(pane) {
            // It floats already: with a place given, that is where it goes.
            Some(_) => match rect {
                Some(rect) => asked.float_to(*pane, *rect)?,
                None => Plan {
                    calls: Vec::new(),
                    makes: Makes::Nothing,
                },
            },
            None => Plan::one(
                "session.float_block",
                asked.of_session(json!({
                    "block_id": asked.block(*pane)?,
                    "bounds": bounds(rect.unwrap_or(FLOAT_PLACE)),
                })),
            ),
        },
        Intent::FloatMove { pane, rect } => asked.float_to(*pane, *rect)?,
        Intent::Minimise { pane, on } => match asked.names.layers.get(pane) {
            Some(_) => {
                let mut args = json!({
                    "layer_id": asked.layer(*pane)?,
                    "state": if *on { "minimized" } else { "normal" },
                });
                if !on {
                    // Brought back, it has the keyboard, as a pane brought back here has.
                    args["focus"] = json!(true);
                }
                Plan::one("session.set_layer_state", asked.of_session(args))
            }
            None if *on => Plan {
                // A server puts away what floats. A pane among the tiles floats first.
                calls: vec![
                    Call::new(
                        "session.float_block",
                        asked.of_session(json!({
                            "block_id": asked.block(*pane)?,
                            "bounds": bounds(FLOAT_PLACE),
                        })),
                    ),
                    Call {
                        method: "session.set_layer_state",
                        args: asked.of_session(json!({ "state": "minimized" })),
                        takes: Some(("layer_id", "layer_id")),
                    },
                ],
                makes: Makes::Nothing,
            },
            // A pane among the tiles is not put away: there is nothing to bring back.
            None => {
                asked.block(*pane)?;
                Plan {
                    calls: Vec::new(),
                    makes: Makes::Nothing,
                }
            }
        },
        Intent::Dock { pane, to } => {
            if !asked.names.layers.contains_key(pane) {
                asked.block(*pane)?;
                return Err(IntentError::NotFloating(*pane).into());
            }
            let (anchor, edge) = match to {
                Some(Place::Beside { pane, edge }) => (*pane, *edge),
                Some(Place::NewTab { .. }) => {
                    return Err(unsupported("move a pane into a tab of its own"));
                }
                // Beside the pane of the tab on show that last had the keyboard, or
                // its first.
                None => {
                    let tab = asked.centre()?.active_tab();
                    let anchor = tab
                        .and_then(|tab| tab.last_focus)
                        .or_else(|| tab.map(|tab| tab.first_pane()));
                    match anchor {
                        Some(anchor) => (anchor, Dir::Right),
                        None => return Err(unsupported("put a pane back where there is no tab")),
                    }
                }
            };
            let (direction, side) = towards(edge);
            Plan::one(
                "session.move_block",
                asked.of_session(json!({
                    "block_id": asked.block(*pane)?,
                    "anchor_block_id": asked.tiled(anchor)?,
                    "direction": direction,
                    "side": side,
                })),
            )
        }
        Intent::PaneIcon { .. } => return Err(unsupported("give a pane a picture")),
        Intent::TabColor { .. } | Intent::TabLock { .. } | Intent::TabMute { .. } => {
            return Err(unsupported("colour, lock or mute a tab"));
        }
        Intent::TabMerge { .. } => return Err(unsupported("merge one tab into another")),
        Intent::GroupNew { .. }
        | Intent::GroupSet { .. }
        | Intent::GroupMove { .. }
        | Intent::GroupUngroup { .. }
        | Intent::GroupClose { .. }
        | Intent::GroupReopen { .. }
        | Intent::GroupForget { .. } => return Err(unsupported("gather tabs in groups")),
        Intent::ScratchShow { .. } | Intent::ScratchHide {} => {
            return Err(unsupported("keep a scratch terminal"));
        }
        Intent::WindowNew { .. } | Intent::WindowClose { .. } => {
            return Err(unsupported("be shown in more than one window"));
        }
        // What is the application's own to do never comes here.
        _ => return Err(unsupported("do this")),
    };
    Ok(plan)
}

/// Where a pane floats that was not told where: in the middle.
const FLOAT_PLACE: UnitRect = UnitRect {
    x: (UNITS - FLOAT_DEFAULT) / 2,
    y: (UNITS - FLOAT_DEFAULT) / 2,
    w: FLOAT_DEFAULT,
    h: FLOAT_DEFAULT,
};

/// A rectangle as a server takes the bounds of a layer: x, y, width and height as
/// fractions of the window, inside it.
fn bounds(rect: UnitRect) -> Value {
    let whole = f64::from(UNITS);
    let w = rect.w.clamp(1, UNITS);
    let h = rect.h.clamp(1, UNITS);
    let x = rect.x.min(UNITS - w);
    let y = rect.y.min(UNITS - h);
    json!([
        f64::from(x) / whole,
        f64::from(y) / whole,
        f64::from(w) / whole,
        f64::from(h) / whole,
    ])
}

impl Asked<'_> {
    /// The tabs of the one window a server's layout is shown in.
    fn centre(&self) -> Result<&TabGroup, LayoutError> {
        match self.workspace.windows.first() {
            Some(window) => Ok(&window.centre),
            None => Err(unsupported("be changed while it is not shown")),
        }
    }

    /// The window a tab is.
    fn window(&self, tab: TabId) -> Result<&str, LayoutError> {
        match self.names.tabs.get(&tab) {
            Some(window) => Ok(window),
            None => Err(IntentError::NoSuchTab(tab).into()),
        }
    }

    /// The layer of a floating pane, which is the pane's alone.
    fn layer(&self, pane: PaneId) -> Result<&str, LayoutError> {
        match self.names.layers.get(&pane) {
            Some(layer) if layer.alone => Ok(&layer.layer_id),
            // A layer is moved, sized and put away as a whole, and here each of its
            // blocks is a pane of its own.
            Some(_) => Err(unsupported(
                "move or put away one of several panes that float together",
            )),
            None => {
                self.block(pane)?;
                Err(IntentError::NotFloating(pane).into())
            }
        }
    }

    /// Gives a floating pane another place.
    fn float_to(&self, pane: PaneId, rect: UnitRect) -> Result<Plan, LayoutError> {
        if rect.w == 0 || rect.h == 0 {
            return Err(IntentError::BadRect.into());
        }
        Ok(Plan::one(
            "session.resize_layer",
            self.of_session(json!({ "layer_id": self.layer(pane)?, "bounds": bounds(rect) })),
        ))
    }

    /// Puts a tab at a place among the tabs, counted without it.
    fn place(&self, tab: TabId, index: u32) -> Result<Plan, LayoutError> {
        let window = self.window(tab)?;
        let others: Vec<TabId> = self
            .centre()?
            .tabs
            .iter()
            .map(|other| other.id)
            .filter(|other| *other != tab)
            .collect();
        let mut args = json!({ "window_id": window });
        // Before the tab that is there now; with none there, at the end.
        if let Some(after) = others.get(index as usize) {
            args["before_window_id"] = json!(self.window(*after)?);
        }
        Ok(Plan::one("session.move_window", self.of_session(args)))
    }
}
