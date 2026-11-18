//! The session host: it owns the terminals and the programs running in them.
//!
//! Everything else reaches a session through [`Host::serve`], which speaks the wire
//! protocol over any byte stream. There is no other way in. The same code therefore
//! serves a client in the same process over an in-memory pipe and a client in another
//! process over a socket, and nothing above the host can come to depend on sharing its
//! memory.
//!
//! The host keeps a copy of every session's screen, fed by the same bytes it moves to the
//! client attached to it. A client that attaches starts from that copy, and the copy
//! answers the program's questions to the terminal whether or not a client is attached.
//! Output reaches a client as fast as it says it can take it and no faster: when the
//! client stops granting credit, the host stops reading the terminal, and the program
//! in it blocks on its next write. With no client attached the host reads on, into the
//! screen alone.

mod conn;
mod integration;
mod launch;
mod pending;
mod scan;
mod screen;
mod session;
mod ssh_exit;
pub mod tuning;

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lume_proto::{
    BackendCaps, DecodeError, ErrorCode, HydrateKinds, ProtoError, Responder, SessionId,
    SessionInfo, SpawnSpec,
};
use lume_pty::{PtySize, PtySystem};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;

use crate::conn::Outbox;
use crate::launch::Launch;
use crate::session::SessionHandle;

pub use crate::screen::vt_version;

/// What a host is built from.
#[derive(Clone)]
pub struct HostConfig {
    /// Where terminals come from.
    pub pty: Arc<dyn PtySystem>,
    /// Whether sessions outlive the connection that started them. A host serving one
    /// client for the life of that client's process ends them with the connection.
    /// Only a persistent host can be asked to stop: one that is not stops with its
    /// client.
    pub persistent: bool,
    /// How long the host stays with no running session and no client before it exits,
    /// as told to clients. Whoever runs the host does the waiting
    /// ([`Host::idle_changes`]).
    pub idle_exit: Option<Duration>,
    /// The most sessions the host runs at once.
    pub max_sessions: usize,
    /// A folder the host may write its shell scripts to. Without one, no shell is
    /// asked to report anything, whatever a session's description says.
    pub shell_integration_dir: Option<PathBuf>,
}

impl Default for HostConfig {
    /// The platform's terminals and the limits in [`tuning`].
    fn default() -> Self {
        Self {
            pty: lume_pty::native(),
            persistent: false,
            idle_exit: None,
            max_sessions: tuning::MAX_SESSIONS,
            shell_integration_dir: None,
        }
    }
}

