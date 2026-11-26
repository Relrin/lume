//! Writing JSON Schemas as TypeScript types.
//!
//! Only what Lume's types use is understood: objects with properties, records, arrays,
//! tuples, strings, numbers, booleans, null, enumerations of strings, constants, unions,
//! intersections and references to named definitions. Anything else is an error, so a
//! type the typings cannot say is found when they are generated, not by a reader.
//!
//! A schema may name its TypeScript type outright with an `x-ts` keyword, for a type
//! whose JSON a schema describes less exactly than TypeScript can, such as a colour
//! written as `#rrggbb`.
//!
//! How the types are written is chosen by [`Options`]: the plugin API's typings are
//! plain and call any JSON value `Json`; the page's copy of the core's types is
//! read-only and calls it `unknown`. The free functions use the plugin API's options.

use serde_json::Value;

/// The keywords that describe a schema without constraining it.
const ANNOTATIONS: [&str; 5] = ["description", "default", "title", "examples", "$schema"];

/// The keyword a schema names its TypeScript type with.
pub const TS_KEYWORD: &str = "x-ts";

/// What a field that may be left out says of `null`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionalNull {
    /// It may be left out instead: `name?: string`. Right for what is written, where a
    /// field that is nothing is left out.
    Strip,
    /// It may be left out or be `null`, and the two may mean different things:
    /// `name?: string | null`. Right for what is read, where `null` can take a value
    /// away and leaving the field out keeps it.
    Keep,
}

/// How types are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// The name of a value that may be any JSON.
    pub json: &'static str,
    /// Fields and arrays are `readonly`.
    pub readonly: bool,
    /// What a field that may be left out says of `null`.
    pub optional_null: OptionalNull,
    /// A union of objects is declared one member to a line, with each member's
    /// description.
    pub multiline_unions: bool,
    /// An object's `type` field, the tag that tells the members of a union apart, comes
    /// first. The others come in the order of their names.
    pub tag_first: bool,
}

impl Options {
    /// The plugin API's typings.
    pub const PLUGIN_API: Options = Options {
        json: "Json",
        readonly: false,
        optional_null: OptionalNull::Strip,
        multiline_unions: false,
        tag_first: false,
    };

    /// The page's copy of what the core writes.
    pub const PAGE: Options = Options {
        json: "unknown",
        readonly: true,
        optional_null: OptionalNull::Strip,
        multiline_unions: true,
        tag_first: true,
    };

    /// The page's copy of what the core reads.
    pub const PAGE_INPUT: Options = Options {
        optional_null: OptionalNull::Keep,
        ..Options::PAGE
    };

