//! Keeps what a Rex server says its API is, for the Rex client's tests to be held to.
//!
//! A server describes every method it has: `rex api call help.all` prints the whole
//! description. What is kept of it is each method's name and the shape of its arguments
//! and of its result. The prose that explains them stays with the server.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};

use crate::paths;

/// The parts of a schema that are prose, not shape.
const PROSE: [&str; 4] = ["description", "examples", "title", "$comment"];

/// A schema without its prose.
fn shape(schema: &Value) -> Value {
    match schema {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                // A property may itself be called `title` or `description`. It is then a
                // schema, which is an object; prose is a text or a list.
                .filter(|(key, value)| !PROSE.contains(&key.as_str()) || value.is_object())
                .map(|(key, value)| (key.clone(), shape(value)))
                .collect::<Map<String, Value>>(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(shape).collect()),
        other => other.clone(),
    }
}

fn take(methods: &mut BTreeMap<String, Value>, method: &Value) -> Result<()> {
    // A block's method is called by its full name, which `invoke` gives.
    let name = method
        .pointer("/invoke/method")
        .or_else(|| method.get("name"))
        .and_then(Value::as_str)
        .context("a method has no name")?;
    methods.insert(
        name.to_owned(),
        json!({
            "input": shape(method.get("input_schema").unwrap_or(&Value::Null)),
            "output": shape(method.get("output_schema").unwrap_or(&Value::Null)),
        }),
    );
    Ok(())
}

/// Writes `crates/lume-rex/api/<revision>.json` from a server's `help.all` output.
pub fn capture(dump: &str, revision: &str) -> Result<()> {
    let revision: u32 = revision
        .parse()
        .with_context(|| format!("`{revision}` is not an API revision"))?;
    let text = std::fs::read_to_string(dump).with_context(|| format!("reading {dump}"))?;
    let described: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .with_context(|| format!("{dump} is not JSON"))?;

    let mut methods = BTreeMap::new();
    let listed = described
        .get("methods")
        .and_then(Value::as_array)
        .context("the description lists no methods")?;
    for method in listed {
        take(&mut methods, method)?;
    }
    for creator in described
        .get("block_types")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for method in creator
            .get("methods")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            take(&mut methods, method)?;
        }
    }
    if methods.is_empty() {
        bail!("the description lists no methods");
    }

    let out = paths::root().join(Path::new("crates/lume-rex/api").join(format!("{revision}.json")));
    let kept = json!({ "api_revision": revision, "methods": methods });
    std::fs::write(&out, serde_json::to_string_pretty(&kept)? + "\n")
        .with_context(|| format!("writing {}", out.display()))?;
    println!(
        "rex-api: {} methods written to {}",
        methods.len(),
        out.display()
    );
    Ok(())
}
