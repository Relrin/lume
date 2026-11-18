//! The session host as the application deals with it: what the user is told about it,
//! the sessions that no pane shows, and the commands that end sessions wholesale.
//!
//! The host keeps sessions running when nothing shows them: after the application has
//! closed, and when a workspace whose programs are kept is put away. A session that no
//! pane of any workspace names, and that no page holds, is *detached*: the user can
//! show it in a new tab, or end it.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lume_layout::{Dir, Intent, NewContent, SpawnRequest, UNITS, WindowId};
use lume_proto::PROTO_VERSION;
use lume_session::cwd::shown_dir;
use lume_session::{BackendId, SessionBackend, SessionId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::carrier::SessionKey;
use crate::error::CommandError;
use crate::panes::Panes;
use crate::state::AppState;
use crate::window::label_of;

/// How long ending every session may take before the application goes on regardless.
/// A program gets a few seconds to end by itself before the host forces it.
const END_ALL_WAIT: Duration = Duration::from_secs(8);

/// How often the host is asked whether the sessions being ended have gone.
const END_ALL_POLL: Duration = Duration::from_millis(50);

/// The command that restarts the host, which the notice about an older host offers.
pub const RESTART_COMMAND: &str = "lume.host.restart";

/// A session that no pane shows, as the page lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DetachedSession {
    pub backend: BackendId,
    pub session: SessionId,
    /// The program the host started.
    pub program: String,
    /// The title the program gave its terminal, if it gave one.
    pub title: Option<String>,
    /// The directory it last said it was in.
    pub cwd: Option<String>,
    pub created_unix_ms: u64,
    /// Whether its program has ended. Such a session still has its last screen.
    pub exited: bool,
    pub pid: Option<u32>,
}

/// Where a session that no pane shows is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Place {
    /// In a tab of its own.
    #[default]
    Tab,
    /// Beside the pane that has the keyboard.
    Split,
    /// In a window of its own.
    Window,
}

/// Tells the user when the host that runs on its own is not this application's
/// version: it was left running by another, and goes on running what that one could.
/// Restarting it updates it, and ends its sessions, so the notice offers it and no
/// more.
pub fn tell_of_an_older_host(state: &AppState, panes: &Panes) {
    for local in state.locals().iter().filter(|local| local.is_daemon()) {
        let hello = local.hello();
        let ours = env!("CARGO_PKG_VERSION");
        if hello.version == ours && hello.proto == PROTO_VERSION {
            continue;
        }
        tracing::info!(
            host = hello.version,
            app = ours,
            "the session host is another version"
        );
        panes.notice_with(
            "info",
            format!(
                "The session host is from Lume {}, and this is Lume {ours}. Restart it to update \
                 it; the programs in your terminals end when it restarts.",
                hello.version
            ),
            Some(json!({ "label": "Restart", "command": RESTART_COMMAND })),
        );
    }
}

/// The sessions of the local backends that no pane of any workspace names and no page
/// holds.
pub async fn detached(
    state: &AppState,
    panes: &Panes,
) -> Result<Vec<DetachedSession>, CommandError> {
    let mut shown: HashSet<SessionKey> = panes
        .workspace()
        .panes()
        .into_iter()
        .filter_map(|pane| panes.session_of(pane.id))
        .collect();
    shown.extend(state.held());
    // The files of the workspaces that are not on show name the sessions kept for
    // them. Those run on the default backend unless a pane says otherwise; a session
    // id is never used twice, so the id alone is enough to tell.
    let named = kept_for_other_workspaces(panes).await;

    let mut detached = Vec::new();
    for local in state.locals() {
        let backend = state.backend_id_of(local);
        for info in local.list().await? {
            let key = SessionKey {
                backend: backend.clone(),
                session: info.id,
            };
            if shown.contains(&key) || named.contains(&info.id) {
                continue;
            }
            detached.push(DetachedSession {
                backend: backend.clone(),
                session: info.id,
                program: info.program,
                title: info.title,
                cwd: info.cwd.as_deref().and_then(shown_dir),
                created_unix_ms: info.created_unix_ms,
                exited: info.exit.is_some(),
                pid: info.pid,
            });
        }
    }
    // The newest first: it is the likeliest to be looked for.
    detached.sort_by_key(|session| std::cmp::Reverse(session.created_unix_ms));
    Ok(detached)
}

