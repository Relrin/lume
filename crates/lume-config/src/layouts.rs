//! Layout files: tabs of panes written by hand, or by a script, to be opened at will.
//!
//! A layout is a TOML file in the `layouts` folder of the configuration directory,
//! named by its file like a theme. It holds one or more `[[tab]]`s; a tab is either one
//! pane or a split of panes, and splits nest. Each pane says what it runs and where.
//!
//! The folder is read with the rest of the configuration, so a layout is taken up as
//! soon as it is saved, and a layout that is changed while tabs opened from it are on
//! show can be applied to them again. A pane is found again by its `key`: a pane the
//! file still names keeps its running session, wherever in the tab it moves. A pane
//! with no key is known by its place in the file.
//!
//! A file that cannot be used is reported with the line of what is wrong in it, and
//! left out; the other layouts are not affected.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lume_layout::tuning::{
    SHARE_MIN, SPLIT_CHILDREN_MAX, TAB_PANES_MAX, TITLE_MAX_CHARS, TREE_DEPTH_MAX, UNITS,
};
use lume_layout::{
    Axis, Node, Pane, PaneContent, PaneId, Split, SplitId, Tab, TabColor, TabId, TerminalPane,
};
use lume_proto::Launcher;
use serde_json::{Map, Value};

use crate::layers::parse_document;
use crate::source::{Position, Problem, ProblemKind, join_path};
use crate::suggest::nearest;
use crate::theme::read_limited;

/// The most tabs one layout may open.
pub const LAYOUT_TABS_MAX: usize = 32;

/// What marks a key the file did not write, made from the pane's place in it.
const PLACE_KEY: char = '#';

/// What separates a layout's id from the tab's key in a tab's origin.
const ORIGIN_SEPARATOR: char = '#';

const TOP_KEYS: [&str; 6] = ["name", "description", "color", "cwd", "env", "tab"];
const TAB_KEYS: [&str; 4] = ["title", "color", "focus", "zoom"];
const SPLIT_KEYS: [&str; 3] = ["split", "sizes", "pane"];
const PANE_KEYS: [&str; 8] = [
    "key", "title", "command", "profile", "cwd", "env", "plugin", "view",
];

/// One layout, as its file says it.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// The name of its file without `.toml`.
    pub id: String,
    /// What a person calls it.
    pub name: String,
    pub description: Option<String>,
    /// The colour of the group its tabs are gathered in, when it has more than one.
    pub color: Option<TabColor>,
    /// The tabs it opens, numbered by the file. Each has the layout as its origin.
    pub tabs: Vec<Tab>,
    pub file: PathBuf,
}

impl Layout {
    /// The tab of the layout that a tab with `origin` was opened from.
    pub fn tab_for(&self, origin: &str) -> Option<&Tab> {
        self.tabs
            .iter()
            .find(|tab| tab.origin.as_deref() == Some(origin))
    }
}

/// The layout and the tab of it that a tab's origin names.
pub fn split_origin(origin: &str) -> Option<(&str, &str)> {
    origin.split_once(ORIGIN_SEPARATOR)
}

/// The layouts there are to open.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layouts {
    layouts: BTreeMap<String, Arc<Layout>>,
}

impl Layouts {
    /// The layouts in the folder at `dir`. A file that cannot be used is reported and
    /// left out; one that is not TOML at all is a problem that keeps the configuration
    /// as it was.
    pub fn load(dir: &Path) -> (Self, Vec<Problem>) {
        let mut all = Self::default();
        let mut problems = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return (all, problems);
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "toml")
            })
            .collect();
        files.sort();
        for file in files {
            let Some(id) = file.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let whole = |kind, message: String| Problem {
                kind,
                path: String::new(),
                file: Some(file.clone()),
                line: 0,
                column: 0,
                message,
                suggestion: None,
            };
            if !valid_id(id) {
                problems.push(whole(
                    ProblemKind::InvalidValue,
                    format!(
                        "`{id}` cannot be a layout's name: use letters, digits, `-`, `_` and \
                         `.` in the file's name"
                    ),
                ));
                continue;
            }
            let text = match read_limited(&file) {
                Ok(text) => text,
                Err(error) => {
                    problems.push(whole(ProblemKind::Io, error));
                    continue;
                }
            };
            match parse_layout(id, &text, &file) {
                Ok((layout, found)) => {
                    let usable = found
                        .iter()
                        .all(|problem| problem.kind != ProblemKind::InvalidValue);
                    problems.extend(found);
                    if usable {
                        all.layouts.insert(id.to_owned(), Arc::new(layout));
                    }
                }
                Err(problem) => problems.push(problem),
            }
        }
        (all, problems)
    }

    /// The layout called `id`.
    pub fn get(&self, id: &str) -> Option<&Arc<Layout>> {
        self.layouts.get(id)
    }

    /// Every layout, in the order of their ids.
    pub fn iter(&self) -> impl Iterator<Item = &Arc<Layout>> {
        self.layouts.values()
    }

    pub fn is_empty(&self) -> bool {
        self.layouts.is_empty()
    }
}

