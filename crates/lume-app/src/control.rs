//! The application's control endpoint.
//!
//! The application serves a local endpoint that only its user can reach. The command-line
//! tool drives the application through it, and a second copy of the application uses it
//! to hand what it was started for to the first one instead of opening another.
//!
//! A connection carries requests one after another, each answered before the next is
//! read. Every answer says what the request changed. What needs a terminal's screen is
//! passed on to the page that shows the terminal; the rest is answered here.
//!
//! A request about one terminal names a pane, a session, or neither, which means the
//! pane that has the keyboard. A request that arrives while the application is still
//! starting waits a while for what it needs: a page, a session in the pane.

use std::future::Future;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use lume_ipc::{Endpoint, Listener};
use lume_layout::{Bounds, Dir, Intent, NewContent, PaneId, SpawnRequest, UNITS, WindowId};
use lume_proto::app::{
    APP_PROTO_VERSION, AppDone, AppError, AppEvent, AppMessage, AppOutput, AppReply, AppReplyBody,
    AppRequest, AppRequestBody, AppResponse, AppSession, Direction, Focused, PageRequestBody,
    SessionNews, SpawnPlace, codes,
};
use lume_proto::{
    Body, BytesKind, ClientFacts, ClientKind, ColorScheme, ControlDecoder, ErrorCode, FrameDecoder,
    HydratePayload, Message, Palette, Responder, Role, Secret, SessionEvent, SessionId, TermSize,
    encode, encode_bytes, negotiate,
};
use lume_session::{Attachment, Hydrate, InputSink, OutputFrame, SessionBackend, SessionError};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::broadcast::error::RecvError;
use zeroize::Zeroizing;

use crate::carrier::{Link, SessionKey};
use crate::rex;
use crate::services::{Asker, Services};
use crate::sessions::News;
use crate::tuning::{ATTACH_CELL_PX, PAGE_ANSWER_TIMEOUT, PAGE_OPEN_TIMEOUT, WAIT_MAX, WAIT_SLACK};
use crate::verbs::{OUTPUT_PIECE_MAX, STDIN_MAX, Step, pieces, read_step};
use crate::window::label_of;

/// How many bytes are read from a client at a time.
const READ_CHUNK: usize = 16 * 1024;

/// The stream an attachment is carried on, on a connection that asked for one.
const ATTACHED_STREAM: u32 = 1;

/// What the endpoint, and the application itself, can do to its windows.
pub trait Shell: Send + Sync + 'static {
    /// Brings the window the user was working in to the front.
    fn activate(&self);
    /// Closes the application as closing its last window would.
    fn quit(&self);
    /// Whether the application serves its own copy of the page.
    fn bundled_ui(&self) -> bool;
    /// Closes the window with this label.
    fn close_window(&self, label: &str);
    /// Hides the window that shows a window of the layout, keeping its page, to be
    /// shown again by `activate`.
    fn hide_window(&self, window: WindowId);
    /// Opens the window that shows a window of the layout, where it was kept if a
    /// place is given. A window that is open already is left as it is.
    fn open_window(&self, window: WindowId, bounds: Option<Bounds>);
    /// The labels of the open windows that show windows of the layout.
    fn layout_windows(&self) -> Vec<String>;
    /// Draws every window with the page's own title bar, or with the system's.
    fn set_title_bar(&self, custom: bool);
    /// Rounds the corners of every window's frame as asked, where the window system
    /// lets the application choose.
    fn set_corners(&self, corners: lume_config::WindowCorners);
    /// Shows the settings window, at a section if one is named: opens it in the middle
    /// of the screen the window `from` is on, or brings it forward if it is open.
    fn open_settings(&self, section: Option<&str>, from: Option<WindowId>);
    /// Puts text on the clipboard, replacing what was there.
    fn copy_text(&self, text: String);
    /// Asks a window to close as pressing its own close control does, which may put a
    /// question to the user first.
    fn ask_close(&self, window: WindowId);
    /// Whether the window that shows a window of the layout is the one the user is
    /// working in: it has the keyboard.
    fn window_focused(&self, window: WindowId) -> bool;
    /// Has a window ask for the user's attention, as the system does it: its taskbar
    /// button flashes, or the dock icon bounces.
    fn request_attention(&self, window: WindowId);
    /// Shows a notification on the desktop.
    fn notify(&self, title: &str, body: &str);
    /// Opens the developer tools of the window that shows a window of the layout,
    /// where this build has them. Says whether it did.
    fn inspect(&self, _window: WindowId) -> bool {
        false
    }
}