/// Why serving a connection ended, other than the client closing it.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error("the connection failed: {0}")]
    Io(#[from] io::Error),
    #[error("the client broke the protocol: {0}")]
    Protocol(#[from] DecodeError),
}

/// A session host. Create one and serve each client connection with [`Host::serve`].
pub struct Host {
    config: HostConfig,
    sessions: Mutex<HashMap<SessionId, SessionHandle>>,
    /// Where the shell scripts were written, once the first session that wants them
    /// has been started. `None` inside: there is nowhere to write them, or it failed.
    scripts: OnceLock<Option<PathBuf>>,
    /// When the host started. Things sessions do at intervals are counted from here.
    started: Instant,
    /// The same moment by the wall clock, for status.
    started_unix_ms: u64,
    /// How many clients are connected, besides command-line tools asking questions.
    clients: AtomicUsize,
    /// How many connections are open, whoever is on the other end and whether or not
    /// it has said hello.
    connections: AtomicUsize,
    /// Bumped whenever a client comes or goes and whenever a session starts, ends or is
    /// removed: whatever can make the host idle, or stop it being idle.
    changes: watch::Sender<u64>,
    /// Set once the host has been asked to stop. Connections close when it is.
    stopping: watch::Sender<bool>,
}

impl Host {
    pub fn new(config: HostConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            sessions: Mutex::new(HashMap::new()),
            scripts: OnceLock::new(),
            started: Instant::now(),
            started_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| {
                    u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
                }),
            clients: AtomicUsize::new(0),
            connections: AtomicUsize::new(0),
            changes: watch::Sender::new(0),
            stopping: watch::Sender::new(false),
        })
    }

    /// Whether the host has nothing to do: no client is connected and no session's
    /// program runs. A session whose program has ended and that waits for a client to
    /// see it does not keep the host busy.
    pub fn is_idle(&self) -> bool {
        self.clients.load(Ordering::SeqCst) == 0
            && self
                .sessions()
                .iter()
                .all(|(id, handle)| handle.info(*id).exit.is_some())
    }

    /// Changes after which [`is_idle`](Self::is_idle) may say something else. The value
    /// itself means nothing; wait for it to change.
    pub fn idle_changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// Resolves once the host has been asked to stop. By then every connection is
    /// closing; whoever runs the host waits a moment for [`clients`](Self::clients) to
    /// reach zero and exits.
    pub async fn stopped(&self) {
        let mut stopping = self.stopping.subscribe();
        let _ = stopping.wait_for(|stopping| *stopping).await;
    }

    /// How many clients are connected, besides command-line tools asking questions.
    pub fn clients(&self) -> usize {
        self.clients.load(Ordering::SeqCst)
    }

    /// How many connections are open. One that has just been made may be a client that
    /// has not said yet what it is, so a host does not exit while any is open.
    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    pub(crate) fn connection_opened(&self) {
        self.connections.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn connection_closed(&self) {
        self.connections.fetch_sub(1, Ordering::SeqCst);
        self.touched();
    }

    pub(crate) fn stopping(&self) -> watch::Receiver<bool> {
        self.stopping.subscribe()
    }

    /// Something that bears on whether the host is idle has changed.
    pub(crate) fn touched(&self) {
        self.changes
            .send_modify(|count| *count = count.wrapping_add(1));
    }

    pub(crate) fn client_came(&self) {
        self.clients.fetch_add(1, Ordering::SeqCst);
        self.touched();
    }

    pub(crate) fn client_went(&self) {
        self.clients.fetch_sub(1, Ordering::SeqCst);
        self.touched();
    }

    /// Ends every session and waits, for a while, until their programs have gone.
    /// Returns how many there were.
    pub(crate) async fn end_all(&self) -> u32 {
        let handles: Vec<SessionHandle> = self.sessions().values().cloned().collect();
        let mut ended = Vec::new();
        for handle in handles {
            if handle.send(session::Command::Kill).await {
                ended.push(handle);
            }
        }
        let all_ended = async {
            for session in &ended {
                session.ended().await;
            }
        };
        if tokio::time::timeout(tuning::ABANDON_WAIT, all_ended)
            .await
            .is_err()
        {
            tracing::warn!("sessions were still ending when the host stopped");
        }
        u32::try_from(ended.len()).unwrap_or(u32::MAX)
    }

    /// Asks every connection to close and whoever runs the host to exit. Sessions are
    /// left as they are.
    pub fn stop(&self) {
        self.stopping.send_replace(true);
        self.touched();
    }

    pub(crate) fn status(&self) -> lume_proto::ResponseBody {
        let sessions = self.sessions().len();
        lume_proto::ResponseBody::HostStatus {
            host_version: env!("CARGO_PKG_VERSION").to_owned(),
            pid: std::process::id(),
            started_unix_ms: self.started_unix_ms,
            persistent: self.config.persistent,
            sessions: u32::try_from(sessions).unwrap_or(u32::MAX),
            clients: u32::try_from(self.clients()).unwrap_or(u32::MAX),
            idle_exit_ms: self
                .config
                .idle_exit
                .map(|wait| u64::try_from(wait.as_millis()).unwrap_or(u64::MAX)),
        }
    }

    /// The next moment after `now` at which sessions look up the program that has
    /// their terminal. Every session looks at the same moments, because reading the
    /// system's list of processes costs milliseconds and one reading serves all that
    /// ask within a short while of each other.
    pub(crate) fn next_poll(&self, now: Instant) -> Instant {
        let interval = tuning::FOREGROUND_POLL_INTERVAL;
        let since = now.saturating_duration_since(self.started);
        let ticks = since.as_millis() / interval.as_millis().max(1) + 1;
        self.started + interval * u32::try_from(ticks).unwrap_or(u32::MAX)
    }

    /// The folder the shell scripts are in, written on first use.
    fn scripts(&self) -> Option<PathBuf> {
        self.scripts
            .get_or_init(|| {
                let root = self.config.shell_integration_dir.as_deref()?;
                integration::install(root)
                    .inspect_err(|error| {
                        tracing::warn!(%error, "the shell scripts could not be written");
                    })
                    .ok()
            })
            .clone()
    }

    /// Serves one client until it disconnects. Unless the host is
    /// [`persistent`](HostConfig::persistent), sessions the client started end with the
    /// connection.
    ///
    /// The connection counts as open from the call, before the future is first polled.
    pub fn serve<S>(
        self: Arc<Self>,
        io: S,
    ) -> impl std::future::Future<Output = Result<(), ServeError>> + Send + 'static
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        self.connection_opened();
        async move {
            let result = conn::serve(Arc::clone(&self), io).await;
            self.connection_closed();
            result
        }
    }

    /// What this host can and cannot do, as told to every client.
    pub(crate) fn caps(&self) -> BackendCaps {
        BackendCaps {
            persistent: self.config.persistent,
            multi_client: false,
            server_layout: false,
            // Every session's screen answers its program, so that a program gets its
            // answer whether or not anybody is looking, and gets it once.
            responder: Responder::Backend,
            hydrate: HydrateKinds {
                snapshot: true,
                vt_replay: true,
            },
        }
    }

    pub(crate) fn persistent(&self) -> bool {
        self.config.persistent
    }

    fn sessions(&self) -> std::sync::MutexGuard<'_, HashMap<SessionId, SessionHandle>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn list(&self) -> Vec<SessionInfo> {
        let mut sessions: Vec<SessionInfo> = self
            .sessions()
            .iter()
            .map(|(id, handle)| handle.info(*id))
            .collect();
        sessions.sort_by_key(|info| (info.created_unix_ms, info.id));
        sessions
    }

    pub(crate) fn session(&self, id: SessionId) -> Option<SessionHandle> {
        self.sessions().get(&id).cloned()
    }

    pub(crate) fn remove(&self, id: SessionId) {
        self.sessions().remove(&id);
        self.touched();
    }

    /// Starts a session. `owner` is the connection that asked for it, which is told when
    /// the session's program ends.
    pub(crate) async fn spawn(
        self: &Arc<Self>,
        spec: SpawnSpec,
        owner: Outbox,
    ) -> Result<SessionId, ProtoError> {
        if self.sessions().len() >= self.config.max_sessions {
            return Err(ProtoError::new(
                ErrorCode::LimitExceeded,
                format!(
                    "the host already runs {} sessions",
                    self.config.max_sessions
                ),
            ));
        }

        let size = spec.size;
        let scrollback = spec
            .scrollback_lines
            .map_or(tuning::SCROLLBACK_DEFAULT_LINES, |lines| lines as usize)
            .min(tuning::SCROLLBACK_MAX_LINES);
        let ssh = matches!(spec.launcher, lume_proto::Launcher::Ssh { .. });
        // Starting a process blocks for a moment, and so may writing the scripts, so
        // both run off the async threads.
        let host = Arc::clone(self);
        let pty_system = Arc::clone(&self.config.pty);
        let spawned = tokio::task::spawn_blocking(move || {
            let scripts = spec.shell_integration.then(|| host.scripts()).flatten();
            let Launch { command, program } = launch::launch(spec, scripts.as_deref())?;
            let started = pty_system.spawn(&command, PtySize::cells(size.cols(), size.rows()));
            Ok::<_, ProtoError>(started.map(|(pty, child)| (pty, child, program)))
        })
        .await
        .map_err(|error| ProtoError::new(ErrorCode::Internal, error.to_string()))??;
        let (pty, child, program) = spawned.map_err(|error| {
            let code = match error.kind() {
                io::ErrorKind::NotFound | io::ErrorKind::InvalidInput => ErrorCode::InvalidSpec,
                _ => ErrorCode::SpawnFailed,
            };
            ProtoError::new(code, error.to_string())
        })?;

        let id = new_session_id()?;
        let handle = session::start(session::Start {
            host: Arc::clone(self),
            id,
            program,
            pty,
            child,
            size,
            owner,
            scrollback,
            resize_pulls_scrollback: self.config.pty.resize_pulls_scrollback(),
            ssh,
        })
        .map_err(|error| ProtoError::new(ErrorCode::SpawnFailed, error.to_string()))?;
        self.sessions().insert(id, handle);
        self.touched();
        tracing::debug!(session = %id, "session started");
        Ok(id)
    }
}

/// A random session id. Random rather than counted, so an id a client kept cannot name
/// a different session after the host has restarted.
fn new_session_id() -> Result<SessionId, ProtoError> {
    loop {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes)
            .map_err(|error| ProtoError::new(ErrorCode::Internal, error.to_string()))?;
        if let Some(id) = SessionId::new(u64::from_le_bytes(bytes)) {
            return Ok(id);
        }
    }
}
