//! A Rex server as a place sessions run.
//!
//! A backend keeps one control connection to its server for as long as it lives. When
//! the connection is lost it connects again, a little later each time, and reads
//! everything anew: a server does not send the events a client missed.
//!
//! The server's sessions are known from the moment the connection opens. A session's
//! windows and blocks are known once the session has been opened here, which attaches
//! the connection to it; the terminals among its blocks are what the application calls
//! sessions.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use lume_layout::LayoutAuthority;
use lume_proto::{
    BackendCaps, BackendId, ClientFacts, ErrorCode, ExitStatus, HydrateKinds, ProtoError,
    Responder, SessionId, SessionInfo, SessionStats, SpawnSpec, TermSize,
};
use lume_session::{Attachment, ScreenText, SessionBackend, SessionError, TextFound};
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::api::{
    Attached, ClientInfo, ServerEvent, ServerVersion, SessionCreated, SessionList, SessionSummary,
    TerminalProcess, TerminalSize, TerminalTitle, View,
};
use crate::authority::{self, RexLayout};
use crate::control::{Control, Events};
use crate::endpoint::Endpoint;
use crate::error::RexError;
use crate::ids::{BlockRef, Ids};
use crate::input::type_text;
use crate::intents::shell_layout;
use crate::screen;
use crate::terminal::FeedRef;
use crate::transport::{Dialed, Options, Reach};
use crate::tuning::{
    ATTACH_RETRY, ATTACH_TIMEOUT, RECONNECT_FIRST, RECONNECT_MOST, RESUME_WINDOW, SCREEN_POLL,
    SESSIONS_MAX,
};

/// How a backend stands with its server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// The first connection is being made.
    Connecting,
    /// Connected, to a server of this version.
    Connected(ServerVersion),
    /// The connection was lost or could not be made, and will be tried again.
    Waiting {
        /// What went wrong, for a person to read.
        error: String,
        /// How many times in a row it has gone wrong.
        attempts: u32,
    },
    /// The server cannot be used, and trying again would not help: it refused this
    /// user, or speaks no revision of its API that this client does.
    Refused {
        /// A short, stable name for the kind of refusal.
        code: String,
        error: String,
    },
    /// The backend was shut down.
    Closed,
}

/// A session of a server, as the server lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RexSession {
    /// The server's id for it, such as `session:01a2…`.
    pub id: String,
    pub label: String,
    /// Whether it has been opened here, so that its blocks are known.
    pub opened: bool,
}

/// Makes the streams a backend's connections run over.
#[async_trait]
pub trait Dialer: Send + Sync + 'static {
    async fn dial(&self) -> Result<Dialed, RexError>;
}

#[async_trait]
impl Dialer for Reach {
    async fn dial(&self) -> Result<Dialed, RexError> {
        Reach::dial(self).await
    }
}

#[derive(Default)]
struct State {
    control: Option<Arc<Control>>,
    sessions: Vec<SessionSummary>,
    /// The sessions opened here, with their views where they have been read.
    opened: HashMap<String, Option<View>>,
    ids: Ids,
    /// What continues the client after a lost connection, and when it was lost.
    resume: Option<(String, Instant)>,
    /// The terminals that have views, each with the feed that serves them.
    feeds: HashMap<SessionId, FeedRef>,
    /// The number of the last feed made.
    serial: u64,
}

/// What a backend holds of a session's view.
pub(crate) enum Seen {
    /// The session is not opened here: it never was, was let go of, or is gone.
    Gone,
    /// It is opened and its view has not been read yet.
    Unread,
    Read(View),
}

pub(crate) struct Shared {
    /// The server, in what is said of failures.
    pub(crate) name: String,
    dialer: Box<dyn Dialer>,
    client: ClientInfo,
    state: Mutex<State>,
    link: watch::Sender<Link>,
    /// Goes up whenever the sessions or a view change.
    changed: watch::Sender<u64>,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn touch(&self) {
        self.changed
            .send_modify(|count| *count = count.wrapping_add(1));
    }