/// What a request needs to be carried out.
pub struct Control {
    services: Arc<Services>,
    /// How long a request waits for a page that has not connected yet, or for a pane
    /// that has no session yet.
    page_wait: Duration,
}

/// What to do once an answer has been written.
enum Then {
    Carry,
    /// The application was asked to close; it does once the client has its answer.
    Quit,
    /// The client asked to follow sessions; their reports are sent to it from here on.
    Follow(Followed),
    /// The client asked to be shown a session; the connection carries the attachment
    /// from here on.
    Attach(Box<Attachment>),
    /// The client asked for a verb; what it prints is sent to it from here on.
    Verb(Box<VerbRun>),
}

/// A verb that is about to be run for a client.
struct VerbRun {
    /// The plugin's command that carries it out, by its full id.
    command: String,
    /// What the command is first called with.
    first: serde_json::Value,
}

/// Which sessions a client follows: the one in a pane, whichever that is at the time,
/// one session, or with neither named, all of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Followed {
    pane: Option<PaneId>,
    session: Option<SessionId>,
}

fn reply(effect: impl Into<String>, body: AppReplyBody) -> Result<AppReply, AppError> {
    Ok(AppReply {
        effect: effect.into(),
        body,
    })
}

fn no_window() -> AppError {
    AppError::new(
        codes::NO_WINDOW,
        "the application has no window with a page that could answer",
    )
}

/// The time a request may wait: what it asked for, within what any request may.
fn wait_limit(timeout_ms: u64) -> Duration {
    Duration::from_millis(timeout_ms).min(WAIT_MAX)
}

fn layout_dir(direction: Direction) -> Dir {
    match direction {
        Direction::Left => Dir::Left,
        Direction::Right => Dir::Right,
        Direction::Up => Dir::Up,
        Direction::Down => Dir::Down,
    }
}

/// A pane and the window it is in.
#[derive(Clone, Copy, Debug)]
struct Place {
    pane: PaneId,
    window: WindowId,
}

impl Control {
    /// `page_wait` is how long a request waits for a page that has not connected yet.
    pub fn new(services: Arc<Services>, page_wait: Duration) -> Arc<Self> {
        Arc::new(Self {
            services,
            page_wait,
        })
    }

    fn focused_window(&self) -> Result<WindowId, AppError> {
        self.services.panes.focused_window().ok_or_else(no_window)
    }

    /// The page of a window, waiting a while for one that is still loading.
    async fn page_of(&self, window: WindowId) -> Result<Arc<Link>, AppError> {
        let label = label_of(window);
        self.services
            .state
            .soon(self.page_wait, |state| state.page_of(&label))
            .await
            .ok_or_else(no_window)
    }

