//! What a terminal block reports of itself, as the application hears it of any session.
//!
//! A server sends a block's reports as events on the control connection: the title a
//! program set, the directory it reported, the program in the foreground, the status
//! programs report of their work, the bell. Each is turned into the report a session
//! of the application's own would make. What a terminal reported before anyone looked
//! is asked for when a view arrives.
//!
//! Everything in them comes from programs in the terminal by way of the server: it is
//! cut to size and shown, and nothing else.

use lume_proto::tuning::STATUS_RECORDS_MAX;
use lume_proto::{BlockKind, Foreground, ProgramState, ProgramStatus, SessionEvent, SessionState};
use serde_json::Value;

use crate::api::cut;
use crate::control::Control;
use crate::ids::BlockRef;
use crate::input::of_block;
use crate::tuning::TEXT_MAX_BYTES;

/// A text field, cut to size. Nothing for one that is missing, not text, or empty.
fn text(value: &Value, field: &str) -> Option<String> {
    let mut text = value.get(field)?.as_str()?.to_owned();
    cut(&mut text, TEXT_MAX_BYTES);
    Some(text).filter(|text| !text.is_empty())
}

/// The program in the foreground, as a server describes a process.
fn foreground_of(process: &Value) -> Option<Foreground> {
    Some(Foreground {
        pid: u32::try_from(process.get("pid")?.as_u64()?).ok()?,
        name: text(process, "name")?,
    })
}

/// One record of a program's status. `id` is the record's name; the root record has
/// none.
fn status_of(id: Option<String>, record: &Value) -> Option<ProgramStatus> {
    let state = match record.get("state")?.as_str()? {
        "working" => ProgramState::Working,
        "done" => ProgramState::Done,
        "blocked" => ProgramState::Blocked,
        "error" => ProgramState::Error,
        // A state this client does not know is one at rest.
        _ => ProgramState::Idle,
    };
    let kind = match record.get("kind").and_then(Value::as_str) {
        Some("permission") => Some(BlockKind::Permission),
        Some("question") => Some(BlockKind::Question),
        Some("auth") => Some(BlockKind::Auth),
        _ => None,
    };
    let progress = record
        .get("progress")
        .and_then(Value::as_u64)
        .and_then(|progress| u8::try_from(progress).ok())
        .filter(|progress| *progress <= 100);
    Some(ProgramStatus {
        state,
        id,
        kind,
        progress,
        app: text(record, "app"),
        title: text(record, "title"),
        message: text(record, "msg"),
    })
}

