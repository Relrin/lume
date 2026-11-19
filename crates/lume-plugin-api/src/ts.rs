//! Writing JSON Schemas as TypeScript types.
//!
//! Only what the API's types use is understood: objects with properties, records,
//! arrays, strings, numbers, booleans, null, enumerations of strings, constants, unions
//! and references to named definitions. Anything else is an error, so a type the
//! typings cannot say is found when they are generated, not by an author.

use serde_json::Value;

/// The keywords that describe a schema without constraining it.
const ANNOTATIONS: [&str; 5] = ["description", "default", "title", "examples", "$schema"];

/// A TypeScript type for a schema. `#/$defs/Name` references become `Name`.
pub fn type_of(schema: &Value) -> Result<String, String> {
    match schema {
        // Whatever crosses to a plugin's code is JSON.
        Value::Bool(true) => return Ok("Json".to_owned()),
        Value::Bool(false) => return Ok("never".to_owned()),
        Value::Object(_) => {}
        other => return Err(format!("{other} is not a schema")),
    }
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        return reference
            .strip_prefix("#/$defs/")
            .map(str::to_owned)
            .ok_or_else(|| format!("`{reference}` is not a reference to a definition"));
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
            let types = choices.iter().map(type_of).collect::<Result<Vec<_>, _>>()?;
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
            return Ok("Json".to_owned());
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
            "array" => {
                let items = schema.get("items").map_or(Ok("Json".to_owned()), type_of)?;
                if items.contains(' ') {
                    format!("({items})[]")
                } else {
                    format!("{items}[]")
                }
            }
            "object" => object(schema)?,
            other => return Err(format!("`{other}` is not a type the typings can say")),
        });
    }
    Ok(union(types))
}

fn union(types: Vec<String>) -> String {
    let mut seen: Vec<String> = Vec::new();
    for kind in types {
        if !seen.contains(&kind) {
            seen.push(kind);
        }
    }
    seen.join(" | ")
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

/// The fields of an object schema.
pub fn fields(schema: &Value) -> Result<Vec<Field>, String> {
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut found = Vec::new();
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for (name, property) in properties {
            let optional = !required.contains(&name.as_str());
            let mut kind = type_of(property)?;
            // A field that may be left out may also be sent as nothing.
            if optional {
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
    Ok(found)
}

fn object(schema: &Value) -> Result<String, String> {
    let fields = fields(schema)?;
    let extra = match schema.get("additionalProperties") {
        None | Some(Value::Bool(false)) => None,
        Some(Value::Bool(true)) => Some("Json".to_owned()),
        Some(other) => Some(type_of(other)?),
    };
    if fields.is_empty() {
        return Ok(match extra {
            Some(value) => format!("Record<string, {value}>"),
            None => "Record<string, never>".to_owned(),
        });
    }
    let mut out = String::from("{ ");
    for (name, kind, optional, _) in &fields {
        out.push_str(&format!(
            "{name}{}: {kind}; ",
            if *optional { "?" } else { "" }
        ));
    }
    if let Some(value) = extra {
        out.push_str(&format!("[key: string]: {value}; "));
    }
    out.push('}');
    Ok(out)
}

/// A named definition as a TypeScript declaration.
pub fn declaration(name: &str, schema: &Value) -> Result<String, String> {
    let description = schema.get("description").and_then(Value::as_str);
    let mut out = doc(description, "");
    let is_object = schema.get("type").and_then(Value::as_str) == Some("object")
        && schema.get("properties").is_some();
    if is_object {
        out.push_str(&format!("export interface {name} {{\n"));
        for (field, kind, optional, description) in fields(schema)? {
            out.push_str(&doc(description.as_deref(), "  "));
            out.push_str(&format!(
                "  {field}{}: {kind};\n",
                if optional { "?" } else { "" }
            ));
        }
        out.push_str("}\n");
    } else if let Some(choices) = schema.get("oneOf").and_then(Value::as_array)
        && choices.iter().all(|choice| choice.get("const").is_some())
    {
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
    } else {
        out.push_str(&format!("export type {name} = {};\n", type_of(schema)?));
    }
    Ok(out)
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
}
