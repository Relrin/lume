//! Views as a server sends them, made by hand: the parts of a layout, and windows and
//! sessions put together from them.

use serde_json::{Value, json};

/// The creator of the blocks that are terminals.
pub const TERMINAL: &str = "com.superlogical.terminal";

pub fn leaf(block: &str) -> Value {
    json!({"rect": [0, 0, 1, 1], "block_id": block})
}

pub fn leaf_at(block: &str, rect: [f64; 4]) -> Value {
    json!({"rect": rect, "block_id": block})
}

pub fn split(id: &str, direction: &str, ratio: f64, before: Value, after: Value) -> Value {
    json!({"rect": [0, 0, 1, 1], "split": {
        "split_id": id, "direction": direction, "ratio": ratio, "before": before, "after": after,
    }})
}

pub fn across(id: &str, ratio: f64, before: Value, after: Value) -> Value {
    split(id, "horizontal", ratio, before, after)
}

pub fn down(id: &str, ratio: f64, before: Value, after: Value) -> Value {
    split(id, "vertical", ratio, before, after)
}

/// The blocks a layout names, in the order they are laid out.
pub fn blocks_in(node: &Value, found: &mut Vec<String>) {
    if let Some(block) = node["block_id"].as_str() {
        found.push(block.to_owned());
    }
    if node["split"].is_object() {
        blocks_in(&node["split"]["before"], found);
        blocks_in(&node["split"]["after"], found);
    }
}

pub fn terminal(block: &str) -> Value {
    json!({
        "block_id": block, "label": "shell", "creator_name": TERMINAL,
        "flavor": "com.superlogical.terminal.shell",
    })
}

/// A layer whose blocks are all terminals.
pub fn layer(id: &str, kind: &str, layout: Value) -> Value {
    let mut blocks = Vec::new();
    blocks_in(&layout, &mut blocks);
    let blocks: Vec<Value> = blocks.iter().map(|block| terminal(block)).collect();
    json!({"layer_id": id, "kind": kind, "bounds": [0, 0, 1, 1], "layout": layout, "blocks": blocks})
}

pub fn window(id: &str, layers: Vec<Value>) -> Value {
    json!({"window_id": id, "label": "", "layers": layers})
}

/// A window with a layout and nothing floating.
pub fn tiled(id: &str, layout: Value) -> Value {
    window(id, vec![layer(&format!("layer:{id}"), "tiled", layout)])
}

pub fn view(windows: Vec<Value>) -> Value {
    let active = windows.first().map(|window| window["window_id"].clone());
    json!({
        "revision": 1, "session_id": "session:s", "label": "work",
        "active_window_id": active, "windows": windows,
    })
}