    /// A number that goes up whenever the sessions or a view change.
    pub(crate) fn changed(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// The view held of a session.
    pub(crate) fn seen(&self, session_id: &str) -> Seen {
        match self.state().opened.get(session_id) {
            None => Seen::Gone,
            Some(None) => Seen::Unread,
            Some(Some(view)) => Seen::Read(view.clone()),
        }
    }

    /// The revision of the view held of a session, if one is held.
    pub(crate) fn revision_of(&self, session_id: &str) -> Option<u64> {
        let state = self.state();
        Some(state.opened.get(session_id)?.as_ref()?.revision)
    }

    /// Reads a session's view and keeps it, if the session is still opened and nothing
    /// newer was read meanwhile.
    pub(crate) async fn read_view(
        &self,
        control: &Control,
        session_id: &str,
    ) -> Result<(), RexError> {
        let (view, _) = control
            .call_as::<View>("session.view", json!({ "session_id": session_id }))
            .await?;
        let view = view.check()?;
        let mut state = self.state();
        if let Some(held) = state.opened.get_mut(session_id)
            && held
                .as_ref()
                .is_none_or(|held| held.revision < view.revision)
        {
            *held = Some(view);
            drop(state);
            self.touch();
        }
        Ok(())
    }

    /// The number the application knows a block's terminal by.
    pub(crate) fn session_number(&self, session_id: &str, block_id: &str) -> SessionId {
        self.state().ids.of(session_id, block_id)
    }

    /// The control connection, while there is one.
    pub(crate) fn control(&self) -> Result<Arc<Control>, RexError> {
        self.state().control.clone().ok_or(RexError::Disconnected)
    }

    /// A new stream to the server, for a connection of its own.
    pub(crate) async fn dial(&self) -> Result<Dialed, RexError> {
        self.dialer.dial().await
    }

    /// How the backend stands with its server, now and as it changes.
    pub(crate) fn link(&self) -> watch::Receiver<Link> {
        self.link.subscribe()
    }

    /// Whether the server's terminals take lines back out of scrollback when they
    /// grow taller, which a view's own terminal must then do as well. A server on
    /// Windows runs its programs behind the system's console host, which redraws the
    /// screen for itself, and its terminals leave the scrollback where it is.
    pub(crate) fn resize_pulls_scrollback(&self) -> bool {
        match &*self.link.borrow() {
            Link::Connected(version) => !version.platform.starts_with("windows"),
            _ => true,
        }
    }

    /// Notes that a terminal's feed is over, so that the terminal can be attached to
    /// anew.
    pub(crate) fn forget(&self, id: SessionId, serial: u64) {
        let mut state = self.state();
        if state
            .feeds
            .get(&id)
            .is_some_and(|feed| feed.serial == serial)
        {
            state.feeds.remove(&id);
        }
    }

    /// Ends every view of every terminal: the backend is going.
    fn end_attachments(&self) {
        for feed in self.state().feeds.values() {
            feed.close();
        }
    }
}

/// A Rex server as a place sessions run.
pub struct RexBackend {
    id: BackendId,
    shared: Arc<Shared>,
    keeper: JoinHandle<()>,
    /// The layout of the session on show, and what keeps it level with the server.
    layout: Arc<RexLayout>,
    layout_keeper: JoinHandle<()>,
}

impl Drop for RexBackend {
    fn drop(&mut self) {
        self.keeper.abort();
        self.layout_keeper.abort();
        self.layout.close();
        self.shared.end_attachments();
        if let Some(control) = self.shared.state().control.take() {
            control.close();
        }
    }
}

pub(crate) fn unsupported(what: &str) -> SessionError {
    SessionError::Refused(ProtoError::new(
        ErrorCode::Unsupported,
        format!("a Rex server cannot {what} yet"),
    ))
}

impl RexBackend {
    /// A backend for the server at `endpoint`, which the application knows as `id`. It
    /// connects in the background, at once, and says how that goes through
    /// [`link`](Self::link). Must be called inside a Tokio runtime.
    pub fn connect(id: BackendId, endpoint: Endpoint, client: ClientInfo) -> Arc<Self> {
        Self::connect_with(id, endpoint, client, Options::default())
    }

    /// A backend as [`connect`](Self::connect) makes one, with what the user said of
    /// how the server may be reached. What was said holds for this backend only.
    pub fn connect_with(
        id: BackendId,
        endpoint: Endpoint,
        client: ClientInfo,
        options: Options,
    ) -> Arc<Self> {
        let name = endpoint.to_string();
        Self::over(id, name, Reach::new(endpoint, options), client)
    }

