//! Reading the files and laying them over each other.
//!
//! The configuration is the built-in defaults with the user's file laid over them and
//! the active workspace's file over that. A file is read in three steps. It is parsed
//! with the place of every key kept. Each key is then looked up in the schema: one the
//! schema does not know is reported with its line and the key that was probably meant,
//! and dropped; a value the schema does not allow is reported and dropped, so that the
//! layer below shows through. What is left is laid over the layers below, and every
//! value remembers which file and line it came from.
//!
//! A file that is not TOML at all contributes nothing, and says so.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use toml::Spanned;
use toml::de::{DeTable, DeValue};

use crate::schema::{
    Config, NOT_IN_A_WORKSPACE, OnSwitch, WORKSPACE_PROFILE_KEY, WORKSPACE_ROOT_KEY,
    WorkspaceSettings, config_schema,
};
use crate::source::{Position, Problem, ProblemKind, Source, join_path};
use crate::suggest::nearest;

/// Which layer a file is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayerKind {
    /// The user's `config.toml`.
    User,
    /// The file of the workspace called `name`.
    Workspace { name: String },
}

/// One file to lay over the layers before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerInput {
    pub kind: LayerKind,
    /// Where the text came from, for the messages.
    pub file: PathBuf,
    pub text: String,
}

/// The layers laid over each other.
#[derive(Clone, Debug, PartialEq)]
pub struct Layered {
    pub config: Config,
    /// The layer each value came from, by the value's path.
    pub sources: BTreeMap<String, Source>,
    /// What was wrong in the files and was passed over.
    pub problems: Vec<Problem>,
    /// What made a whole file unusable. Such a file contributed nothing.
    pub fatal: Vec<Problem>,
    /// What the workspace's file says besides settings.
    pub workspace: WorkspaceSettings,
}

/// A TOML file as a tree of values, with the place of every key.
pub(crate) struct Document {
    pub tree: Map<String, Value>,
    /// Where each key is written, by the key's path.
    pub positions: BTreeMap<String, Position>,
}

/// A file that is not TOML: where it stops being so, and why.
pub(crate) struct SyntaxError {
    pub position: Position,
    pub message: String,
}

impl SyntaxError {
    pub fn problem(self, file: &Path) -> Problem {
        Problem {
            kind: ProblemKind::Syntax,
            path: String::new(),
            file: Some(file.to_owned()),
            line: self.position.line,
            column: self.position.column,
            message: self.message,
            suggestion: None,
        }
    }
}

/// Parses a TOML file, keeping where every key is.
pub(crate) fn parse_document(text: &str) -> Result<Document, SyntaxError> {
    let parsed = DeTable::parse(text).map_err(|error| SyntaxError {
        position: Position::of(text, error.span().map_or(0, |span| span.start)),
        message: error.message().to_owned(),
    })?;
    let mut positions = BTreeMap::new();
    let mut path = Vec::new();
    let tree = table(parsed.get_ref(), text, &mut path, &mut positions);
    Ok(Document { tree, positions })
}

fn table(
    table: &DeTable<'_>,
    text: &str,
    path: &mut Vec<String>,
    positions: &mut BTreeMap<String, Position>,
) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in table.iter() {
        path.push(key.get_ref().to_string());
        positions.insert(join_path(path), Position::of(text, key.span().start));
        out.insert(
            key.get_ref().to_string(),
            convert(value, text, path, positions),
        );
        path.pop();
    }
    out
}

fn convert(
    value: &Spanned<DeValue<'_>>,
    text: &str,
    path: &mut Vec<String>,
    positions: &mut BTreeMap<String, Position>,
) -> Value {
    match value.get_ref() {
        DeValue::String(string) => Value::String(string.to_string()),
        DeValue::Integer(integer) => {
            match i64::from_str_radix(integer.as_str(), integer.radix()) {
                Ok(number) => Value::from(number),
                // Too large to be a number anything here takes. Kept as its text, so
                // that the check says a number was expected.
                Err(_) => Value::String(integer.as_str().to_owned()),
            }
        }
        DeValue::Float(float) => float
            .as_str()
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map_or_else(
                // Infinity and not-a-number are numbers to TOML and to nothing here.
                || Value::String(float.as_str().to_owned()),
                Value::Number,
            ),
        DeValue::Boolean(boolean) => Value::Bool(*boolean),
        DeValue::Datetime(datetime) => Value::String(datetime.to_string()),
        DeValue::Array(array) => Value::Array(
            array
                .iter()
                .map(|item| convert(item, text, path, positions))
                .collect(),
        ),
        DeValue::Table(inner) => Value::Object(table(inner, text, path, positions)),
    }
}

