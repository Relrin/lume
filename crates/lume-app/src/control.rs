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

use std::io;
use std::sync::Arc;
use std::time::Duration;

use lume_ipc::{Endpoint, Listener};
use lume_layout::{Bounds, Dir, Intent, NewContent, PaneId, SpawnRequest, UNITS, WindowId};
use lume_proto::app::{
    APP_PROTO_VERSION, AppError, AppEvent, AppMessage, AppReply, AppReplyBody, AppRequest,
    AppRequestBody, AppResponse, AppSession, Direction, Focused, PageRequestBody, SessionNews,
    SpawnPlace, codes,
};
use lume_proto::{ClientKind, ControlDecoder, Responder, SessionEvent, SessionId, negotiate};
use lume_session::SessionBackend;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::broadcast::error::RecvError;

use crate::carrier::{Link, SessionKey};
use crate::services::{Asker, Services};
use crate::sessions::News;
use crate::tuning::{PAGE_ANSWER_TIMEOUT, PAGE_OPEN_TIMEOUT, WAIT_MAX, WAIT_SLACK};
use crate::window::label_of;

/// How many bytes are read from a client at a time.
const READ_CHUNK: usize = 16 * 1024;

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
}

/// What a request needs to be carried out.
pub struct Control {
    services: Arc<Services>,
    /// How long a request waits for a page that has not connected yet, or for a pane
    /// that has no session yet.
    page_wait: Duration,
}

