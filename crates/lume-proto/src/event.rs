//! What a session reports besides its output, and the state those reports add up to.
//!
//! Everything here is a hint for display. A program in a terminal can print whatever it
//! likes, so a title, a directory or a status says what the program claimed, never what
//! is true. Nothing may be decided on the strength of an event that the user could not
//! have decided for themselves.

use serde::{Deserialize, Serialize};

use crate::tuning::{STATE_STATUSES_MAX_BYTES, STATUS_RECORDS_MAX};

/// Something a session reports besides its output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum SessionEvent {
    Title {
        title: String,
    },
    /// The working directory the program reported, as a URL.
    Cwd {
        url: String,
    },
    Bell,
    /// The shell began to run a command line. `command` is the line, where the shell
    /// said what it was.
    CommandStarted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
    },
    /// The command line the shell was running ended. `exit` is its exit code, where the
    /// shell gave one.
    CommandFinished {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit: Option<i32>,
    },
    /// A program said how its work stands, or took back what it had said.
    ProgramStatus(ProgramStatus),
    /// Another program now has the terminal: the one that reads what is typed.
    Foreground(Foreground),
    /// The session printed something while its view did not have the keyboard.
    Activity,
    /// The connection a session ran over has ended, and this is how.
    Disconnected(Disconnected),
    /// Everything the session has reported so far, in one piece. Sent to a client that
    /// attaches, and after reports had to be left out because they came too fast. It
    /// replaces what the client held and is not news: nothing in it has just happened.
    State {
        state: SessionState,
    },
}

/// How a program's work stands, as the program reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramState {
    /// At rest.
    Idle,
    /// Running.
    Working,
    /// Finished, with a result nobody has looked at yet.
    Done,
    /// Unable to go on until the user does something.
    Blocked,
    /// Failed and stopped.
    Error,
    /// Not a state: the record named, and every record beneath it, is taken back.
    Clear,
}

/// What a blocked program is waiting for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Permission,
    Question,
    Auth,
}

/// One report of a program's status. A terminal holds one record for each `id`; a
/// report replaces its record whole.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramStatus {
    pub state: ProgramState,
    /// Which record this is, as a path such as `build/test`. `None` is the root record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<BlockKind>,
    /// How far the work has come, from 0 to 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
    /// A short name for the program, such as `cargo`. Not a label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// One line for a person to read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ProgramStatus {
    /// Roughly how many bytes a record takes when it is sent. Text may need escaping,
    /// which at worst doubles it.
    fn weight(&self) -> usize {
        let len = |text: &Option<String>| text.as_ref().map_or(0, String::len);
        64 + len(&self.id) + len(&self.app) + 2 * (len(&self.title) + len(&self.message))
    }

    /// A report that says only what state the record named is in.
    pub fn new(state: ProgramState, id: Option<String>) -> Self {
        Self {
            state,
            id,
            kind: None,
            progress: None,
            app: None,
            title: None,
            message: None,
        }
    }

    /// Whether a record with this `id` is the one `self` names or lies beneath it.
    fn covers(&self, id: Option<&str>) -> bool {
        match (&self.id, id) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(own), Some(other)) => other
                .strip_prefix(own.as_str())
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/')),
        }
    }
}

/// The program that has a terminal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Foreground {
    pub pid: u32,
    /// The program's name, without its folder.
    pub name: String,
}

/// Why the connection a session ran over ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisconnectReason {
    /// The shell at the other end ended, and its exit code is the session's.
    Exited,
    /// The connection was closed in good order, by the other end or because the shell
    /// there was ended. The two cannot be told apart.
    Closed,
    /// The other machine did not accept who the user said they were.
    AuthFailed,
    /// The other machine's key is not the one it had before.
    HostKey,
    /// The other machine could not be reached at all.
    Unreachable,
    /// The connection was there and broke.
    ConnectionLost,
    /// Something else, which the client's last words may explain.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Disconnected {
    pub reason: DisconnectReason,
    /// The last line the client printed, where it had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Where a shell is with the command lines it runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommandState {
    Running {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
    },
    Finished {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit: Option<i32>,
    },
}

