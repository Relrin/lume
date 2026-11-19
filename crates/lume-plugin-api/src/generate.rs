//! The files made from the API's definitions: the typings authors write against, the
//! prelude the runtime loads before a plugin's code, and the reference.
//!
//! They are kept in the repository and held equal to what this generates by a test, as
//! the configuration's schemas are.

use std::collections::BTreeMap;

use lume_config::generate::{Generated, GeneratedKind};
use schemars::SchemaGenerator;
use serde_json::Value;

use crate::types::{
    ErrorCode, FsEvent, MetricKind, MetricsSample, PanelExpand, SessionReport, WorkspaceInfo,
};
use crate::{METHODS, Method, ts};

/// The hand-written parts.
const PRELUDE_TEMPLATE: &str = include_str!("prelude.js");
const TYPINGS_TEMPLATE: &str = include_str!("typings.d.ts");

/// Where each generated file goes, from the root of the repository.
pub const TYPINGS_PATH: &str = "packages/plugin-api/src/index.d.ts";
pub const PRELUDE_PATH: &str = "crates/lume-plugin-runtime/src/prelude.generated.js";
pub const REFERENCE_PATH: &str = "docs/plugins/api.md";

/// The calls' schemas, with every type they name.
struct Schemas {
    params: Vec<Value>,
    results: Vec<Value>,
    definitions: BTreeMap<String, Value>,
}

fn schemas() -> Schemas {
    let mut settings = schemars::generate::SchemaSettings::draft2020_12();
    settings.inline_subschemas = false;
    let mut generator: SchemaGenerator = settings.into_generator();
    let mut params = Vec::new();
    let mut results = Vec::new();
    for method in METHODS {
        params.push(Value::from((method.params)(&mut generator)));
        results.push(Value::from((method.result)(&mut generator)));
    }
    // Types the hand-written parts name.
    generator.subschema_for::<SessionReport>();
    generator.subschema_for::<ErrorCode>();
    generator.subschema_for::<MetricsSample>();
    generator.subschema_for::<MetricKind>();
    generator.subschema_for::<FsEvent>();
    generator.subschema_for::<WorkspaceInfo>();
    generator.subschema_for::<PanelExpand>();
    let definitions = generator
        .take_definitions(true)
        .into_iter()
        .collect::<BTreeMap<String, Value>>();
    Schemas {
        params,
        results,
        definitions,
    }
}

/// The schema a reference points at.
fn resolve<'a>(schema: &'a Value, definitions: &'a BTreeMap<String, Value>) -> &'a Value {
    schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|path| path.strip_prefix("#/$defs/"))
        .and_then(|name| definitions.get(name))
        .unwrap_or(schema)
}

/// Each argument of a call in JavaScript: its name, its type and whether it may be left
/// out, with its description.
fn arguments(
    method: &Method,
    params: &Value,
    definitions: &BTreeMap<String, Value>,
) -> Result<Vec<ts::Field>, String> {
    let fields = ts::fields(resolve(params, definitions))?;
    method
        .args
        .iter()
        .map(|arg| {
            fields
                .iter()
                .find(|(name, ..)| name == arg)
                .cloned()
                .ok_or_else(|| format!("{}: `{arg}` is not one of its fields", method.name))
        })
        .collect()
}

fn result_type(result: &Value) -> Result<String, String> {
    let kind = ts::type_of(result)?;
    Ok(if kind == "null" {
        "void".to_owned()
    } else {
        kind
    })
}

/// The calls by namespace, in order, with the parts of each name.
fn namespaces() -> BTreeMap<String, Vec<(usize, &'static Method)>> {
    let mut found: BTreeMap<String, Vec<(usize, &'static Method)>> = BTreeMap::new();
    for (index, method) in METHODS.iter().enumerate() {
        if method.internal {
            continue;
        }
        let (namespace, _) = method.name.rsplit_once('.').unwrap_or(("", method.name));
        found
            .entry(namespace.to_owned())
            .or_default()
            .push((index, method));
    }
    found
}

fn function_name(method: &Method) -> &'static str {
    method.name.rsplit('.').next().unwrap_or(method.name)
}