/// What to do once an answer has been written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Then {
    Carry,
    /// The application was asked to close; it does once the client has its answer.
    Quit,
    /// The client asked to follow sessions; their reports are sent to it from here on.
    Follow(Followed),
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
        if let AppRequestBody::Events { pane, session } = &body {
            return match self.followed(pane.as_deref(), *session).await {
                Ok(followed) => (
                    reply("following what sessions report", AppReplyBody::Following {}),
                    Then::Follow(followed),
                ),
                Err(error) => (Err(error), Then::Carry),
            };
        }
        let quit = matches!(body, AppRequestBody::Quit {});
        let result = self.carry_out(body).await;
        let then = if quit && result.is_ok() {
            Then::Quit
        } else {
            Then::Carry
        };
        (result, then)
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

    async fn carry_out(&self, body: AppRequestBody) -> Result<AppReply, AppError> {
        let Services {
            state,
            panes,
            registry,
            shell,
            ..
        } = self.services.as_ref();
        match body {
            AppRequestBody::Hello { .. } => Err(AppError::new(
                codes::BAD_REQUEST,
                "hello has been said on this connection",
            )),
            AppRequestBody::Launch {
                workspace,
                exec,
                cwd,
            } => {
                let pane = self
                    .services
                    .launch(workspace.as_deref(), &exec, cwd, self.page_wait)
                    .await?;
                let on_show = panes.workspace().name;
                let mut effect = "brought the window to the front".to_owned();
                if workspace.is_some() {
                    effect.push_str(&format!(", showing the workspace {on_show}"));
                }
                if let Some(pane) = pane {
                    effect.push_str(&format!(", and started the program in pane {pane}"));
                }
                reply(
                    effect,
                    AppReplyBody::Launched {
                        workspace: on_show,
                        pane: pane.map(|pane| pane.to_string()),
                    },
                )
            }
            AppRequestBody::Quit {} => reply("closing the application", AppReplyBody::Quitting {}),
            AppRequestBody::Status {} => {
                let focused = panes.focused_window().and_then(|window| {
                    let pane = panes.focused_pane(window)?;
                    Some(Focused {
                        pane: pane.to_string(),
                        session: panes.session_of(pane).map(|key| key.session),
                    })
                });
                reply(
                    "read the application's status",
                    AppReplyBody::Status {
                        app_version: env!("CARGO_PKG_VERSION").to_owned(),
                        pid: std::process::id(),
                        bundled_ui: shell.bundled_ui(),
                        window: state.page().is_some(),
                        focused,
                        marks: state.marks().read(),
                    },
                )
            }
            AppRequestBody::Spawn { spec, place } => {
                let window = self.focused_window()?;
                // The page says how large the pane will be and shows what is started.
                let page = self.page_of(window).await?;
                let size = spec.size;
                // A profile that is not there is refused here, where the answer can
                // say so, rather than replaced by another when the pane starts.
                if let Some(id) = &spec.profile
                    && panes.profiles.resolve(id).await.is_none()
                {
                    return Err(AppError::new(
                        codes::NOT_FOUND,
                        format!("there is no profile called `{id}`"),
                    ));
                }
                let content = NewContent::Terminal(SpawnRequest {
                    backend: None,
                    profile: spec.profile,
                    launcher: spec.launcher,
                    cwd: spec.cwd,
                    env: spec.env,
                    size,
                    session: None,
                });
                let intent = match place {
                    SpawnPlace::Tab {} => Intent::TabNew {
                        window,
                        content,
                        index: None,
                        activate: true,
                        group: None,
                    },
                    SpawnPlace::Split { dir } => Intent::Split {
                        pane: self.place(None, None).await?.pane,
                        dir: layout_dir(dir),
                        ratio: UNITS / 2,
                        content,
                    },
                    SpawnPlace::Window {} => Intent::WindowNew {
                        content,
                        bounds: None,
                    },
                };
                let applied = panes.apply(intent).await?;
                let Some(pane) = applied.created.pane else {
                    return Err(AppError::new(
                        codes::INTERNAL,
                        "the intent was applied and made no pane",
                    ));
                };
                // The pane of a new window gets its session once the window's page has
                // loaded and said how large the pane is.
                let shown_in = applied.created.window.unwrap_or(window);
                let Some(key) = state
                    .soon(PAGE_OPEN_TIMEOUT, |_| panes.session_of(pane))
                    .await
                else {
                    return Err(AppError::new(
                        codes::TIMEOUT,
                        format!("pane {pane} was made, but no session was started in it in time"),
                    ));
                };
                // Typing into the session, or reading its screen, needs the page to
                // have attached to it. The answer waits for that, so a script's next
                // request finds the terminal ready.
                let label = label_of(shown_in);
                let page = if shown_in == window {
                    page
                } else {
                    self.page_of(shown_in).await?
                };
                let attached = state
                    .soon(PAGE_OPEN_TIMEOUT, |state| {
                        state.page_of(&label)?.input_of(&key)
                    })
                    .await;
                if attached.is_none() {
                    return Err(AppError::new(
                        codes::TIMEOUT,
                        format!(
                            "session {} was started in pane {pane}, but the window did not \
                             show it in time",
                            key.session
                        ),
                    ));
                }
                if let Some(size) = size {
                    // The caller chose the size: it holds, whatever fits the pane.
                    let pinned = PageRequestBody::Resize {
                        pane: Some(pane.to_string()),
                        session: None,
                        size,
                    };
                    page.ask(pinned, PAGE_ANSWER_TIMEOUT).await?;
                }
                let tab = applied.created.tab.map(|tab| tab.to_string());
                let effect = match (applied.created.window, &tab) {
                    (Some(opened), _) => format!(
                        "started session {} in pane {pane} of the new window {opened}",
                        key.session
                    ),
                    (None, Some(tab)) => format!(
                        "started session {} in pane {pane} of the new tab {tab}",
                        key.session
                    ),
                    (None, None) => {
                        format!("started session {} in the new pane {pane}", key.session)
                    }
                };
                reply(
                    effect,
                    AppReplyBody::Spawned {
                        session: key.session,
                        pane: pane.to_string(),
                        tab,
                    },
                )
            }
            AppRequestBody::List {} => {
                let sessions: Vec<AppSession> = state
                    .list_all()
                    .await?
                    .into_iter()
                    .map(|listed| {
                        let key = SessionKey {
                            backend: listed.backend.clone(),
                            session: listed.info.id,
                        };
                        AppSession {
                            pane: panes.pane_of(&key).map(|pane| pane.to_string()),
                            busy: state.reports().busy(&key, listed.info.exit.is_some()),
                            backend: listed.backend,
                            info: listed.info,
                        }
                    })
                    .collect();
                reply(
                    format!("listed {} sessions", sessions.len()),
                    AppReplyBody::Sessions { sessions },
                )
            }
            AppRequestBody::Kill { session } => {
                let key = state.find(session).await?;
                state.end(&key).await?;
                reply(format!("ended session {session}"), AppReplyBody::Killed {})
            }
            AppRequestBody::Send {
                pane,
                session,
                text,
            } => {
                let (key, place) = self.session(pane.as_deref(), session).await?;
                let not_shown = || {
                    AppError::new(
                        codes::NOT_SHOWN,
                        format!(
                            "session {} is not shown in a window, and only a shown session \
                             can be typed into",
                            key.session
                        ),
                    )
                };
                let label = label_of(place.ok_or_else(not_shown)?.window);
                // A session that has just been started is attached to a moment later.
                let input = state
                    .soon(self.page_wait, |state| {
                        state.page_of(&label)?.input_of(&key)
                    })
                    .await
                    .ok_or_else(not_shown)?;
                let bytes = text.into_bytes();
                if !bytes.is_empty() {
                    input
                        .write(&bytes)
                        .await
                        .map_err(|error| AppError::from(crate::error::CommandError::from(error)))?;
                }
                reply(
                    format!("typed {} bytes into session {}", bytes.len(), key.session),
                    AppReplyBody::Sent {
                        bytes: bytes.len() as u64,
                    },
                )
            }
            AppRequestBody::Resize {
                pane,
                session,
                size,
            } => {
                let (place, page) = self.screen_of(pane.as_deref(), session).await?;
                let resized = page
                    .ask(
                        PageRequestBody::Resize {
                            pane: Some(place.pane.to_string()),
                            session: None,
                            size,
                        },
                        PAGE_ANSWER_TIMEOUT,
                    )
                    .await?;
                match &resized {
                    AppReplyBody::Resized { size } => reply(
                        format!(
                            "the terminal in pane {} is now {} by {}",
                            place.pane,
                            size.cols(),
                            size.rows()
                        ),
                        resized,
                    ),
                    _ => Err(unexpected("resize")),
                }
            }
            AppRequestBody::WaitExit {
                pane,
                session,
                timeout_ms,
            } => {
                let (key, _) = self.session(pane.as_deref(), session).await?;
                let status = state.wait_exit(&key, wait_limit(timeout_ms)).await?;
                reply(
                    format!("session {} has ended", key.session),
                    AppReplyBody::Exited { status },
                )
            }
            AppRequestBody::Screen { pane, session } => {
                if let Some(answer) = self.host_screen(pane.as_deref(), session).await? {
                    return answer;
                }
                let (place, page) = self.screen_of(pane.as_deref(), session).await?;
                let screen = page
                    .ask(
                        PageRequestBody::Screen {
                            pane: Some(place.pane.to_string()),
                            session: None,
                        },
                        PAGE_ANSWER_TIMEOUT,
                    )
                    .await?;
                match &screen {
                    AppReplyBody::Screen { lines, .. } => reply(
                        format!(
                            "read {} lines of the screen of pane {}",
                            lines.len(),
                            place.pane
                        ),
                        screen,
                    ),
                    _ => Err(unexpected("screen")),
                }
            }
            AppRequestBody::WaitText {
                pane,
                session,
                text,
                timeout_ms,
            } => {
                let limit = wait_limit(timeout_ms);
                if let Some(answer) = self
                    .host_wait_text(pane.as_deref(), session, &text, limit)
                    .await?
                {
                    return answer;
                }
                let (place, page) = self.screen_of(pane.as_deref(), session).await?;
                // The page keeps the time. It is asked for a little longer, so that its
                // own answer that the time ran out arrives before this gives up on it.
                let found = page
                    .ask(
                        PageRequestBody::WaitText {
                            pane: Some(place.pane.to_string()),
                            session: None,
                            text,
                            timeout_ms: u64::try_from(limit.as_millis()).unwrap_or(u64::MAX),
                        },
                        limit + WAIT_SLACK,
                    )
                    .await?;
                match &found {
                    AppReplyBody::Matched { row, .. } => {
                        reply(format!("the text is on row {row}"), found)
                    }
                    _ => Err(unexpected("wait_text")),
                }
            }
            AppRequestBody::Layout {} => {
                let snapshot = panes.snapshot();
                let workspace = serde_json::to_value(&snapshot.workspace)
                    .map_err(|error| AppError::new(codes::INTERNAL, error.to_string()))?;
                reply(
                    format!("read the layout at revision {}", snapshot.revision),
                    AppReplyBody::Layout {
                        window: panes.focused_window().map(|window| window.to_string()),
                        epoch: snapshot.epoch,
                        revision: snapshot.revision,
                        workspace,
                    },
                )
            }
            AppRequestBody::LayoutApply { intent } => {
                let intent: Intent = serde_json::from_value(intent).map_err(|error| {
                    AppError::new(codes::BAD_REQUEST, format!("not an intent: {error}"))
                })?;
                let before = panes.snapshot().revision;
                let applied = panes.apply(intent).await?;
                let created = serde_json::to_value(applied.created)
                    .map_err(|error| AppError::new(codes::INTERNAL, error.to_string()))?;
                let effect = if applied.revision == before {
                    "the layout is as it was: the intent changed nothing".to_owned()
                } else {
                    format!("the layout is now at revision {}", applied.revision)
                };
                reply(
                    effect,
                    AppReplyBody::Applied {
                        epoch: applied.epoch,
                        revision: applied.revision,
                        created,
                    },
                )
            }
            AppRequestBody::Workspaces {} => {
                let (active, workspaces) = panes.workspaces()?;
                reply(
                    format!("listed {} workspaces", workspaces.len()),
                    AppReplyBody::Workspaces { active, workspaces },
                )
            }
            AppRequestBody::Profiles {} => {
                let mut profiles = panes.profiles.view();
                // The looks are for drawing a terminal, which the command line does not.
                if let Some(view) = profiles.as_object_mut() {
                    view.remove("looks");
                }
                let count = profiles["profiles"].as_array().map_or(0, Vec::len);
                reply(
                    format!("listed {count} profiles"),
                    AppReplyBody::Profiles { profiles },
                )
            }
            // Answered before it gets here, since its answer is not the end of it.
            AppRequestBody::Events { .. } => Err(AppError::new(
                codes::INTERNAL,
                "a request to follow sessions was handled as a plain request",
            )),
            AppRequestBody::Commands {} => {
                let commands = registry.list();
                let count = commands.len();
                let commands = serde_json::to_value(commands)
                    .map_err(|error| AppError::new(codes::INTERNAL, error.to_string()))?;
                reply(
                    format!("listed {count} commands"),
                    AppReplyBody::Commands { commands },
                )
            }
            AppRequestBody::Command { id, args, window } => {
                let window = match window {
                    Some(text) => {
                        let window: WindowId = text
                            .parse()
                            .map_err(|reason: &str| AppError::new(codes::BAD_REQUEST, reason))?;
                        if panes.workspace().window(window).is_none() {
                            return Err(AppError::new(
                                codes::NOT_FOUND,
                                format!("there is no window {window}"),
                            ));
                        }
                        window
                    }
                    None => self.focused_window()?,
                };
                // A command that runs in a page needs the page; one that starts a
                // session wants it too, to say how large the pane will be.
                let _ = self.page_of(window).await;
                self.services
                    .run_command(window, &id, &args, Asker::Outside)
                    .await?;
                reply(
                    format!("ran {id} in window {window}"),
                    AppReplyBody::Ran { command: id },
                )
            }
        }
    }
}

