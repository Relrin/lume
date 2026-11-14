//! The session host: it owns the terminals and the programs running in them.
//!
//! Everything else reaches a session through [`Host::serve`], which speaks the wire
//! protocol over any byte stream. There is no other way in. The same code therefore
//! serves a client in the same process over an in-memory pipe and a client in another
//! process over a socket, and nothing above the host can come to depend on sharing its
//! memory.
//!
//! The host does not interpret terminal output. It moves bytes from each terminal to
//! the client attached to it, as fast as that client says it can take them and no
//! faster: when a client stops granting credit, the host stops reading the terminal,
//! and the program in it blocks on its next write.

mod conn;
mod pending;
mod session;
pub mod tuning;

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use lume_proto::{
    BackendCaps, DecodeError, ErrorCode, HydrateKinds, Launcher, ProtoError, Responder, SessionId,
    SessionInfo, SpawnSpec,
};
use lume_pty::{CommandSpec, PtySize, PtySystem};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::conn::Outbox;
use crate::session::SessionHandle;

/// What a host is built from.
#[derive(Clone)]
pub struct HostConfig {
    /// Where terminals come from.
    pub pty: Arc<dyn PtySystem>,
    /// How much output a session keeps before its first client attaches.
    pub pre_attach_bytes: usize,
    /// The most sessions the host runs at once.
    pub max_sessions: usize,
}

impl Default for HostConfig {
    /// The platform's terminals and the limits in [`tuning`].
    fn default() -> Self {
        Self {
            pty: lume_pty::native(),
            pre_attach_bytes: tuning::PRE_ATTACH_BYTES,
            max_sessions: tuning::MAX_SESSIONS,
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
}

impl Host {
    pub fn new(config: HostConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            sessions: Mutex::new(HashMap::new()),
        })
    }

    /// Serves one client until it disconnects. Sessions the client started end with the
    /// connection: this host does not keep sessions for a client that has gone.
    pub async fn serve<S>(self: Arc<Self>, io: S) -> Result<(), ServeError>
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        conn::serve(self, io).await
    }

    /// What this host can and cannot do, as told to every client.
    pub(crate) fn caps(&self) -> BackendCaps {
        BackendCaps {
            persistent: false,
            multi_client: false,
            server_layout: false,
            // The host keeps no terminal state of its own, so it cannot answer a
            // program's questions about the terminal. The client's terminal does.
            responder: Responder::Client,
            hydrate: HydrateKinds {
                snapshot: false,
                vt_replay: false,
            },
        }
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

        let Launcher::Shell {
            program,
            args,
            login,
        } = spec.launcher
        else {
            return Err(ProtoError::new(
                ErrorCode::Unsupported,
                "this host cannot start that",
            ));
        };
        let mut command = CommandSpec::new(&program).args(&args);
        command.login = login;
        command.cwd = spec.cwd.map(Into::into);
        command.env = spec
            .env
            .into_iter()
            .map(|(name, value)| (name.into(), value.into()))
            .collect();
        let size = spec.size;

        // Starting a process blocks for a moment, so it runs off the async threads.
        let pty_system = Arc::clone(&self.config.pty);
        let spawned = tokio::task::spawn_blocking(move || {
            pty_system.spawn(&command, PtySize::cells(size.cols(), size.rows()))
        })
        .await
        .map_err(|error| ProtoError::new(ErrorCode::Internal, error.to_string()))?;
        let (pty, child) = spawned.map_err(|error| {
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
            pre_attach_bytes: self.config.pre_attach_bytes,
            resize_pulls_scrollback: self.config.pty.resize_pulls_scrollback(),
        })
        .map_err(|error| ProtoError::new(ErrorCode::SpawnFailed, error.to_string()))?;
        self.sessions().insert(id, handle);
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
