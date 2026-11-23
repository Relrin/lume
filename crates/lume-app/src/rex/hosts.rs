//! The Rex servers there are, and the connections to them.
//!
//! A server is one the user wrote into the configuration, or the one of their account
//! on this machine while it runs. What tells servers apart is where they are reached;
//! what they are called may change while they are connected.
//!
//! Nothing is connected to until somebody asks. A server elsewhere is connected when
//! the user chooses it and stays so until they disconnect it or the application ends.
//! The server of this machine is connected while something that lists servers is on
//! show, so that its sessions can be listed too. It is let go of soon afterwards, unless
//! a session of it is on show, because a server that was started for its clients goes
//! on running for as long as it has any. It is never started here.
//!
//! A connection that was never made is not tried again by itself: it stands as failed,
//! with the reason, until the user tries it. One that was made and is lost is made again
//! by the backend, for as long as that could work.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use lume_config::Config;
use lume_proto::BackendId;
use lume_rex::{ClientInfo, Endpoint, Link, Options, RexBackend, RexError};
use lume_session::SessionBackend as _;
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio::task::AbortHandle;
use tokio::time::Instant;

use super::installed;
use crate::configsvc::ConfigService;
use crate::error::CommandError;
use crate::panes::Panes;
use crate::state::AppState;
use crate::tuning::{REX_CONNECT_WAIT, REX_LEASE, REX_LOOK_EVERY};

/// What the server of this machine is called, where the user has not called it
/// anything.
const LOCAL_LABEL: &str = "This computer";
/// What the ids of these backends begin with.
const ID_PREFIX: &str = "rex:";

/// A server, as far as it is known without talking to it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Host {
    /// What the application knows its backend as.
    id: BackendId,
    label: String,
    /// The address as the user wrote it.
    address: String,
    endpoint: Endpoint,
}

impl Host {
    fn is_local(&self) -> bool {
        self.endpoint == Endpoint::Local
    }
}

/// An address the user wrote that cannot be reached.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Unusable {
    label: String,
    address: String,
    failure: Failure,
}

/// Why a server is not connected, though it was to be.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Failure {
    /// A short, stable name for the kind of failure.
    code: String,
    error: String,
}

/// A connection that was asked for and has not failed.
struct Connection {
    backend: Arc<RexBackend>,
    watcher: AbortHandle,
    /// Whether it is to the server of this machine.
    local: bool,
    /// Whether the user asked for it, so that it is theirs to end.
    kept: bool,
    /// Whether it was ever made.
    up: bool,
    /// How long the server last took to answer, in milliseconds.
    latency_ms: Option<u64>,
    /// The session of it that was put on show and has not been put away since.
    showing: Option<String>,
    /// Whether there was anything in that session while it was on show.
    filled: bool,
}

#[derive(Default)]
struct Inner {
    hosts: Vec<Host>,
    unusable: Vec<Unusable>,
    connections: HashMap<BackendId, Connection>,
    failures: HashMap<BackendId, Failure>,
    /// The session last put on show, by server.
    last: HashMap<BackendId, String>,
    /// Whether something answered as the server of this machine when last looked.
    local_there: bool,
    /// Whether a server is installed here, so that one that does not run is listed.
    local_installed: bool,
    /// Until when something that lists servers is taken to be on show.
    lease: Option<Instant>,
    /// Whether the application is ending.
    closed: bool,
}

/// The Rex servers.
pub struct Hosts {
    config: Arc<ConfigService>,
    state: Weak<AppState>,
    panes: Weak<Panes>,
    inner: Mutex<Inner>,
    /// What pages were last told.
    told: Mutex<Value>,
    revision: AtomicU64,
    /// Goes up whenever how a server stands changes.
    ticks: watch::Sender<u64>,
    /// One session is put on show at a time.
    opening: tokio::sync::Mutex<()>,
}

