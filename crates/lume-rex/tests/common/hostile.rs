//! What a server that means harm, or is broken, might say: names, codes, and every
//! message a real server sent with something in it changed.

use std::collections::HashSet;

use serde_json::{Value, json};

use super::{Recorded, SESSION};

/// Names that show something other than what they are, or are no names.
pub const NAMES: [&str; 12] = [
    "prod\u{202e}gnissim",
    "\u{2066}left\u{2069}\u{200f}right",
    "zero\u{200b}width\u{2060}\u{feff}",
    "two\nlines\r\n\u{2028}more",
    "\u{1b}]52;c;aGVsbG8=\u{7}\u{1b}[2J\u{9b}31m",
    "\u{e0041}\u{e0042}\u{e007f}",
    "",
    "   \t ",
    "\u{200d}\u{200d}",
    "__proto__",
    "<img src=x onerror=alert(1)>",
    "a\u{30d}\u{30d}\u{30d}\u{30d}\u{30d}\u{30d}\u{30d}\u{30d}\u{30d}\u{30d}\u{30d}b",
];

/// What a server might call a failure so that it is taken for one of the application's
/// own, or for a part of the language the page is written in.
pub const CODES: [&str; 16] = [
    "confirmation_required",
    "given_up",
    "ssh",
    "untrusted",
    "not_installed",
    "unreachable",
    "version_mismatch",
    "no_session",
    "forbidden",
    "disconnected",
    "unsupported",
    "__proto__",
    "constructor",
    "toString",
    "",
    "\u{202e}\n",
];

/// The names this client gives a failure the server answered with.
pub const OWN_CODES: [&str; 6] = [
    "bad_request",
    "not_found",
    "conflict",
    "too_large",
    "timeout",
    "refused",
];

/// Whether a text has nothing in it but what is seen, on one line.
pub fn is_plain(text: &str) -> bool {
    let unseen = |char: char| {
        char.is_control()
            || matches!(char,
                '\u{61c}' | '\u{200b}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}'
                | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{e0000}'..='\u{e0fff}')
    };
    !text.contains(unseen)
}

/// Numbers from a seed, the same every time.
pub struct Numbers(pub u64);

impl Numbers {
    pub fn next(&mut self) -> u64 {
        // SplitMix64.
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, end: usize) -> usize {
        usize::try_from(self.next() % end.max(1) as u64).unwrap()
    }

    pub fn bytes(&mut self, count: usize) -> Vec<u8> {
        (0..count).map(|_| self.next() as u8).collect()
    }
}

/// One of each kind of report a real server made, as its name and fields: of the session
/// that is open, and not of the one it was recorded of.
pub fn reports() -> Vec<(String, Value)> {
    let recorded = Recorded::load();
    let mut kinds = HashSet::new();
    let mut found: Vec<(String, Value)> = recorded
        .events()
        .into_iter()
        .filter(|(name, payload)| {
            let event = &payload["event"];
            kinds.insert(format!(
                "{name} {} {}",
                event["type"], event["payload"]["name"]
            ))
        })
        .map(|(name, payload)| (name.to_owned(), payload.clone()))
        .collect();
    for (_, payload) in &mut found {
        if let Some(session) = payload.get_mut("session_id") {
            *session = json!(SESSION);
        }
    }
    found.push((
        "event_dropped".to_owned(),
        json!({ "event_type": "block_event", "session_id": SESSION, "size": 9 }),
    ));
    found
}

/// What is put in place of a value.
fn instead() -> Vec<Value> {
    vec![
        Value::Null,
        json!(true),
        json!(-1),
        json!(0),
        json!(u64::MAX),
        json!(1e308),
        json!(""),
        json!("x".repeat(5_000)),
        json!("\u{202e}\u{1b}]52;c;aGVsbG8=\u{7}"),
        json!([]),
        json!({}),
        json!([[[[[[[[]]]]]]]]),
        json!({ "__proto__": { "x": 1 }, "constructor": null }),
    ]
}

fn pointers(value: &Value, at: &str, found: &mut Vec<String>) {
    match value {
        Value::Object(fields) => {
            for (name, field) in fields {
                let at = format!("{at}/{name}");
                found.push(at.clone());
                pointers(field, &at, found);
            }
        }
        Value::Array(items) => {
            for (nth, item) in items.iter().enumerate() {
                let at = format!("{at}/{nth}");
                found.push(at.clone());
                pointers(item, &at, found);
            }
        }
        _ => {}
    }
}

/// Every way a message is changed in one place: the place, and the message. Each of its
/// values is taken out, and put to each of a list of others.
pub fn changed(message: &Value) -> Vec<(String, Value)> {
    let mut places = Vec::new();
    pointers(message, "", &mut places);
    let mut all = Vec::new();
    for other in instead() {
        all.push((format!("(whole) = {}", brief(&other)), other));
    }
    for place in places {
        let (parent, name) = place.rsplit_once('/').unwrap();
        let mut without = message.clone();
        match without.pointer_mut(parent) {
            Some(Value::Object(fields)) => {
                fields.remove(name);
            }
            Some(Value::Array(items)) => {
                items.remove(name.parse().unwrap());
            }
            _ => unreachable!(),
        }
        all.push((format!("{place} taken out"), without));
        for other in instead() {
            let mut with = message.clone();
            let named = format!("{place} = {}", brief(&other));
            *with.pointer_mut(&place).unwrap() = other;
            all.push((named, with));
        }
    }
    all
}

fn brief(value: &Value) -> String {
    value.to_string().chars().take(24).collect()
}
