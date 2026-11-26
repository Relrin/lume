//! Files that are made from Lume's Rust definitions and kept in the repository.
//!
//! Several crates write files that other tools read: JSON Schemas for editors,
//! TypeScript for the page and for plugin authors, the token stylesheet. Each crate
//! decides what goes in its files; this crate holds what they share: what a generated
//! file is ([`Generated`]), how JSON is written so the same value is always the same
//! text ([`pretty_json`]), and how a JSON Schema is said in TypeScript ([`ts`]).
//!
//! The files are written by `cargo xtask` and held equal to what the code generates by
//! a test in each crate, so a definition that changes without generating again fails
//! the tests instead of drifting.

pub mod ts;

use serde_json::{Map, Value};

/// Which of the generated files a file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeneratedKind {
    /// A JSON Schema.
    Schema,
    /// Part of the page's token package.
    Tokens,
    /// Made from the plugin API's definitions: its typings, its prelude, its reference.
    Api,
    /// The page's copy of what the core sends and is sent: the layout, the configuration
    /// view, the commands.
    Types,
}

/// One generated file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Generated {
    pub kind: GeneratedKind,
    /// Where the file is, from the root of the repository, with `/` between the parts.
    pub path: &'static str,
    pub text: String,
}

/// A copy of `value` whose tables list their keys in order.
fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(entries) => {
            let mut keys: Vec<&String> = entries.keys().collect();
            keys.sort();
            let mut ordered = Map::new();
            for key in keys {
                ordered.insert(key.clone(), sorted(&entries[key]));
            }
            Value::Object(ordered)
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

/// A JSON value as text a person can read: indented, keys in order, a line break at
/// the end. The same value always comes out as the same text.
pub fn pretty_json(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(&sorted(value)).unwrap_or_default();
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn json_is_written_with_its_keys_in_order_whatever_order_they_were_put_in() {
        let mut first = Map::new();
        first.insert("b".to_owned(), json!({ "z": 1, "a": [{ "y": 1, "x": 2 }] }));
        first.insert("a".to_owned(), json!(true));
        let mut second = Map::new();
        second.insert("a".to_owned(), json!(true));
        second.insert("b".to_owned(), json!({ "a": [{ "x": 2, "y": 1 }], "z": 1 }));
        let text = pretty_json(&Value::Object(first));
        assert_eq!(text, pretty_json(&Value::Object(second)));
        assert!(text.find("\"a\": true").unwrap() < text.find("\"b\"").unwrap());
        assert!(text.ends_with("}\n"));
    }
}