/// The servers a configuration names, and the addresses in it that cannot be reached.
fn hosts_of(config: &Config) -> (Vec<Host>, Vec<Unusable>) {
    let mut hosts: Vec<Host> = Vec::new();
    let mut unusable = Vec::new();
    for entry in &config.rex.hosts {
        let failure = match Endpoint::parse(&entry.address) {
            Ok(endpoint) => {
                let id = id_of(&endpoint);
                match hosts.iter().find(|host| host.id == id) {
                    None => {
                        hosts.push(Host {
                            id,
                            label: entry.label.clone(),
                            address: entry.address.clone(),
                            endpoint,
                        });
                        continue;
                    }
                    Some(same) => Failure {
                        code: "bad_request".to_owned(),
                        error: format!(
                            "the address is the same server's as that of `{}`",
                            same.label
                        ),
                    },
                }
            }
            Err(error) => Failure {
                code: error.code().to_owned(),
                error: error.to_string(),
            },
        };
        unusable.push(Unusable {
            label: entry.label.clone(),
            address: entry.address.clone(),
            failure,
        });
    }
    if config.rex.local && !hosts.iter().any(Host::is_local) {
        hosts.insert(
            0,
            Host {
                id: id_of(&Endpoint::Local),
                label: LOCAL_LABEL.to_owned(),
                address: Endpoint::Local.to_string(),
                endpoint: Endpoint::Local,
            },
        );
    }
    (hosts, unusable)
}

fn id_of(endpoint: &Endpoint) -> BackendId {
    BackendId(format!("{ID_PREFIX}{endpoint}"))
}

fn command_error(error: &RexError) -> CommandError {
    CommandError::new(error.code().to_owned(), error.to_string())
}