/// What a session's reports add up to: the latest of each kind.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The working directory as last reported, as a URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<CommandState>,
    /// The status records there are, the one reported longest ago first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statuses: Vec<ProgramStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground: Option<Foreground>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disconnected: Option<Disconnected>,
}

impl SessionState {
    /// Takes one report into account. Reports of something that happened and left
    /// nothing behind (a bell, activity) change nothing.
    pub fn apply(&mut self, event: &SessionEvent) {
        match event {
            SessionEvent::Title { title } => {
                self.title = Some(title.clone()).filter(|title| !title.is_empty());
            }
            SessionEvent::Cwd { url } => self.cwd = Some(url.clone()),
            SessionEvent::CommandStarted { command } => {
                self.command = Some(CommandState::Running {
                    command: command.clone(),
                });
            }
            SessionEvent::CommandFinished { exit } => {
                self.command = Some(CommandState::Finished { exit: *exit });
            }
            SessionEvent::ProgramStatus(status) => self.apply_status(status),
            SessionEvent::Foreground(foreground) => self.foreground = Some(foreground.clone()),
            SessionEvent::Disconnected(disconnected) => {
                self.disconnected = Some(disconnected.clone());
            }
            SessionEvent::State { state } => *self = state.clone(),
            SessionEvent::Bell | SessionEvent::Activity => {}
        }
    }

    fn apply_status(&mut self, status: &ProgramStatus) {
        if status.state == ProgramState::Clear {
            self.statuses
                .retain(|record| !status.covers(record.id.as_deref()));
            return;
        }
        self.statuses.retain(|record| record.id != status.id);
        if self.statuses.len() >= STATUS_RECORDS_MAX {
            // The record nobody has touched for longest makes room.
            self.statuses.remove(0);
        }
        self.statuses.push(status.clone());
    }

    /// The whole state as one event. A state with more status records than one event
    /// can carry goes without the ones reported longest ago.
    pub fn whole(&self) -> SessionEvent {
        let mut state = self.clone();
        let mut total: usize = state.statuses.iter().map(ProgramStatus::weight).sum();
        while total > STATE_STATUSES_MAX_BYTES && !state.statuses.is_empty() {
            total -= state.statuses.remove(0).weight();
        }
        SessionEvent::State { state }
    }

    /// Whether a command line is running, as far as the shell has said.
    pub fn command_running(&self) -> bool {
        matches!(self.command, Some(CommandState::Running { .. }))
    }