    fn readonly(&self) -> &'static str {
        if self.readonly { "readonly " } else { "" }
    }

    /// A TypeScript type for a schema. `#/$defs/Name` references become `Name`.
    pub fn type_of(&self, schema: &Value) -> Result<String, String> {
        match schema {
            Value::Bool(true) => return Ok(self.json.to_owned()),
            Value::Bool(false) => return Ok("never".to_owned()),
            Value::Object(_) => {}
            other => return Err(format!("{other} is not a schema")),
        }
        if let Some(named) = schema.get(TS_KEYWORD).and_then(Value::as_str) {
            return Ok(named.to_owned());
        }
        // An internally tagged variant of a struct: its tag beside a reference to the
        // struct, or the parts all of which it is.
        let mut parts = Vec::new();
        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            parts.push(reference_name(reference)?);
        }
        if let Some(all) = schema.get("allOf").and_then(Value::as_array) {
            for part in all {
                parts.push(self.type_of(part)?);
            }
        }
        if !parts.is_empty() {
            if schema.get("properties").is_some() {
                parts.insert(0, self.object(schema)?);
            }
            return Ok(intersection(parts));
        }
        if let Some(constant) = schema.get("const") {
            return Ok(constant.to_string());
        }
        if let Some(choices) = schema.get("enum").and_then(Value::as_array) {
            let names: Vec<String> = choices.iter().map(Value::to_string).collect();
            return Ok(names.join(" | "));
        }
        for key in ["anyOf", "oneOf"] {
            if let Some(choices) = schema.get(key).and_then(Value::as_array) {
                let types = choices
                    .iter()
                    .map(|choice| self.type_of(choice))
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(union(types));
            }
        }
        let kinds: Vec<&str> = match schema.get("type") {
            Some(Value::String(kind)) => vec![kind.as_str()],
            Some(Value::Array(kinds)) => kinds.iter().filter_map(Value::as_str).collect(),
            // A schema that only describes, any value at all.
            None if schema
                .as_object()
                .is_some_and(|keys| keys.keys().all(|key| ANNOTATIONS.contains(&key.as_str()))) =>
            {
                return Ok(self.json.to_owned());
            }
            _ => return Err(format!("{schema} has no type the typings can say")),
        };
        let mut types = Vec::new();
        for kind in kinds {
            types.push(match kind {
                "string" => "string".to_owned(),
                "number" | "integer" => "number".to_owned(),
                "boolean" => "boolean".to_owned(),
                "null" => "null".to_owned(),
                "array" => self.array(schema)?,
                "object" => self.object(schema)?,
                other => return Err(format!("`{other}` is not a type the typings can say")),
            });
        }
        Ok(union(types))
    }

    fn array(&self, schema: &Value) -> Result<String, String> {
        let readonly = self.readonly();
        if let Some(items) = schema.get("prefixItems").and_then(Value::as_array) {
            let types = items
                .iter()
                .map(|item| self.type_of(item))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(format!("{readonly}[{}]", types.join(", ")));
        }
        let items = schema
            .get("items")
            .map_or(Ok(self.json.to_owned()), |items| self.type_of(items))?;
        Ok(if items.contains(' ') {
            format!("{readonly}({items})[]")
        } else {
            format!("{readonly}{items}[]")
        })
    }

    /// The fields of an object schema.
    pub fn fields(&self, schema: &Value) -> Result<Vec<Field>, String> {
        let required: Vec<&str> = schema
            .get("required")
            .and_then(Value::as_array)
            .map(|names| names.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let mut found = Vec::new();
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (name, property) in properties {
                let optional = !required.contains(&name.as_str());
                let mut kind = self.type_of(property)?;
                // A field that may be left out may also be sent as nothing.
                if optional && self.optional_null == OptionalNull::Strip {
                    kind = kind
                        .split(" | ")
                        .filter(|part| *part != "null")
                        .collect::<Vec<_>>()
                        .join(" | ");
                    if kind.is_empty() {
                        kind = "null".to_owned();
                    }
                }
                let description = property
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                found.push((name.clone(), kind, optional, description));
            }
        }
        if self.tag_first
            && let Some(at) = found.iter().position(|(name, ..)| name == "type")
        {
            let tag = found.remove(at);
            found.insert(0, tag);
        }
        Ok(found)
    }

    /// The type of the values of an object's other keys, if it may have others. Beside
    /// named fields it admits their types too, as TypeScript asks of an index signature.
    fn extra(&self, schema: &Value) -> Result<Option<String>, String> {
        let Some(extra) = (match schema.get("additionalProperties") {
            None | Some(Value::Bool(false)) => None,
            Some(Value::Bool(true)) => Some(self.json.to_owned()),
            Some(other) => Some(self.type_of(other)?),
        }) else {
            return Ok(None);
        };
        let mut types = vec![extra];
        for (_, kind, optional, _) in self.fields(schema)? {
            types.push(kind);
            if optional {
                types.push("undefined".to_owned());
            }
        }
        if types.iter().any(|kind| kind == self.json) {
            return Ok(Some(self.json.to_owned()));
        }
        Ok(Some(union(types)))
    }

    fn object(&self, schema: &Value) -> Result<String, String> {
        let fields = self.fields(schema)?;
        let extra = self.extra(schema)?;
        if fields.is_empty() {
            let record = match extra {
                Some(value) => format!("Record<string, {value}>"),
                None => "Record<string, never>".to_owned(),
            };
            return Ok(if self.readonly {
                format!("Readonly<{record}>")
            } else {
                record
            });
        }
        let readonly = self.readonly();
        let mut out = String::from("{ ");
        for (name, kind, optional, _) in &fields {
            out.push_str(&format!(
                "{readonly}{}{}: {kind}; ",
                key(name),
                if *optional { "?" } else { "" }
            ));
        }
        if let Some(value) = extra {
            out.push_str(&format!("{readonly}[key: string]: {value}; "));
        }
        out.push('}');
        Ok(out)
    }

    /// The fields of an object, one to a line with their descriptions, at `indent`.
    fn field_lines(&self, schema: &Value, indent: &str) -> Result<String, String> {
        let readonly = self.readonly();
        let mut out = String::new();
        for (field, kind, optional, description) in self.fields(schema)? {
            out.push_str(&doc(description.as_deref(), indent));
            out.push_str(&format!(
                "{indent}{readonly}{}{}: {kind};\n",
                key(&field),
                if optional { "?" } else { "" }
            ));
        }
        if let Some(value) = self.extra(schema)? {
            out.push_str(&format!("{indent}{readonly}[key: string]: {value};\n"));
        }
        Ok(out)
    }

    /// A named definition as a TypeScript declaration.
    pub fn declaration(&self, name: &str, schema: &Value) -> Result<String, String> {
        let description = schema.get("description").and_then(Value::as_str);
        let mut out = doc(description, "");
        let is_object = schema.get("type").and_then(Value::as_str) == Some("object")
            && schema.get("properties").is_some()
            && schema.get(TS_KEYWORD).is_none()
            && schema.get("$ref").is_none()
            && schema.get("allOf").is_none();
        let choices = ["oneOf", "anyOf"]
            .iter()
            .find_map(|key| schema.get(*key).and_then(Value::as_array))
            .filter(|_| schema.get(TS_KEYWORD).is_none());
        if is_object {
            out.push_str(&format!("export interface {name} {{\n"));
            out.push_str(&self.field_lines(schema, "  ")?);
            out.push_str("}\n");
        } else if let Some(choices) = choices.filter(|choices| {
            schema.get("oneOf").is_some() && choices.iter().all(|c| c.get("const").is_some())
        }) {
            out.push_str(&format!("export type {name} =\n"));
            for choice in choices {
                out.push_str(&doc(
                    choice.get("description").and_then(Value::as_str),
                    "  ",
                ));
                out.push_str(&format!("  | {}\n", choice["const"]));
            }
            let end = out.trim_end().len();
            out.truncate(end);
            out.push_str(";\n");
        } else if let Some(choices) =
            choices.filter(|choices| self.multiline_unions && choices.iter().any(is_plain_object))
        {
            out.push_str(&format!("export type {name} =\n"));
            for choice in choices {
                out.push_str(&doc(
                    choice.get("description").and_then(Value::as_str),
                    "  ",
                ));
                out.push_str(&format!("  | {}\n", self.member(choice)?));
            }
            let end = out.trim_end().len();
            out.truncate(end);
            out.push_str(";\n");
        } else {
            out.push_str(&format!(
                "export type {name} = {};\n",
                self.type_of(schema)?
            ));
        }
        Ok(out)
    }

    /// One member of a union declared a member to a line: on one line when nothing of it
    /// needs saying, or one field to a line with their descriptions.
    fn member(&self, choice: &Value) -> Result<String, String> {
        let short = self.type_of(choice)?;
        if !is_plain_object(choice) {
            return Ok(short);
        }
        let described = self
            .fields(choice)?
            .iter()
            .any(|(_, _, _, description)| description.is_some());
        if !described && short.len() <= 90 {
            return Ok(short);
        }
        Ok(format!("{{\n{}    }}", self.field_lines(choice, "      ")?))
    }
}