/// Whether `id` can name a layout: a plain file name, of characters every platform
/// takes and that cannot be mistaken for the separator in a tab's origin.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && !id.starts_with('.')
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
}

/// The id a layout called `name` is written under: its name, lower case, with what a
/// file name cannot hold made a `-`.
pub fn layout_id(name: &str) -> Option<String> {
    let mut id = String::new();
    for character in name.trim().chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() || character == '_' || character == '.' {
            id.push(character);
        } else if !id.ends_with('-') {
            id.push('-');
        }
    }
    let id = id.trim_matches(['-', '.']).to_owned();
    let id: String = id.chars().take(64).collect();
    valid_id(&id).then_some(id)
}

/// Where something in the file is, by the path of its key.
struct Reader<'a> {
    file: &'a Path,
    positions: BTreeMap<String, Position>,
    problems: Vec<Problem>,
    /// The panes made so far, to number the next one.
    next: u32,
}

impl Reader<'_> {
    fn problem(
        &mut self,
        kind: ProblemKind,
        path: &[String],
        message: String,
        suggestion: Option<String>,
    ) {
        let joined = join_path(path);
        let position = self.positions.get(&joined).copied().unwrap_or_default();
        self.problems.push(Problem {
            kind,
            path: joined,
            file: Some(self.file.to_owned()),
            line: position.line,
            column: position.column,
            message,
            suggestion,
        });
    }

    fn invalid(&mut self, path: &[String], message: String) {
        self.problem(ProblemKind::InvalidValue, path, message, None);
    }

    fn number(&mut self) -> u32 {
        self.next += 1;
        self.next
    }

    /// Reports the keys of `table` that are not among `known`.
    fn unknown_keys(&mut self, table: &Map<String, Value>, path: &[String], known: &[&[&str]]) {
        let all: Vec<&str> = known.iter().flat_map(|keys| keys.iter().copied()).collect();
        for key in table.keys() {
            if !all.contains(&key.as_str()) {
                let mut at = path.to_vec();
                at.push(key.clone());
                let meant = nearest(key, all.iter().copied()).map(str::to_owned);
                self.problem(
                    ProblemKind::UnknownKey,
                    &at,
                    format!("unknown key `{key}`"),
                    meant,
                );
            }
        }
    }

    fn string(&mut self, table: &Map<String, Value>, key: &str, path: &[String]) -> Option<String> {
        match table.get(key)? {
            Value::String(text) => Some(text.clone()),
            _ => {
                self.invalid(&at(path, key), format!("`{key}`: expected a string"));
                None
            }
        }
    }

    fn title(&mut self, table: &Map<String, Value>, path: &[String]) -> Option<String> {
        let title = self.string(table, "title", path)?;
        let title: String = title.trim().chars().take(TITLE_MAX_CHARS).collect();
        (!title.is_empty()).then_some(title)
    }

    fn color(&mut self, table: &Map<String, Value>, path: &[String]) -> Option<TabColor> {
        let written = self.string(table, "color", path)?;
        let color = TabColor::parse(&written);
        if color.is_none() {
            let words = TabColor::NAMED.iter().map(|(_, word)| *word);
            let meant = nearest(&written, words).map(str::to_owned);
            self.problem(
                ProblemKind::InvalidValue,
                &at(path, "color"),
                format!("`color`: `{written}` is not a colour; name one or write `#rrggbb`"),
                meant,
            );
        }
        color
    }

    fn env(
        &mut self,
        table: &Map<String, Value>,
        path: &[String],
    ) -> Option<Vec<(String, String)>> {
        let value = table.get("env")?;
        let Value::Object(entries) = value else {
            self.invalid(
                &at(path, "env"),
                "`env`: expected a table of names and values".to_owned(),
            );
            return None;
        };
        let mut env = Vec::new();
        for (name, value) in entries {
            match value {
                Value::String(text) => env.push((name.clone(), text.clone())),
                _ => self.invalid(
                    &at(&at(path, "env"), name),
                    format!("`env.{name}`: expected a string"),
                ),
            }
        }
        Some(env)
    }

    fn command(&mut self, table: &Map<String, Value>, path: &[String]) -> Option<Launcher> {
        let words = match table.get("command")? {
            Value::String(line) => words(line),
            Value::Array(items) => {
                let mut words = Vec::new();
                for item in items {
                    if let Value::String(word) = item {
                        words.push(word.clone());
                    } else {
                        self.invalid(
                            &at(path, "command"),
                            "`command`: expected a list of strings".to_owned(),
                        );
                        return None;
                    }
                }
                words
            }
            _ => {
                self.invalid(
                    &at(path, "command"),
                    "`command`: expected a string or a list of strings".to_owned(),
                );
                return None;
            }
        };
        let mut words = words.into_iter();
        let Some(program) = words.next().filter(|program| !program.trim().is_empty()) else {
            self.invalid(
                &at(path, "command"),
                "`command`: names no program".to_owned(),
            );
            return None;
        };
        Some(Launcher::Shell {
            program,
            args: words.collect(),
            login: false,
        })
    }
}