    /// Whether nothing has been reported.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn status(state: ProgramState, id: Option<&str>) -> ProgramStatus {
        ProgramStatus::new(state, id.map(str::to_owned))
    }

    fn ids(state: &SessionState) -> Vec<Option<&str>> {
        state
            .statuses
            .iter()
            .map(|record| record.id.as_deref())
            .collect()
    }

    #[test]
    fn events_have_one_json_shape_each() {
        let cases = [
            (
                SessionEvent::Title {
                    title: "vim".to_owned(),
                },
                json!({"type": "title", "title": "vim"}),
            ),
            (SessionEvent::Bell, json!({"type": "bell"})),
            (
                SessionEvent::CommandStarted { command: None },
                json!({"type": "command_started"}),
            ),
            (
                SessionEvent::CommandFinished { exit: Some(-1) },
                json!({"type": "command_finished", "exit": -1}),
            ),
            (
                SessionEvent::ProgramStatus(ProgramStatus {
                    kind: Some(BlockKind::Question),
                    progress: Some(40),
                    app: Some("make".to_owned()),
                    ..status(ProgramState::Blocked, Some("build/test"))
                }),
                json!({
                    "type": "program_status", "state": "blocked", "id": "build/test",
                    "kind": "question", "progress": 40, "app": "make",
                }),
            ),
            (
                SessionEvent::Foreground(Foreground {
                    pid: 7,
                    name: "vim".to_owned(),
                }),
                json!({"type": "foreground", "pid": 7, "name": "vim"}),
            ),
            (SessionEvent::Activity, json!({"type": "activity"})),
            (
                SessionEvent::Disconnected(Disconnected {
                    reason: DisconnectReason::ConnectionLost,
                    detail: None,
                }),
                json!({"type": "disconnected", "reason": "connection_lost"}),
            ),
            (
                SessionEvent::State {
                    state: SessionState {
                        cwd: Some("file://box/tmp".to_owned()),
                        command: Some(CommandState::Finished { exit: Some(2) }),
                        ..SessionState::default()
                    },
                },
                json!({"type": "state", "state": {
                    "cwd": "file://box/tmp",
                    "command": {"state": "finished", "exit": 2},
                }}),
            ),
        ];
        for (event, expected) in cases {
            assert_eq!(serde_json::to_value(&event).unwrap(), expected);
            assert_eq!(
                serde_json::from_value::<SessionEvent>(expected).unwrap(),
                event
            );
        }
    }

    #[test]
    fn a_field_an_event_does_not_have_is_refused() {
        for text in [
            r#"{"type":"foreground","pid":7,"name":"vim","more":1}"#,
            r#"{"type":"program_status","state":"paused"}"#,
            r#"{"type":"disconnected","reason":"bored"}"#,
        ] {
            assert!(
                serde_json::from_str::<SessionEvent>(text).is_err(),
                "{text}"
            );
        }
    }

    #[test]
    fn the_latest_report_of_each_kind_is_what_the_state_holds() {
        let mut state = SessionState::default();
        assert!(state.is_empty());
        for event in [
            SessionEvent::Title {
                title: "one".to_owned(),
            },
            SessionEvent::Bell,
            SessionEvent::Activity,
            SessionEvent::Title {
                title: "two".to_owned(),
            },
            SessionEvent::CommandStarted {
                command: Some("make".to_owned()),
            },
        ] {
            state.apply(&event);
        }
        assert_eq!(state.title.as_deref(), Some("two"));
        assert!(state.command_running());
        state.apply(&SessionEvent::CommandFinished { exit: Some(1) });
        assert_eq!(
            state.command,
            Some(CommandState::Finished { exit: Some(1) })
        );
        // A title set to nothing is no title.
        state.apply(&SessionEvent::Title {
            title: String::new(),
        });
        assert_eq!(state.title, None);
    }

    #[test]
    fn a_status_replaces_its_record_and_a_clear_takes_a_record_and_those_beneath_it() {
        let mut state = SessionState::default();
        let report = |state: &mut SessionState, status: ProgramStatus| {
            state.apply(&SessionEvent::ProgramStatus(status));
        };
        report(&mut state, status(ProgramState::Working, None));
        report(&mut state, status(ProgramState::Working, Some("build")));
        report(
            &mut state,
            status(ProgramState::Working, Some("build/test")),
        );
        report(&mut state, status(ProgramState::Working, Some("builder")));
        // A record reported again moves behind the others and holds only the new report.
        report(
            &mut state,
            ProgramStatus {
                app: Some("make".to_owned()),
                ..status(ProgramState::Done, None)
            },
        );
        assert_eq!(
            ids(&state),
            [Some("build"), Some("build/test"), Some("builder"), None]
        );
        report(&mut state, status(ProgramState::Idle, None));
        assert_eq!(state.statuses.last().unwrap().app, None);

        report(&mut state, status(ProgramState::Clear, Some("build")));
        assert_eq!(ids(&state), [Some("builder"), None]);
        report(&mut state, status(ProgramState::Clear, None));
        assert!(state.statuses.is_empty());
    }

    #[test]
    fn the_records_of_a_session_are_limited_and_the_oldest_goes_first() {
        let mut state = SessionState::default();
        for n in 0..STATUS_RECORDS_MAX + 3 {
            let id = format!("job{n}");
            state.apply(&SessionEvent::ProgramStatus(status(
                ProgramState::Working,
                Some(&id),
            )));
        }
        assert_eq!(state.statuses.len(), STATUS_RECORDS_MAX);
        assert_eq!(state.statuses[0].id.as_deref(), Some("job3"));
    }

    #[test]
    fn a_state_in_one_piece_replaces_what_was_held() {
        let mut state = SessionState {
            title: Some("old".to_owned()),
            ..SessionState::default()
        };
        let whole = SessionState {
            cwd: Some("file://box/srv".to_owned()),
            ..SessionState::default()
        };
        state.apply(&SessionEvent::State {
            state: whole.clone(),
        });
        assert_eq!(state, whole);
    }
}