/// The page answered with something that is not an answer to what it was asked.
fn session_error(error: lume_session::SessionError) -> AppError {
    AppError::from(crate::error::CommandError::from(error))
}

fn unexpected(request: &str) -> AppError {
    AppError::new(
        codes::INTERNAL,
        format!("the page gave an answer that does not fit a {request} request"),
    )
}

/// The endpoint the application serves: the one named, or the default.
pub fn endpoint(name: Option<&str>) -> io::Result<Endpoint> {
    Endpoint::named(name.unwrap_or(lume_proto::app::ENDPOINT_NAME))
}

/// Starts serving `endpoint`, or reports that another process already does. Must be
/// called inside the runtime.
pub fn bind(endpoint: &Endpoint) -> io::Result<Option<Listener>> {
    match Listener::bind(endpoint) {
        Ok(listener) => Ok(Some(listener)),
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => Ok(None),
        Err(error) => Err(error),
    }
}

/// Serves clients for as long as the endpoint can accept them.
pub async fn serve(mut listener: Listener, control: Arc<Control>) {
    loop {
        match listener.accept().await {
            Ok(client) => {
                let control = Arc::clone(&control);
                tokio::spawn(async move {
                    if let Err(error) = serve_client(client, &control).await {
                        tracing::debug!(%error, "a control connection ended with an error");
                    }
                });
            }
            Err(error) => {
                tracing::error!(%error, "the control endpoint stopped accepting clients");
                return;
            }
        }
    }
}

async fn write_event<S: AsyncWrite + Unpin>(io: &mut S, event: AppEvent) -> io::Result<()> {
    let mut frames = Vec::new();
    AppMessage::Event(event)
        .encode(&mut frames)
        .map_err(io::Error::other)?;
    io.write_all(&frames).await?;
    io.flush().await
}

async fn write_response<S: AsyncWrite + Unpin>(
    io: &mut S,
    id: u64,
    result: Result<AppReply, AppError>,
) -> io::Result<()> {
    let mut frames = Vec::new();
    AppMessage::Response(AppResponse { id, result })
        .encode(&mut frames)
        .map_err(io::Error::other)?;
    io.write_all(&frames).await?;
    io.flush().await
}

/// Answers the first request, which must be `hello` and must share a version with us.
fn greet(body: &AppRequestBody) -> Result<(AppReply, ClientKind), AppError> {
    let AppRequestBody::Hello {
        proto,
        proto_min,
        client_kind,
    } = body
    else {
        return Err(AppError::new(
            codes::HELLO_REQUIRED,
            "the first request on a connection must be hello",
        ));
    };
    let agreed = negotiate(APP_PROTO_VERSION, *proto, *proto_min).ok_or_else(|| {
        AppError::new(
            codes::VERSION_MISMATCH,
            format!(
                "the application speaks version {APP_PROTO_VERSION}, the client \
                 {proto_min} to {proto}"
            ),
        )
    })?;
    let reply = AppReply {
        effect: "connected".to_owned(),
        body: AppReplyBody::Hello {
            proto: agreed,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            pid: std::process::id(),
        },
    };
    Ok((reply, *client_kind))
}

