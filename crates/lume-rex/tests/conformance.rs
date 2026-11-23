//! What this client sends and reads, held to what a server says its API is.
//!
//! `api/19.json` is the shape of every method's arguments and result as a server of
//! that revision describes them (`cargo xtask rex-api`). `fixtures/local.json` is what
//! such a server answered. Here the two are held to each other, and what this client
//! sends and reads is held to both: a request it makes is one the description allows,
//! and a recorded result reads as the type this client reads it as.

mod common;

use std::collections::HashMap;

use common::Recorded;
use lume_rex::View;
use lume_rex::tuning::API_REVISION;
use serde_json::{Value, json};

fn described() -> Value {
    serde_json::from_str(include_str!("../api/19.json")).unwrap()
}

/// The parts of a schema that others point at by name.
fn anchors<'a>(schema: &'a Value, found: &mut HashMap<String, &'a Value>) {
    match schema {
        Value::Object(fields) => {
            if let Some(name) = fields.get("$anchor").and_then(Value::as_str) {
                found.insert(format!("#{name}"), schema);
            }
            fields.values().for_each(|value| anchors(value, found));
        }
        Value::Array(items) => items.iter().for_each(|item| anchors(item, found)),
        _ => {}
    }
}

fn is(value: &Value, kind: &str) -> bool {
    match kind {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        other => panic!("a kind of value this check does not know: {other}"),
    }
}

/// Says what in `value` is not as `schema` describes: as much of JSON Schema as a
/// server's descriptions use.
fn check(
    value: &Value,
    schema: &Value,
    known: &HashMap<String, &Value>,
    at: &str,
    wrong: &mut Vec<String>,
) {
    let Some(schema) = schema.as_object() else {
        return;
    };
    if let Some(name) = schema.get("$ref").and_then(Value::as_str) {
        let target = known
            .get(name)
            .unwrap_or_else(|| panic!("{at}: nothing is called {name}"));
        return check(value, target, known, at, wrong);
    }
    match schema.get("type") {
        Some(Value::String(kind)) if !is(value, kind) => {
            return wrong.push(format!("{at}: {value} is not {kind}"));
        }
        Some(Value::Array(kinds))
            if !kinds
                .iter()
                .filter_map(Value::as_str)
                .any(|kind| is(value, kind)) =>
        {
            return wrong.push(format!("{at}: {value} is none of {kinds:?}"));
        }
        _ => {}
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array)
        && !allowed.contains(value)
    {
        wrong.push(format!("{at}: {value} is not one of {allowed:?}"));
    }
    if let Some(fields) = value.as_object() {
        let properties = schema.get("properties").and_then(Value::as_object);
        for required in schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !fields.contains_key(required) {
                wrong.push(format!("{at}: `{required}` is missing"));
            }
        }
        for (name, field) in fields {
            match properties.and_then(|properties| properties.get(name)) {
                Some(described) => check(field, described, known, &format!("{at}.{name}"), wrong),
                None if schema.get("additionalProperties") == Some(&json!(false)) => {
                    wrong.push(format!("{at}: `{name}` is not described"));
                }
                None => {}
            }
        }
    }
    if let Some(items) = value.as_array() {
        let count = items.len() as u64;
        if schema
            .get("minItems")
            .and_then(Value::as_u64)
            .is_some_and(|least| count < least)
            || schema
                .get("maxItems")
                .and_then(Value::as_u64)
                .is_some_and(|most| count > most)
        {
            wrong.push(format!("{at}: {count} items"));
        }
        if let Some(described) = schema.get("items") {
            for (index, item) in items.iter().enumerate() {
                check(item, described, known, &format!("{at}[{index}]"), wrong);
            }
        }
    }
}

fn wrong_with(value: &Value, schema: &Value, at: &str) -> Vec<String> {
    let mut known = HashMap::new();
    anchors(schema, &mut known);
    let mut wrong = Vec::new();
    check(value, schema, &known, at, &mut wrong);
    wrong
}

