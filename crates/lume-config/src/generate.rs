//! The files that are made from this crate's definitions and kept in the repository.
//!
//! Editors complete from the JSON Schemas, and the page's code is typed against the
//! token set, so both are files that other tools read. They are generated here, written
//! by a build command, and held equal to what the code generates by a test: a change to
//! a setting or a token that is not followed by generating again fails the test.

use serde_json::{Map, Value};

use crate::schema::config_schema;
use crate::theme::{default_tokens, light_tokens, theme_schema};
use crate::tokens::typescript;

/// Which of the generated files a file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeneratedKind {
    /// A JSON Schema.
    Schema,
    /// Part of the page's token package.
    Tokens,
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

/// Every generated file, with the text it should have.
pub fn generated_files() -> Vec<Generated> {
    let defaults = default_tokens();
    vec![
        Generated {
            kind: GeneratedKind::Schema,
            path: "schemas/config.schema.json",
            text: pretty_json(config_schema()),
        },
        Generated {
            kind: GeneratedKind::Schema,
            path: "schemas/theme.schema.json",
            text: pretty_json(&theme_schema()),
        },
        Generated {
            kind: GeneratedKind::Tokens,
            path: "packages/tokens/src/generated.ts",
            text: typescript(&defaults, &light_tokens()),
        },
        Generated {
            kind: GeneratedKind::Tokens,
            path: "packages/tokens/src/defaults.css",
            text: defaults.css(),
        },
    ]
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

    #[test]
    fn there_is_a_schema_for_the_settings_and_for_themes_and_the_token_files() {
        let files = generated_files();
        let paths: Vec<&str> = files.iter().map(|file| file.path).collect();
        assert_eq!(
            paths,
            [
                "schemas/config.schema.json",
                "schemas/theme.schema.json",
                "packages/tokens/src/generated.ts",
                "packages/tokens/src/defaults.css",
            ]
        );
        for file in &files {
            assert!(file.text.ends_with('\n'), "{}", file.path);
            assert!(!file.text.contains('\r'), "{}", file.path);
        }
        assert!(files[2].text.contains("export const TOKEN_KINDS = {"));
        assert!(files[3].text.contains("--lume-color-surface: #121416;"));
    }
}