    /// A backend whose connections run over streams made by `dialer`. `name` is the
    /// server in what is said of failures.
    pub fn over(id: BackendId, name: String, dialer: impl Dialer, client: ClientInfo) -> Arc<Self> {
        let (link, _) = watch::channel(Link::Connecting);
        let (changed, _) = watch::channel(0);
        let shared = Arc::new(Shared {
            name,
            dialer: Box::new(dialer),
            client,
            state: Mutex::new(State::default()),
            link,
            changed,
        });
        let keeper = tokio::spawn(keep(Arc::clone(&shared)));
        let layout = RexLayout::new(Arc::clone(&shared), id.clone());
        let layout_keeper = tokio::spawn(authority::follow(Arc::clone(&layout)));
        Arc::new(Self {
            id,
            shared,
            keeper,
            layout,
            layout_keeper,
        })
    }

    /// What the application knows this backend as.
    pub fn id(&self) -> &BackendId {
        &self.id
    }

    /// How the backend stands with its server, now and as it changes.
    pub fn link(&self) -> watch::Receiver<Link> {
        self.shared.link.subscribe()
    }

    /// A number that goes up whenever the server's sessions, or the view of one that
    /// was opened, change.
    pub fn changed(&self) -> watch::Receiver<u64> {
        self.shared.changed.subscribe()
    }

    /// Waits until the backend is connected, or cannot be.
    pub async fn ready(&self) -> Result<ServerVersion, RexError> {
        let mut link = self.link();
        loop {
            match &*link.borrow_and_update() {
                Link::Connected(version) => return Ok(version.clone()),
                Link::Refused { code, error } => {
                    return Err(RexError::Refused {
                        endpoint: self.shared.name.clone(),
                        reason: format!("{error} ({code})"),
                    });
                }
                Link::Closed => return Err(RexError::Disconnected),
                Link::Connecting | Link::Waiting { .. } => {}
            }
            if link.changed().await.is_err() {
                return Err(RexError::Disconnected);
            }
        }
    }

    /// The server's sessions, in the server's order.
    pub fn sessions(&self) -> Vec<RexSession> {
        let state = self.shared.state();
        state
            .sessions
            .iter()
            .map(|session| RexSession {
                id: session.session_id.clone(),
                label: session.label.clone(),
                opened: state.opened.contains_key(&session.session_id),
            })
            .collect()
    }

    /// Makes a session of one window with one shell in it, called `label`, or what the
    /// server calls it when that is left out. Returns its id. It is not opened here.
    pub async fn create_session(&self, label: Option<&str>) -> Result<String, RexError> {
        let control = self.shared.control()?;
        let mut asked = json!({ "initial_windows": [{ "layout": shell_layout() }] });
        if let Some(label) = label.map(str::trim).filter(|label| !label.is_empty()) {
            asked["label"] = json!(label);
        }
        let (created, _) = control
            .call_as::<SessionCreated>("session.create", asked)
            .await?;
        Ok(created.session_id)
    }

    /// Calls a session something else. The server tells every client of it, this one
    /// too.
    pub async fn rename_session(&self, session_id: &str, label: &str) -> Result<(), RexError> {
        let control = self.shared.control()?;
        control
            .call(
                "session.set_label",
                json!({ "session_id": session_id, "label": label.trim() }),
            )
            .await?;
        Ok(())
    }

    /// Ends a session, with what runs in it, for every client of the server.
    pub async fn end_session(&self, session_id: &str) -> Result<(), RexError> {
        let control = self.shared.control()?;
        control
            .call("session.destroy", json!({ "session_id": session_id }))
            .await?;
        Ok(())
    }

    /// How long the server takes to answer, measured by asking it something it answers
    /// at once.
    pub async fn round_trip(&self) -> Result<Duration, RexError> {
        let control = self.shared.control()?;
        let asked = Instant::now();
        control.call("server.version", Value::Null).await?;
        Ok(asked.elapsed())
    }

    /// Opens a session: attaches the connection to it and reads its view. Its terminal
    /// blocks are among this backend's sessions from then on, and its view is kept up
    /// to date. A session that is opened stays so across reconnects.
    pub async fn open_session(&self, session_id: &str) -> Result<View, RexError> {
        let control = self.shared.control()?;
        let view = attach(&control, session_id).await?;
        self.shared
            .state()
            .opened
            .insert(session_id.to_owned(), Some(view.clone()));
        self.shared.touch();
        Ok(view)
    }