/// Every request this client makes of a server, with arguments as it sends them.
fn requests() -> Vec<(&'static str, Value)> {
    let of_session = json!({ "session_id": "session:1" });
    let of_block = json!({ "session_id": "session:1", "block_id": "block:1" });
    vec![
        ("server.version", Value::Null),
        ("session.attach_all", json!({ "event_scope": "session" })),
        ("session.attach", of_session.clone()),
        ("session.view", of_session.clone()),
        ("session.detach", of_session),
        ("com.superlogical.terminal.size", of_block.clone()),
        ("com.superlogical.terminal.process", of_block.clone()),
        ("com.superlogical.terminal.title", of_block.clone()),
        ("com.superlogical.terminal.modes", of_block.clone()),
        (
            "data.connect",
            json!({
                "session_id": "session:1", "block_id": "block:1",
                "data_ref_id": "terminal", "subprotocol": "rex.data.terminal.v1",
            }),
        ),
        (
            "com.superlogical.terminal.write",
            json!({ "session_id": "session:1", "block_id": "block:1", "args": { "data": "ls\r" } }),
        ),
        (
            "com.superlogical.terminal.resize",
            json!({
                "session_id": "session:1", "block_id": "block:1",
                "args": { "columns": 80, "rows": 24, "cell_width_px": 8, "cell_height_px": 16 },
            }),
        ),
        (
            "com.superlogical.terminal.format",
            json!({
                "session_id": "session:1", "block_id": "block:1",
                "args": { "format": "vt", "max_bytes": 900_000, "extra_cursor": true },
            }),
        ),
        (
            "com.superlogical.terminal.blob_read",
            json!({
                "session_id": "session:1", "block_id": "block:1",
                "args": { "id": 7, "offset": 900_000, "max_bytes": 900_000 },
            }),
        ),
        (
            "session.own_block",
            json!({ "session_id": "session:1", "block_ids": ["block:1"], "mode": "take" }),
        ),
        (
            "session.disown_block",
            json!({ "session_id": "session:1", "block_ids": ["block:1"] }),
        ),
        (
            "block.event",
            json!({
                "session_id": "session:1", "block_id": "block:1",
                "event": { "type": "focus", "payload": { "action": "in" } },
            }),
        ),
        ("block.close", of_block),
    ]
}

#[test]
fn the_record_and_the_description_are_of_the_revision_this_client_speaks() {
    let record: Value = serde_json::from_str(include_str!("fixtures/local.json")).unwrap();
    assert_eq!(record["api_revision"], API_REVISION);
    assert_eq!(described()["api_revision"], API_REVISION);
    let welcome = Recorded::load().welcome();
    assert_eq!(welcome["api_revision"], API_REVISION);
}

#[test]
fn every_request_this_client_makes_is_one_the_server_describes() {
    let described = described();
    for (method, payload) in requests() {
        let entry = &described["methods"][method];
        assert!(entry.is_object(), "{method} is not a method of the server");
        // A block's method takes the block and its own arguments; the description
        // gives no shape for the two together.
        if entry["input"].is_null() || payload.is_null() {
            continue;
        }
        let wrong = wrong_with(&payload, &entry["input"], method);
        assert_eq!(wrong, Vec::<String>::new());
    }
}

#[test]
fn what_the_server_answered_is_what_it_describes() {
    let described = described();
    let recorded = Recorded::load();
    let mut checked = 0;
    for (request, response) in recorded.calls() {
        let method = request["method"].as_str().unwrap();
        let entry = &described["methods"][method];
        if !entry.is_object() || response.get("error").is_some() {
            continue;
        }
        let wrong = wrong_with(&response["payload"], &entry["output"], method);
        assert_eq!(wrong, Vec::<String>::new());
        checked += 1;
    }
    assert!(checked >= 10, "{checked}");
}

#[test]
fn the_check_itself_notices_what_is_wrong() {
    let described = described();
    let output = &described["methods"]["session.view"]["output"];
    let mut view = Recorded::load().result("session.view", 0);
    assert_eq!(wrong_with(&view, output, "view"), Vec::<String>::new());

    view["windows"][0]["layers"][0]["bounds"] = json!([0, 0, 1]);
    view["windows"][0]["layers"][0]["layout"]["split"]["direction"] = json!("diagonal");
    view["revision"] = json!("seven");
    view["unheard_of"] = json!(true);
    let wrong = wrong_with(&view, output, "view");
    assert_eq!(wrong.len(), 4, "{wrong:?}");
}

