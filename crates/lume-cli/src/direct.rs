//! `lume ctl` with no application running: the session commands, asked of the session
//! host directly.
//!
//! The host that runs on its own keeps its sessions after the application has gone, and
//! serves an endpoint of its own. What concerns one session by its id, and listing them,
//! starting a program and ending one, needs nothing of the application, so it is asked
//! of the host, and answered in the same shape as the application answers it. What
//! needs panes, profiles, the layout or the application's commands is refused as
//! needing the application. Nothing here starts a host: one that is not running has no
//! sessions to ask about.

use std::time::{Duration, Instant};

use lume_ipc::Endpoint;
use lume_proto::app::{AppError, AppReply, AppReplyBody, AppRequestBody, AppSession, codes};
use lume_proto::{
    BackendId, ProtoError, RequestBody, ResponseBody, SessionId, SpawnSpec, TermSize,
};

use crate::client::Failure;
use crate::ctl::NOT_RUNNING;
use crate::host::{self, HostClient};

/// How often the list is read while waiting for a program to end.
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// The size a program started here is given: there is no window to fit.
const SPAWN_SIZE: (u16, u16) = (80, 24);

/// The backend the host's sessions belong to, as the application names it.
const LOCAL_BACKEND: &str = "local";

/// Said of every answer, so that it is plain who gave it.
const FROM_HOST: &str = "(the application is not running; the session host answered)";

/// The host the application on `endpoint` uses, if one serves its endpoint.
pub(crate) async fn reach(endpoint: Option<&str>) -> Result<Option<HostClient>, Failure> {
    let name = lume_proto::host_endpoint_name(endpoint);
    let endpoint = Endpoint::named(&name).map_err(Failure::Broken)?;
    host::reach(&endpoint).await.map_err(Failure::Broken)
}

/// A refusal of something only the application can do.
pub(crate) fn needs_app(what: &str) -> Failure {
    Failure::Refused(AppError::new(
        NOT_RUNNING,
        format!("the application is not running, and {what} needs it"),
    ))
}

fn refused(error: ProtoError) -> Failure {
    Failure::Refused(AppError::new(error.code.as_str(), error.message))
}

fn unexpected(what: &str) -> Failure {
    Failure::Broken(std::io::Error::other(format!(
        "the session host answered {what} with something else"
    )))
}

/// The session a request means, which with no application has to be named by its id.
fn session_named(pane: Option<&String>, session: Option<SessionId>) -> Result<SessionId, Failure> {
    match (pane, session) {
        (None, Some(session)) => Ok(session),
        (Some(_), _) => Err(needs_app("a pane")),
        (None, None) => Err(needs_app("the pane that has the keyboard")),
    }
}

fn answer(effect: impl std::fmt::Display, body: AppReplyBody) -> Result<AppReply, Failure> {
    Ok(AppReply {
        effect: format!("{effect} {FROM_HOST}"),
        body,
    })
}

async fn ask(
    client: &mut HostClient,
    body: RequestBody,
    limit: Option<Duration>,
) -> Result<ResponseBody, Failure> {
    let asked = match limit {
        Some(limit) => client.ask_within(body, limit).await,
        None => client.ask(body).await,
    };
    asked.map_err(Failure::Broken)?.map_err(refused)
}

/// Reads a session's screen, in the shape the application gives it.
pub(crate) async fn screen(
    client: &mut HostClient,
    session: SessionId,
) -> Result<AppReply, Failure> {
    match ask(client, RequestBody::Screen { session }, None).await? {
        ResponseBody::Screen {
            size,
            cursor,
            alternate,
            lines,
        } => answer(
            format!(
                "read {} lines of the screen of session {session}",
                lines.len()
            ),
            AppReplyBody::Screen {
                pane: String::new(),
                session,
                size,
                cursor,
                alternate,
                lines,
            },
        ),
        _ => Err(unexpected("a screen request")),
    }
}