fn at(path: &[String], key: &str) -> Vec<String> {
    let mut at = path.to_vec();
    at.push(key.to_owned());
    at
}

/// The words of a command line: split at spaces, except within quotes.
pub(crate) fn words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for character in line.chars() {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (Some(_), _) => word.push(character),
            (None, '"' | '\'') => {
                quote = Some(character);
                started = true;
            }
            (None, _) if character.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (None, _) => {
                word.push(character);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

/// What a pane inherits from the tab and the layout it is in.
#[derive(Clone, Default)]
struct Inherited {
    cwd: Option<String>,
    env: Vec<(String, String)>,
}

impl Inherited {
    fn under(&self, reader: &mut Reader<'_>, table: &Map<String, Value>, path: &[String]) -> Self {
        let mut env = self.env.clone();
        for (name, value) in reader.env(table, path).unwrap_or_default() {
            env.retain(|(held, _)| *held != name);
            env.push((name, value));
        }
        Self {
            cwd: reader
                .string(table, "cwd", path)
                .or_else(|| self.cwd.clone()),
            env,
        }
    }
}

/// Reads a node of a tab: a split of panes when the table has `pane`s, one pane
/// otherwise. `place` is the node's place in the file, for a pane that has no key.
fn read_node(
    reader: &mut Reader<'_>,
    table: &Map<String, Value>,
    path: &[String],
    place: &str,
    inherited: &Inherited,
    depth: usize,
) -> Option<Node> {
    let inherited = inherited.under(reader, table, path);
    let Some(children) = table.get("pane") else {
        return read_pane(reader, table, path, place, &inherited);
    };
    let Value::Array(children) = children else {
        reader.invalid(
            &at(path, "pane"),
            "`pane`: expected `[[pane]]` tables".to_owned(),
        );
        return None;
    };
    for key in ["key", "command", "profile", "plugin", "view"] {
        if table.contains_key(key) {
            reader.invalid(
                &at(path, key),
                format!("`{key}` belongs to a pane; this one is split into `[[pane]]`s"),
            );
        }
    }
    if depth >= TREE_DEPTH_MAX {
        reader.invalid(path, format!("splits nest more than {TREE_DEPTH_MAX} deep"));
        return None;
    }
    if children.is_empty() {
        reader.invalid(
            &at(path, "pane"),
            "`pane`: a split needs at least one pane".to_owned(),
        );
        return None;
    }
    if children.len() > SPLIT_CHILDREN_MAX {
        reader.invalid(
            &at(path, "pane"),
            format!("a split may have at most {SPLIT_CHILDREN_MAX} panes side by side"),
        );
        return None;
    }
    let axis = match reader.string(table, "split", path).as_deref() {
        None | Some("row") => Axis::Row,
        Some("column") => Axis::Column,
        Some(other) => {
            let meant = nearest(other, ["row", "column"]).map(str::to_owned);
            reader.problem(
                ProblemKind::InvalidValue,
                &at(path, "split"),
                format!("`split`: `{other}` is not `row` (side by side) or `column` (one above the other)"),
                meant,
            );
            Axis::Row
        }
    };
    let mut nodes = Vec::with_capacity(children.len());
    for (index, child) in children.iter().enumerate() {
        let child_path = at(&at(path, "pane"), &index.to_string());
        let Value::Object(child) = child else {
            reader.invalid(&child_path, "expected a `[[pane]]` table".to_owned());
            return None;
        };
        reader.unknown_keys(child, &child_path, &[&SPLIT_KEYS, &PANE_KEYS]);
        let place = format!("{place}.{}", index + 1);
        nodes.push(read_node(
            reader,
            child,
            &child_path,
            &place,
            &inherited,
            depth + 1,
        )?);
    }
    if nodes.len() == 1 {
        return nodes.pop();
    }
    let ratios = ratios(reader, table, path, nodes.len());
    Some(Node::Split(Split {
        id: SplitId::new(reader.number()),
        axis,
        ratios,
        children: nodes,
    }))
}

/// The shares of a split's children from its `sizes`, which may be in any unit: they
/// are taken as parts of their sum. Without sizes the children share alike.
fn ratios(
    reader: &mut Reader<'_>,
    table: &Map<String, Value>,
    path: &[String],
    count: usize,
) -> Vec<u32> {
    let even = || {
        let share = UNITS / count as u32;
        let mut ratios = vec![share; count];
        ratios[count - 1] = UNITS - share * (count as u32 - 1);
        ratios
    };
    let Some(sizes) = table.get("sizes") else {
        return even();
    };
    let sizes: Option<Vec<f64>> = match sizes {
        Value::Array(items) => items
            .iter()
            .map(|item| item.as_f64().filter(|size| size.is_finite() && *size > 0.0))
            .collect(),
        _ => None,
    };
    let Some(sizes) = sizes.filter(|sizes| sizes.len() == count) else {
        reader.invalid(
            &at(path, "sizes"),
            format!("`sizes`: expected {count} numbers above nought, one for each pane"),
        );
        return even();
    };
    let total: f64 = sizes.iter().sum();
    let mut ratios: Vec<u32> = sizes
        .iter()
        .map(|size| (size / total * f64::from(UNITS)).floor() as u32)
        .collect();
    let given: u32 = ratios.iter().sum();
    ratios[count - 1] += UNITS - given;
    if ratios.iter().any(|ratio| *ratio < SHARE_MIN) {
        reader.invalid(
            &at(path, "sizes"),
            format!(
                "`sizes`: each pane needs at least {}% of the split",
                SHARE_MIN * 100 / UNITS
            ),
        );
        return even();
    }
    ratios
}

fn read_pane(
    reader: &mut Reader<'_>,
    table: &Map<String, Value>,
    path: &[String],
    place: &str,
    inherited: &Inherited,
) -> Option<Node> {
    let key = match reader.string(table, "key", path) {
        Some(key) if key.trim().is_empty() || key.starts_with(PLACE_KEY) => {
            reader.invalid(
                &at(path, "key"),
                format!("`key`: `{key}` is empty or starts with `{PLACE_KEY}`"),
            );
            return None;
        }
        Some(key) => key,
        None => format!("{PLACE_KEY}{place}"),
    };
    let plugin = reader.string(table, "plugin", path);
    let view = reader.string(table, "view", path);
    let content = match (plugin, view) {
        (Some(plugin), Some(view)) => {
            for key in ["command", "profile"] {
                if table.contains_key(key) {
                    reader.invalid(
                        &at(path, key),
                        format!("`{key}`: a plugin's view runs nothing"),
                    );
                }
            }
            PaneContent::PluginView {
                plugin,
                view,
                state: Value::Null,
            }
        }
        (None, None) => PaneContent::Terminal(TerminalPane {
            backend: None,
            session: None,
            profile: reader.string(table, "profile", path),
            launcher: reader.command(table, path),
            cwd: inherited.cwd.clone(),
            env: inherited.env.clone(),
        }),
        _ => {
            reader.invalid(
                path,
                "a plugin's view needs both `plugin` and `view`".to_owned(),
            );
            return None;
        }
    };
    Some(Node::Pane(Pane {
        id: PaneId::new(reader.number()),
        title: None,
        icon: None,
        content,
        key: Some(key),
    }))
}

/// Reads a layout file. `file` is where the text came from. The error is a file that
/// is not TOML; anything else wrong is reported beside the layout, which is not to be
/// used when any of it is an invalid value.
pub fn parse_layout(id: &str, text: &str, file: &Path) -> Result<(Layout, Vec<Problem>), Problem> {
    let document = parse_document(text).map_err(|error| error.problem(file))?;
    let mut reader = Reader {
        file,
        positions: document.positions,
        problems: Vec::new(),
        next: 0,
    };
    let tree = document.tree;
    let top: Vec<String> = Vec::new();
    reader.unknown_keys(&tree, &top, &[&TOP_KEYS]);
    let name = reader
        .string(&tree, "name", &top)
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| id.to_owned());
    let description = reader.string(&tree, "description", &top);
    let color = reader.color(&tree, &top);
    let inherited = Inherited::default().under(&mut reader, &tree, &top);

    let mut tabs = Vec::new();
    match tree.get("tab") {
        Some(Value::Array(written)) if !written.is_empty() => {
            if written.len() > LAYOUT_TABS_MAX {
                reader.invalid(
                    &at(&top, "tab"),
                    format!("a layout may open at most {LAYOUT_TABS_MAX} tabs"),
                );
            }
            for (index, entry) in written.iter().take(LAYOUT_TABS_MAX).enumerate() {
                let path = at(&at(&top, "tab"), &index.to_string());
                let Value::Object(table) = entry else {
                    reader.invalid(&path, "expected a `[[tab]]` table".to_owned());
                    continue;
                };
                reader.unknown_keys(table, &path, &[&TAB_KEYS, &SPLIT_KEYS, &PANE_KEYS]);
                // A tab is known by its place among the file's tabs.
                let tab_key = (index + 1).to_string();
                if let Some(tab) =
                    read_tab(&mut reader, table, &path, index, &tab_key, id, &inherited)
                {
                    tabs.push(tab);
                }
            }
        }
        Some(_) => reader.invalid(
            &at(&top, "tab"),
            "`tab`: expected `[[tab]]` tables".to_owned(),
        ),
        None => reader.invalid(&top, "a layout needs at least one `[[tab]]`".to_owned()),
    }
    let layout = Layout {
        id: id.to_owned(),
        name,
        description,
        color,
        tabs,
        file: file.to_owned(),
    };
    Ok((layout, reader.problems))
}

fn read_tab(
    reader: &mut Reader<'_>,
    table: &Map<String, Value>,
    path: &[String],
    index: usize,
    tab_key: &str,
    layout: &str,
    inherited: &Inherited,
) -> Option<Tab> {
    let title = reader.title(table, path);
    let color = reader.color(table, path);
    let focus = reader.string(table, "focus", path);
    let zoom = reader.string(table, "zoom", path);
    // A tab with no panes of its own is one pane, and `key` is that pane's.
    let place = (index + 1).to_string();
    let root = read_node(reader, table, path, &place, inherited, 0)?;

    let panes = root.panes();
    if panes.len() > TAB_PANES_MAX {
        reader.invalid(
            path,
            format!("a tab may hold at most {TAB_PANES_MAX} panes"),
        );
        return None;
    }
    let mut seen = HashSet::new();
    for pane in &panes {
        let key = pane.key.as_deref().unwrap_or_default();
        if !seen.insert(key) {
            reader.invalid(path, format!("two panes of this tab have the key `{key}`"));
            return None;
        }
    }
    let mut find = |key: Option<String>, name: &str| {
        let key = key?;
        let found = panes
            .iter()
            .find(|pane| pane.key.as_deref() == Some(&key))
            .map(|pane| pane.id);
        if found.is_none() {
            reader.invalid(
                &at(path, name),
                format!("`{name}`: no pane of this tab has the key `{key}`"),
            );
        }
        found
    };
    let last_focus = find(focus, "focus");
    let zoomed = find(zoom, "zoom");
    Some(Tab {
        id: TabId::new(reader.number()),
        title,
        root,
        zoomed,
        last_focus,
        color,
        locked: false,
        muted: false,
        group: None,
        origin: Some(format!("{layout}{ORIGIN_SEPARATOR}{tab_key}")),
    })
}

/// The JSON Schema of a layout file, for editors to complete and check one with.
pub fn layout_schema() -> Value {
    let colors: Vec<&str> = TabColor::NAMED.iter().map(|(_, word)| *word).collect();
    let color = serde_json::json!({
        "description": "A colour: one of the named ones, or `#rrggbb`.",
        "type": "string",
        "anyOf": [{ "enum": colors }, { "pattern": "^#[0-9a-fA-F]{6}$" }],
    });
    let env = serde_json::json!({
        "description": "Variables the panes start with, on top of the environment.",
        "type": "object",
        "additionalProperties": { "type": "string" },
    });
    let cwd = serde_json::json!({
        "description": "Where the panes start. A leading `~` is the home folder.",
        "type": "string",
    });
    let node = serde_json::json!({
        "type": "object",
        "properties": {
            "key": {
                "description": "What the pane is called in this file. Applied again, the file finds the pane by it and leaves its program running.",
                "type": "string",
            },
            "title": { "description": "The pane's name.", "type": "string" },
            "command": {
                "description": "The program the pane runs and its arguments: a list, or one line split at spaces outside quotes. It runs directly, not in a shell.",
                "anyOf": [
                    { "type": "string" },
                    { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                ],
            },
            "profile": { "description": "The profile the pane starts from, by its id.", "type": "string" },
            "plugin": { "description": "The plugin whose view the pane shows.", "type": "string" },
            "view": { "description": "The plugin's view the pane shows.", "type": "string" },
            "cwd": cwd,
            "env": env,
            "split": {
                "description": "How the panes inside are laid out: side by side, or one above the other.",
                "enum": ["row", "column"],
            },
            "sizes": {
                "description": "The panes' shares, in any unit: each is a part of their sum.",
                "type": "array",
                "items": { "type": "number", "exclusiveMinimum": 0 },
            },
            "pane": {
                "description": "The panes this one is split into.",
                "type": "array",
                "items": { "$ref": "#/$defs/node" },
                "minItems": 1,
                "maxItems": SPLIT_CHILDREN_MAX,
            },
        },
    });
    let mut tab = node.clone();
    if let Some(Value::Object(properties)) = tab.get_mut("properties") {
        properties.insert(
            "title".to_owned(),
            serde_json::json!({ "description": "The tab's name.", "type": "string" }),
        );
        properties.insert("color".to_owned(), color.clone());
        properties.insert(
            "focus".to_owned(),
            serde_json::json!({ "description": "The key of the pane that has the keyboard.", "type": "string" }),
        );
        properties.insert(
            "zoom".to_owned(),
            serde_json::json!({ "description": "The key of the pane that fills the tab.", "type": "string" }),
        );
    }
    serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Lume layout file",
        "description": "Tabs of panes to open by name, from the `layouts` folder of Lume's configuration.",
        "type": "object",
        "properties": {
            "name": { "description": "What the layout is called. Left out, the file's name.", "type": "string" },
            "description": { "description": "A few words shown beside the name.", "type": "string" },
            "color": color,
            "cwd": cwd,
            "env": env,
            "tab": {
                "description": "The tabs the layout opens.",
                "type": "array",
                "items": { "$ref": "#/$defs/tab" },
                "minItems": 1,
                "maxItems": LAYOUT_TABS_MAX,
            },
        },
        "required": ["tab"],
        "$defs": { "node": node, "tab": tab },
    })
}