    /// Lets go of a session that was opened. The session carries on.
    pub async fn close_session(&self, session_id: &str) -> Result<(), RexError> {
        if self.shared.state().opened.remove(session_id).is_none() {
            return Ok(());
        }
        self.shared.touch();
        let control = self.shared.control()?;
        control
            .call("session.detach", json!({ "session_id": session_id }))
            .await?;
        Ok(())
    }

    /// The view of a session that was opened, as last read.
    pub fn view(&self, session_id: &str) -> Option<View> {
        self.shared.state().opened.get(session_id)?.clone()
    }

    /// Puts a session on show: its layout is the one [`layout`](SessionBackend::layout)
    /// gives from now on, and whoever follows that layout is given it whole. The
    /// session is opened if it was not. A backend has one session on show at a time.
    ///
    /// A session whose layout cannot be shown is refused, and what was on show stays.
    pub async fn show_session(&self, session_id: &str) -> Result<(), RexError> {
        if !self.shared.state().opened.contains_key(session_id) {
            self.open_session(session_id).await?;
        }
        self.layout.show(Some(session_id))
    }

    /// Takes the session on show off it. The layout is an empty window until another
    /// is put on show. The session stays opened.
    pub fn show_nothing(&self) {
        // Nothing is always shown.
        let _ = self.layout.show(None);
    }

    /// The session on show.
    pub fn shown_session(&self) -> Option<String> {
        self.layout.on_show()
    }

    /// Ends the backend: its connection is closed and not made again.
    pub fn shutdown(&self) {
        self.keeper.abort();
        self.layout_keeper.abort();
        self.layout.close();
        self.shared.end_attachments();
        if let Some(control) = self.shared.state().control.take() {
            control.close();
        }
        self.shared.link.send_replace(Link::Closed);
    }

    /// The control connection and the block a number names, for something asked of
    /// that block.
    fn block(&self, id: SessionId) -> lume_session::Result<(Arc<Control>, BlockRef)> {
        let state = self.shared.state();
        let block = state.ids.block(id).cloned().ok_or_else(|| {
            SessionError::Refused(ProtoError::new(
                ErrorCode::NotFound,
                "the Rex server has no such terminal",
            ))
        })?;
        let control = state.control.clone().ok_or(SessionError::Disconnected)?;
        Ok((control, block))
    }

    /// What one terminal block is, as the application lists a session.
    async fn describe(
        &self,
        control: &Control,
        id: SessionId,
        session_id: &str,
        block_id: &str,
    ) -> Result<SessionInfo, RexError> {
        let of_block = || json!({ "session_id": session_id, "block_id": block_id });
        let (size, _) = control
            .call_as::<TerminalSize>("com.superlogical.terminal.size", of_block())
            .await?;
        let (process, _) = control
            .call_as::<TerminalProcess>("com.superlogical.terminal.process", of_block())
            .await?;
        // A terminal that has no title yet, or cannot say, simply has none.
        let title = control
            .call_as::<TerminalTitle>("com.superlogical.terminal.title", of_block())
            .await
            .ok()
            .map(|(title, _)| title.title)
            .filter(|title| !title.is_empty());
        let shown = process.foreground.as_ref().or(process.child.as_ref());
        let fits = |count: u32| u16::try_from(count).unwrap_or(TermSize::MAX);
        Ok(SessionInfo {
            id,
            program: shown
                .map(|process| process.name.clone())
                .unwrap_or_default(),
            pid: process
                .child
                .as_ref()
                .and_then(|child| u32::try_from(child.pid).ok()),
            size: TermSize::new(fits(size.columns), fits(size.rows))
                .map_err(|_| RexError::Protocol("a terminal has no size".to_owned()))?,
            // A server does not say when a block was made.
            created_unix_ms: 0,
            attached: false,
            exit: match (&process.child, &process.last_exit) {
                (None, Some(exit)) => Some(
                    u32::try_from(exit.exit_code)
                        .map(ExitStatus::Code)
                        .unwrap_or(ExitStatus::Unknown),
                ),
                _ => None,
            },
            stats: SessionStats {
                read_bytes: 0,
                sent_bytes: 0,
                pending_bytes: 0,
                paused: false,
            },
            title,
            cwd: shown.and_then(|process| process.cwd.clone()),
        })
    }
}

/// Attaches a connection to a session and reads its view.
async fn attach(control: &Control, session_id: &str) -> Result<View, RexError> {
    let (attached, _) = control
        .call_as::<Attached>("session.attach", json!({ "session_id": session_id }))
        .await?;
    attached.view.check()
}

#[async_trait]
impl SessionBackend for RexBackend {
    fn caps(&self) -> BackendCaps {
        BackendCaps {
            persistent: true,
            multi_client: true,
            server_layout: true,
            // The server's own terminal answers what programs ask of it.
            responder: Responder::Backend,
            hydrate: HydrateKinds {
                snapshot: true,
                vt_replay: true,
            },
        }
    }

