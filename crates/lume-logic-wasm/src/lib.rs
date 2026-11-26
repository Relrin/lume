//! The core's layout logic for WebAssembly.
//!
//! The UI harness runs the page against a pretend core so that scenes are quick and need
//! no sessions. A pretend core that changes the layout itself has to be taught every
//! intent the real one knows, and falls behind. This crate builds `lume-layout`'s
//! [`apply`] for `wasm32-unknown-unknown` instead, so the harness changes a layout exactly
//! as the core does.
//!
//! The interface is plain: every call is a name and a JSON document in, a JSON document
//! out, through three exported functions. No bindings generator is needed on either
//! side.
//!
//! - `lume_alloc(len) -> ptr`: room for `len` bytes the caller fills.
//! - `lume_free(ptr, len)`: gives back what `lume_alloc` or `lume_call` handed out.
//! - `lume_call(name_ptr, name_len, input_ptr, input_len) -> u64`: answers a call; the
//!   answer's address is the high 32 bits and its length the low 32.
//!
//! An answer is `{ "ok": true, "value": … }` or `{ "ok": false, "code": …, "error": … }`.

use lume_layout::{Intent, Started, Workspace, apply, needs};
use lume_proto::{BackendId, SessionId};
use serde::Deserialize;
use serde_json::{Value, json};

/// The session started for the pane an intent makes, as the caller says it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GivenSession {
    backend: BackendId,
    session: SessionId,
    #[serde(default)]
    cwd: Option<String>,
}

/// What `layout.apply` and `layout.needs` are given.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IntentOn {
    workspace: Workspace,
    intent: Intent,
    #[serde(default)]
    started: Option<GivenSession>,
}

/// A refusal: a code a program can match on, and what a person reads.
struct Refusal {
    code: String,
    error: String,
}

impl Refusal {
    fn bad_input(error: impl ToString) -> Self {
        Self {
            code: "bad_input".to_owned(),
            error: error.to_string(),
        }
    }
}

/// Answers one call: its name and its JSON in, an answer's JSON out.
pub fn answer(name: &str, input: &str) -> String {
    let result = match name {
        "layout.apply" => layout_apply(input),
        "layout.needs" => layout_needs(input),
        other => Err(Refusal {
            code: "no_such_call".to_owned(),
            error: format!("there is no call `{other}`"),
        }),
    };
    match result {
        Ok(value) => json!({ "ok": true, "value": value }),
        Err(refusal) => json!({ "ok": false, "code": refusal.code, "error": refusal.error }),
    }
    .to_string()
}

/// Applies an intent to a workspace: the workspace after it, what changed, and what it
/// made.
fn layout_apply(input: &str) -> Result<Value, Refusal> {
    let IntentOn {
        mut workspace,
        intent,
        started,
    } = serde_json::from_str(input).map_err(Refusal::bad_input)?;
    let started = started.map(|given| Started {
        backend: given.backend,
        session: given.session,
        cwd: given.cwd,
    });
    let (changes, created) = apply(&mut workspace, intent, started).map_err(|error| Refusal {
        code: error.code().to_owned(),
        error: error.to_string(),
    })?;
    Ok(json!({ "workspace": workspace, "changes": changes, "created": created }))
}

/// Whether an intent needs a session started before it is applied, and how.
fn layout_needs(input: &str) -> Result<Value, Refusal> {
    let IntentOn {
        workspace, intent, ..
    } = serde_json::from_str(input).map_err(Refusal::bad_input)?;
    Ok(json!(needs(&workspace, &intent)))
}

/// Room for `len` bytes, which the caller fills and gives back with [`lume_free`].
#[unsafe(no_mangle)]
pub extern "C" fn lume_alloc(len: usize) -> *mut u8 {
    Box::into_raw(vec![0_u8; len].into_boxed_slice()).cast::<u8>()
}

/// Gives back what [`lume_alloc`] or [`lume_call`] handed out.
///
/// # Safety
///
/// `ptr` and `len` are exactly what one of them handed out, given back once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lume_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: the caller gives back a box of `len` bytes this module made, once.
    drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) });
}