/// An object with properties of its own and nothing besides.
fn is_plain_object(schema: &Value) -> bool {
    schema.get("properties").is_some()
        && schema.get("$ref").is_none()
        && schema.get("allOf").is_none()
        && schema.get(TS_KEYWORD).is_none()
}

fn reference_name(reference: &str) -> Result<String, String> {
    reference
        .strip_prefix("#/$defs/")
        .map(str::to_owned)
        .ok_or_else(|| format!("`{reference}` is not a reference to a definition"))
}

/// A key of an object as TypeScript writes it: quoted unless it is a plain name.
fn key(name: &str) -> String {
    let plain = name
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if plain {
        name.to_owned()
    } else {
        Value::from(name).to_string()
    }
}

fn union(types: Vec<String>) -> String {
    let mut seen: Vec<String> = Vec::new();
    for kind in types {
        let kind = if kind.contains(" & ") && !kind.starts_with('(') {
            format!("({kind})")
        } else {
            kind
        };
        if !seen.contains(&kind) {
            seen.push(kind);
        }
    }
    seen.join(" | ")
}

fn intersection(parts: Vec<String>) -> String {
    parts
        .into_iter()
        .map(|part| {
            if part.contains(" | ") {
                format!("({part})")
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" & ")
}

/// A TypeScript type for a schema, as the plugin API's typings write it.
pub fn type_of(schema: &Value) -> Result<String, String> {
    Options::PLUGIN_API.type_of(schema)
}

/// A doc comment for a description, indented.
pub fn doc(description: Option<&str>, indent: &str) -> String {
    match description.map(str::trim).filter(|text| !text.is_empty()) {
        None => String::new(),
        Some(text) => {
            let mut out = format!("{indent}/**\n");
            for line in text.lines() {
                let line = line.trim_end();
                if line.is_empty() {
                    out.push_str(&format!("{indent} *\n"));
                } else {
                    out.push_str(&format!("{indent} * {line}\n"));
                }
            }
            out.push_str(&format!("{indent} */\n"));
            out
        }
    }
}

/// A field of an object: its name, its type, whether it may be left out, and its
/// description.
pub type Field = (String, String, bool, Option<String>);

/// The fields of an object schema, as the plugin API's typings write them.
pub fn fields(schema: &Value) -> Result<Vec<Field>, String> {
    Options::PLUGIN_API.fields(schema)
}

/// A named definition as a TypeScript declaration, as the plugin API's typings write
/// it.
pub fn declaration(name: &str, schema: &Value) -> Result<String, String> {
    Options::PLUGIN_API.declaration(name, schema)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn schemas_become_types() {
        assert_eq!(type_of(&json!(true)).unwrap(), "Json");
        assert_eq!(type_of(&json!({ "default": null })).unwrap(), "Json");
        assert_eq!(type_of(&json!({ "type": "string" })).unwrap(), "string");
        assert_eq!(
            type_of(&json!({ "type": ["string", "null"] })).unwrap(),
            "string | null"
        );
        assert_eq!(
            type_of(&json!({ "type": "array", "items": { "$ref": "#/$defs/Row" } })).unwrap(),
            "Row[]"
        );
        assert_eq!(
            type_of(&json!({ "type": "array", "items": { "type": ["number", "null"] } })).unwrap(),
            "(number | null)[]"
        );
        assert_eq!(
            type_of(&json!({ "enum": ["a", "b"] })).unwrap(),
            "\"a\" | \"b\""
        );
        assert_eq!(
            type_of(&json!({ "type": "object", "additionalProperties": { "type": "number" } }))
                .unwrap(),
            "Record<string, number>"
        );
        assert!(type_of(&json!({ "type": "tuple" })).is_err());
        assert!(type_of(&json!({ "$ref": "elsewhere.json" })).is_err());
    }

    #[test]
    fn an_object_becomes_an_interface_with_its_descriptions() {
        let text = declaration(
            "Thing",
            &json!({
                "type": "object",
                "description": "A thing.",
                "properties": {
                    "id": { "type": "string", "description": "Its name." },
                    "size": { "type": ["integer", "null"] }
                },
                "required": ["id"]
            }),
        )
        .unwrap();
        assert_eq!(
            text,
            "/**\n * A thing.\n */\nexport interface Thing {\n  /**\n   * Its name.\n   */\n  id: string;\n  size?: number;\n}\n"
        );
    }

    #[test]
    fn the_page_reads_its_types_and_cannot_change_them() {
        let page = Options::PAGE;
        assert_eq!(
            page.type_of(&json!({ "type": "array", "items": { "type": "string" } }))
                .unwrap(),
            "readonly string[]"
        );
        assert_eq!(
            page.type_of(&json!({
                "type": "array",
                "prefixItems": [{ "type": "string" }, { "type": "string" }]
            }))
            .unwrap(),
            "readonly [string, string]"
        );
        assert_eq!(page.type_of(&json!(true)).unwrap(), "unknown");
        assert_eq!(
            page.type_of(&json!({ "type": "object", "additionalProperties": true }))
                .unwrap(),
            "Readonly<Record<string, unknown>>"
        );
        assert_eq!(
            page.type_of(&json!({
                "type": "object",
                "properties": { "pane-header": { "type": "string" } },
                "required": ["pane-header"]
            }))
            .unwrap(),
            "{ readonly \"pane-header\": string; }"
        );
    }

    #[test]
    fn a_tagged_variant_of_a_struct_is_its_tag_and_the_struct() {
        let variant = json!({
            "type": "object",
            "properties": { "type": { "type": "string", "const": "split" } },
            "required": ["type"],
            "$ref": "#/$defs/Split"
        });
        assert_eq!(
            Options::PAGE.type_of(&variant).unwrap(),
            "{ readonly type: \"split\"; } & Split"
        );
        let via_all_of = json!({
            "type": "object",
            "properties": { "type": { "const": "pane" } },
            "required": ["type"],
            "allOf": [{ "$ref": "#/$defs/Pane" }]
        });
        assert_eq!(
            Options::PAGE.type_of(&via_all_of).unwrap(),
            "{ readonly type: \"pane\"; } & Pane"
        );
        let node = json!({ "oneOf": [variant, via_all_of] });
        assert_eq!(
            Options::PAGE.type_of(&node).unwrap(),
            "({ readonly type: \"split\"; } & Split) | ({ readonly type: \"pane\"; } & Pane)"
        );
    }

    #[test]
    fn what_is_read_keeps_null_apart_from_left_out() {
        let schema = json!({
            "type": "object",
            "properties": { "name": { "type": ["string", "null"] } }
        });
        assert_eq!(
            Options::PAGE_INPUT.type_of(&schema).unwrap(),
            "{ readonly name?: string | null; }"
        );
        assert_eq!(
            Options::PAGE.type_of(&schema).unwrap(),
            "{ readonly name?: string; }"
        );
    }

    #[test]
    fn a_schema_may_name_its_type() {
        assert_eq!(
            Options::PAGE
                .type_of(&json!({ "type": "string", "x-ts": "`#${string}`" }))
                .unwrap(),
            "`#${string}`"
        );
    }

    #[test]
    fn a_union_of_objects_is_declared_a_member_to_a_line() {
        let text = Options::PAGE
            .declaration(
                "Target",
                &json!({
                    "oneOf": [
                        {
                            "type": "object",
                            "description": "A pane by id.",
                            "properties": {
                                "type": { "const": "pane" },
                                "pane": { "type": "string" }
                            },
                            "required": ["type", "pane"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "type": { "const": "next" },
                                "window": { "type": "string", "description": "Whose." }
                            },
                            "required": ["type", "window"]
                        }
                    ]
                }),
            )
            .unwrap();
        assert_eq!(
            text,
            "export type Target =\n  /**\n   * A pane by id.\n   */\n  | { readonly type: \"pane\"; readonly pane: string; }\n  | {\n      readonly type: \"next\";\n      /**\n       * Whose.\n       */\n      readonly window: string;\n    };\n"
        );
    }
}