/// A string as TOML writes it.
fn quoted(text: &str) -> String {
    toml::Value::String(text.to_owned()).to_string()
}

/// The text of a layout file that opens `tabs` as they are now: what each pane runs
/// and where, and the shares of the splits. Panes are given keys, kept from the file
/// they came from where they have one.
pub fn layout_text(name: &str, tabs: &[&Tab]) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "# A layout: tabs of panes to open from the new-tab menu, or with"
    );
    let _ = writeln!(
        text,
        "# `lume ctl layout-open <name>`. Lume reads this file again whenever"
    );
    let _ = writeln!(
        text,
        "# it is saved. A pane keeps its `key` when the file is applied again to"
    );
    let _ = writeln!(
        text,
        "# a tab opened from it, and with it its running program."
    );
    let _ = writeln!(text);
    let _ = writeln!(text, "name = {}", quoted(name));
    for tab in tabs {
        let _ = writeln!(text);
        let _ = writeln!(text, "[[tab]]");
        if let Some(title) = &tab.title {
            let _ = writeln!(text, "title = {}", quoted(title));
        }
        if let Some(color) = tab.color {
            let _ = writeln!(text, "color = {}", quoted(&color.written()));
        }
        let keys = assign_keys(&tab.root);
        if let Node::Split(_) = tab.root {
            // A tab's own keys come before its panes: after them they would be the
            // last pane's.
            if let Some(focus) = tab.last_focus.and_then(|pane| keys.get(&pane)) {
                let _ = writeln!(text, "focus = {}", quoted(focus));
            }
        }
        write_node(&mut text, &tab.root, "tab", &keys, true);
    }
    text
}

