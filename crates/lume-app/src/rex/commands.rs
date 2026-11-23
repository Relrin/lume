//! The commands about Rex servers, and what a page asks about them.

use std::sync::Arc;

use lume_layout::WindowId;
use serde_json::Value;
use tauri::State;

use crate::error::CommandError;
use crate::host::{Question, ask_first};
use crate::services::{Asker, Services};

type Shared<'a> = State<'a, Arc<Services>>;

/// What every command about servers begins with.
pub const PREFIX: &str = "lume.rex.";

/// How the Rex servers stand.
// Run beside the main thread, which the windows need while they start.
#[tauri::command(async)]
pub fn rex_hosts(services: Shared<'_>) -> Value {
    services.rex.view()
}

/// How the Rex servers stand, asked by something that lists them and is on show. It is
/// asked again for as long as that is so.
#[tauri::command]
pub async fn rex_hosts_look(services: Shared<'_>) -> Result<Value, CommandError> {
    Ok(services.rex.look().await)
}

fn text<'a>(args: &'a Value, name: &str) -> Result<Option<&'a str>, CommandError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(CommandError::bad_request(format!(
            "the argument `{name}` is not text"
        ))),
    }
}

fn needed<'a>(args: &'a Value, name: &str, what: &str) -> Result<&'a str, CommandError> {
    text(args, name)?
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| CommandError::bad_request(format!("{what}: `{name}`")))
}

/// Runs a command about Rex servers.
///
/// A connection that would not be encrypted is made only when the user agreed to it, and
/// a session is ended only when they did. A window asks them and runs the command again
/// with their agreement; whoever asks from outside a window has to say that it is given.
/// A window that says it is not given asks the user itself, and is told what there is to
/// agree to. The agreement is for this once, and is kept nowhere.
pub async fn run(
    services: &Services,
    window: WindowId,
    id: &str,
    args: &Value,
    asker: Asker,
) -> Result<(), CommandError> {
    let host = needed(args, "host", "which server")?;
    let confirmed = args.get("confirmed").and_then(Value::as_bool);
    let agreed = confirmed == Some(true);
    let rex = &services.rex;
    let done = match id {
        "lume.rex.open" => rex.open(host, text(args, "session")?, agreed).await,
        "lume.rex.new-session" => rex.new_session(host, text(args, "label")?, agreed).await,
        "lume.rex.connect" => rex.connect(host, agreed).await,
        "lume.rex.disconnect" => rex.disconnect(host).await,
        "lume.rex.rename-session" => {
            let session = needed(args, "session", "which session")?;
            let label = needed(args, "label", "what it is to be called")?;
            rex.rename_session(host, session, label).await
        }
        "lume.rex.end-session" => {
            let session = needed(args, "session", "which session")?;
            let label = rex.session_label(host, session)?;
            if agreed {
                rex.end_session(host, session).await
            } else {
                Err(CommandError::new(
                    "confirmation_required",
                    format!("ending `{label}` ends what runs in it"),
                ))
            }
        }
        _ => {
            return Err(CommandError::not_found(format!(
                "the application has no command called `{id}`"
            )));
        }
    };
    match done {
        Err(error)
            if error.code == "confirmation_required"
                && asker == Asker::Page
                && confirmed.is_none() =>
        {
            let name = rex.label_of(host).unwrap_or_else(|| host.to_owned());
            let question = if id == "lume.rex.end-session" {
                let session = rex.session_label(host, needed(args, "session", "which session")?)?;
                Question {
                    title: format!("End “{session}”?"),
                    message: format!(
                        "The session is ended on {name}, with every program in it, for \
                         everyone who is connected to it."
                    ),
                    confirm: "End session".to_owned(),
                }
            } else {
                Question {
                    title: "Connect without encryption?".to_owned(),
                    message: format!(
                        "The connection to {name} would not be encrypted. Whoever is on the \
                         network between could read what its terminals show and what you type, \
                         and change it."
                    ),
                    confirm: "Connect anyway".to_owned(),
                }
            };
            ask_first(&services.state, window, id, args, question)
        }
        done => done,
    }
}