/// Serves one client until it disconnects or breaks the rules.
pub async fn serve_client<S>(mut io: S, control: &Control) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut decoder = ControlDecoder::new();
    let mut chunk = vec![0; READ_CHUNK];
    let mut greeted = false;
    loop {
        let json = loop {
            if let Some(json) = decoder.next().map_err(io::Error::other)? {
                break json;
            }
            let read = io.read(&mut chunk).await?;
            if read == 0 {
                return Ok(());
            }
            decoder.push(&chunk[..read]);
        };
        let AppRequest { id, body } = match AppMessage::decode(&json).map_err(io::Error::other)? {
            AppMessage::Request(request) => request,
            AppMessage::Response(_) | AppMessage::Event(_) => {
                return Err(io::Error::other(
                    "a client sent what only the application sends",
                ));
            }
        };

        if !greeted {
            match greet(&body) {
                Ok((reply, client_kind)) => {
                    tracing::debug!(?client_kind, "a control client connected");
                    greeted = true;
                    write_response(&mut io, id, Ok(reply)).await?;
                }
                Err(error) => {
                    // Nothing else is said to a client that did not start properly.
                    write_response(&mut io, id, Err(error)).await?;
                    return Ok(());
                }
            }
            continue;
        }

        let (result, then) = control.handle(body).await;
        write_response(&mut io, id, result).await?;
        match then {
            Then::Carry => {}
            Then::Quit => {
                control.services.shell.quit();
                return Ok(());
            }
            Then::Follow(followed) => return control.follow(&mut io, followed).await,
        }
    }
}