    /// The pane a request means: the one it names, the one that shows the session it
    /// names, or the one that has the keyboard.
    async fn place(
        &self,
        pane: Option<&str>,
        session: Option<SessionId>,
    ) -> Result<Place, AppError> {
        let panes = &self.services.panes;
        if let Some(text) = pane {
            let pane: PaneId = text
                .parse()
                .map_err(|reason: &str| AppError::new(codes::BAD_REQUEST, reason))?;
            let workspace = panes.workspace();
            if workspace.pane(pane).is_none() {
                return Err(AppError::new(
                    codes::NOT_FOUND,
                    format!("there is no pane {pane}"),
                ));
            }
            let window = workspace.window_of(pane).ok_or_else(|| {
                AppError::new(codes::NOT_SHOWN, format!("pane {pane} is in no window"))
            })?;
            return Ok(Place { pane, window });
        }
        if let Some(session) = session {
            let key = self.services.state.find(session).await?;
            let pane = panes.pane_of(&key);
            let window = pane.and_then(|pane| panes.workspace().window_of(pane));
            return match (pane, window) {
                (Some(pane), Some(window)) => Ok(Place { pane, window }),
                _ => Err(AppError::new(
                    codes::NOT_SHOWN,
                    format!("session {session} is in no pane of a window"),
                )),
            };
        }
        let focused = |_: &_| {
            let window = panes.focused_window()?;
            let pane = panes.focused_pane(window)?;
            Some(Place { pane, window })
        };
        self.services
            .state
            .soon(self.page_wait, focused)
            .await
            .ok_or_else(|| {
                AppError::new(
                    codes::NO_WINDOW,
                    "no pane was named and no window has a pane with the keyboard",
                )
            })
    }

    /// The session a request means, and the pane that shows it if one does. A pane
    /// whose session is still being started is waited for.
    async fn session(
        &self,
        pane: Option<&str>,
        session: Option<SessionId>,
    ) -> Result<(SessionKey, Option<Place>), AppError> {
        if pane.is_none()
            && let Some(session) = session
        {
            let key = self.services.state.find(session).await?;
            let place = self.place(None, Some(session)).await.ok();
            return Ok((key, place));
        }
        let place = self.place(pane, session).await?;
        let panes = &self.services.panes;
        let key = self
            .services
            .state
            .soon(self.page_wait, |_| panes.session_of(place.pane))
            .await
            .ok_or_else(|| {
                AppError::new(
                    codes::NOT_SHOWN,
                    format!("pane {} has no session", place.pane),
                )
            })?;
        Ok((key, Some(place)))
    }

    /// The pane a request about a screen means, and the page that shows it, once the
    /// page has attached to the pane's session. A pane whose session is still being
    /// started, or attached to, is waited for: a request sent just after the
    /// application starts, or just after a pane was made, is early rather than wrong.
    async fn screen_of(
        &self,
        pane: Option<&str>,
        session: Option<SessionId>,
    ) -> Result<(Place, Arc<Link>), AppError> {
        let place = self.place(pane, session).await?;
        // Without a page there is nobody to ask, whatever state the pane is in.
        self.page_of(place.window).await?;
        let state = &self.services.state;
        let panes = &self.services.panes;
        let not_shown = || {
            AppError::new(
                codes::NOT_SHOWN,
                format!(
                    "pane {} shows no session that its window has attached to",
                    place.pane
                ),
            )
        };
        let key = state
            .soon(self.page_wait, |_| panes.session_of(place.pane))
            .await
            .ok_or_else(not_shown)?;
        let label = label_of(place.window);
        let page = state
            .soon(self.page_wait, |state| {
                let page = state.page_of(&label)?;
                page.input_of(&key).map(|_| page)
            })
            .await
            .ok_or_else(not_shown)?;
        Ok((place, page))
    }

    /// Reads a session's screen from its backend, which keeps one when it is the
    /// responder for the session. `None` means the backend keeps none, and a page
    /// showing the session has to be asked instead.
    async fn host_screen(
        &self,
        pane: Option<&str>,
        session: Option<SessionId>,
    ) -> Result<Option<Result<AppReply, AppError>>, AppError> {
        let (key, place) = self.session(pane, session).await?;
        let backend = self.backend_of(&key)?;
        if backend.caps().responder != Responder::Backend {
            return Ok(None);
        }
        let screen = backend.screen(key.session).await.map_err(session_error)?;
        let pane = place
            .map(|place| place.pane.to_string())
            .unwrap_or_default();
        let what = format!(
            "read {} lines of the screen of session {}",
            screen.lines.len(),
            key.session
        );
        Ok(Some(reply(
            what,
            AppReplyBody::Screen {
                pane,
                session: key.session,
                size: screen.size,
                cursor: screen.cursor,
                alternate: screen.alternate,
                lines: screen.lines,
            },
        )))
    }