impl Hosts {
    /// The servers the configuration names. Nothing is connected to.
    pub fn new(
        config: &Arc<ConfigService>,
        state: &Arc<AppState>,
        panes: &Arc<Panes>,
    ) -> Arc<Self> {
        let (hosts, unusable) = hosts_of(&config.current().config);
        let this = Arc::new(Self {
            config: Arc::clone(config),
            state: Arc::downgrade(state),
            panes: Arc::downgrade(panes),
            inner: Mutex::new(Inner {
                hosts,
                unusable,
                ..Inner::default()
            }),
            told: Mutex::new(Value::Null),
            revision: AtomicU64::new(1),
            ticks: watch::Sender::new(0),
            opening: tokio::sync::Mutex::new(()),
        });
        *this.told() = this.body();
        this
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn told(&self) -> MutexGuard<'_, Value> {
        self.told.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn served_by(&self) -> Option<BackendId> {
        self.panes.upgrade()?.served_by()
    }

    /// How every server stands, without a revision.
    fn body(&self) -> Value {
        let served = self.served_by();
        let inner = self.inner();
        let mut listed = Vec::new();
        for host in &inner.hosts {
            let connection = inner.connections.get(&host.id);
            let failure = inner.failures.get(&host.id);
            if host.is_local()
                && connection.is_none()
                && failure.is_none()
                && !inner.local_there
                && !inner.local_installed
            {
                // There is no such server, and nothing to say of it.
                continue;
            }
            let state = match (connection, failure) {
                (Some(connection), _) => match &*connection.backend.link().borrow() {
                    Link::Connecting => json!({ "kind": "connecting" }),
                    Link::Connected(version) => json!({
                        "kind": "connected",
                        "version": version.rex_version,
                        "platform": version.platform,
                        "api_revision": version.api_revision,
                    }),
                    Link::Waiting { error, .. } => json!({ "kind": "waiting", "error": error }),
                    Link::Refused { code, error } => {
                        json!({ "kind": "failed", "code": code, "error": error })
                    }
                    Link::Closed => json!({ "kind": "idle" }),
                },
                (None, Some(failure)) => {
                    json!({ "kind": "failed", "code": failure.code, "error": failure.error })
                }
                (None, None) if host.is_local() && !inner.local_there => {
                    json!({ "kind": "absent" })
                }
                (None, None) => json!({ "kind": "idle" }),
            };
            let sessions: Vec<Value> = connection
                .filter(|connection| {
                    matches!(
                        &*connection.backend.link().borrow(),
                        Link::Connected(_) | Link::Waiting { .. }
                    )
                })
                .map(|connection| connection.backend.sessions())
                .unwrap_or_default()
                .into_iter()
                .map(|session| json!({ "id": session.id, "label": session.label }))
                .collect();
            let on_show = connection
                .and_then(|connection| connection.showing.as_ref())
                .filter(|_| served.as_ref() == Some(&host.id));
            listed.push(json!({
                "id": host.id,
                "label": host.label,
                "address": host.address,
                "local": host.is_local(),
                "state": state,
                "sessions": sessions,
                "on_show": on_show,
                "latency_ms": connection.and_then(|connection| connection.latency_ms),
            }));
        }
        for unusable in &inner.unusable {
            listed.push(json!({
                // No backend is ever made for it.
                "id": format!("{ID_PREFIX}?{}", unusable.label),
                "label": unusable.label,
                "address": unusable.address,
                "local": false,
                "state": {
                    "kind": "failed",
                    "code": unusable.failure.code,
                    "error": unusable.failure.error,
                },
                "sessions": [],
                "on_show": null,
                "latency_ms": null,
            }));
        }
        json!({ "hosts": listed, "local": self.config.current().config.rex.local })
    }

    /// How every server stands, as a page or the command line is told.
    pub fn view(&self) -> Value {
        self.announce();
        let mut view = self.told().clone();
        if let Some(view) = view.as_object_mut() {
            view.insert(
                "revision".to_owned(),
                json!(self.revision.load(Ordering::Relaxed)),
            );
        }
        view
    }

    /// Tells the pages if how the servers stand is not what they were last told.
    fn announce(&self) {
        let notice = {
            // Read and told under one lock, so that what is told last is what is so.
            let mut told = self.told();
            let body = self.body();
            if *told == body {
                return;
            }
            *told = body;
            let revision = self.revision.fetch_add(1, Ordering::Relaxed) + 1;
            let mut notice = told.clone();
            if let Some(notice) = notice.as_object_mut() {
                notice.insert("type".to_owned(), json!("rex_hosts_changed"));
                notice.insert("revision".to_owned(), json!(revision));
            }
            notice
        };
        if let Some(state) = self.state.upgrade() {
            state.broadcast(&notice);
        }
    }

    /// Something about a server changed.
    fn touch(&self) {
        self.ticks
            .send_modify(|ticks| *ticks = ticks.wrapping_add(1));
        self.announce();
    }

    /// The configuration changed: servers it no longer names are let go of.
    pub fn config_changed(self: &Arc<Self>) {
        let (hosts, unusable) = hosts_of(&self.config.current().config);
        let gone: Vec<BackendId> = {
            let mut inner = self.inner();
            inner.hosts = hosts;
            inner.unusable = unusable;
            let Inner {
                hosts,
                connections,
                failures,
                ..
            } = &mut *inner;
            let named = |id: &BackendId| hosts.iter().any(|host| host.id == *id);
            failures.retain(|id, _| named(id));
            connections
                .keys()
                .filter(|id| !named(id))
                .cloned()
                .collect()
        };
        for id in gone {
            let this = Arc::clone(self);
            if let Some(state) = self.state.upgrade() {
                state.runtime().spawn(async move { this.end(&id).await });
            }
        }
        self.touch();
    }

    /// Says that something that lists servers is on show: the server of this machine is
    /// looked for and connected if it runs, and the connected servers are asked how
    /// fast they answer. This has to be said again before the lease on it runs out.
    pub async fn look(self: &Arc<Self>) -> Value {
        let local = {
            let mut inner = self.inner();
            inner.lease = Some(Instant::now() + REX_LEASE);
            inner.hosts.iter().find(|host| host.is_local()).cloned()
        };
        if let Some(local) = local {
            let connected = self.inner().connections.contains_key(&local.id);
            let there = connected || lume_rex::listens(&local.endpoint).await;
            let installed = there
                || tokio::task::spawn_blocking(installed::is_installed)
                    .await
                    .unwrap_or(false);
            let failed = {
                let mut inner = self.inner();
                inner.local_there = there;
                inner.local_installed = installed;
                if !there {
                    // Why it could not be used went with it.
                    inner.failures.remove(&local.id);
                }
                inner.failures.contains_key(&local.id)
            };
            // One that refused is not asked again by itself.
            if there && !connected && !failed {
                self.begin(&local, false, false);
            }
        }
        let connected: Vec<(BackendId, Arc<RexBackend>)> = self
            .inner()
            .connections
            .iter()
            .map(|(id, connection)| (id.clone(), Arc::clone(&connection.backend)))
            .collect();
        for (id, backend) in connected {
            self.measure(id, backend);
        }
        self.view()
    }

    /// Asks a server how fast it answers, beside whatever else goes on.
    fn measure(self: &Arc<Self>, id: BackendId, backend: Arc<RexBackend>) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let this = Arc::downgrade(self);
        state.runtime().spawn(async move {
            let Ok(took) = backend.round_trip().await else {
                return;
            };
            let Some(this) = this.upgrade() else { return };
            let noted = match this.inner().connections.get_mut(&id) {
                Some(connection) if Arc::ptr_eq(&connection.backend, &backend) => {
                    connection.latency_ms = u64::try_from(took.as_millis()).ok();
                    true
                }
                _ => false,
            };
            if noted {
                this.announce();
            }
        });
    }