/// Carries out `request` with the host. `timeout` is how long a wait may last.
pub(crate) async fn carry_out(
    client: &mut HostClient,
    request: &AppRequestBody,
    timeout: Duration,
) -> Result<AppReply, Failure> {
    match request {
        AppRequestBody::List {} => match ask(client, RequestBody::List {}, None).await? {
            ResponseBody::Sessions { sessions } => {
                let count = sessions.len();
                let sessions = sessions
                    .into_iter()
                    .map(|info| AppSession {
                        backend: BackendId(LOCAL_BACKEND.to_owned()),
                        pane: None,
                        info,
                        busy: false,
                    })
                    .collect();
                answer(
                    format!("listed {count} sessions"),
                    AppReplyBody::Sessions { sessions },
                )
            }
            _ => Err(unexpected("a list")),
        },
        AppRequestBody::Spawn { spec, .. } => {
            let (Some(launcher), None) = (&spec.launcher, &spec.profile) else {
                return Err(needs_app("starting a profile"));
            };
            let size = spec.size.or(TermSize::new(SPAWN_SIZE.0, SPAWN_SIZE.1).ok());
            let spec = SpawnSpec {
                launcher: launcher.clone(),
                cwd: spec.cwd.clone(),
                env: spec.env.clone(),
                size: size.ok_or_else(|| unexpected("a size"))?,
                shell_integration: false,
                scrollback_lines: None,
            };
            match ask(client, RequestBody::Spawn { spec }, None).await? {
                ResponseBody::Spawned { session } => answer(
                    format!("started session {session}, in no pane"),
                    AppReplyBody::Spawned {
                        session,
                        pane: String::new(),
                        tab: None,
                    },
                ),
                _ => Err(unexpected("a spawn")),
            }
        }
        AppRequestBody::Send {
            pane,
            session,
            text,
        } => {
            let session = session_named(pane.as_ref(), *session)?;
            let body = RequestBody::Send {
                session,
                text: text.clone(),
            };
            match ask(client, body, None).await? {
                ResponseBody::Sent { bytes } => answer(
                    format!("typed {bytes} bytes into session {session}"),
                    AppReplyBody::Sent { bytes },
                ),
                _ => Err(unexpected("a send")),
            }
        }
        AppRequestBody::Screen { pane, session } => {
            let session = session_named(pane.as_ref(), *session)?;
            screen(client, session).await
        }
        AppRequestBody::WaitText {
            pane,
            session,
            text,
            timeout_ms,
        } => {
            let session = session_named(pane.as_ref(), *session)?;
            let body = RequestBody::WaitText {
                session,
                text: text.clone(),
                timeout_ms: *timeout_ms,
            };
            match ask(client, body, Some(timeout + Duration::from_secs(10))).await? {
                ResponseBody::Matched { row, line } => answer(
                    format!("found the text on row {row} of session {session}"),
                    AppReplyBody::Matched {
                        pane: String::new(),
                        session,
                        row,
                        line,
                    },
                ),
                _ => Err(unexpected("a wait")),
            }
        }
        AppRequestBody::WaitExit { pane, session, .. } => {
            let session = session_named(pane.as_ref(), *session)?;
            let deadline = Instant::now() + timeout;
            loop {
                let ResponseBody::Sessions { sessions } =
                    ask(client, RequestBody::List {}, None).await?
                else {
                    return Err(unexpected("a list"));
                };
                let Some(info) = sessions.iter().find(|info| info.id == session) else {
                    return Err(Failure::Refused(AppError::new(
                        codes::NOT_FOUND,
                        format!("the session host has no session {session}"),
                    )));
                };
                if let Some(status) = info.exit {
                    return answer(
                        format!("the program of session {session} ended"),
                        AppReplyBody::Exited { status },
                    );
                }
                if Instant::now() >= deadline {
                    return Err(Failure::Refused(AppError::new(
                        codes::TIMEOUT,
                        format!("the program of session {session} did not end in time"),
                    )));
                }
                tokio::time::sleep(EXIT_POLL_INTERVAL).await;
            }
        }
        AppRequestBody::Kill { session } => {
            match ask(client, RequestBody::Kill { session: *session }, None).await? {
                ResponseBody::Killed {} => {
                    answer(format!("ended session {session}"), AppReplyBody::Killed {})
                }
                _ => Err(unexpected("a kill")),
            }
        }
        AppRequestBody::Status {} => Err(needs_app(
            "its status (lume host status says how the session host is)",
        )),
        _ => Err(needs_app("this command")),
    }
}