    /// Waits for text on a session's screen in its backend; `None` as for
    /// [`host_screen`](Self::host_screen).
    async fn host_wait_text(
        &self,
        pane: Option<&str>,
        session: Option<SessionId>,
        text: &str,
        limit: Duration,
    ) -> Result<Option<Result<AppReply, AppError>>, AppError> {
        let (key, place) = self.session(pane, session).await?;
        let backend = self.backend_of(&key)?;
        if backend.caps().responder != Responder::Backend {
            return Ok(None);
        }
        let found = backend
            .wait_text(key.session, text, limit)
            .await
            .map_err(session_error)?;
        let pane = place
            .map(|place| place.pane.to_string())
            .unwrap_or_default();
        Ok(Some(reply(
            format!("the text is on row {}", found.row),
            AppReplyBody::Matched {
                pane,
                session: key.session,
                row: found.row,
                line: found.line,
            },
        )))
    }

    fn backend_of(&self, key: &SessionKey) -> Result<Arc<dyn SessionBackend>, AppError> {
        self.services
            .state
            .router()
            .get(&key.backend)
            .map_err(session_error)
    }

    async fn handle(&self, body: AppRequestBody) -> (Result<AppReply, AppError>, Then) {
        if let AppRequestBody::Attach {
            pane,
            session,
            size,
        } = &body
        {
            return match self.attach(pane.as_deref(), *session, *size).await {
                Ok((answer, attachment)) => (Ok(answer), Then::Attach(Box::new(attachment))),
                Err(error) => (Err(error), Then::Carry),
            };
        }
        if let AppRequestBody::Events { pane, session } = &body {
            return match self.followed(pane.as_deref(), *session).await {
                Ok(followed) => (
                    reply("following what sessions report", AppReplyBody::Following {}),
                    Then::Follow(followed),
                ),
                Err(error) => (Err(error), Then::Carry),
            };
        }
        let body = match body {
            AppRequestBody::Verb { words, cwd, stdin } => {
                return match self.verb(&words, cwd, stdin) {
                    Ok((answer, run)) => (Ok(answer), Then::Verb(Box::new(run))),
                    Err(error) => (Err(error), Then::Carry),
                };
            }
            other => other,
        };
        let quit = matches!(body, AppRequestBody::Quit {});
        let result = self.carry_out(body).await;
        let then = if quit && result.is_ok() {
            Then::Quit
        } else {
            Then::Carry
        };
        (result, then)
    }

    /// Finds the verb the words begin with and what its command is first called with.
    fn verb(
        &self,
        words: &[String],
        cwd: String,
        stdin: Option<String>,
    ) -> Result<(AppReply, VerbRun), AppError> {
        if stdin.as_ref().is_some_and(|text| text.len() > STDIN_MAX) {
            return Err(AppError::new(
                codes::BAD_REQUEST,
                format!("a verb is given {} KiB at most", STDIN_MAX / 1024),
            ));
        }
        let Some((verb, taken)) = self.services.plugins.verb_for(words) else {
            return Err(AppError::new(
                codes::NOT_FOUND,
                match words.first() {
                    Some(word) => format!("no add-on adds a verb that begins with {word}"),
                    None => "no verb was named".to_owned(),
                },
            ));
        };
        let mut first = serde_json::json!({ "argv": words[taken..], "cwd": cwd });
        if let Some(stdin) = stdin {
            first["stdin"] = serde_json::Value::String(stdin);
        }
        let answer = reply(
            format!("running {} of {}", verb.verb, verb.plugin),
            AppReplyBody::Running {
                verb: verb.verb,
                plugin: verb.plugin,
            },
        )?;
        Ok((
            answer,
            VerbRun {
                command: verb.command,
                first,
            },
        ))
    }

