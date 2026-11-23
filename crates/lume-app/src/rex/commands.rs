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

/// Runs a command about Rex servers.
///
/// A connection that would not be encrypted is made only when the user agreed to it. A
/// window asks them and runs the command again with their agreement; whoever asks from
/// outside a window has to say that it is given. The agreement is for the connection
/// being made, and is kept nowhere.
pub async fn run(
    services: &Services,
    window: WindowId,
    id: &str,
    args: &Value,
    asker: Asker,
) -> Result<(), CommandError> {
    let host = text(args, "host")?
        .filter(|host| !host.trim().is_empty())
        .ok_or_else(|| CommandError::bad_request("which server: `host`"))?;
    let agreed = args.get("confirmed").and_then(Value::as_bool) == Some(true);
    let rex = &services.rex;
    let done = match id {
        "lume.rex.open" => rex.open(host, text(args, "session")?, agreed).await,
        "lume.rex.new-session" => rex.new_session(host, text(args, "label")?, agreed).await,
        "lume.rex.connect" => rex.connect(host, agreed).await,
        "lume.rex.disconnect" => rex.disconnect(host).await,
        _ => {
            return Err(CommandError::not_found(format!(
                "the application has no command called `{id}`"
            )));
        }
    };
    match done {
        Err(error) if error.code == "confirmation_required" && asker == Asker::Page => ask_first(
            &services.state,
            window,
            id,
            args,
            Question {
                title: "Connect without encryption?".to_owned(),
                message: format!(
                    "The connection to {host} would not be encrypted. Whoever is on the \
                     network between could read what its terminals show and what you type, \
                     and change it."
                ),
                confirm: "Connect anyway".to_owned(),
            },
        ),
        done => done,
    }
}