/// The typings of the API.
pub fn typings() -> Result<String, String> {
    let schemas = schemas();
    let mut types = String::new();
    for (name, schema) in &schemas.definitions {
        types.push_str(&ts::declaration(name, schema)?);
        types.push('\n');
    }
    let mut calls =
        String::from("/** The calls of the API, by namespace. */\nexport interface LumeCalls {\n");
    for (namespace, methods) in namespaces() {
        calls.push_str(&format!("  readonly {namespace}: {{\n"));
        for (index, method) in methods {
            let args = arguments(method, &schemas.params[index], &schemas.definitions)?;
            let mut doc = method.doc.trim().to_owned();
            if let Some(permission) = method.permission {
                doc.push_str(&format!("\n\nNeeds the `{permission}` permission."));
            }
            calls.push_str(&ts::doc(Some(&doc), "    "));
            let list: Vec<String> = args
                .iter()
                .map(|(name, kind, optional, _)| {
                    format!("{name}{}: {kind}", if *optional { "?" } else { "" })
                })
                .collect();
            calls.push_str(&format!(
                "    {}({}): Promise<{}>;\n",
                function_name(method),
                list.join(", "),
                result_type(&schemas.results[index])?
            ));
        }
        calls.push_str("  };\n");
    }
    calls.push_str("}\n");
    Ok(TYPINGS_TEMPLATE
        .replace("/*TYPES*/", types.trim_end())
        .replace("/*CALLS*/", calls.trim_end()))
}

/// The prelude, with a function on `lume` for each call.
pub fn prelude() -> String {
    let mut methods = String::new();
    for (namespace, list) in namespaces() {
        let path: Vec<String> = namespace
            .split('.')
            .map(|part| format!("\"{part}\""))
            .collect();
        methods.push_str(&format!(
            "Object.assign(namespace([{}]), {{\n",
            path.join(", ")
        ));
        for (_, method) in list {
            let args = method.args.join(", ");
            let sent = if args.is_empty() {
                "{}".to_owned()
            } else {
                format!("{{ {args} }}")
            };
            // What only changes what the plugin shows is sent without waiting for an
            // answer.
            let send = if crate::TOLD.contains(&method.name) {
                "told"
            } else {
                "request"
            };
            methods.push_str(&format!(
                "    {}: ({args}) => {send}(\"{}\", {sent}),\n",
                function_name(method),
                method.name
            ));
        }
        methods.push_str("  });\n  ");
    }
    let header = "// Generated by `cargo xtask api` from Lume's definitions of the plugin API.\n\
                  // Do not edit by hand: change the definitions or the template, then generate again.\n";
    let body = PRELUDE_TEMPLATE.replace("/*METHODS*/", methods.trim_end());
    format!("{header}{body}")
}

