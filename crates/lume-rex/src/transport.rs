//! Reaching a server: the stream of bytes a connection is made over.

use std::io;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::endpoint::Endpoint;
use crate::error::RexError;

/// A stream of bytes to a server and back.
pub trait Io: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Io for T {}

fn unreachable(endpoint: &Endpoint, source: io::Error) -> RexError {
    RexError::Unreachable {
        endpoint: endpoint.to_string(),
        source,
    }
}

/// Opens a stream to the server at an endpoint. Nothing is said on it yet.
pub async fn dial(endpoint: &Endpoint) -> Result<Box<dyn Io>, RexError> {
    match endpoint {
        Endpoint::Local => platform::local(endpoint).await,
        Endpoint::Pipe(name) => platform::pipe(endpoint, name).await,
        Endpoint::Unix(path) => platform::unix(endpoint, path).await,
    }
}

#[cfg(windows)]
mod platform {
    use std::io;
    use std::path::Path;
    use std::time::Duration;

    use tokio::net::windows::named_pipe::ClientOptions;

    use super::{Io, unreachable};
    use crate::endpoint::Endpoint;
    use crate::error::RexError;

    /// Every instance of the pipe is taken by another client for the moment.
    const ERROR_PIPE_BUSY: i32 = 231;
    /// How often a busy pipe is tried again, and how long between tries.
    const BUSY_TRIES: u32 = 20;
    const BUSY_WAIT: Duration = Duration::from_millis(25);

    /// The pipe of the user's own server: `rex-` and the account's security identifier.
    pub(super) async fn local(endpoint: &Endpoint) -> Result<Box<dyn Io>, RexError> {
        let sid = lume_ipc::user_sid().map_err(|error| unreachable(endpoint, error))?;
        pipe(endpoint, &format!(r"\\.\pipe\rex-{sid}")).await
    }

    pub(super) async fn pipe(endpoint: &Endpoint, name: &str) -> Result<Box<dyn Io>, RexError> {
        let mut tries = 0;
        loop {
            // The pipe is opened so that the server can tell who is calling and no more
            // than that, which is what a newly made `ClientOptions` asks for. A server
            // that cannot tell refuses every request on the connection.
            match ClientOptions::new().open(name) {
                Ok(client) => return Ok(Box::new(client)),
                Err(error)
                    if error.raw_os_error() == Some(ERROR_PIPE_BUSY) && tries < BUSY_TRIES =>
                {
                    tries += 1;
                    tokio::time::sleep(BUSY_WAIT).await;
                }
                Err(error) => return Err(unreachable(endpoint, error)),
            }
        }
    }

    pub(super) async fn unix(endpoint: &Endpoint, _path: &Path) -> Result<Box<dyn Io>, RexError> {
        Err(unreachable(
            endpoint,
            io::Error::new(
                io::ErrorKind::Unsupported,
                "a Unix socket is not reached from Windows",
            ),
        ))
    }
}

#[cfg(unix)]
mod platform {
    use std::io;
    use std::os::unix::fs::FileTypeExt;
    use std::path::{Path, PathBuf};

    use tokio::net::UnixStream;

    use super::{Io, unreachable};
    use crate::endpoint::Endpoint;
    use crate::error::RexError;

    /// The directories a user's own server keeps its socket in, the likeliest first.
    fn runtime_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
            dirs.push(PathBuf::from(dir).join("rex"));
        }
        if let Some(home) = std::env::home_dir() {
            dirs.push(home.join(".local/state/rex/run"));
        }
        dirs
    }

    /// The sockets in a directory, in the order of their names.
    fn sockets_in(dir: &Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut found: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_socket()))
            .map(|entry| entry.path())
            .collect();
        found.sort();
        found
    }

    /// The socket of the user's own server. A server does not say what its socket is
    /// called, only which directory it is in, so each socket there is tried in turn.
    pub(super) async fn local(endpoint: &Endpoint) -> Result<Box<dyn Io>, RexError> {
        let mut last = io::Error::new(
            io::ErrorKind::NotFound,
            "no socket of a Rex server was found in the user's runtime directory",
        );
        for socket in runtime_dirs().iter().flat_map(|dir| sockets_in(dir)) {
            match UnixStream::connect(&socket).await {
                Ok(stream) => return Ok(Box::new(stream)),
                Err(error) => last = error,
            }
        }
        Err(unreachable(endpoint, last))
    }

    pub(super) async fn unix(endpoint: &Endpoint, path: &Path) -> Result<Box<dyn Io>, RexError> {
        match UnixStream::connect(path).await {
            Ok(stream) => Ok(Box::new(stream)),
            Err(error) => Err(unreachable(endpoint, error)),
        }
    }

    pub(super) async fn pipe(endpoint: &Endpoint, _name: &str) -> Result<Box<dyn Io>, RexError> {
        Err(unreachable(
            endpoint,
            io::Error::new(
                io::ErrorKind::Unsupported,
                "a named pipe is only reached from Windows",
            ),
        ))
    }
}