/// Hands what this copy of the application was started for to the copy already
/// serving `endpoint`: it brings its window to the front, shows the workspace named and
/// runs the program named.
pub async fn launch_running(
    endpoint: &Endpoint,
    workspace: Option<String>,
    exec: Vec<String>,
    cwd: Option<String>,
) -> io::Result<()> {
    let mut io = lume_ipc::connect(endpoint).await?;
    let requests = [
        AppRequestBody::Hello {
            proto: APP_PROTO_VERSION,
            proto_min: APP_PROTO_VERSION,
            client_kind: ClientKind::App,
        },
        AppRequestBody::Launch {
            workspace,
            exec,
            cwd,
        },
    ];
    let mut decoder = ControlDecoder::new();
    let mut chunk = vec![0; READ_CHUNK];
    for (id, body) in requests.into_iter().enumerate() {
        let mut frames = Vec::new();
        AppMessage::Request(AppRequest {
            id: id as u64,
            body,
        })
        .encode(&mut frames)
        .map_err(io::Error::other)?;
        io.write_all(&frames).await?;
        io.flush().await?;

        let json = loop {
            if let Some(json) = decoder.next().map_err(io::Error::other)? {
                break json;
            }
            let read = io.read(&mut chunk).await?;
            if read == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            decoder.push(&chunk[..read]);
        };
        match AppMessage::decode(&json).map_err(io::Error::other)? {
            AppMessage::Response(AppResponse { result: Ok(_), .. }) => {}
            AppMessage::Response(AppResponse {
                result: Err(error), ..
            }) => return Err(io::Error::other(error)),
            AppMessage::Request(_) | AppMessage::Event(_) => {
                return Err(io::Error::other(
                    "the application sent something that is not an answer",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use lume_layout::Store;
    use lume_proto::app::OpenSpec;
    use lume_proto::{Body, Message};
    use lume_session::{ExitStatus, HostConfig, LocalBackend, Router};
    use tokio::io::DuplexStream;
    use tokio::runtime::Handle;
    use tokio::time::timeout;

    use super::*;
    use crate::builtins;
    use crate::configsvc::ConfigService;
    use crate::marks::Marks;
    use crate::panes::{PaneStart, Panes};
    use crate::state::AppState;
    use crate::testing::{
        CREDIT, FakeShell, RecordingCarrier, STEP, TempDir, client_frames, facts, size,
    };
    use crate::tuning::WORKSPACE_NAME;

    struct Running {
        services: Arc<Services>,
        shell: Arc<FakeShell>,
        control: Arc<Control>,
        client: Client,
        window: WindowId,
        _dir: TempDir,
    }

    impl Running {
        /// A second connection to the endpoint.
        fn connect(&self) -> Client {
            connect(&self.control)
        }
    }

    /// A client of the endpoint, speaking over an in-memory connection.
    struct Client {
        io: DuplexStream,
        decoder: ControlDecoder,
        next_id: u64,
    }

    impl Client {
        async fn ask(&mut self, body: AppRequestBody) -> Result<AppReply, AppError> {
            let id = self.next_id;
            self.next_id += 1;
            let mut frames = Vec::new();
            AppMessage::Request(AppRequest { id, body })
                .encode(&mut frames)
                .unwrap();
            self.io.write_all(&frames).await.unwrap();
            let mut chunk = [0; 4096];
            let json = loop {
                if let Some(json) = self.decoder.next().unwrap() {
                    break json;
                }
                let read = timeout(STEP, self.io.read(&mut chunk))
                    .await
                    .expect("the endpoint answers")
                    .unwrap();
                assert_ne!(read, 0, "the endpoint closed the connection");
                self.decoder.push(&chunk[..read]);
            };
            match AppMessage::decode(&json).unwrap() {
                AppMessage::Response(response) => {
                    assert_eq!(response.id, id);
                    response.result
                }
                other => panic!("the endpoint sent {other:?} for an answer"),
            }
        }

        /// The next report the endpoint sends to a client that follows sessions.
        async fn event(&mut self) -> AppEvent {
            let mut chunk = vec![0; READ_CHUNK];
            let json = loop {
                if let Some(json) = self.decoder.next().unwrap() {
                    break json;
                }
                let read = tokio::time::timeout(Duration::from_secs(20), self.io.read(&mut chunk))
                    .await
                    .expect("the endpoint sends a report")
                    .unwrap();
                assert_ne!(read, 0, "the endpoint closed the connection");
                self.decoder.push(&chunk[..read]);
            };
            match AppMessage::decode(&json).unwrap() {
                AppMessage::Event(event) => event,
                other => panic!("the endpoint sent {other:?} for a report"),
            }
        }

        async fn hello(&mut self) {
            self.ask(AppRequestBody::Hello {
                proto: APP_PROTO_VERSION,
                proto_min: APP_PROTO_VERSION,
                client_kind: ClientKind::Test,
            })
            .await
            .unwrap();
        }
    }

    async fn running(name: &str) -> Running {
        let dir = TempDir::new(name);
        // A shell that starts fast and keeps no history, with a mark a command can print
        // that does not appear in the command's own echo.
        let (shell, args) = if cfg!(windows) {
            ("cmd.exe", "[\"/d\"]")
        } else {
            ("/bin/sh", "[]")
        };
        std::fs::create_dir_all(&dir.dirs().config).unwrap();
        std::fs::write(
            dir.dirs().config_file(),
            format!("[terminal]\nshell = \"{shell}\"\nshell-args = {args}\n"),
        )
        .unwrap();

        let local = Arc::new(
            LocalBackend::in_process(HostConfig::default())
                .await
                .unwrap(),
        );
        let router = Router::new();
        router.insert(Router::local_id(), local.clone());
        let state = AppState::new(
            Handle::current(),
            router,
            vec![local],
            Duration::from_secs(600),
            Arc::new(Marks::start()),
        );
        let config = ConfigService::load(dir.dirs(), Some(WORKSPACE_NAME));
        let store = Store::new(dir.dirs().state_workspaces_dir());
        let (panes, _) =
            Panes::open(&state, Arc::clone(&config), store, WORKSPACE_NAME, None).await;
        state.set_panes(Arc::clone(&panes));
        let shell = Arc::new(FakeShell::default());
        panes.run(shell.clone());
        let window = panes.focused_window().unwrap();
        let services = Arc::new(Services {
            state,
            panes,
            config,
            registry: Arc::new(builtins::registry()),
            shell: shell.clone(),
        });
        let control = Control::new(Arc::clone(&services), Duration::from_millis(200));
        let client = connect(&control);
        Running {
            services,
            shell,
            control,
            client,
            window,
            _dir: dir,
        }
    }

    /// A new connection to the endpoint, as another client would make.
    fn connect(control: &Arc<Control>) -> Client {
        let control = Arc::clone(control);
        let (ours, theirs) = tokio::io::duplex(64 * 1024);
        tokio::spawn(async move {
            let _ = serve_client(theirs, &control).await;
        });
        Client {
            io: ours,
            decoder: ControlDecoder::new(),
            next_id: 1,
        }
    }

    /// What stands in for the window's page: a carrier, and the streams it has used.
    struct Page {
        carrier: Arc<RecordingCarrier>,
        id: u32,
        label: String,
        next_stream: u32,
    }

    impl Page {
        async fn open(app: &Running) -> Self {
            let carrier = RecordingCarrier::new();
            let label = label_of(app.window);
            let id = app.services.state.open(&label, carrier.clone()).await;
            Self {
                carrier,
                id,
                label,
                next_stream: 1,
            }
        }

        /// Attaches to a session and lets its output flow.
        async fn attach(&mut self, state: &Arc<AppState>, key: &SessionKey) -> u32 {
            let stream = self.next_stream;
            self.next_stream += 1;
            state
                .attach(
                    &self.label,
                    self.id,
                    &key.backend,
                    key.session,
                    stream,
                    facts(),
                )
                .await
                .unwrap();
            let credit = client_frames(&[Message::new(stream, Body::Credit(CREDIT))]);
            state.send(&self.label, self.id, &credit).await.unwrap();
            stream
        }

        fn text(&self) -> String {
            let mut text = String::new();
            for message in self.carrier.messages() {
                if let Body::Data(bytes) = &message.body {
                    text.push_str(&String::from_utf8_lossy(bytes));
                }
            }
            text
        }
    }

    /// Opens the window's page, starts the session of its one pane and attaches to it.
    async fn shown(app: &Running) -> (Page, PaneId, SessionKey) {
        let mut page = Page::open(app).await;
        let pane = app.services.panes.workspace().panes()[0].id;
        let outcome = app
            .services
            .panes
            .start_sessions(
                app.window,
                vec![PaneStart {
                    pane,
                    size: size(80, 24),
                }],
            )
            .await;
        let started = &outcome.started[0];
        let key = SessionKey {
            backend: started.backend.clone(),
            session: started.session,
        };
        page.attach(&app.services.state, &key).await;
        (page, pane, key)
    }

    async fn until(what: &str, mut done: impl FnMut() -> bool) {
        let waiting = async {
            while !done() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        if timeout(STEP, waiting).await.is_err() {
            panic!("{what} never happened");
        }
    }

    async fn stop(app: Running) {
        app.services.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_client_must_say_hello_first() {
        let mut app = running("hello").await;
        let refused = app.client.ask(AppRequestBody::List {}).await.unwrap_err();
        assert_eq!(refused.code, codes::HELLO_REQUIRED);
        stop(app).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_client_of_another_version_is_refused() {
        let mut app = running("version").await;
        let refused = app
            .client
            .ask(AppRequestBody::Hello {
                proto: APP_PROTO_VERSION + 2,
                proto_min: APP_PROTO_VERSION + 1,
                client_kind: ClientKind::Test,
            })
            .await
            .unwrap_err();
        assert_eq!(refused.code, codes::VERSION_MISMATCH);
        stop(app).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn activate_and_quit_reach_the_window() {
        let mut app = running("quit").await;
        app.client.hello().await;

        let launched = app
            .client
            .ask(AppRequestBody::Launch {
                workspace: None,
                exec: Vec::new(),
                cwd: None,
            })
            .await
            .unwrap();
        assert_eq!(
            launched.body,
            AppReplyBody::Launched {
                workspace: WORKSPACE_NAME.to_owned(),
                pane: None,
            }
        );
        assert_eq!(app.shell.activations(), 1);
        assert_eq!(app.shell.quits(), 0);

        let quitting = app.client.ask(AppRequestBody::Quit {}).await.unwrap();
        assert_eq!(quitting.body, AppReplyBody::Quitting {});
        // The window is told once the answer is on its way.
        let shell = app.shell.clone();
        until("quit", || shell.quits() == 1).await;
        stop(app).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_focused_pane_is_listed_typed_into_and_ended() {
        let mut app = running("focused").await;
        app.client.hello().await;
        let (page, pane, key) = shown(&app).await;

        let listed = app.client.ask(AppRequestBody::List {}).await.unwrap();
        let AppReplyBody::Sessions { sessions } = listed.body else {
            panic!("not a session list");
        };
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].info.id, key.session);
        assert_eq!(sessions[0].pane, Some(pane.to_string()));

        // Nothing named: the pane that has the keyboard.
        let command = if cfg!(windows) {
            "echo lume-%OS:~0,0%k7q-done\r"
        } else {
            "echo \"lume-\"\"k7q-done\"\r"
        };
        let sent = app
            .client
            .ask(AppRequestBody::Send {
                pane: None,
                session: None,
                text: command.to_owned(),
            })
            .await
            .unwrap();
        assert_eq!(
            sent.body,
            AppReplyBody::Sent {
                bytes: command.len() as u64
            }
        );
        until("the command's output", || {
            page.text().contains("lume-k7q-done")
        })
        .await;

        let status = app.client.ask(AppRequestBody::Status {}).await.unwrap();
        let AppReplyBody::Status {
            window,
            focused,
            marks,
            ..
        } = status.body
        else {
            panic!("not a status");
        };
        assert!(window);
        assert_eq!(
            focused,
            Some(Focused {
                pane: pane.to_string(),
                session: Some(key.session)
            })
        );
        assert!(marks.carrier_ms.is_some() && marks.attached_ms.is_some());

        let waiting = app
            .client
            .ask(AppRequestBody::WaitExit {
                pane: Some(pane.to_string()),
                session: None,
                timeout_ms: 50,
            })
            .await
            .unwrap_err();
        assert_eq!(waiting.code, codes::TIMEOUT);

        app.client
            .ask(AppRequestBody::Kill {
                session: key.session,
            })
            .await
            .unwrap();
        let exited = app
            .client
            .ask(AppRequestBody::WaitExit {
                pane: None,
                session: Some(key.session),
                timeout_ms: 20_000,
            })
            .await
            .unwrap();
        assert!(matches!(exited.body, AppReplyBody::Exited { .. }));
        stop(app).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_exit_code_is_reported_to_whoever_waits() {
        let mut app = running("code").await;
        app.client.hello().await;
        let (_page, pane, key) = shown(&app).await;
        app.client
            .ask(AppRequestBody::Send {
                pane: Some(pane.to_string()),
                session: None,
                text: "exit 7\r".to_owned(),
            })
            .await
            .unwrap();
        let exited = app
            .client
            .ask(AppRequestBody::WaitExit {
                pane: None,
                session: Some(key.session),
                timeout_ms: 20_000,
            })
            .await
            .unwrap();
        assert_eq!(
            exited.body,
            AppReplyBody::Exited {
                status: ExitStatus::Code(7)
            }
        );
        stop(app).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_client_that_follows_sessions_is_told_how_they_stand_and_what_they_report() {
        let mut app = running("follow").await;
        app.client.hello().await;
        let (_page, pane, key) = shown(&app).await;

        // What is not there cannot be followed, and the connection stays an ordinary one.
        let missing = app
            .client
            .ask(AppRequestBody::Events {
                pane: Some("p999".to_owned()),
                session: None,
            })
            .await
            .unwrap_err();
        assert_eq!(missing.code, codes::NOT_FOUND);
        let missing = app
            .client
            .ask(AppRequestBody::Events {
                pane: None,
                session: SessionId::new(0x0bad_0bad),
            })
            .await
            .unwrap_err();
        assert_eq!(missing.code, codes::NOT_FOUND);

        let mut follower = app.connect();
        follower.hello().await;
        let following = follower
            .ask(AppRequestBody::Events {
                pane: Some(pane.to_string()),
                session: None,
            })
            .await
            .unwrap();
        assert_eq!(following.body, AppReplyBody::Following {});

        // The program that has the terminal is looked up when a page shows the
        // session. The follower learns of it as part of how the session stands, or as
        // a report of its own, whichever way round the two happened.
        let name = loop {
            let event = follower.event().await;
            assert_eq!(event.pane, Some(pane.to_string()));
            assert_eq!(event.session, key.session);
            let foreground = match event.what {
                SessionNews::Event(SessionEvent::Foreground(foreground)) => Some(foreground),
                SessionNews::Event(SessionEvent::State { state }) => state.foreground,
                _ => None,
            };
            if let Some(foreground) = foreground {
                break foreground.name;
            }
        };
        if cfg!(windows) {
            assert_eq!(name, "cmd");
        } else {
            assert!(!name.is_empty());
        }

        app.client
            .ask(AppRequestBody::Send {
                pane: Some(pane.to_string()),
                session: None,
                text: "exit 3\r".to_owned(),
            })
            .await
            .unwrap();
        loop {
            if let SessionNews::Exit(status) = follower.event().await.what {
                assert_eq!(status, ExitStatus::Code(3));
                break;
            }
        }
        stop(app).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_follower_of_one_pane_hears_nothing_of_the_others() {
        let mut app = running("follow-one").await;
        app.client.hello().await;
        let (_page, pane, key) = shown(&app).await;
        let other = SessionKey {
            backend: key.backend.clone(),
            session: SessionId::new(0x0d0d).unwrap(),
        };
        let title = |text: &str| SessionEvent::Title {
            title: text.to_owned(),
        };

        let mut follower = app.connect();
        follower.hello().await;
        follower
            .ask(AppRequestBody::Events {
                pane: Some(pane.to_string()),
                session: None,
            })
            .await
            .unwrap();
        let reports = app.services.state.reports();
        reports.on_event(&other, &title("not this one"));
        reports.on_event(&key, &title("lume-k7q"));
        loop {
            let event = follower.event().await;
            assert_eq!(event.session, key.session);
            if event.what == SessionNews::Event(title("lume-k7q")) {
                break;
            }
        }

        // With nothing named, every session is followed, in a pane or not.
        let mut all = app.connect();
        all.hello().await;
        all.ask(AppRequestBody::Events {
            pane: None,
            session: None,
        })
        .await
        .unwrap();
        reports.on_event(&other, &title("this one too"));
        loop {
            let event = all.event().await;
            if event.session == other.session
                && event.what == SessionNews::Event(title("this one too"))
            {
                assert_eq!(event.pane, None);
                break;
            }
        }
        stop(app).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn what_needs_a_page_is_refused_without_one_and_passed_on_with_one() {
        let mut app = running("page").await;
        app.client.hello().await;

        let refused = app
            .client
            .ask(AppRequestBody::Resize {
                pane: None,
                session: None,
                size: size(4, 1),
            })
            .await
            .unwrap_err();
        assert_eq!(refused.code, codes::NO_WINDOW);
        let refused = app
            .client
            .ask(AppRequestBody::Spawn {
                spec: OpenSpec::default(),
                place: SpawnPlace::Tab {},
            })
            .await
            .unwrap_err();
        assert_eq!(refused.code, codes::NO_WINDOW);

        let (page, pane, key) = shown(&app).await;
        // The page answers as soon as it is asked.
        let state = Arc::clone(&app.services.state);
        let (carrier, id, label) = (page.carrier.clone(), page.id, page.label.clone());
        let answering = tokio::spawn(async move {
            // Notices arrive on the same stream; the request is among them.
            let asked = || {
                carrier
                    .requests()
                    .into_iter()
                    .find(|message| message.get("request").is_some())
            };
            until("the request to the page", || asked().is_some()).await;
            let request = asked().unwrap();
            assert_eq!(request["request"]["body"]["type"], "resize");
            assert_eq!(request["request"]["body"]["pane"], pane.to_string());
            let asked = request["request"]["id"].as_u64().unwrap();
            let answer = AppReplyBody::Resized { size: size(4, 1) };
            state.answer(&label, id, asked, Ok(answer)).unwrap();
        });
        let resized = app
            .client
            .ask(AppRequestBody::Resize {
                pane: None,
                session: None,
                size: size(4, 1),
            })
            .await
            .unwrap();
        answering.await.unwrap();
        assert_eq!(
            resized.effect,
            format!("the terminal in pane {pane} is now 4 by 1")
        );

        // The screen is the host's to read; the page is not asked.
        let before = page.carrier.requests().len();
        let screen = app
            .client
            .ask(AppRequestBody::Screen {
                pane: None,
                session: None,
            })
            .await
            .unwrap();
        assert_eq!(
            screen.effect,
            format!("read 24 lines of the screen of session {}", key.session)
        );
        assert!(
            matches!(&screen.body, AppReplyBody::Screen { pane: shown, session, .. }
                if *shown == pane.to_string() && *session == key.session),
            "{:?}",
            screen.body
        );
        assert_eq!(page.carrier.requests().len(), before);
        stop(app).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_session_is_started_in_a_new_tab_or_beside_the_focused_pane() {
        let mut app = running("spawn").await;
        app.client.hello().await;
        let (mut page, first, _) = shown(&app).await;

        for place in [
            SpawnPlace::Tab {},
            SpawnPlace::Split {
                dir: Direction::Down,
            },
        ] {
            let before = app.services.panes.workspace().panes().len();
            let asking = app.client.ask(AppRequestBody::Spawn {
                spec: OpenSpec::default(),
                place,
            });
            // The page attaches to the new pane's session when the layout tells it of
            // the pane, as the real page does.
            let panes = Arc::clone(&app.services.panes);
            let state = Arc::clone(&app.services.state);
            let attaching = async {
                until("the new pane", || panes.workspace().panes().len() > before).await;
                let workspace = panes.workspace();
                let made = workspace.panes().into_iter().last().unwrap().id;
                let key = panes.session_of(made).unwrap();
                page.attach(&state, &key).await;
                (made, key)
            };
            let (answer, (made, key)) = tokio::join!(asking, attaching);
            let tab = match place {
                SpawnPlace::Tab {} => app.services.panes.workspace().windows[0]
                    .centre
                    .active
                    .map(|tab| tab.to_string()),
                _ => None,
            };
            assert_eq!(
                answer.unwrap().body,
                AppReplyBody::Spawned {
                    session: key.session,
                    pane: made.to_string(),
                    tab,
                }
            );
            assert_ne!(made, first);
        }
        let workspace = app.services.panes.workspace();
        assert_eq!(workspace.windows[0].centre.tabs.len(), 2);
        assert_eq!(workspace.panes().len(), 3);

        stop(app).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_second_start_shows_a_workspace_and_runs_a_program_in_a_new_tab() {
        let mut app = running("launch").await;
        app.client.hello().await;
        let (program, args) = if cfg!(windows) {
            ("cmd.exe", "/d")
        } else {
            ("/bin/sh", "-i")
        };

        let launched = app
            .client
            .ask(AppRequestBody::Launch {
                workspace: Some("play".to_owned()),
                exec: vec![program.to_owned(), args.to_owned()],
                cwd: None,
            })
            .await
            .unwrap();
        let AppReplyBody::Launched { workspace, pane } = launched.body else {
            panic!("not an answer to a launch");
        };
        assert_eq!(workspace, "play");
        assert_eq!(app.shell.activations(), 1);

        // The workspace was begun with a window of its own, and the program has a tab.
        let on_show = app.services.panes.workspace();
        assert_eq!(on_show.name, "play");
        assert_eq!(on_show.windows[0].centre.tabs.len(), 2);
        let pane: PaneId = pane.unwrap().parse().unwrap();
        let lume_layout::PaneContent::Terminal(terminal) = &on_show.pane(pane).unwrap().content
        else {
            panic!("the program's pane is not a terminal");
        };
        assert!(terminal.session.is_some());
        assert_eq!(
            terminal.launcher,
            Some(lume_session::Launcher::Shell {
                program: program.to_owned(),
                args: vec![args.to_owned()],
                login: false,
            })
        );

        let listed = app.client.ask(AppRequestBody::Workspaces {}).await.unwrap();
        let AppReplyBody::Workspaces { active, workspaces } = listed.body else {
            panic!("not a list of workspaces");
        };
        assert_eq!(active, "play");
        let names: Vec<&str> = workspaces.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, vec![WORKSPACE_NAME, "play"]);
        stop(app).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_command_opens_a_window_and_another_closes_it() {
        let mut app = running("windows").await;
        app.client.hello().await;
        let run = |id: &str, window: Option<String>| AppRequestBody::Command {
            id: id.to_owned(),
            args: serde_json::Value::Null,
            window,
        };

        app.client.ask(run("lume.window.new", None)).await.unwrap();
        let workspace = app.services.panes.workspace();
        assert_eq!(workspace.windows.len(), 2);
        let second = workspace.windows[1].id;
        let shell = app.shell.clone();
        until("the new window to be opened", || {
            shell.open() == vec![label_of(second)]
        })
        .await;

        app.client
            .ask(run("lume.window.close", Some(second.to_string())))
            .await
            .unwrap();
        let shell = app.shell.clone();
        until("the window to be closed", || {
            shell.closed() == vec![label_of(second)]
        })
        .await;
        assert_eq!(app.services.panes.workspace().windows.len(), 1);
        assert_eq!(app.shell.quits(), 0);

        // Closing the only window that is left closes the application, and the layout
        // keeps the window for the next start.
        app.client
            .ask(run("lume.window.close", None))
            .await
            .unwrap();
        assert_eq!(app.shell.quits(), 1);
        assert_eq!(app.services.panes.workspace().windows.len(), 1);
        stop(app).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_settings_are_opened_by_a_command_at_the_section_named() {
        let mut app = running("settings").await;
        app.client.hello().await;
        let run = |args: serde_json::Value| AppRequestBody::Command {
            id: "lume.settings.open".to_owned(),
            args,
            window: None,
        };

        app.client.ask(run(serde_json::Value::Null)).await.unwrap();
        app.client
            .ask(run(serde_json::json!({ "section": "appearance" })))
            .await
            .unwrap();
        assert_eq!(
            app.shell.settings(),
            vec![None, Some("appearance".to_owned())]
        );
        // Showing the settings changes nothing in the layout.
        assert_eq!(app.services.panes.workspace().windows.len(), 1);
        stop(app).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_window_closed_from_its_own_page_is_asked_to_close_and_not_closed_outright() {
        let app = running("ask-close").await;
        let window = app.services.panes.workspace().windows[0].id;

        app.services
            .run_command(
                window,
                "lume.window.close",
                &serde_json::Value::Null,
                crate::services::Asker::Page,
            )
            .await
            .unwrap();
        // The window is asked, as its close control asks: whether the user is asked
        // first is decided where a window's closing is.
        assert_eq!(app.shell.asked_to_close(), vec![window]);
        assert_eq!(app.shell.quits(), 0);
        assert_eq!(app.shell.closed(), Vec::<String>::new());
        stop(app).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_layout_is_read_and_changed_and_commands_are_listed_and_run() {
        let mut app = running("layout").await;
        app.client.hello().await;
        let pane = app.services.panes.workspace().panes()[0].id;

        let read = app.client.ask(AppRequestBody::Layout {}).await.unwrap();
        let AppReplyBody::Layout {
            window,
            epoch,
            revision,
            workspace,
        } = read.body
        else {
            panic!("not a layout");
        };
        assert_eq!(window, Some(app.window.to_string()));
        assert_eq!(epoch, 1);
        assert_eq!(workspace["name"], WORKSPACE_NAME);

        let renamed = app
            .client
            .ask(AppRequestBody::LayoutApply {
                intent: serde_json::json!({
                    "type": "pane_rename", "pane": pane.to_string(), "title": "logs"
                }),
            })
            .await
            .unwrap();
        assert_eq!(
            renamed.body,
            AppReplyBody::Applied {
                epoch: 1,
                revision: revision + 1,
                created: serde_json::json!({"pane": null, "tab": null, "window": null}),
            }
        );
        let again = app
            .client
            .ask(AppRequestBody::LayoutApply {
                intent: serde_json::json!({
                    "type": "pane_rename", "pane": pane.to_string(), "title": "logs"
                }),
            })
            .await
            .unwrap();
        assert!(again.effect.contains("changed nothing"), "{}", again.effect);

        for (intent, code) in [
            (
                serde_json::json!({"type": "no_such_intent"}),
                codes::BAD_REQUEST,
            ),
            (
                serde_json::json!({"type": "close", "pane": "p999"}),
                codes::NOT_FOUND,
            ),
            (
                serde_json::json!({"type": "window_focus", "window": app.window.to_string()}),
                "forbidden",
            ),
        ] {
            let refused = app
                .client
                .ask(AppRequestBody::LayoutApply { intent })
                .await
                .unwrap_err();
            assert_eq!(refused.code, code);
        }

        let listed = app.client.ask(AppRequestBody::Commands {}).await.unwrap();
        let AppReplyBody::Commands { commands } = listed.body else {
            panic!("not a list of commands");
        };
        assert!(
            commands
                .as_array()
                .unwrap()
                .iter()
                .any(|command| command["id"] == "lume.pane.zoom" && command["runs"] == "core")
        );

        let ran = app
            .client
            .ask(AppRequestBody::Command {
                id: "lume.pane.zoom".to_owned(),
                args: serde_json::Value::Null,
                window: None,
            })
            .await
            .unwrap();
        assert_eq!(
            ran.body,
            AppReplyBody::Ran {
                command: "lume.pane.zoom".to_owned()
            }
        );
        assert!(app.services.panes.is_zoomed(pane));

        let unknown = app
            .client
            .ask(AppRequestBody::Command {
                id: "lume.pane.fly".to_owned(),
                args: serde_json::Value::Null,
                window: None,
            })
            .await
            .unwrap_err();
        assert_eq!(unknown.code, codes::NOT_FOUND);
        // A command that runs in the page needs the window's page, and there is none.
        let pageless = app
            .client
            .ask(AppRequestBody::Command {
                id: "lume.terminal.copy".to_owned(),
                args: serde_json::Value::Null,
                window: None,
            })
            .await
            .unwrap_err();
        assert_eq!(pageless.code, codes::NO_WINDOW);
        stop(app).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_pane_or_session_that_is_not_there_is_said_to_be_missing() {
        let mut app = running("missing").await;
        app.client.hello().await;
        let (_page, _pane, _key) = shown(&app).await;

        let unknown = app
            .client
            .ask(AppRequestBody::Send {
                pane: Some("p999".to_owned()),
                session: None,
                text: "x".to_owned(),
            })
            .await
            .unwrap_err();
        assert_eq!(unknown.code, codes::NOT_FOUND);
        let malformed = app
            .client
            .ask(AppRequestBody::Screen {
                pane: Some("nine".to_owned()),
                session: None,
            })
            .await
            .unwrap_err();
        assert_eq!(malformed.code, codes::BAD_REQUEST);

        // A session in no pane has no screen and no keyboard.
        let stray = app
            .services
            .state
            .router()
            .get(&Router::local_id())
            .unwrap()
            .spawn(crate::testing::shell())
            .await
            .unwrap();
        let refused = app
            .client
            .ask(AppRequestBody::Send {
                pane: None,
                session: Some(stray),
                text: "x".to_owned(),
            })
            .await
            .unwrap_err();
        assert_eq!(refused.code, codes::NOT_SHOWN);

        let unknown = app
            .client
            .ask(AppRequestBody::Kill {
                session: SessionId::new(1).unwrap(),
            })
            .await
            .unwrap_err();
        assert_eq!(unknown.code, codes::NOT_FOUND);
        stop(app).await;
    }
}