    /// The server somebody means by its id, its label or its address.
    fn find(&self, host: &str) -> Result<Host, CommandError> {
        let inner = self.inner();
        let wanted = host.trim();
        let lower = wanted.to_lowercase();
        let by_label = |host: &&Host| host.label.to_lowercase() == lower;
        if let Some(found) = inner
            .hosts
            .iter()
            .find(|host| host.id.0 == wanted)
            .or_else(|| inner.hosts.iter().find(by_label))
            .or_else(|| inner.hosts.iter().find(|host| host.address == wanted))
        {
            return Ok(found.clone());
        }
        if let Some(unusable) = inner
            .unusable
            .iter()
            .find(|host| host.label.to_lowercase() == lower || host.address == wanted)
        {
            return Err(CommandError::new(
                unusable.failure.code.clone(),
                unusable.failure.error.clone(),
            ));
        }
        // An address the user did not list may still be one that is.
        if let Ok(endpoint) = Endpoint::parse(wanted)
            && let Some(found) = inner.hosts.iter().find(|host| host.endpoint == endpoint)
        {
            return Ok(found.clone());
        }
        Err(CommandError::not_found(format!(
            "there is no Rex server called `{wanted}`"
        )))
    }

    /// Starts a connection to a server, in place of one that has failed. `agreed` is
    /// whether the user agreed to one that is not encrypted.
    fn begin(self: &Arc<Self>, host: &Host, agreed: bool, kept: bool) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let _in_runtime = state.runtime().enter();
        let mut inner = self.inner();
        if inner.closed || inner.connections.contains_key(&host.id) {
            return;
        }
        inner.failures.remove(&host.id);
        let backend = RexBackend::connect_with(
            host.id.clone(),
            host.endpoint.clone(),
            ClientInfo::app("Lume", env!("CARGO_PKG_VERSION")),
            Options::new().plain_http_agreed(agreed),
        );
        state.router().insert(host.id.clone(), backend.clone());
        let watcher = state
            .runtime()
            .spawn(watch(
                Arc::downgrade(self),
                host.id.clone(),
                Arc::clone(&backend),
            ))
            .abort_handle();
        inner.connections.insert(
            host.id.clone(),
            Connection {
                backend,
                watcher,
                local: host.is_local(),
                kept,
                up: false,
                latency_ms: None,
                showing: None,
                filled: false,
            },
        );
        drop(inner);
        tracing::info!(server = %host.address, "connecting to a Rex server");
        self.touch();
    }

    /// Takes a connection out of the application and ends it.
    fn take(&self, id: &BackendId) -> Option<Connection> {
        let connection = self.inner().connections.remove(id)?;
        self.let_go(id, &connection);
        Some(connection)
    }

    /// Ends a connection that has been taken out.
    fn let_go(&self, id: &BackendId, connection: &Connection) {
        if let Some(state) = self.state.upgrade() {
            state.router().remove(id);
        }
        connection.backend.shutdown();
    }

    /// Ends the connection to a server, if there is one. If a session of it is on show,
    /// the application's own workspace is shown again.
    async fn end(self: &Arc<Self>, id: &BackendId) {
        if self.served_by().as_ref() == Some(id) {
            self.back_to_own().await;
        }
        if let Some(connection) = self.take(id) {
            connection.watcher.abort();
        }
        self.inner().failures.remove(id);
        self.touch();
    }

    async fn back_to_own(&self) {
        if let Some(panes) = self.panes.upgrade()
            && let Err(error) = panes.back_to_own().await
        {
            tracing::warn!(%error, "the workspace before the server's could not be shown again");
        }
    }

    /// Connects to a server if it is not connected, and waits until it is.
    pub async fn connect(self: &Arc<Self>, host: &str, agreed: bool) -> Result<(), CommandError> {
        let host = self.find(host)?;
        self.connected(&host, agreed, true).await.map(drop)
    }

    /// The connection to a server, once it is made. `keep` is whether it stays when
    /// nothing of the server is on show and nothing lists servers.
    async fn connected(
        self: &Arc<Self>,
        host: &Host,
        agreed: bool,
        keep: bool,
    ) -> Result<Arc<RexBackend>, CommandError> {
        // Subscribed first, so that nothing is missed between a look and the wait.
        let mut ticks = self.ticks.subscribe();
        self.begin(host, agreed, keep);
        if let Some(connection) = self.inner().connections.get_mut(&host.id) {
            connection.kept |= keep;
        }
        let waiting = async {
            loop {
                ticks.borrow_and_update();
                {
                    let inner = self.inner();
                    if let Some(failure) = inner.failures.get(&host.id) {
                        return Err(CommandError::new(
                            failure.code.clone(),
                            failure.error.clone(),
                        ));
                    }
                    let Some(connection) = inner.connections.get(&host.id) else {
                        if host.is_local() {
                            return Err(CommandError::new(
                                "unreachable",
                                "no Rex server runs on this computer",
                            ));
                        }
                        return Err(CommandError::new(
                            "disconnected",
                            format!("{} was disconnected", host.label),
                        ));
                    };
                    let stands = connection.backend.link().borrow().clone();
                    match stands {
                        Link::Connected(_) => return Ok(Arc::clone(&connection.backend)),
                        Link::Waiting { error, .. } if connection.up => {
                            return Err(CommandError::new("unreachable", error));
                        }
                        _ => {}
                    }
                }
                if ticks.changed().await.is_err() {
                    return Err(CommandError::internal("the application is closing"));
                }
            }
        };
        tokio::time::timeout(REX_CONNECT_WAIT, waiting)
            .await
            .unwrap_or_else(|_| {
                Err(CommandError::new(
                    "timeout",
                    format!("{} did not answer in time", host.label),
                ))
            })
    }

    /// Ends the connection to a server.
    pub async fn disconnect(self: &Arc<Self>, host: &str) -> Result<(), CommandError> {
        let host = self.find(host)?;
        self.end(&host.id).await;
        Ok(())
    }

    /// Puts a session of a server on show, connecting to the server first if need be.
    /// `session` is an id or a label; left out, the session of it that was last on show,
    /// else the first.
    pub async fn open(
        self: &Arc<Self>,
        host: &str,
        session: Option<&str>,
        agreed: bool,
    ) -> Result<(), CommandError> {
        let host = self.find(host)?;
        let _one_at_a_time = self.opening.lock().await;
        let backend = self.connected(&host, agreed, !host.is_local()).await?;
        let sessions = backend.sessions();
        let chosen = match session.map(str::trim).filter(|session| !session.is_empty()) {
            Some(wanted) => sessions
                .iter()
                .find(|session| session.id == wanted)
                .or_else(|| sessions.iter().find(|session| session.label == wanted))
                .ok_or_else(|| {
                    CommandError::not_found(format!(
                        "{} has no session called `{wanted}`",
                        host.label
                    ))
                })?,
            None => {
                let last = self.inner().last.get(&host.id).cloned();
                sessions
                    .iter()
                    .find(|session| Some(&session.id) == last.as_ref())
                    .or_else(|| sessions.first())
                    .ok_or_else(|| {
                        CommandError::new(
                            "no_session",
                            format!("{} has no session to show", host.label),
                        )
                    })?
            }
        };
        self.show(&host, &backend, &chosen.id).await
    }

    /// Makes a session on a server and puts it on show.
    pub async fn new_session(
        self: &Arc<Self>,
        host: &str,
        label: Option<&str>,
        agreed: bool,
    ) -> Result<(), CommandError> {
        let host = self.find(host)?;
        let _one_at_a_time = self.opening.lock().await;
        let backend = self.connected(&host, agreed, !host.is_local()).await?;
        let made = backend
            .create_session(label)
            .await
            .map_err(|error| command_error(&error))?;
        self.show(&host, &backend, &made).await
    }

    /// Puts a session on show. The caller holds `opening`.
    async fn show(
        self: &Arc<Self>,
        host: &Host,
        backend: &Arc<RexBackend>,
        session: &str,
    ) -> Result<(), CommandError> {
        let panes = self
            .panes
            .upgrade()
            .ok_or_else(|| CommandError::internal("the application is closing"))?;
        let before = backend.shown_session();
        backend
            .show_session(session)
            .await
            .map_err(|error| command_error(&error))?;
        // The layout of a backend is followed from one session of it to the next. One
        // that has ended with an earlier connection is not.
        let followed = backend
            .layout()
            .is_some_and(|layout| panes.follows(&host.id, &layout));
        if !followed && let Err(error) = panes.show_served(&host.id).await {
            match &before {
                Some(before) => drop(backend.show_session(before).await),
                None => backend.show_nothing(),
            }
            return Err(error);
        }
        {
            let mut inner = self.inner();
            inner.last.insert(host.id.clone(), session.to_owned());
            if let Some(connection) = inner.connections.get_mut(&host.id) {
                connection.filled = backend
                    .view(session)
                    .is_some_and(|view| !view.windows.is_empty());
                connection.showing = Some(session.to_owned());
            }
        }
        // Others who look at the session that was on show are not told that this client
        // still does.
        if let Some(before) = before.filter(|before| before != session)
            && let Err(error) = backend.close_session(&before).await
        {
            tracing::debug!(%error, "a session that is no longer on show was not let go of");
        }
        // The session of another server that was on show is let go of by its watcher.
        self.touch();
        Ok(())
    }

    /// A connection has failed for good, or was never made.
    fn failed(self: &Arc<Self>, id: &BackendId, failure: Failure) {
        tracing::info!(server = %id.0, code = failure.code, "a Rex server is not connected");
        // Under one lock: whoever waits for the connection sees it, or why it failed.
        let (connection, label) = {
            let mut inner = self.inner();
            let Some(connection) = inner.connections.remove(id) else {
                return;
            };
            // A server of this machine that went between being looked for and being
            // connected to is simply not there.
            let gone = connection.local && !connection.kept && failure.code == "unreachable";
            inner.local_there &= !connection.local;
            if !gone {
                inner.failures.insert(id.clone(), failure.clone());
            }
            let label = inner
                .hosts
                .iter()
                .find(|host| host.id == *id)
                .map_or_else(|| id.0.clone(), |host| host.label.clone());
            (connection, label)
        };
        self.let_go(id, &connection);
        self.touch();
        if connection.up
            && self.served_by().as_ref() == Some(id)
            && let Some(panes) = self.panes.upgrade()
        {
            panes.notice_with(
                "warning",
                format!("The connection to {label} has ended: {}.", failure.error),
                Some(json!({
                    "label": "Connect again",
                    "command": "lume.rex.open",
                    "args": { "host": id, "session": connection.showing },
                })),
            );
        }
    }

    /// Sees to the session of a server that was put on show: whether it still is, and
    /// whether it is still there to be shown. `served` is whether a layout of the server
    /// is what the windows show.
    async fn look_after(
        &self,
        id: &BackendId,
        backend: &Arc<RexBackend>,
        session: &str,
        served: bool,
    ) {
        let there = backend.sessions().iter().any(|listed| listed.id == session);
        let windows = backend.view(session).map(|view| view.windows.len());
        let emptied = {
            let mut inner = self.inner();
            let Some(connection) = inner.connections.get_mut(id) else {
                return;
            };
            connection.filled |= windows.is_some_and(|windows| windows > 0);
            // A session with nothing in it can be shown, and something be made in it.
            let emptied = connection.filled && windows == Some(0);
            if !served || !there || emptied {
                connection.showing = None;
            }
            emptied
        };
        if !served {
            // Another workspace was put on show meanwhile.
            backend.show_nothing();
            let _ = backend.close_session(session).await;
        } else if !there || emptied {
            self.back_to_own().await;
            if emptied {
                backend.show_nothing();
                let _ = backend.close_session(session).await;
            } else if let Some(panes) = self.panes.upgrade() {
                panes.notice(
                    "info",
                    "The session that was on show has ended on its server.",
                );
            }
        }
        self.announce();
    }

    /// Acts on how a connection stands now. Says whether to go on watching it.
    async fn observe(self: &Arc<Self>, id: &BackendId, backend: &Arc<RexBackend>) -> bool {
        let link = backend.link().borrow().clone();
        let connected = matches!(link, Link::Connected(_));
        let was_up = {
            let mut inner = self.inner();
            let Some(connection) = inner.connections.get_mut(id) else {
                return false;
            };
            let was_up = connection.up;
            connection.up |= connected;
            was_up
        };
        match link {
            Link::Connected(_) if !was_up => self.measure(id.clone(), Arc::clone(backend)),
            Link::Waiting { error, .. } if !was_up => {
                self.failed(
                    id,
                    Failure {
                        code: "unreachable".to_owned(),
                        error,
                    },
                );
                return false;
            }
            Link::Refused { code, error } => {
                self.failed(id, Failure { code, error });
                return false;
            }
            Link::Closed => return false,
            _ => {}
        }
        self.touch();
        // While a session is being put on show, what is on show is about to change.
        let Ok(_nothing_opens) = self.opening.try_lock() else {
            return true;
        };
        let served = self.served_by().as_ref() == Some(id);
        let showing = self
            .inner()
            .connections
            .get(id)
            .and_then(|connection| connection.showing.clone());
        if connected && let Some(session) = showing {
            self.look_after(id, backend, &session, served).await;
        }
        let idle = {
            let inner = self.inner();
            let leased = inner.lease.is_some_and(|lease| Instant::now() < lease);
            inner
                .connections
                .get(id)
                .is_some_and(|connection| connection.local && !connection.kept)
                && !leased
                && !served
        };
        if idle {
            tracing::debug!("the Rex server of this machine is let go of");
            self.take(id);
            self.touch();
            return false;
        }
        true
    }

    /// Ends every connection: the application is closing.
    pub fn shutdown(&self) {
        let ids: Vec<BackendId> = {
            let mut inner = self.inner();
            inner.closed = true;
            inner.connections.keys().cloned().collect()
        };
        for id in ids {
            if let Some(connection) = self.take(&id) {
                connection.watcher.abort();
            }
        }
    }
}