/// What a JSON value is called in a message.
fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "nothing",
        Value::Bool(_) => "true or false",
        Value::Number(number) if number.is_f64() => "a number",
        Value::Number(_) => "a whole number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "a table",
    }
}

/// What a schema type is called in a message.
fn type_name(name: &str) -> &'static str {
    match name {
        "string" => "a string",
        "integer" => "a whole number",
        "number" => "a number",
        "boolean" => "true or false",
        "array" => "a list",
        "object" => "a table",
        _ => "nothing",
    }
}

fn has_type(value: &Value, name: &str) -> bool {
    match name {
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "null" => value.is_null(),
        _ => false,
    }
}

/// The types a schema allows, if it names any.
fn types(schema: &Map<String, Value>) -> Vec<&str> {
    match schema.get("type") {
        Some(Value::String(one)) => vec![one.as_str()],
        Some(Value::Array(many)) => many.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

/// A number as a person writes it: no `.0` on a whole one.
fn plain(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{number:.0}")
    } else {
        number.to_string()
    }
}

/// A value as it is quoted in a message.
fn quoted(value: &Value) -> String {
    match value {
        Value::String(text) => format!("`{text}`"),
        other => format!("`{other}`"),
    }
}

/// The value a schema allows and no other, if it is that kind of schema.
fn literal(schema: &Value) -> Option<&Value> {
    let schema = schema.as_object()?;
    if let Some(only) = schema.get("const") {
        return Some(only);
    }
    match schema.get("enum") {
        Some(Value::Array(allowed)) if allowed.len() == 1 => allowed.first(),
        _ => None,
    }
}

fn one_of(allowed: &[&Value]) -> String {
    let listed: Vec<String> = allowed.iter().map(|value| quoted(value)).collect();
    format!("expected one of {}", listed.join(", "))
}

/// Why none of several schemas allows a value.
fn none_of(value: &Value, options: &[Value]) -> String {
    let options: Vec<&Value> = options
        .iter()
        // "Or nothing" is how an optional setting is described; a file cannot say it.
        .filter(|option| option.get("type") != Some(&Value::from("null")))
        .collect();
    let literals: Vec<&Value> = options
        .iter()
        .filter_map(|option| literal(option))
        .collect();
    if !literals.is_empty() && literals.len() == options.len() {
        return one_of(&literals);
    }
    // The schema that takes this kind of value says best what is wrong with it.
    for option in &options {
        let fits = option
            .as_object()
            .is_some_and(|schema| types(schema).iter().any(|name| has_type(value, name)));
        if fits && let Err(message) = check(value, option) {
            return message;
        }
    }
    let mut names: Vec<&str> = options
        .iter()
        .filter_map(|option| option.as_object())
        .flat_map(types)
        .map(type_name)
        .collect();
    names.dedup();
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => {
            format!(
                "expected {} or {last}, not {}",
                rest.join(", "),
                kind_of(value)
            )
        }
        Some((only, _)) => format!("expected {only}, not {}", kind_of(value)),
        None => format!("{} is not allowed here", kind_of(value)),
    }
}