#[test]
fn every_recorded_result_this_client_reads_is_read_as_its_type() {
    let recorded = Recorded::load();
    // The view, each time it was sent: alone, and inside the answer to an attach.
    let attached = recorded.result("session.attach", 0);
    let first: View = serde_json::from_value(attached["view"].clone()).unwrap();
    assert_eq!(first.windows.len(), 1);
    assert_eq!(first.blocks().count(), 1);
    assert!(first.blocks().all(|block| block.is_terminal()));
    assert_eq!(first.label, "lume-fixture");

    let split: View = serde_json::from_value(recorded.result("session.view", 0)).unwrap();
    assert!(split.revision > first.revision);
    assert_eq!(split.blocks().count(), 2);
    let layer = &split.windows[0].layers[0];
    assert_eq!(layer.kind, "tiled");
    assert_eq!(layer.bounds, [0.0, 0.0, 1.0, 1.0]);
    let halves = layer.layout.as_ref().unwrap().split.as_ref().unwrap();
    assert_eq!(halves.direction, "horizontal");
    assert!(halves.before.block_id.is_some() && halves.after.block_id.is_some());
    // The focus stayed where it was when the block beside it was made.
    assert_eq!(split.windows[0].focused_block_id, halves.before.block_id);
}

#[test]
fn the_recorded_events_are_the_ones_the_page_names() {
    let recorded = Recorded::load();
    let events = recorded.events();
    let names: Vec<&str> = events.iter().map(|(name, _)| *name).collect();
    assert_eq!(names.first(), Some(&"session_created"));
    assert_eq!(names.last(), Some(&"session_destroyed"));
    // What happens in a session comes wrapped, with the session it happened in.
    let inner: Vec<&str> = events
        .iter()
        .filter(|(name, _)| *name == "session_event")
        .map(|(_, payload)| {
            assert!(
                payload["session_id"]
                    .as_str()
                    .unwrap()
                    .starts_with("session:")
            );
            payload["event"]["type"].as_str().unwrap()
        })
        .collect();
    for expected in [
        "session_view_changed",
        "session_label_changed",
        "block_created",
    ] {
        assert!(inner.contains(&expected), "{expected} was not recorded");
    }
    // Events are numbered from one with none left out.
    let numbers: Vec<u64> = recorded
        .received()
        .filter(|message| message["type"] == "event")
        .map(|message| message["seq"].as_u64().unwrap())
        .collect();
    assert_eq!(numbers, (1..=numbers.len() as u64).collect::<Vec<_>>());
    // A response's number is that of an event already sent.
    for (_, response) in recorded.calls() {
        assert!(response["seq"].as_u64().unwrap() <= numbers.len() as u64);
    }
}

/// The events of a terminal block that this client reads.
const TERMINAL_EVENTS: [&str; 6] = [
    "bell",
    "process_changed",
    "program_status_changed",
    "program_status_removed",
    "pwd_changed",
    "title_changed",
];

#[test]
fn the_recorded_reports_of_a_terminal_are_as_the_server_describes_them() {
    let recorded = Recorded::load();
    // A server describes the events of its blocks itself, creator by creator.
    let creators = recorded.result("block_creator.list", 0);
    let terminal = creators["creators"]
        .as_array()
        .unwrap()
        .iter()
        .find(|creator| creator["name"] == "com.superlogical.terminal")
        .expect("the terminal's creator was not recorded");
    let described = &terminal["api"]["events"];
    for name in TERMINAL_EVENTS {
        assert!(described[name].is_object(), "{name} is not described");
    }

    let mut read = Vec::new();
    for (name, payload) in recorded.events() {
        if name != "session_event" || payload["event"]["type"] != "block_event" {
            continue;
        }
        let report = &payload["event"]["payload"];
        assert!(report["block_id"].as_str().unwrap().starts_with("block:"));
        assert_eq!(report["creator_name"], "com.superlogical.terminal");
        let name = report["name"].as_str().unwrap();
        if !TERMINAL_EVENTS.contains(&name) {
            continue;
        }
        let fields = report.get("payload").cloned().unwrap_or(json!({}));
        let wrong = wrong_with(&fields, &described[name], name);
        assert!(wrong.is_empty(), "{name}: {wrong:?}");
        read.push(name);
    }
    for expected in ["process_changed", "title_changed"] {
        assert!(read.contains(&expected), "{expected} was not recorded");
    }
}