    /// The terminal blocks of the sessions that were opened here.
    async fn list(&self) -> lume_session::Result<Vec<SessionInfo>> {
        let control = self.shared.control()?;
        let blocks: Vec<(SessionId, String, String)> = {
            let mut state = self.shared.state();
            let State { opened, ids, .. } = &mut *state;
            let mut blocks = Vec::new();
            for (session_id, view) in opened.iter() {
                let Some(view) = view else { continue };
                for block in view.blocks().filter(|block| block.is_terminal()) {
                    let Some(block_id) = &block.block_id else {
                        continue;
                    };
                    blocks.push((
                        ids.of(session_id, block_id),
                        session_id.clone(),
                        block_id.clone(),
                    ));
                }
            }
            blocks
        };
        let mut listed = Vec::with_capacity(blocks.len());
        for (id, session_id, block_id) in blocks {
            match self.describe(&control, id, &session_id, &block_id).await {
                Ok(info) => listed.push(info),
                // The block closed between the view and now.
                Err(RexError::Call { status: 404, .. }) => {}
                Err(error) => return Err(error.into()),
            }
        }
        listed.sort_by_key(|info| info.id);
        Ok(listed)
    }

    /// The layout of the session on show.
    fn layout(&self) -> Option<Arc<dyn LayoutAuthority>> {
        Some(Arc::clone(&self.layout) as Arc<dyn LayoutAuthority>)
    }

    async fn spawn(&self, _spec: SpawnSpec) -> lume_session::Result<SessionId> {
        Err(unsupported("start a session"))
    }

    /// Attaches to a terminal block of a session that was opened here. A terminal's
    /// views share one stream, since a server gives a client no more than one.
    async fn attach(&self, id: SessionId, facts: ClientFacts) -> lume_session::Result<Attachment> {
        let until = tokio::time::Instant::now() + ATTACH_TIMEOUT;
        loop {
            let joining = {
                let mut state = self.shared.state();
                let State {
                    feeds, ids, serial, ..
                } = &mut *state;
                let Some(block) = ids.block(id) else {
                    return Err(SessionError::Refused(ProtoError::new(
                        ErrorCode::NotFound,
                        "the Rex server has no such terminal",
                    )));
                };
                let feed = feeds.entry(id).or_insert_with(|| {
                    *serial += 1;
                    FeedRef::start(Arc::clone(&self.shared), id, *serial, block.clone())
                });
                feed.join(facts.clone())
            };
            let joined = match joining {
                Some(joining) => joining.await.ok(),
                None => None,
            };
            if let Some(joined) = joined {
                return Ok(joined?);
            }
            // The feed was just ending, with the terminal's last view. Its stream is
            // closed before it is forgotten, and then a new feed can begin one.
            if tokio::time::Instant::now() > until {
                return Err(SessionError::Disconnected);
            }
            tokio::time::sleep(ATTACH_RETRY).await;
        }
    }

    /// Closes a terminal block, which ends the program in it.
    async fn kill(&self, id: SessionId) -> lume_session::Result<()> {
        let (control, block) = self.block(id)?;
        control
            .call(
                "block.close",
                json!({ "session_id": block.session_id, "block_id": block.block_id }),
            )
            .await?;
        Ok(())
    }

    async fn send(&self, id: SessionId, text: &str) -> lume_session::Result<()> {
        let (control, block) = self.block(id)?;
        Ok(type_text(&control, &block, text).await?)
    }

    async fn screen(&self, id: SessionId) -> lume_session::Result<ScreenText> {
        let (control, block) = self.block(id)?;
        Ok(screen::read(&control, &block).await?)
    }

