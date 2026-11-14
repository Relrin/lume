//! The endpoint on Unix: a socket in a directory only the current user can enter.
//!
//! The directory is `$XDG_RUNTIME_DIR/lume` on Linux, which the system creates per user
//! and removes at logout, and `<temporary directory>/lume-<uid>` elsewhere. It must be a
//! real directory owned by the user with no access for anyone else; a symbolic link or
//! a directory someone else made is refused. The socket itself is readable and writable
//! by its owner only, and both ends check that the other belongs to the same user.

use std::fmt;
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use tokio::net::{UnixListener, UnixStream};

pub(crate) type Address = PathBuf;
pub(crate) type Stream = UnixStream;

pub(crate) fn address(name: &str) -> io::Result<Address> {
    Ok(runtime_dir().join(format!("{name}.sock")))
}

pub(crate) fn display(address: &Address, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "{}", address.display())
}

fn user_id() -> u32 {
    // SAFETY: takes no arguments and cannot fail.
    unsafe { libc::geteuid() }
}

fn runtime_dir() -> PathBuf {
    if cfg!(target_os = "linux")
        && let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)
        && dir.is_absolute()
    {
        return dir.join("lume");
    }
    std::env::temp_dir().join(format!("lume-{}", user_id()))
}

fn refused(what: String) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, what)
}

/// Creates the endpoint directory if needed and checks nobody else can use it.
fn prepare_dir(dir: &Path) -> io::Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    // Not following links: a link here could point the socket anywhere.
    let meta = fs::symlink_metadata(dir)?;
    if !meta.file_type().is_dir() {
        return Err(refused(format!("{} is not a directory", dir.display())));
    }
    if meta.uid() != user_id() {
        return Err(refused(format!(
            "{} belongs to another user",
            dir.display()
        )));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(refused(format!(
            "{} is accessible to other users",
            dir.display()
        )));
    }
    Ok(())
}

/// Removes a socket a previous server left behind. A socket that still has a server, or
/// anything that is not ours to remove, is an error.
fn clear_stale_socket(path: &Path) -> io::Result<()> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if meta.uid() != user_id() {
        return Err(refused(format!(
            "{} belongs to another user",
            path.display()
        )));
    }
    if !meta.file_type().is_socket() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} exists and is not a socket", path.display()),
        ));
    }
    if std::os::unix::net::UnixStream::connect(path).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("the endpoint {} is already being served", path.display()),
        ));
    }
    fs::remove_file(path)
}

/// Whether the other end of `stream` runs as the current user.
fn same_user(stream: &UnixStream) -> io::Result<bool> {
    Ok(stream.peer_cred()?.uid() == user_id())
}

pub(crate) struct Listener {
    listener: UnixListener,
    path: PathBuf,
}

impl Listener {
    pub(crate) fn bind(path: &Address) -> io::Result<Self> {
        let dir = path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "the socket path has no directory",
            )
        })?;
        prepare_dir(dir)?;
        clear_stale_socket(path)?;
        let listener = UnixListener::bind(path)?;
        // The directory already keeps other users out; this makes the socket itself say
        // the same, whatever the process's file-creation mask was.
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            listener,
            path: path.clone(),
        })
    }

    pub(crate) async fn accept(&mut self) -> io::Result<Stream> {
        loop {
            let (stream, _) = self.listener.accept().await?;
            if same_user(&stream)? {
                return Ok(stream);
            }
            tracing::warn!("dropped a connection from another user");
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub(crate) async fn connect(path: &Address) -> io::Result<Stream> {
    let meta = fs::symlink_metadata(path)?;
    if meta.uid() != user_id() || !meta.file_type().is_socket() {
        return Err(refused(format!(
            "{} is not a socket of this user",
            path.display()
        )));
    }
    let stream = UnixStream::connect(path).await?;
    if !same_user(&stream)? {
        return Err(refused(format!(
            "the server at {} belongs to another user",
            path.display()
        )));
    }
    Ok(stream)
}