/// The reference of the calls, in Markdown.
pub fn reference() -> Result<String, String> {
    let schemas = schemas();
    let mut out = String::from(
        "# Plugin API reference\n\n\
         Generated by `cargo xtask api` from Lume's definitions of the API. Every call\n\
         returns a promise; a call that fails rejects it with a `LumeError`, whose `code`\n\
         says why (`denied`, `invalid`, `not_found`, `busy`, `failed`, `stopped`). The\n\
         calls that only change what the plugin shows (`status.set`, `status.clear`,\n\
         `context.set`, `panels.set`, `panels.clear`, `panels.setChildren`) are not\n\
         answered: their promise is settled at once, and a failure goes to the plugin's\n\
         log.\n\n\
         Besides the calls below, the API has `lume.self`, `lume.commands.register`,\n\
         `lume.timers`, `lume.sessions.onEvent`, `lume.settings.onChange`,\n\
         `lume.metrics.subscribe`, `lume.log`, `lume.onDeactivate` and `console`; the\n\
         typings describe them.\n\n",
    );
    for (namespace, methods) in namespaces() {
        out.push_str(&format!("## lume.{namespace}\n\n"));
        for (index, method) in methods {
            let args = arguments(method, &schemas.params[index], &schemas.definitions)?;
            let list: Vec<String> = args
                .iter()
                .map(|(name, _, optional, _)| format!("{name}{}", if *optional { "?" } else { "" }))
                .collect();
            out.push_str(&format!(
                "### `lume.{}({})`\n\n{}\n\n",
                method.name,
                list.join(", "),
                method.doc.trim()
            ));
            if let Some(permission) = method.permission {
                out.push_str(&format!("Needs the `{permission}` permission.\n\n"));
            }
            if !args.is_empty() {
                out.push_str("| Argument | Type | |\n|---|---|---|\n");
                for (name, kind, optional, description) in &args {
                    out.push_str(&format!(
                        "| `{name}` | `{}` | {}{} |\n",
                        kind.replace('|', "\\|"),
                        if *optional { "optional. " } else { "" },
                        description.as_deref().unwrap_or("").replace('\n', " ")
                    ));
                }
                out.push('\n');
            }
            out.push_str(&format!(
                "Gives `{}`.\n\n",
                result_type(&schemas.results[index])?.replace('|', "\\|")
            ));
        }
    }
    out.push_str("## Types\n\n");
    for (name, schema) in &schemas.definitions {
        out.push_str(&format!("### `{name}`\n\n"));
        if let Some(description) = schema.get("description").and_then(Value::as_str) {
            out.push_str(&format!("{}\n\n", description.trim()));
        }
        let fields = ts::fields(schema)?;
        if fields.is_empty() {
            out.push_str(&format!(
                "`{}`\n\n",
                ts::type_of(schema)?.replace('|', "\\|")
            ));
            continue;
        }
        out.push_str("| Field | Type | |\n|---|---|---|\n");
        for (field, kind, optional, description) in fields {
            out.push_str(&format!(
                "| `{field}` | `{}` | {}{} |\n",
                kind.replace('|', "\\|"),
                if optional { "optional. " } else { "" },
                description.unwrap_or_default().replace('\n', " ")
            ));
        }
        out.push('\n');
    }
    Ok(out)
}

/// Every file generated from the API's definitions.
pub fn generated_files() -> Vec<Generated> {
    let failed = |what: &str, error: String| format!("// {what} could not be generated: {error}\n");
    vec![
        Generated {
            kind: GeneratedKind::Api,
            path: TYPINGS_PATH,
            text: typings().unwrap_or_else(|error| failed("the typings", error)),
        },
        Generated {
            kind: GeneratedKind::Api,
            path: PRELUDE_PATH,
            text: prelude(),
        },
        Generated {
            kind: GeneratedKind::Api,
            path: REFERENCE_PATH,
            text: reference().unwrap_or_else(|error| failed("the reference", error)),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_part_can_be_generated() {
        let typings = typings().unwrap();
        assert!(
            typings.contains("export interface StatusValue {"),
            "{typings}"
        );
        assert!(
            typings.contains("set(id: string, value: StatusValue, pane?: string): Promise<void>;"),
            "{typings}"
        );
        assert!(
            typings.contains("get(key: string): Promise<Json>;"),
            "{typings}"
        );
        assert!(typings.contains("list(): Promise<string[]>;"), "{typings}");
        assert!(!typings.contains("/*TYPES*/") && !typings.contains("/*CALLS*/"));
        let prelude = prelude();
        assert!(
            prelude
                .contains("set: (id, value, pane) => told(\"status.set\", { id, value, pane }),"),
            "what only shows something is sent without waiting for an answer"
        );
        assert!(prelude.contains("get: (key) => request(\"kv.get\", { key }),"));
        assert!(
            !prelude.contains("subscribe: (topic)"),
            "internal calls are not on lume"
        );
        let reference = reference().unwrap();
        assert!(reference.contains("### `lume.kv.set(key, value)`"));
    }
}