    /// Reads the screen again and again until the text is on it.
    async fn wait_text(
        &self,
        id: SessionId,
        text: &str,
        timeout: Duration,
    ) -> lume_session::Result<TextFound> {
        let until = tokio::time::Instant::now() + timeout;
        loop {
            let (control, block) = self.block(id)?;
            if let Some(found) = screen::find(&screen::read(&control, &block).await?, text) {
                return Ok(found);
            }
            if tokio::time::Instant::now() + SCREEN_POLL > until {
                return Err(SessionError::Refused(ProtoError::new(
                    ErrorCode::Timeout,
                    "the text did not appear on the terminal's screen in time",
                )));
            }
            tokio::time::sleep(SCREEN_POLL).await;
        }
    }
}

/// What reading a new connection's state gave.
struct Opened {
    control: Arc<Control>,
    events: Events,
    version: ServerVersion,
    /// The number of the last event the list of sessions already holds.
    floor: u64,
}

/// Opens a connection and reads the server's state over it.
async fn open(shared: &Shared) -> Result<Opened, RexError> {
    let token = {
        let mut state = shared.state();
        state
            .resume
            .take()
            .filter(|(_, lost)| lost.elapsed() < RESUME_WINDOW)
            .map(|(token, _)| token)
    };
    let dialed = shared.dialer.dial().await?;
    let (control, events) = Control::open_at(
        dialed.io,
        &shared.name,
        &dialed.authority,
        &shared.client,
        token.as_deref(),
    )
    .await?;
    let control = Arc::new(control);
    let (version, _) = control
        .call_as::<ServerVersion>("server.version", Value::Null)
        .await?;
    // Events begin before the list is read, so nothing falls between the two.
    let (list, floor) = control
        .call_as::<SessionList>("session.attach_all", json!({ "event_scope": "session" }))
        .await?;
    if list.sessions.len() > SESSIONS_MAX {
        return Err(RexError::Protocol(
            "the server lists more sessions than any server holds".to_owned(),
        ));
    }
    let sessions: Vec<SessionSummary> =
        list.sessions.into_iter().map(SessionSummary::cut).collect();

    // The sessions that were opened before are opened again; one that is gone is let go.
    let wanted: Vec<String> = shared.state().opened.keys().cloned().collect();
    let mut views = HashMap::new();
    for session_id in wanted {
        match attach(&control, &session_id).await {
            Ok(view) => {
                views.insert(session_id, Some(view));
            }
            Err(RexError::Call { status: 404, .. }) => {}
            Err(error) => return Err(error),
        }
    }

    let mut state = shared.state();
    state.sessions = sessions;
    state.opened = views;
    state.control = Some(Arc::clone(&control));
    Ok(Opened {
        control,
        events,
        version: version.cut(),
        floor,
    })
}

/// Applies one event to what is known of the server. Returns the session whose view is
/// to be read again, if the event calls for that.
fn apply(shared: &Shared, event: ServerEvent) -> Option<String> {
    let mut state = shared.state();
    match event {
        ServerEvent::SessionCreated(session) => {
            if state.sessions.len() < SESSIONS_MAX
                && !state
                    .sessions
                    .iter()
                    .any(|known| known.session_id == session.session_id)
            {
                state.sessions.push(session);
            }
        }
        ServerEvent::SessionDestroyed { session_id } => {
            state
                .sessions
                .retain(|session| session.session_id != session_id);
            state.opened.remove(&session_id);
        }
        ServerEvent::SessionMoved { session_id, before } => {
            let from = state
                .sessions
                .iter()
                .position(|session| session.session_id == session_id)?;
            let moved = state.sessions.remove(from);
            let to = before
                .and_then(|before| {
                    state
                        .sessions
                        .iter()
                        .position(|session| session.session_id == before)
                })
                .unwrap_or(state.sessions.len());
            state.sessions.insert(to, moved);
        }
        ServerEvent::SessionLabel { session_id, label } => {
            let session = state
                .sessions
                .iter_mut()
                .find(|session| session.session_id == session_id)?;
            session.label = label;
        }
        ServerEvent::ViewChanged {
            session_id,
            revision,
        } => {
            // These can arrive out of order: only a revision above the one held says
            // something new.
            let held = state.opened.get(&session_id)?;
            let stale = held.as_ref().is_none_or(|view| view.revision < revision);
            return stale.then_some(session_id);
        }
        // A terminal's reports go to the views of the terminal, where it has any.
        ServerEvent::Block { block_id, events } => {
            let feed = state
                .ids
                .known(&block_id)
                .and_then(|id| state.feeds.get(&id));
            if let Some(feed) = feed {
                feed.report(events);
            }
            return None;
        }
        // What was dropped is read again by whoever follows the events.
        ServerEvent::Dropped { .. } | ServerEvent::Other => return None,
    }
    drop(state);
    shared.touch();
    None
}

/// Reads the list of sessions again and puts it in place of the one held. A session
/// that is no longer listed is no longer opened.
async fn read_sessions(shared: &Shared, control: &Control) {
    let listed = control
        .call_as::<SessionList>("session.list", Value::Null)
        .await;
    let sessions: Vec<SessionSummary> = match listed {
        Ok((list, _)) if list.sessions.len() <= SESSIONS_MAX => {
            list.sessions.into_iter().map(SessionSummary::cut).collect()
        }
        Ok(_) => {
            tracing::debug!(server = %shared.name, "the server lists more sessions than any server holds");
            return;
        }
        Err(error) => {
            tracing::debug!(server = %shared.name, %error, "the sessions could not be read again");
            return;
        }
    };
    let mut state = shared.state();
    state
        .opened
        .retain(|opened, _| sessions.iter().any(|session| &session.session_id == opened));
    state.sessions = sessions;
    drop(state);
    shared.touch();
}

/// Follows a connection's events until it ends, and says why it ended.
async fn follow(shared: &Shared, opened: Opened) -> RexError {
    let Opened {
        control,
        mut events,
        floor,
        ..
    } = opened;
    while let Some(event) = events.next().await {
        // What the list of sessions already holds.
        if event.seq <= floor {
            continue;
        }
        let session_id = match ServerEvent::read(&event.name, &event.payload) {
            // An event was too large to send. What it would have changed is read anew:
            // the list of sessions, and the view of the session it was about.
            ServerEvent::Dropped { session_id } => {
                read_sessions(shared, &control).await;
                let opened =
                    session_id.filter(|session| shared.state().opened.contains_key(session));
                match opened {
                    Some(session_id) => session_id,
                    None => continue,
                }
            }
            event => match apply(shared, event) {
                Some(session_id) => session_id,
                None => continue,
            },
        };
        if let Err(error) = shared.read_view(&control, &session_id).await {
            tracing::debug!(server = %shared.name, %error, "a session's view could not be read");
        }
    }
    control.until_ended().await
}

/// How long to wait before the connection is tried again.
pub(crate) fn backoff(attempts: u32) -> Duration {
    RECONNECT_FIRST
        .saturating_mul(1u32 << attempts.saturating_sub(1).min(16))
        .min(RECONNECT_MOST)
}

/// Keeps a backend connected for as long as it lives.
async fn keep(shared: Arc<Shared>) {
    let mut attempts = 0u32;
    loop {
        let error = match open(&shared).await {
            Ok(opened) => {
                attempts = 0;
                shared
                    .link
                    .send_replace(Link::Connected(opened.version.clone()));
                shared.touch();
                let control = Arc::clone(&opened.control);
                let error = follow(&shared, opened).await;
                let mut state = shared.state();
                state.control = None;
                state.resume = control
                    .reconnect_token()
                    .map(|token| (token.to_owned(), Instant::now()));
                error
            }
            Err(error) => error,
        };
        if !error.is_transient() && !matches!(error, RexError::Protocol(_) | RexError::Call { .. })
        {
            tracing::warn!(server = %shared.name, %error, "a Rex server cannot be used");
            shared.link.send_replace(Link::Refused {
                code: error.code().to_owned(),
                error: error.to_string(),
            });
            return;
        }
        attempts = attempts.saturating_add(1);
        tracing::debug!(server = %shared.name, %error, attempts, "the connection to a Rex server will be tried again");
        shared.link.send_replace(Link::Waiting {
            error: error.to_string(),
            attempts,
        });
        tokio::time::sleep(backoff(attempts)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_before_trying_again_doubles_up_to_the_most() {
        assert_eq!(backoff(1), RECONNECT_FIRST);
        assert_eq!(backoff(2), RECONNECT_FIRST * 2);
        assert_eq!(backoff(3), RECONNECT_FIRST * 4);
        assert_eq!(backoff(40), RECONNECT_MOST);
        assert_eq!(backoff(u32::MAX), RECONNECT_MOST);
    }
}
