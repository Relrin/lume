//! Pseudo-terminals: start a program attached to a terminal that the caller drives.
//!
//! The caller reads what the program prints, writes what the user types, and tells the
//! terminal its size. On Windows the terminal is a pseudoconsole (ConPTY); elsewhere it
//! is a Unix PTY. Both sit behind the same traits, so nothing above this crate knows
//! which one it has.
//!
//! All I/O here is blocking. Callers that must not block run it on their own threads.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::console::ensure_hidden_console;

/// Variables every child gets, so programs can tell what terminal they run in and that
/// it draws 24-bit colour.
pub(crate) const IDENTITY_ENV: [(&str, &str); 3] = [
    ("COLORTERM", "truecolor"),
    ("TERM_PROGRAM", "lume"),
    ("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION")),
];

/// The program to start and the environment to start it in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandSpec {
    /// A path, or a bare name to look up on `PATH`.
    pub program: OsString,
    pub args: Vec<OsString>,
    /// The working directory. `None` uses the caller's.
    pub cwd: Option<PathBuf>,
    /// Variables set on top of the caller's environment. These win over everything else.
    pub env: Vec<(OsString, OsString)>,
    /// Variables removed from the caller's environment.
    pub env_remove: Vec<OsString>,
    /// Start the program as a login shell. Ignored on Windows, which has no such notion.
    pub login: bool,
}

impl CommandSpec {
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            ..Self::default()
        }
    }

    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, name: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((name.into(), value.into()));
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }
}

/// A terminal size: the grid in cells, and the same area in pixels where it is known.
/// Programs that draw images ask for the pixel size; zero means it is not known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PtySize {
    pub cols: u16,
    pub rows: u16,
    pub px_width: u16,
    pub px_height: u16,
}

impl PtySize {
    /// A size in cells with no pixel size.
    pub fn cells(cols: u16, rows: u16) -> Self {
        Self {
            cols,
            rows,
            px_width: 0,
            px_height: 0,
        }
    }
}

/// How a child ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitStatus {
    Code(u32),
    Signal(u32),
    /// The child ended but the platform did not say how.
    Unknown,
}

/// A way of making terminals.
pub trait PtySystem: Send + Sync {
    /// Starts `command` in a new terminal of the given size.
    fn spawn(
        &self,
        command: &CommandSpec,
        size: PtySize,
    ) -> io::Result<(Box<dyn Pty>, Box<dyn Child>)>;

    /// Whether a client that keeps scrollback should pull lines back out of it when the
    /// terminal grows taller. A pseudoconsole keeps its own copy of the screen that knows
    /// nothing of the client's scrollback, so a client that pulled lines back would
    /// disagree with it about where everything is.
    fn resize_pulls_scrollback(&self) -> bool;
}

/// The caller's end of a terminal. Dropping it closes the terminal, which ends the
/// programs attached to it.
pub trait Pty: Send {
    /// Takes the output stream. It can be taken once.
    fn take_reader(&mut self) -> io::Result<Box<dyn PtyRead>>;

    /// Takes the input stream. It can be taken once. Dropping it tells the terminal its
    /// input has ended, so keep it for as long as the session should live.
    fn take_writer(&mut self) -> io::Result<Box<dyn Write + Send>>;

    fn resize(&self, size: PtySize) -> io::Result<()>;
}

/// The output of a terminal. `read` blocks until there are bytes, and returns `Ok(0)`
/// once the terminal has closed and everything it wrote has been read.
pub trait PtyRead: Read + Send {
    /// A handle that ends a blocked `read` from another thread.
    fn stopper(&self) -> Box<dyn ReadStop>;
}

/// Ends reading from a terminal. This exists because a terminal stays open for as long
/// as any program holds it: a background process that outlives the shell would otherwise
/// keep a reader blocked forever.
pub trait ReadStop: Send + Sync {
    /// Makes the current and every later `read` return `Ok(0)`.
    fn stop(&self);
}

/// The program started in a terminal.
pub trait Child: Send {
    fn pid(&self) -> u32;

    /// Blocks until the child has ended.
    ///
    /// Keep reading the terminal's output while waiting, or let go of the terminal. A
    /// program that has printed something nobody reads is, on some systems, not let go
    /// by the terminal until that output is read or the terminal is closed, and is not
    /// reported as ended before then.
    fn wait(&mut self) -> io::Result<ExitStatus>;

    /// A handle that can end the child while another thread is in [`wait`](Self::wait).
    fn killer(&self) -> Box<dyn ChildKiller>;
}

pub trait ChildKiller: Send + Sync {
    /// Ends the child: it is asked to close as if its terminal had gone away, and is
    /// forced to end if it has not done so shortly after. Returns at once; the exit is
    /// reported by [`Child::wait`].
    fn kill(&self) -> io::Result<()>;
}

/// The terminals of the platform this was built for.
pub fn native() -> Arc<dyn PtySystem> {
    #[cfg(windows)]
    {
        Arc::new(windows::WinPtySystem)
    }
    #[cfg(unix)]
    {
        Arc::new(unix::UnixPtySystem)
    }
}