/// Checks a value against a schema: the part of JSON Schema the settings' schema uses.
/// The error says what was expected, in words a person would use.
pub(crate) fn check(value: &Value, schema: &Value) -> Result<(), String> {
    let schema = match schema {
        Value::Bool(false) => return Err("nothing is allowed here".to_owned()),
        Value::Object(schema) => schema,
        _ => return Ok(()),
    };
    for keyword in ["anyOf", "oneOf"] {
        if let Some(Value::Array(options)) = schema.get(keyword)
            && !options.iter().any(|option| check(value, option).is_ok())
        {
            return Err(none_of(value, options));
        }
    }
    let allowed = types(schema);
    if !allowed.is_empty() && !allowed.iter().any(|name| has_type(value, name)) {
        let names: Vec<&str> = allowed
            .iter()
            .filter(|name| **name != "null")
            .map(|name| type_name(name))
            .collect();
        return Err(format!(
            "expected {}, not {}",
            names.join(" or "),
            kind_of(value)
        ));
    }
    if let Some(Value::Array(allowed)) = schema.get("enum")
        && !allowed.contains(value)
    {
        return Err(one_of(&allowed.iter().collect::<Vec<_>>()));
    }
    if let Some(only) = schema.get("const")
        && only != value
    {
        return Err(format!("expected {}", quoted(only)));
    }
    if let Some(number) = value.as_f64() {
        if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64)
            && number < minimum
        {
            return Err(format!("must be at least {}", plain(minimum)));
        }
        if let Some(maximum) = schema.get("maximum").and_then(Value::as_f64)
            && number > maximum
        {
            return Err(format!("must be at most {}", plain(maximum)));
        }
    }
    if let (Value::Array(items), Some(item)) = (value, schema.get("items")) {
        for (index, element) in items.iter().enumerate() {
            check(element, item).map_err(|message| format!("item {}: {message}", index + 1))?;
        }
    }
    if let Value::Object(entries) = value {
        let properties = schema.get("properties").and_then(Value::as_object);
        let additional = schema.get("additionalProperties");
        // A schema that says nothing about tables allows any.
        let open = properties.is_none() && additional.is_none();
        for (key, entry) in entries {
            let inner = properties.and_then(|known| known.get(key)).or(additional);
            match inner {
                Some(inner) => {
                    check(entry, inner).map_err(|message| format!("`{key}`: {message}"))?;
                }
                None if open => {}
                None => {
                    let known = properties.into_iter().flat_map(|known| known.keys());
                    let hint = nearest(key, known.map(String::as_str))
                        .map(|meant| format!(" (did you mean `{meant}`?)"))
                        .unwrap_or_default();
                    return Err(format!("unknown key `{key}`{hint}"));
                }
            }
        }
        if let Some(Value::Array(required)) = schema.get("required") {
            for name in required.iter().filter_map(Value::as_str) {
                if !entries.contains_key(name) {
                    return Err(format!("`{name}` is missing"));
                }
            }
        }
    }
    Ok(())
}

/// Whether a schema describes a table that is checked key by key rather than as one
/// value: a section with named keys, or a table of entries that are all of one kind.
pub(crate) fn is_table(schema: &Value) -> bool {
    let Some(schema) = schema.as_object() else {
        return false;
    };
    let keyed = schema.contains_key("properties")
        || matches!(
            schema.get("additionalProperties"),
            Some(Value::Object(_) | Value::Bool(true))
        );
    keyed && types(schema).contains(&"object")
}

/// The schema of the entry called `key` in a table, if the table may have one.
pub(crate) fn entry_schema<'a>(schema: &'a Value, key: &str) -> Option<&'a Value> {
    const ANYTHING: &Value = &Value::Bool(true);
    match schema {
        Value::Bool(true) => Some(ANYTHING),
        Value::Object(schema) => schema
            .get("properties")
            .and_then(|known| known.get(key))
            .or_else(|| {
                schema
                    .get("additionalProperties")
                    .filter(|additional| **additional != Value::Bool(false))
            }),
        _ => None,
    }
}

/// What one file contributes, once what is wrong in it has been taken out.
struct Read {
    tree: Map<String, Value>,
    /// The paths of the values the file sets, with the line each is on.
    leaves: Vec<(String, u32)>,
    problems: Vec<Problem>,
    workspace: WorkspaceSettings,
}

struct Checking<'a> {
    file: &'a Path,
    positions: &'a BTreeMap<String, Position>,
    leaves: Vec<(String, u32)>,
    problems: Vec<Problem>,
}