/// Answers a call. The answer's address is the high 32 bits and its length the low 32;
/// the caller gives it back with [`lume_free`].
///
/// # Safety
///
/// Both pointers point at as many bytes as their lengths say, which stay put for the
/// call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lume_call(
    name_ptr: *const u8,
    name_len: usize,
    input_ptr: *const u8,
    input_len: usize,
) -> u64 {
    // SAFETY: the caller vouches for both regions.
    let (name, input) = unsafe {
        (
            std::slice::from_raw_parts(name_ptr, name_len),
            std::slice::from_raw_parts(input_ptr, input_len),
        )
    };
    let text = match (std::str::from_utf8(name), std::str::from_utf8(input)) {
        (Ok(name), Ok(input)) => answer(name, input),
        _ => json!({ "ok": false, "code": "bad_input", "error": "not UTF-8" }).to_string(),
    };
    let bytes = text.into_bytes().into_boxed_slice();
    let len = bytes.len() as u64;
    let ptr = Box::into_raw(bytes).cast::<u8>() as usize as u64;
    (ptr << 32) | len
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    fn vectors() -> Vec<(String, Value)> {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/layout/vectors");
        let mut found: Vec<(String, Value)> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .map(|path| {
                let text = fs::read_to_string(&path).unwrap();
                (
                    path.display().to_string(),
                    serde_json::from_str(&text).unwrap(),
                )
            })
            .collect();
        found.sort_by(|one, other| one.0.cmp(&other.0));
        assert!(!found.is_empty());
        found
    }

    /// The session the vectors' recorder gives the pane of turn `turn`.
    fn started(turn: usize) -> Value {
        json!({ "backend": "local", "session": format!("{:016x}", turn + 1), "cwd": "/work" })
    }

    fn call(name: &str, input: &Value) -> Value {
        serde_json::from_str(&answer(name, &input.to_string())).unwrap()
    }

    #[test]
    fn the_recorded_sequences_come_out_as_the_core_recorded_them() {
        for (name, vector) in vectors() {
            let mut workspace = vector["start"].clone();
            for (turn, step) in vector["steps"].as_array().unwrap().iter().enumerate() {
                let asked = json!({ "workspace": workspace, "intent": step["intent"] });
                let needed = call("layout.needs", &asked);
                assert_eq!(needed["ok"], true, "{name} step {turn}: {needed}");
                let mut input = asked.clone();
                if !needed["value"].is_null() {
                    input["started"] = started(turn);
                }
                let answered = call("layout.apply", &input);
                if let Some(refused) = step.get("refused") {
                    assert_eq!(answered["ok"], false, "{name} step {turn}");
                    assert_eq!(&answered["code"], refused, "{name} step {turn}");
                    continue;
                }
                assert_eq!(answered["ok"], true, "{name} step {turn}: {answered}");
                assert_eq!(
                    answered["value"]["changes"], step["changes"],
                    "{name} step {turn}"
                );
                assert_eq!(
                    answered["value"]["created"], step["created"],
                    "{name} step {turn}"
                );
                workspace = answered["value"]["workspace"].clone();
            }
            assert_eq!(workspace, vector["end"], "{name}");
        }
    }

    #[test]
    fn a_call_it_does_not_know_and_bad_input_are_refused() {
        let unknown = call("layout.nothing", &json!({}));
        assert_eq!(unknown["code"], "no_such_call");
        let bad = call("layout.apply", &json!({ "workspace": 1 }));
        assert_eq!(bad["code"], "bad_input");
    }

    #[test]
    fn what_is_handed_out_comes_back_whole() {
        let name = b"layout.nothing";
        let input = b"{}";
        // SAFETY: both regions are the slices above.
        let packed = unsafe { lume_call(name.as_ptr(), name.len(), input.as_ptr(), input.len()) };
        let len = (packed & 0xffff_ffff) as usize;
        assert!(len > 0);
        let room = lume_alloc(16);
        // SAFETY: what was handed out is given back once.
        unsafe { lume_free(room, 16) };
    }
}