/// What an event of a terminal block says, as reports of a session. Nothing for an
/// event this client does nothing with, or whose fields are not as its name says.
pub(crate) fn read(name: &str, payload: &Value) -> Vec<SessionEvent> {
    let one = |event: Option<SessionEvent>| event.into_iter().collect();
    match name {
        "title_changed" => {
            // A title that was taken back is an empty one.
            let title = payload
                .get("title")
                .and_then(Value::as_str)
                .map(|_| text(payload, "title").unwrap_or_default());
            one(title.map(|title| SessionEvent::Title { title }))
        }
        "pwd_changed" => one(text(payload, "pwd").map(|url| SessionEvent::Cwd { url })),
        "bell" => vec![SessionEvent::Bell],
        "process_changed" => one(payload
            .get("foreground")
            .and_then(foreground_of)
            .map(SessionEvent::Foreground)),
        "program_status_changed" => one(payload
            .get("record")
            .and_then(|record| status_of(text(payload, "id"), record))
            .map(SessionEvent::ProgramStatus)),
        "program_status_removed" => payload
            .get("ids")
            .and_then(Value::as_array)
            .map(|ids| {
                ids.iter()
                    .take(STATUS_RECORDS_MAX)
                    .filter_map(Value::as_str)
                    .map(|id| {
                        let mut id = id.to_owned();
                        cut(&mut id, TEXT_MAX_BYTES);
                        let id = Some(id).filter(|id| !id.is_empty());
                        SessionEvent::ProgramStatus(ProgramStatus::new(ProgramState::Clear, id))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// What a terminal has reported so far, for a view that was not there to hear it.
/// What cannot be read is left out.
pub(crate) async fn state_of(control: &Control, block: &BlockRef) -> SessionState {
    let ask = |method: &'static str| async move {
        control
            .call(method, of_block(block, Value::Null))
            .await
            .map(|response| response.payload)
            .unwrap_or(Value::Null)
    };
    let (title, pwd, process, statuses) = tokio::join!(
        ask("com.superlogical.terminal.title"),
        ask("com.superlogical.terminal.pwd"),
        ask("com.superlogical.terminal.process"),
        ask("com.superlogical.terminal.program_status"),
    );
    let mut state = SessionState {
        title: text(&title, "title"),
        cwd: text(&pwd, "pwd"),
        foreground: process.get("foreground").and_then(foreground_of),
        ..SessionState::default()
    };
    let records = statuses.get("records").and_then(Value::as_array);
    for record in records.into_iter().flatten().take(STATUS_RECORDS_MAX) {
        if let Some(status) = status_of(text(record, "id"), record) {
            state.apply(&SessionEvent::ProgramStatus(status));
        }
    }
    state
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_terminals_events_are_read_as_a_sessions_reports() {
        assert_eq!(
            read(
                "title_changed",
                &json!({ "title": "vim", "owner": { "pid": 4, "name": "vim" } })
            ),
            [SessionEvent::Title {
                title: "vim".to_owned()
            }]
        );
        assert_eq!(
            read("title_changed", &json!({ "title": "" })),
            [SessionEvent::Title {
                title: String::new()
            }]
        );
        assert_eq!(
            read("pwd_changed", &json!({ "pwd": "file://host/home/me" })),
            [SessionEvent::Cwd {
                url: "file://host/home/me".to_owned()
            }]
        );
        assert_eq!(read("bell", &json!({})), [SessionEvent::Bell]);
        assert_eq!(read("bell", &Value::Null), [SessionEvent::Bell]);
        assert_eq!(
            read(
                "process_changed",
                &json!({ "generation": 3, "foreground": { "pid": 77, "user_id": 0, "name": "cargo", "cwd": "/src" } })
            ),
            [SessionEvent::Foreground(Foreground {
                pid: 77,
                name: "cargo".to_owned()
            })]
        );
    }

    #[test]
    fn a_programs_status_is_read_record_by_record() {
        let said = json!({
            "id": "build/test",
            "reason": "report",
            "record": {
                "id": "build/test", "state": "blocked", "kind": "permission", "progress": 40,
                "app": "cargo", "effective_app": "cargo", "title": "tests", "msg": "may I?",
                "updated_at": "2026-11-23T10:00:00Z", "owner": { "pid": 1, "name": "cargo" },
            },
        });
        assert_eq!(
            read("program_status_changed", &said),
            [SessionEvent::ProgramStatus(ProgramStatus {
                state: ProgramState::Blocked,
                id: Some("build/test".to_owned()),
                kind: Some(BlockKind::Permission),
                progress: Some(40),
                app: Some("cargo".to_owned()),
                title: Some("tests".to_owned()),
                message: Some("may I?".to_owned()),
            })]
        );
        // The root record has no name; a state or a kind not known is no state or kind
        // of its own; progress past the end is none.
        let root = json!({
            "id": "", "reason": "report",
            "record": { "state": "dreaming", "kind": "wish", "progress": 200, "updated_at": "" },
        });
        assert_eq!(
            read("program_status_changed", &root),
            [SessionEvent::ProgramStatus(ProgramStatus::new(
                ProgramState::Idle,
                None
            ))]
        );
        assert_eq!(
            read(
                "program_status_removed",
                &json!({ "ids": ["build", "", 7], "reason": "exit" })
            ),
            [
                SessionEvent::ProgramStatus(ProgramStatus::new(
                    ProgramState::Clear,
                    Some("build".to_owned())
                )),
                SessionEvent::ProgramStatus(ProgramStatus::new(ProgramState::Clear, None)),
            ]
        );
    }

    #[test]
    fn what_is_not_as_it_should_be_says_nothing() {
        for (name, payload) in [
            ("title_changed", json!({ "title": 7 })),
            ("title_changed", json!({})),
            ("pwd_changed", json!({ "pwd": "" })),
            (
                "process_changed",
                json!({ "generation": 1, "foreground": null }),
            ),
            (
                "process_changed",
                json!({ "foreground": { "pid": -1, "name": "x" } }),
            ),
            (
                "process_changed",
                json!({ "foreground": { "pid": 5_000_000_000u64, "name": "x" } }),
            ),
            ("process_changed", json!({ "foreground": { "pid": 5 } })),
            (
                "program_status_changed",
                json!({ "id": "a", "record": { "kind": "auth" } }),
            ),
            (
                "program_status_changed",
                json!({ "id": "a", "record": "working" }),
            ),
            ("program_status_removed", json!({ "ids": "all" })),
            ("clipboard_written", json!({ "text": "secret" })),
            ("url_clicked", json!({ "url": "https://example.com" })),
            ("something_new", json!({ "value": 1 })),
        ] {
            assert_eq!(read(name, &payload), [], "{name} {payload}");
        }
        // What a program said is cut to size, and a flood of names is not followed.
        let long = "x".repeat(100_000);
        let [SessionEvent::Title { title }] = &read("title_changed", &json!({ "title": long }))[..]
        else {
            panic!("no title");
        };
        assert_eq!(title.len(), TEXT_MAX_BYTES);
        let many: Vec<String> = (0..10_000).map(|n| format!("id{n}")).collect();
        assert_eq!(
            read("program_status_removed", &json!({ "ids": many })).len(),
            STATUS_RECORDS_MAX
        );
    }
}