    /// Runs a verb for the client on `io`: calls its command step by step, sends what
    /// each step prints, and ends with the status of the last. A client that leaves
    /// stops it: the command is called once more, told that the run is cancelled.
    async fn run_verb<S>(&self, io: &mut S, run: VerbRun) -> io::Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let plugins = &self.services.plugins;
        let command = run.command.as_str();
        let cancel = |next: Option<serde_json::Value>| async move {
            if let Some(next) = next {
                let told = serde_json::json!({ "next": next, "cancel": true });
                if let Err(error) = plugins.call_command(command, &told).await {
                    tracing::debug!(
                        error = %error.message,
                        command,
                        "a verb's command did not take its cancelling"
                    );
                }
            }
        };
        let mut args = run.first;
        // What the call that is out was given to go on from, once there has been one.
        let mut held: Option<serde_json::Value> = None;
        let mut chunk = vec![0; READ_CHUNK];
        loop {
            let result = {
                let call = plugins.call_command(command, &args);
                tokio::pin!(call);
                loop {
                    tokio::select! {
                        result = &mut call => break result,
                        // Nothing more is taken from the client; this is how its
                        // leaving is seen.
                        read = io.read(&mut chunk) => {
                            if matches!(read, Ok(0) | Err(_)) {
                                // Only the first call can say what the run is called,
                                // so it is waited for.
                                let next = match held {
                                    Some(next) => Some(next),
                                    None => call
                                        .await
                                        .ok()
                                        .and_then(|value| read_step(value).ok())
                                        .and_then(|step| step.next),
                                };
                                cancel(next).await;
                                return Ok(());
                            }
                        }
                    }
                }
            };
            let step = match result {
                Ok(value) => {
                    read_step(value).map_err(|problem| (format!("{command} {problem}"), true))
                }
                // A command that failed has ended the run itself. One that is still
                // at it after its time is told that nobody waits any more.
                Err(error) => {
                    let late = error.code == codes::TIMEOUT;
                    Err((error.message, late))
                }
            };
            let step = match step {
                Ok(step) => step,
                Err((problem, tell)) => {
                    if tell {
                        cancel(held).await;
                    }
                    let failed = Step {
                        err: format!("lume: {problem}\n"),
                        code: 1,
                        ..Step::default()
                    };
                    write_step(io, &failed).await?;
                    return write_message(io, &AppMessage::Done(AppDone { code: 1 })).await;
                }
            };
            if let Err(error) = write_step(io, &step).await {
                cancel(step.next).await;
                return Err(error);
            }
            match step.next {
                Some(next) => {
                    args = serde_json::json!({ "next": next });
                    held = Some(next);
                }
                None => {
                    return write_message(io, &AppMessage::Done(AppDone { code: step.code })).await;
                }
            }
        }
    }

    /// Attaches to the session a request means, for a client that shows it in a real
    /// terminal of `size`: the client is told the screen as output to replay, and gives
    /// the terminal its size, as a window that is focused does.
    async fn attach(
        &self,
        pane: Option<&str>,
        session: Option<SessionId>,
        size: TermSize,
    ) -> Result<(AppReply, Attachment), AppError> {
        let (key, place) = self.session(pane, session).await?;
        let backend = self.backend_of(&key)?;
        let facts = ClientFacts {
            size,
            cell_px: ATTACH_CELL_PX,
            palette: Palette::default(),
            scheme: ColorScheme::Dark,
            focused: true,
            visible: true,
        };
        let attachment = backend
            .attach_replaying(key.session, facts)
            .await
            .map_err(session_error)?;
        let answer = reply(
            format!(
                "attached to session {} on stream {ATTACHED_STREAM}",
                key.session
            ),
            AppReplyBody::Attached {
                pane: place.map(|place| place.pane.to_string()),
                session: key.session,
                size: attachment.size,
            },
        )?;
        Ok((answer, attachment))
    }

    /// Carries an attachment over a client's connection until the client detaches or
    /// goes, or the session ends. `rest` is what the client sent after its request.
    async fn carry_attachment<S>(
        &self,
        io: &mut S,
        rest: Vec<u8>,
        attachment: Attachment,
    ) -> io::Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let Attachment {
            hydrate,
            mut output,
            input,
            ..
        } = attachment;
        let carried = carry(io, rest, hydrate, &mut output, &input).await;
        if let Err(error) = input.detach().await
            && !matches!(error, SessionError::Detached)
        {
            tracing::debug!(%error, "an attachment carried for a client could not be ended");
        }
        carried
    }

    /// Which sessions a request to follow them means. A pane or a session that is
    /// named has to be there.
    async fn followed(
        &self,
        pane: Option<&str>,
        session: Option<SessionId>,
    ) -> Result<Followed, AppError> {
        let pane = match pane {
            Some(_) => Some(self.place(pane, None).await?.pane),
            None => None,
        };
        if pane.is_none()
            && let Some(session) = session
        {
            self.services.state.find(session).await?;
        }
        Ok(Followed {
            pane,
            session: session.filter(|_| pane.is_none()),
        })
    }

    /// What a client that follows `followed` is told of one piece of news, if it is
    /// told of it at all.
    fn told(&self, followed: Followed, key: &SessionKey, what: SessionNews) -> Option<AppEvent> {
        let pane = self.services.panes.pane_of(key);
        let wanted = match followed {
            Followed {
                pane: Some(wanted), ..
            } => pane == Some(wanted),
            Followed {
                session: Some(wanted),
                ..
            } => key.session == wanted,
            _ => true,
        };
        wanted.then(|| AppEvent {
            pane: pane.map(|pane| pane.to_string()),
            session: key.session,
            what,
        })
    }

    /// Sends a client what the sessions it follows report, beginning with how each
    /// stands, until the client goes.
    async fn follow<S>(&self, io: &mut S, followed: Followed) -> io::Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let reports = self.services.state.reports();
        let (mut so_far, mut news) = reports.follow();
        let mut chunk = vec![0; READ_CHUNK];
        loop {
            // First, and again whenever the client fell too far behind to be sent
            // what it missed: every session's state as it stands.
            for (key, state) in so_far.drain(..) {
                let whole = SessionNews::Event(SessionEvent::State { state });
                if let Some(event) = self.told(followed, &key, whole) {
                    write_event(io, event).await?;
                }
            }
            tokio::select! {
                next = news.recv() => match next {
                    Ok(News { key, what }) => {
                        if let Some(event) = self.told(followed, &key, what) {
                            write_event(io, event).await?;
                        }
                    }
                    Err(RecvError::Lagged(missed)) => {
                        tracing::debug!(missed, "a client that follows sessions fell behind");
                        (so_far, news) = reports.follow();
                    }
                    Err(RecvError::Closed) => return Ok(()),
                },
                // Nothing more is taken from the client; this is how its leaving is seen.
                read = io.read(&mut chunk) => {
                    if read? == 0 {
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Puts a question of the SSH client's to the user.
    async fn askpass(
        &self,
        token: &Secret,
        prompt: &str,
        hint: Option<&str>,
    ) -> Result<AppReply, AppError> {
        let secret = self.services.rex.askpass(token, prompt, hint).await?;
        let effect = match secret {
            Some(_) => "answered",
            None => "there is no answer",
        };
        reply(effect, AppReplyBody::Secret { secret })
    }
}

mod endpoint;
mod requests;

use endpoint::*;
pub use endpoint::{bind, endpoint, launch_running, serve};
#[cfg(test)]
pub use endpoint::serve_client;

#[cfg(test)]
mod tests;