impl Checking<'_> {
    fn position(&self, path: &str) -> Position {
        self.positions.get(path).copied().unwrap_or_default()
    }

    fn problem(&mut self, kind: ProblemKind, path: &[String], message: String) {
        let path = join_path(path);
        let position = self.position(&path);
        self.problems.push(Problem {
            kind,
            path,
            file: Some(self.file.to_owned()),
            line: position.line,
            column: position.column,
            message,
            suggestion: None,
        });
    }

    fn unknown(&mut self, path: &[String], known: Option<&Map<String, Value>>) {
        let key = path.last().map(String::as_str).unwrap_or_default();
        let suggestion = known
            .and_then(|known| nearest(key, known.keys().map(String::as_str)))
            .map(str::to_owned);
        self.problem(
            ProblemKind::UnknownKey,
            path,
            format!("unknown key `{}`", join_path(path)),
        );
        if let Some(problem) = self.problems.last_mut() {
            problem.suggestion = suggestion;
        }
    }

    /// Notes that the file sets the value at `path`.
    ///
    /// A table that may hold anything is laid over the one below it key by key, so it
    /// is the values inside it that the file sets, however deep. Any other value
    /// replaces what was there, whole.
    fn leaf(&mut self, path: &mut Vec<String>, value: &Value, merged: bool) {
        match value {
            Value::Object(entries) if merged && !entries.is_empty() => {
                for (key, entry) in entries {
                    path.push(key.clone());
                    self.leaf(path, entry, merged);
                    path.pop();
                }
            }
            _ => {
                let joined = join_path(path);
                let line = self.position(&joined).line;
                self.leaves.push((joined, line));
            }
        }
    }

    /// Takes out of `table` what its schema does not know or does not allow.
    fn prune(&mut self, table: &mut Map<String, Value>, schema: &Value, path: &mut Vec<String>) {
        let known = schema.get("properties").and_then(Value::as_object);
        let keys: Vec<String> = table.keys().cloned().collect();
        for key in keys {
            path.push(key.clone());
            let keep = match (entry_schema(schema, &key), table.get_mut(&key)) {
                (None, _) | (_, None) => {
                    self.unknown(path, known);
                    false
                }
                (Some(inner), Some(Value::Object(entries))) if is_table(inner) => {
                    self.prune(entries, inner, path);
                    true
                }
                (Some(inner), Some(value)) if is_table(inner) => {
                    let message =
                        format!("`{}` is a table, not {}", join_path(path), kind_of(value));
                    self.problem(ProblemKind::InvalidValue, path, message);
                    false
                }
                (Some(inner), Some(value)) => match check(value, inner) {
                    Ok(()) => {
                        self.leaf(path, value, *inner == Value::Bool(true));
                        true
                    }
                    Err(message) => {
                        let message = format!("`{}`: {message}", join_path(path));
                        self.problem(ProblemKind::InvalidValue, path, message);
                        false
                    }
                },
            };
            if !keep {
                table.remove(&key);
            }
            path.pop();
        }
    }
}

/// Takes a string setting out of the top of a workspace's file.
fn take_string(
    tree: &mut Map<String, Value>,
    key: &str,
    checking: &mut Checking<'_>,
) -> Option<String> {
    match tree.remove(key)? {
        Value::String(text) => Some(text),
        other => {
            let message = format!("`{key}`: expected a string, not {}", kind_of(&other));
            checking.problem(ProblemKind::InvalidValue, &[key.to_owned()], message);
            None
        }
    }
}

fn read(input: &LayerInput) -> Result<Read, Problem> {
    let Document {
        mut tree,
        positions,
    } = parse_document(&input.text).map_err(|error| error.problem(&input.file))?;
    let mut checking = Checking {
        file: &input.file,
        positions: &positions,
        leaves: Vec::new(),
        problems: Vec::new(),
    };
    let mut workspace = WorkspaceSettings::default();
    if matches!(input.kind, LayerKind::Workspace { .. }) {
        workspace.root = take_string(&mut tree, WORKSPACE_ROOT_KEY, &mut checking);
        workspace.default_profile = take_string(&mut tree, WORKSPACE_PROFILE_KEY, &mut checking);
        for section in NOT_IN_A_WORKSPACE {
            if tree.remove(section).is_some() {
                let message = format!(
                    "`{section}` is set in config.toml, not for one workspace; it is ignored here"
                );
                checking.problem(ProblemKind::InvalidValue, &[section.to_owned()], message);
            }
        }
    }
    checking.prune(&mut tree, config_schema(), &mut Vec::new());
    // In the order of the file, whatever order the keys were looked at in.
    checking
        .problems
        .sort_by_key(|problem| (problem.line, problem.column));
    Ok(Read {
        tree,
        leaves: checking.leaves,
        problems: checking.problems,
        workspace,
    })
}

/// Lays `upper` over `lower`. Sections and tables of entries are merged key by key; a
/// value, and a table that is one value (a key's binding), replaces what was there.
fn overlay(lower: &mut Map<String, Value>, upper: Map<String, Value>, schema: &Value) {
    for (key, value) in upper {
        let inner = entry_schema(schema, &key).unwrap_or(&Value::Bool(true));
        let merges = *inner == Value::Bool(true) || is_table(inner);
        match (lower.get_mut(&key), value) {
            (Some(Value::Object(below)), Value::Object(above)) if merges => {
                overlay(below, above, inner);
            }
            (_, value) => {
                lower.insert(key, value);
            }
        }
    }
}

/// Notes the default layer as the source of every value in `tree`.
fn default_sources(
    tree: &Map<String, Value>,
    path: &mut Vec<String>,
    sources: &mut BTreeMap<String, Source>,
) {
    for (key, value) in tree {
        path.push(key.clone());
        match value {
            Value::Object(inner) if !inner.is_empty() => default_sources(inner, path, sources),
            _ => {
                sources.insert(join_path(path), Source::Default);
            }
        }
        path.pop();
    }
}