/// The sessions the saved files of the workspaces that are not on show name.
async fn kept_for_other_workspaces(panes: &Panes) -> HashSet<SessionId> {
    let store = Arc::clone(&panes.store);
    let on_show = panes.workspace().name;
    tokio::task::spawn_blocking(move || {
        let names = store.list().unwrap_or_default();
        names
            .iter()
            .filter(|name| !name.eq_ignore_ascii_case(&on_show))
            .flat_map(|name| store.sessions_in(name).unwrap_or_default())
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// Shows a session in a new tab of `window`, beside the pane that has the keyboard
/// there, or in a new window. The session may be shown in other panes already: each
/// shows it as it is, and the one focused last gives the terminal its size.
pub async fn reattach(
    panes: &Panes,
    window: WindowId,
    key: SessionKey,
    place: Place,
) -> Result<(), CommandError> {
    let content = NewContent::Terminal(SpawnRequest {
        backend: Some(key.backend.clone()),
        session: Some(key.session),
        ..SpawnRequest::default()
    });
    let intent = match (place, panes.focused_pane(window)) {
        (Place::Split, Some(pane)) => Intent::Split {
            pane,
            dir: Dir::Right,
            ratio: UNITS / 2,
            content,
        },
        (Place::Window, _) => Intent::WindowNew {
            content,
            bounds: None,
        },
        _ => Intent::TabNew {
            window,
            content,
            index: None,
            activate: true,
            group: None,
        },
    };
    panes.apply(intent).await.map(drop)
}

/// Ends a session that no pane shows, and waits a while for its program to have gone,
/// so that a list of sessions read afterwards no longer has it running.
pub async fn end(state: &AppState, key: &SessionKey) -> Result<(), CommandError> {
    state.end(key).await?;
    let ending = std::slice::from_ref(key);
    let deadline = Instant::now() + END_ALL_WAIT;
    while Instant::now() < deadline && still_running(state, ending).await {
        tokio::time::sleep(END_ALL_POLL).await;
    }
    Ok(())
}

/// Ends every session the local backends run, and waits for them to have gone, for a
/// while. The panes that showed them keep their places: they are not closed because
/// their programs ended. Returns how many there were.
pub async fn end_all(state: &AppState, panes: &Panes) -> usize {
    let mut ending = Vec::new();
    for local in state.locals() {
        let backend = state.backend_id_of(local);
        let Ok(listed) = local.list().await else {
            continue;
        };
        for info in listed.into_iter().filter(|info| info.exit.is_none()) {
            ending.push(SessionKey {
                backend: backend.clone(),
                session: info.id,
            });
        }
    }
    panes.expect_ends(&ending);
    for key in &ending {
        if let Err(error) = state.end(key).await {
            tracing::debug!(session = %key.session, %error, "a session could not be ended");
        }
    }
    let deadline = Instant::now() + END_ALL_WAIT;
    while Instant::now() < deadline && still_running(state, &ending).await {
        tokio::time::sleep(END_ALL_POLL).await;
    }
    ending.len()
}

/// Whether any of `keys` is still running.
async fn still_running(state: &AppState, keys: &[SessionKey]) -> bool {
    for local in state.locals() {
        let backend = state.backend_id_of(local);
        let Ok(listed) = local.list().await else {
            continue;
        };
        let running = listed.iter().any(|info| {
            info.exit.is_none()
                && keys
                    .iter()
                    .any(|key| key.backend == backend && key.session == info.id)
        });
        if running {
            return true;
        }
    }
    false
}

/// Ends every session, stops the host that runs on its own and starts a new one, then
/// starts a fresh session in every terminal pane of the workspace on show. Returns how
/// many sessions were ended.
pub async fn restart(state: &AppState, panes: &Panes) -> Result<u32, CommandError> {
    let Some(local) = state
        .locals()
        .iter()
        .find(|local| local.is_daemon())
        .cloned()
    else {
        return Err(CommandError::new(
            "unsupported",
            "the session host runs inside the application; it restarts with Lume",
        ));
    };
    // Every session goes, and the panes that showed one stay for the new one.
    let shown: Vec<SessionKey> = panes
        .workspace()
        .panes()
        .into_iter()
        .filter_map(|pane| panes.session_of(pane.id))
        .collect();
    panes.expect_ends(&shown);
    let ended = local.restart_host().await?;
    for key in &shown {
        state.reports().forget(key);
    }
    let hello = local.hello();
    tracing::info!(
        pid = hello.pid,
        version = hello.version,
        ended,
        "the session host was restarted"
    );
    for pane in panes.workspace().panes() {
        if panes.terminal(pane.id).is_none() {
            continue;
        }
        if let Err(error) = panes.restart(pane.id).await {
            tracing::debug!(pane = %pane.id, %error, "a pane's session was not started again");
        }
    }
    Ok(ended)
}

/// Asks the page of `window` to have the user confirm `command`, which it runs again
/// with `args` and `confirmed` on a yes.
pub fn ask_first(
    state: &AppState,
    window: WindowId,
    command: &str,
    args: &Value,
    question: Question,
) -> Result<(), CommandError> {
    let page = state
        .page_of(&label_of(window))
        .ok_or_else(|| CommandError::not_found("the window has no page to ask in"))?;
    let mut args = match args {
        Value::Object(map) => map.clone(),
        _ => serde_json::Map::new(),
    };
    args.insert("confirmed".to_owned(), Value::Bool(true));
    page.notify(&json!({
        "type": "confirm_requested",
        "title": question.title,
        "message": question.message,
        "confirm": question.confirm,
        "command": command,
        "args": args,
    }));
    Ok(())
}

/// What a page asks the user before a command that cannot be undone.
pub struct Question {
    pub title: String,
    pub message: String,
    /// The label of the button that goes ahead.
    pub confirm: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_is_shown_in_a_tab_unless_a_split_is_asked_for() {
        assert_eq!(Place::default(), Place::Tab);
        let split: Place = serde_json::from_value(json!("split")).unwrap();
        assert_eq!(split, Place::Split);
    }

    #[test]
    fn a_detached_session_is_listed_with_its_place_and_state() {
        let listed = DetachedSession {
            backend: BackendId("local".to_owned()),
            session: SessionId::new(0x0123_4567_89ab_cdef).unwrap(),
            program: "cmd.exe".to_owned(),
            title: None,
            cwd: Some("C:\\work".to_owned()),
            created_unix_ms: 5,
            exited: false,
            pid: Some(7),
        };
        assert_eq!(
            serde_json::to_value(&listed).unwrap(),
            json!({
                "backend": "local",
                "session": "0123456789abcdef",
                "program": "cmd.exe",
                "title": null,
                "cwd": "C:\\work",
                "created_unix_ms": 5,
                "exited": false,
                "pid": 7,
            })
        );
    }
}