/// The key each pane of a tree is written with, none the same as another.
pub fn assign_keys(root: &Node) -> BTreeMap<PaneId, String> {
    let mut used = HashSet::new();
    root.panes()
        .into_iter()
        .map(|pane| (pane.id, key_for(pane, &mut used)))
        .collect()
}

/// The key a pane is written with: its own, or one made from what it runs.
fn key_for(pane: &Pane, used: &mut HashSet<String>) -> String {
    let base = match (&pane.key, &pane.content) {
        (Some(key), _) if !key.starts_with(PLACE_KEY) => key.clone(),
        (
            _,
            PaneContent::Terminal(TerminalPane {
                launcher: Some(Launcher::Shell { program, .. }),
                ..
            }),
        ) => Path::new(program)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(layout_id)
            .unwrap_or_else(|| "pane".to_owned()),
        (_, PaneContent::PluginView { view, .. }) => {
            layout_id(view).unwrap_or_else(|| "view".to_owned())
        }
        _ => "shell".to_owned(),
    };
    let mut key = base.clone();
    let mut number = 1;
    while !used.insert(key.clone()) {
        number += 1;
        key = format!("{base}-{number}");
    }
    key
}

fn write_node(
    text: &mut String,
    node: &Node,
    table: &str,
    keys: &BTreeMap<PaneId, String>,
    top: bool,
) {
    match node {
        Node::Pane(pane) => {
            if !top {
                let _ = writeln!(text);
                let _ = writeln!(text, "[[{table}]]");
            }
            if let Some(key) = keys.get(&pane.id) {
                let _ = writeln!(text, "key = {}", quoted(key));
            }
            match &pane.content {
                PaneContent::Terminal(terminal) => {
                    if let Some(profile) = &terminal.profile {
                        let _ = writeln!(text, "profile = {}", quoted(profile));
                    }
                    if let Some(Launcher::Shell { program, args, .. }) = &terminal.launcher {
                        let words: Vec<String> = std::iter::once(program)
                            .chain(args)
                            .map(|word| quoted(word))
                            .collect();
                        let _ = writeln!(text, "command = [{}]", words.join(", "));
                    }
                    if let Some(cwd) = &terminal.cwd {
                        let _ = writeln!(text, "cwd = {}", quoted(cwd));
                    }
                    if !terminal.env.is_empty() {
                        let pairs: Vec<String> = terminal
                            .env
                            .iter()
                            .map(|(name, value)| format!("{} = {}", quoted(name), quoted(value)))
                            .collect();
                        let _ = writeln!(text, "env = {{ {} }}", pairs.join(", "));
                    }
                }
                PaneContent::PluginView { plugin, view, .. } => {
                    let _ = writeln!(text, "plugin = {}", quoted(plugin));
                    let _ = writeln!(text, "view = {}", quoted(view));
                }
                PaneContent::Placeholder { .. } => {}
            }
        }
        Node::Split(split) => {
            if !top {
                let _ = writeln!(text);
                let _ = writeln!(text, "[[{table}]]");
            }
            let axis = match split.axis {
                Axis::Row => "row",
                Axis::Column => "column",
            };
            let _ = writeln!(text, "split = {}", quoted(axis));
            let sizes: Vec<String> = split
                .ratios
                .iter()
                .map(|ratio| format!("{}", (f64::from(*ratio) * 100.0 / f64::from(UNITS)).round()))
                .collect();
            let _ = writeln!(text, "sizes = [{}]", sizes.join(", "));
            let child_table = format!("{table}.pane");
            for child in &split.children {
                write_node(text, child, &child_table, keys, false);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn parse(text: &str) -> (Layout, Vec<Problem>) {
        parse_layout("dev", text, Path::new("dev.toml")).unwrap()
    }

    fn keys(tab: &Tab) -> Vec<String> {
        tab.root
            .panes()
            .iter()
            .map(|pane| pane.key.clone().unwrap())
            .collect()
    }

    fn launcher(tab: &Tab, index: usize) -> Option<Launcher> {
        match &tab.root.panes()[index].content {
            PaneContent::Terminal(terminal) => terminal.launcher.clone(),
            _ => None,
        }
    }

    const DEV: &str = r#"
name = "Lume dev"
description = "Editor, checks and logs"
color = "blue"
cwd = "~/src/lume"

[[tab]]
title = "code"
split = "row"
sizes = [60, 40]
focus = "editor"

  [[tab.pane]]
  key = "editor"
  command = "nvim ."

  [[tab.pane]]
  split = "column"
  cwd = "~/src/lume/crates"
    [[tab.pane.pane]]
    key = "check"
    command = ["cargo", "watch", "-x", "check"]
    env = { RUST_LOG = "debug" }
    [[tab.pane.pane]]
    key = "shell"

[[tab]]
title = "logs"
command = 'tail -f "app log.txt"'
"#;

    #[test]
    fn a_layout_opens_its_tabs_with_what_each_pane_runs_and_where() {
        let (layout, problems) = parse(DEV);
        assert_eq!(problems, vec![]);
        assert_eq!(layout.name, "Lume dev");
        assert_eq!(
            layout.description.as_deref(),
            Some("Editor, checks and logs")
        );
        assert_eq!(layout.color, Some(TabColor::Blue));
        assert_eq!(layout.tabs.len(), 2);

        let code = &layout.tabs[0];
        assert_eq!(code.title.as_deref(), Some("code"));
        assert_eq!(code.origin.as_deref(), Some("dev#1"));
        assert_eq!(keys(code), ["editor", "check", "shell"]);
        let Node::Split(split) = &code.root else {
            panic!("a split")
        };
        assert_eq!(split.axis, Axis::Row);
        assert_eq!(split.ratios, [UNITS * 6 / 10, UNITS * 4 / 10]);
        assert!(matches!(&split.children[1], Node::Split(inner) if inner.axis == Axis::Column));
        assert_eq!(code.last_focus, Some(code.root.panes()[0].id));
        assert_eq!(
            launcher(code, 1),
            Some(Launcher::Shell {
                program: "cargo".to_owned(),
                args: vec!["watch".into(), "-x".into(), "check".into()],
                login: false,
            })
        );
        let PaneContent::Terminal(check) = &code.root.panes()[1].content else {
            panic!()
        };
        assert_eq!(check.cwd.as_deref(), Some("~/src/lume/crates"));
        assert_eq!(check.env, [("RUST_LOG".to_owned(), "debug".to_owned())]);
        let PaneContent::Terminal(editor) = &code.root.panes()[0].content else {
            panic!()
        };
        assert_eq!(editor.cwd.as_deref(), Some("~/src/lume"));

        // A tab of one pane, which has no key and is known by its place.
        let logs = &layout.tabs[1];
        assert_eq!(keys(logs), ["#2"]);
        assert_eq!(logs.origin.as_deref(), Some("dev#2"));
        assert_eq!(
            launcher(logs, 0),
            Some(Launcher::Shell {
                program: "tail".to_owned(),
                args: vec!["-f".into(), "app log.txt".into()],
                login: false,
            })
        );
    }

    #[test]
    fn what_is_wrong_is_said_with_its_line_and_what_was_meant() {
        let (_, problems) = parse(
            "[[tab]]\nsplit = \"colum\"\nsizes = [1]\n[[tab.pane]]\nkey = \"a\"\ncomand = \"x\"\n[[tab.pane]]\nkey = \"a\"\n",
        );
        let messages: Vec<(&str, u32, Option<&str>)> = problems
            .iter()
            .map(|problem| {
                (
                    problem.path.as_str(),
                    problem.line,
                    problem.suggestion.as_deref(),
                )
            })
            .collect();
        assert!(
            messages.contains(&("tab.0.split", 2, Some("column"))),
            "{messages:?}"
        );
        assert!(messages.contains(&("tab.0.sizes", 3, None)), "{messages:?}");
        assert!(
            messages.contains(&("tab.0.pane.0.comand", 6, Some("command"))),
            "{messages:?}"
        );
        assert!(
            problems
                .iter()
                .any(|problem| problem.message.contains("key `a`"))
        );
    }

    #[test]
    fn a_layout_needs_a_tab_and_a_focus_must_name_a_pane() {
        let (_, problems) = parse("name = \"x\"\n");
        assert!(problems[0].message.contains("at least one `[[tab]]`"));
        let (_, problems) = parse("[[tab]]\nfocus = \"nope\"\n");
        assert!(problems[0].message.contains("`nope`"));
    }

    #[test]
    fn the_folder_holds_the_layouts_and_leaves_out_what_cannot_be_used() {
        let dir = TempDir::new("layouts");
        dir.write("layouts/dev.toml", DEV);
        dir.write(
            "layouts/bad.toml",
            "[[tab]]\nsplit = \"diagonal\"\n[[tab.pane]]\n[[tab.pane]]\n",
        );
        dir.write("layouts/has space.toml", "[[tab]]\n");
        dir.write("layouts/notes.txt", "not a layout");
        let (layouts, problems) = Layouts::load(&dir.path().join("layouts"));
        let ids: Vec<&str> = layouts.iter().map(|layout| layout.id.as_str()).collect();
        assert_eq!(ids, ["dev"]);
        assert_eq!(problems.len(), 2, "{problems:?}");
        let (none, problems) = Layouts::load(&dir.path().join("nowhere"));
        assert!(none.is_empty() && problems.is_empty());
    }

    #[test]
    fn a_tab_written_out_reads_back_the_same() {
        let (layout, _) = parse(DEV);
        let tabs: Vec<&Tab> = layout.tabs.iter().collect();
        let text = layout_text("Lume dev", &tabs);
        let (again, problems) = parse_layout("dev", &text, Path::new("dev.toml")).unwrap();
        assert_eq!(problems, vec![], "{text}");
        assert_eq!(again.tabs.len(), 2);
        assert_eq!(keys(&again.tabs[0]), ["editor", "check", "shell"]);
        assert!(again.tabs[0].root.same_shape(&layout.tabs[0].root));
        assert_eq!(launcher(&again.tabs[0], 1), launcher(&layout.tabs[0], 1));
        // A pane known only by its place is given a key from what it runs.
        assert_eq!(keys(&again.tabs[1]), ["tail"]);
    }

    #[test]
    fn a_name_becomes_an_id_a_file_can_have() {
        assert_eq!(layout_id("Lume dev").as_deref(), Some("lume-dev"));
        assert_eq!(layout_id("  Ops / Logs!! ").as_deref(), Some("ops-logs"));
        assert_eq!(layout_id("///"), None);
        assert!(valid_id("lume-dev.2"));
        assert!(!valid_id("a#b"));
    }

    #[test]
    fn a_command_line_is_split_at_spaces_outside_quotes() {
        assert_eq!(
            words(r#"git log --format="%h %s"  -n 3"#),
            ["git", "log", "--format=%h %s", "-n", "3"]
        );
        assert_eq!(words("say ''"), ["say", ""]);
    }
}