/// The defaults as a tree, which the files are laid over.
fn default_tree() -> Map<String, Value> {
    match serde_json::to_value(Config::default()) {
        Ok(Value::Object(tree)) => tree,
        // The settings are a struct of plain values; they cannot fail to be a table.
        _ => Map::new(),
    }
}

/// What is understood but does nothing yet, said once the layers are merged.
fn not_available(config: &Config, sources: &BTreeMap<String, Source>, problems: &mut Vec<Problem>) {
    if config.workspace.on_switch == OnSwitch::Keep {
        let path = "workspace.on-switch";
        let location = sources.get(path).and_then(Source::location);
        problems.push(Problem {
            kind: ProblemKind::NotAvailable,
            path: path.to_owned(),
            file: location.map(|(file, _)| file.clone()),
            line: location.map_or(0, |(_, line)| line),
            column: 0,
            message: "`workspace.on-switch = \"keep\"` is not available yet: a workspace's \
                      programs are ended when another workspace is opened"
                .to_owned(),
            suggestion: None,
        });
    }
}

/// Lays the files over the defaults, the first lowest.
pub fn resolve_layers(inputs: &[LayerInput]) -> Layered {
    let schema = config_schema();
    let mut tree = default_tree();
    let mut sources = BTreeMap::new();
    default_sources(&tree, &mut Vec::new(), &mut sources);
    let mut problems = Vec::new();
    let mut fatal = Vec::new();
    let mut workspace = WorkspaceSettings::default();

    for input in inputs {
        let read = match read(input) {
            Ok(read) => read,
            Err(problem) => {
                fatal.push(problem);
                continue;
            }
        };
        problems.extend(read.problems);
        if matches!(input.kind, LayerKind::Workspace { .. }) {
            workspace = read.workspace;
        }
        for (path, line) in read.leaves {
            // What the value replaces had values of its own inside it, perhaps.
            let inside = format!("{path}.");
            sources.retain(|known, _| !known.starts_with(&inside));
            let source = match &input.kind {
                LayerKind::User => Source::User {
                    file: input.file.clone(),
                    line,
                },
                LayerKind::Workspace { name } => Source::Workspace {
                    name: name.clone(),
                    file: input.file.clone(),
                    line,
                },
            };
            sources.insert(path, source);
        }
        overlay(&mut tree, read.tree, schema);
    }

    let config = match serde_json::from_value::<Config>(Value::Object(tree)) {
        Ok(config) => config,
        Err(error) => {
            // The check lets through only what the settings can hold, so this is a
            // fault in the check. The defaults are used rather than nothing.
            problems.push(Problem {
                kind: ProblemKind::InvalidValue,
                path: String::new(),
                file: None,
                line: 0,
                column: 0,
                message: format!(
                    "the settings could not be read, so the defaults are used: {error}"
                ),
                suggestion: None,
            });
            sources.clear();
            default_sources(&default_tree(), &mut Vec::new(), &mut sources);
            Config::default()
        }
    };
    not_available(&config, &sources, &mut problems);
    Layered {
        config,
        sources,
        problems,
        fatal,
        workspace,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{
        BindingEntry, BindingValue, CloseOnExit, CursorStyle, TabPlacement, default_config_text,
    };

    fn user(text: &str) -> LayerInput {
        LayerInput {
            kind: LayerKind::User,
            file: PathBuf::from("config.toml"),
            text: text.to_owned(),
        }
    }

    fn workspace(text: &str) -> LayerInput {
        LayerInput {
            kind: LayerKind::Workspace {
                name: "work".to_owned(),
            },
            file: PathBuf::from("work.toml"),
            text: text.to_owned(),
        }
    }

    #[test]
    fn with_no_files_the_settings_are_the_defaults() {
        let layered = resolve_layers(&[]);
        assert_eq!(layered.config, Config::default());
        assert_eq!(layered.problems, vec![]);
        assert_eq!(layered.fatal, vec![]);
        assert_eq!(layered.sources["font.size"], Source::Default);
        assert_eq!(layered.sources["terminal.close-on-exit"], Source::Default);
    }

    #[test]
    fn a_later_layer_wins_and_every_value_knows_its_layer() {
        let layered = resolve_layers(&[
            user("[font]\nsize = 16\nfamily = \"Iosevka\"\n\n[terminal]\ncursor-style = \"bar\"\n"),
            workspace("root = \"D:/code\"\n\n[font]\nsize = 12.5\n"),
        ]);
        assert_eq!(layered.problems, vec![]);
        assert_eq!(layered.config.font.size, 12.5);
        assert_eq!(layered.config.font.family, "Iosevka");
        assert_eq!(layered.config.terminal.cursor_style, CursorStyle::Bar);
        assert_eq!(layered.workspace.root.as_deref(), Some("D:/code"));

        assert_eq!(
            layered.sources["font.size"],
            Source::Workspace {
                name: "work".to_owned(),
                file: PathBuf::from("work.toml"),
                line: 4,
            }
        );
        assert_eq!(
            layered.sources["font.family"],
            Source::User {
                file: PathBuf::from("config.toml"),
                line: 3,
            }
        );
        assert_eq!(layered.sources["font.line-height"], Source::Default);
    }

    #[test]
    fn an_unknown_key_is_reported_with_its_line_and_the_key_that_was_meant() {
        let layered = resolve_layers(&[user(
            "[font]\nsize = 15\nfamly = \"Iosevka\"\n\n[termnal]\nshell = \"zsh\"\n",
        )]);
        // What is right in the file still counts.
        assert_eq!(layered.config.font.size, 15.0);
        assert_eq!(layered.config.font.family, "JetBrains Mono");
        assert_eq!(layered.problems.len(), 2);

        let key = &layered.problems[0];
        assert_eq!(key.kind, ProblemKind::UnknownKey);
        assert_eq!(key.path, "font.famly");
        assert_eq!((key.line, key.column), (3, 1));
        assert_eq!(key.suggestion.as_deref(), Some("family"));
        assert_eq!(
            key.to_string(),
            "config.toml:3:1: unknown key `font.famly` (did you mean `family`?)"
        );

        let section = &layered.problems[1];
        assert_eq!(section.path, "termnal");
        assert_eq!((section.line, section.column), (5, 2));
        assert_eq!(section.suggestion.as_deref(), Some("terminal"));
    }

    #[test]
    fn a_value_that_is_not_allowed_leaves_the_layer_below_in_place() {
        let layered = resolve_layers(&[
            user("[font]\nsize = 18\n[terminal]\ncursor-style = \"bar\"\n"),
            workspace(
                "[font]\nsize = 500\nweight = \"bold\"\n[terminal]\ncursor-style = \"beam\"\n\
                 scrollback-lines = 2.5\n",
            ),
        ]);
        assert_eq!(layered.config.font.size, 18.0);
        assert_eq!(layered.config.font.weight, 400);
        assert_eq!(layered.config.terminal.cursor_style, CursorStyle::Bar);
        assert_eq!(layered.config.terminal.scrollback_lines, 10_000);

        let messages: Vec<&str> = layered
            .problems
            .iter()
            .map(|problem| problem.message.as_str())
            .collect();
        assert_eq!(
            messages,
            [
                "`font.size`: must be at most 72",
                "`font.weight`: expected a whole number, not a string",
                "`terminal.cursor-style`: expected one of `block`, `bar`, `underline`",
                "`terminal.scrollback-lines`: expected a whole number, not a number",
            ]
        );
        assert!(
            layered
                .problems
                .iter()
                .all(|problem| problem.kind == ProblemKind::InvalidValue && problem.line > 0)
        );
        // The value that was passed over is not the workspace's.
        assert!(matches!(
            layered.sources["font.size"],
            Source::User { line: 2, .. }
        ));
    }

    #[test]
    fn a_number_toml_allows_and_the_settings_do_not_is_refused() {
        let layered = resolve_layers(&[user(
            "[appearance]\ngap = nan\ncard-radius = inf\n[font]\nsize = 99999999999999999999\n",
        )]);
        assert_eq!(layered.config.appearance.gap, None);
        assert_eq!(layered.config.appearance.card_radius, None);
        assert_eq!(layered.config.font.size, 14.0);
        assert_eq!(layered.problems.len() + layered.fatal.len(), 3);
    }

    #[test]
    fn a_file_that_is_not_toml_contributes_nothing_and_the_others_still_count() {
        let layered =
            resolve_layers(&[user("[font]\nsize = 16\n"), workspace("[font\nsize = 12\n")]);
        assert_eq!(layered.config.font.size, 16.0);
        assert_eq!(layered.problems, vec![]);
        assert_eq!(layered.fatal.len(), 1);
        let syntax = &layered.fatal[0];
        assert_eq!(syntax.kind, ProblemKind::Syntax);
        assert_eq!(syntax.file.as_deref(), Some(Path::new("work.toml")));
        assert_eq!(syntax.line, 1);
        assert!(syntax.is_fatal());
    }

    #[test]
    fn a_workspace_file_names_its_folder_and_cannot_set_the_applications_own_sections() {
        let layered = resolve_layers(&[workspace(
            "root = \"/home/u/project\"\ndefault-profile = \"zsh\"\n\n[app]\nkeep-running = true\n\n\
             [workspace]\non-switch = \"keep\"\n\n[layout]\ntab-placement = \"left\"\n",
        )]);
        assert_eq!(layered.workspace.root.as_deref(), Some("/home/u/project"));
        assert_eq!(layered.workspace.default_profile.as_deref(), Some("zsh"));
        assert!(!layered.config.app.keep_running);
        assert_eq!(layered.config.workspace.on_switch, OnSwitch::Close);
        assert_eq!(layered.config.layout.tab_placement, TabPlacement::Left);
        let paths: Vec<&str> = layered
            .problems
            .iter()
            .map(|problem| problem.path.as_str())
            .collect();
        assert_eq!(paths, ["app", "workspace"]);
    }

    #[test]
    fn the_folder_of_a_workspace_is_not_a_setting_of_the_users_file() {
        let layered = resolve_layers(&[user("root = \"/tmp\"\n")]);
        assert_eq!(layered.workspace.root, None);
        assert_eq!(layered.problems.len(), 1);
        assert_eq!(layered.problems[0].kind, ProblemKind::UnknownKey);
    }

    #[test]
    fn keeping_a_workspaces_programs_is_accepted_and_said_to_do_nothing_yet() {
        let layered = resolve_layers(&[user("[workspace]\non-switch = \"keep\"\n")]);
        assert_eq!(layered.config.workspace.on_switch, OnSwitch::Keep);
        assert_eq!(layered.problems.len(), 1);
        let problem = &layered.problems[0];
        assert_eq!(problem.kind, ProblemKind::NotAvailable);
        assert_eq!(problem.line, 2);
        assert!(problem.message.contains("not available yet"));
    }

    #[test]
    fn each_key_binding_is_checked_on_its_own_and_replaces_the_one_below() {
        let layered = resolve_layers(&[
            user(
                "[keybindings]\n\"ctrl+shift+t\" = { command = \"lume.tab.new\", when = \"terminal\" }\n\
                 \"ctrl+shift+y\" = 7\n\"ctrl+shift+u\" = { comand = \"x\" }\n\
                 \"ctrl+shift+i\" = [{ command = \"a\", when = \"palette\" }, { command = \"b\" }]\n\
                 [keybindings.macos]\n\"cmd+d\" = \"lume.pane.split-right\"\n",
            ),
            workspace("[keybindings]\n\"ctrl+shift+t\" = { command = \"lume.window.new\" }\n"),
        ]);
        let keys = &layered.config.keybindings;
        // Replaced whole: the context of the binding below does not leak into this one.
        assert_eq!(
            keys.all["ctrl+shift+t"],
            BindingValue::Entry(BindingEntry {
                command: "lume.window.new".to_owned(),
                when: None,
                args: None,
            })
        );
        assert!(
            matches!(keys.all["ctrl+shift+i"], BindingValue::Entries(ref list) if list.len() == 2)
        );
        assert!(!keys.all.contains_key("ctrl+shift+y"));
        assert!(!keys.all.contains_key("ctrl+shift+u"));
        assert_eq!(keys.macos.len(), 1);

        assert_eq!(layered.problems.len(), 2);
        assert_eq!(layered.problems[0].path, "keybindings.\"ctrl+shift+y\"");
        assert_eq!(layered.problems[1].path, "keybindings.\"ctrl+shift+u\"");
        assert!(
            layered.problems[1]
                .message
                .contains("unknown key `comand` (did you mean `command`?)"),
            "{}",
            layered.problems[1].message
        );
        assert!(matches!(
            layered.sources["keybindings.\"ctrl+shift+t\""],
            Source::Workspace { line: 2, .. }
        ));
    }

    #[test]
    fn token_overrides_of_two_layers_are_merged_token_by_token() {
        let layered = resolve_layers(&[
            user("[appearance.tokens.color]\naccent = \"#ff0000\"\nsurface = \"#000000\"\n"),
            workspace("[appearance.tokens.color]\naccent = \"#00ff00\"\n"),
        ]);
        assert_eq!(layered.problems, vec![]);
        let color = &layered.config.appearance.tokens["color"];
        assert_eq!(color["accent"], "#00ff00");
        assert_eq!(color["surface"], "#000000");
        assert!(matches!(
            layered.sources["appearance.tokens.color.accent"],
            Source::Workspace { line: 2, .. }
        ));
        assert!(matches!(
            layered.sources["appearance.tokens.color.surface"],
            Source::User { line: 3, .. }
        ));
    }

    #[test]
    fn a_section_written_as_a_value_is_refused() {
        let layered = resolve_layers(&[user("font = \"Iosevka\"\n")]);
        assert_eq!(layered.config.font, Config::default().font);
        assert_eq!(
            layered.problems[0].message,
            "`font` is a table, not a string"
        );
    }

    /// The lines of the default file that show a setting at its default, with the
    /// comment mark taken off.
    fn uncommented_defaults() -> String {
        let mut text = String::new();
        for line in default_config_text().lines() {
            let setting = line.strip_prefix("# ").filter(|rest| {
                let key: String = rest.chars().take_while(|c| *c != ' ').collect();
                !key.is_empty()
                    && key.chars().all(|c| c.is_ascii_lowercase() || c == '-')
                    && rest[key.len()..].starts_with(" = ")
            });
            text.push_str(setting.unwrap_or(line));
            text.push('\n');
        }
        text
    }

    #[test]
    fn the_file_a_new_user_is_given_changes_nothing() {
        let as_written = resolve_layers(&[user(default_config_text())]);
        assert_eq!(as_written.config, Config::default());
        assert_eq!(as_written.problems, vec![]);
        assert_eq!(as_written.fatal, vec![]);
    }

    #[test]
    fn the_file_a_new_user_is_given_shows_every_setting_at_its_default() {
        let text = uncommented_defaults();
        let layered = resolve_layers(&[user(&text)]);
        assert_eq!(layered.fatal, vec![], "{text}");
        assert_eq!(layered.problems, vec![]);
        assert_eq!(layered.config, Config::default());

        // Every setting is in the file, at its default or as an example.
        let sections = config_schema()["properties"].as_object().unwrap();
        for (section, schema) in sections {
            // The keys of the bindings are whatever the user binds.
            if section == "keybindings" {
                continue;
            }
            let Some(keys) = schema.get("properties").and_then(Value::as_object) else {
                continue;
            };
            let start = text
                .find(&format!("[{section}]"))
                .unwrap_or_else(|| panic!("the file has no [{section}]"));
            let body = &text[start..];
            let body = &body[..body[1..].find("\n[").map_or(body.len(), |at| at + 1)];
            for key in keys.keys().filter(|key| *key != "tokens") {
                let shown = body.lines().any(|line| {
                    line.trim_start_matches('#')
                        .trim_start()
                        .starts_with(&format!("{key} = "))
                });
                assert!(shown, "[{section}] does not show `{key}`");
            }
        }
    }

    #[test]
    fn whatever_the_check_lets_through_the_settings_can_hold() {
        // One file with a value of every kind the settings take.
        let text = "\
[app]\nkeep-running = true\nstart-workspace = \"work\"\n\
[window]\ntitle-bar = \"system\"\nclose-with-last-tab = false\nconfirm-close = \"always\"\n\
restore-bounds = false\n\
[appearance]\ntheme = \"mine\"\ncard-radius = 4\ngap = 2.5\npane-header = \"auto\"\n\
dim-unfocused = 0\nbackground-image = \"x.png\"\nbackground-blur = 12\nbackground-opacity = 0.5\n\
[appearance.tokens.terminal]\npalette = [\"#000000\"]\n\
[font]\nfallback = []\nsize = 6\nline-height = 1\nweight = 1\nbold-weight = 1000\n\
[terminal]\nshell = \"pwsh\"\nshell-args = [\"-NoLogo\"]\nscrollback-lines = 0\n\
cursor-blink = false\npadding = 0\nclose-on-exit = \"never\"\nconfirm-close = \"always\"\n\
[layout]\ntab-placement = \"right\"\nnew-pane-cwd = \"home\"\n\
[renderer]\ntier = \"canvas2d\"\n\
[plugins.\"lume.metrics\"]\ninterval = 2\n";
        let layered = resolve_layers(&[user(text)]);
        assert_eq!(layered.problems, vec![]);
        assert_eq!(layered.fatal, vec![]);
        assert_eq!(layered.config.font.size, 6.0);
        assert_eq!(layered.config.font.line_height, 1.0);
        assert_eq!(layered.config.terminal.close_on_exit, CloseOnExit::Never);
        assert_eq!(layered.config.plugins["lume.metrics"]["interval"], 2);
    }
}