/// Follows a connection until it ends.
async fn watch(hosts: Weak<Hosts>, id: BackendId, backend: Arc<RexBackend>) {
    let mut link = backend.link();
    let mut changed = backend.changed();
    loop {
        link.borrow_and_update();
        changed.borrow_and_update();
        match hosts.upgrade() {
            Some(hosts) => {
                if !hosts.observe(&id, &backend).await {
                    return;
                }
            }
            None => return,
        }
        let gone = tokio::select! {
            seen = link.changed() => seen.is_err(),
            seen = changed.changed() => seen.is_err(),
            () = tokio::time::sleep(REX_LOOK_EVERY) => false,
        };
        if gone {
            return;
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
mod naming {
    use lume_config::{RexConfig, RexHostEntry};

    use super::*;

    fn config(local: bool, hosts: &[(&str, &str)]) -> Config {
        Config {
            rex: RexConfig {
                local,
                hosts: hosts
                    .iter()
                    .map(|(label, address)| RexHostEntry {
                        label: (*label).to_owned(),
                        address: (*address).to_owned(),
                    })
                    .collect(),
            },
            ..Config::default()
        }
    }

    #[test]
    fn the_servers_are_those_written_and_the_one_of_this_machine() {
        let (hosts, unusable) = hosts_of(&config(
            true,
            &[
                ("build", "HTTPS://Build.example"),
                ("again", "https://build.example:443/"),
                ("odd", "ftp://x"),
            ],
        ));
        let listed: Vec<(&str, &str)> = hosts
            .iter()
            .map(|host| (host.id.0.as_str(), host.label.as_str()))
            .collect();
        assert_eq!(
            listed,
            [
                ("rex:local", "This computer"),
                ("rex:https://build.example", "build"),
            ]
        );
        // The address is kept as it was written.
        assert_eq!(hosts[1].address, "HTTPS://Build.example");
        let wrong: Vec<(&str, &str)> = unusable
            .iter()
            .map(|host| (host.label.as_str(), host.failure.code.as_str()))
            .collect();
        assert_eq!(wrong, [("again", "bad_request"), ("odd", "bad_request")]);
    }

    #[test]
    fn the_server_of_this_machine_can_be_called_something_or_left_out() {
        let (hosts, _) = hosts_of(&config(true, &[("mine", "local")]));
        assert_eq!(hosts.len(), 1);
        assert!(hosts[0].is_local() && hosts[0].label == "mine");
        assert_eq!(hosts_of(&config(false, &[])).0, []);
    }
}
