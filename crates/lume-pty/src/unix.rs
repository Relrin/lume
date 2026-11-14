//! Terminals on Unix: PTYs.
//!
//! Opening the PTY and starting the child in it (a new session with the PTY as its
//! controlling terminal) is `portable-pty`'s. Reading and writing are done here on
//! duplicates of the master descriptor, for two reasons:
//!
//! - A read must be interruptible from another thread, so it waits with a timeout
//!   instead of blocking for ever.
//! - Closing the input must not type anything. `portable-pty`'s writer sends a newline
//!   and an end-of-file character when it is dropped, which would run whatever the user
//!   had typed and not yet submitted.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, MasterPty, native_pty_system};

use crate::{
    Child, ChildKiller, CommandSpec, ExitStatus, IDENTITY_ENV, Pty, PtyRead, PtySize, PtySystem,
    ReadStop,
};

/// The terminal type programs are told. The terminal behaves as xterm does in every way
/// that entry describes, and the entry is installed everywhere.
const TERM: &str = "xterm-256color";

/// How often a blocked read looks for a request to stop.
const READ_POLL_MS: libc::c_int = 100;

/// How long a child has to end by itself after being told its terminal has gone.
/// Programs use this time to save their state; one that is still running afterwards is
/// ended by force.
const KILL_WAIT: Duration = Duration::from_secs(3);

pub(crate) struct UnixPtySystem;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.into())
}

fn other(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("{error:#}"))
}

fn portable_size(size: PtySize) -> portable_pty::PtySize {
    portable_pty::PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: size.px_width,
        pixel_height: size.px_height,
    }
}

impl PtySystem for UnixPtySystem {
    fn spawn(
        &self,
        command: &CommandSpec,
        size: PtySize,
    ) -> io::Result<(Box<dyn Pty>, Box<dyn Child>)> {
        // Everything that can fail on the caller's input is checked before anything is
        // created, so these failures have a precise error kind.
        if size.cols == 0 || size.rows == 0 {
            return Err(invalid(format!(
                "a terminal cannot be {}x{} cells",
                size.cols, size.rows
            )));
        }
        if command.program.is_empty() {
            return Err(invalid("the program name is empty"));
        }
        let cwd = match &command.cwd {
            Some(dir) if dir.is_dir() => dir.clone(),
            Some(dir) => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("the working directory `{}` does not exist", dir.display()),
                ));
            }
            None => std::env::current_dir()?,
        };
        let path = command
            .env
            .iter()
            .rev()
            .find(|(name, _)| name == "PATH")
            .map(|(_, value)| value.clone())
            .or_else(|| std::env::var_os("PATH"));
        if !program_exists(&command.program, &cwd, path.as_deref()) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "the program `{}` was not found",
                    Path::new(&command.program).display()
                ),
            ));
        }

        let mut builder = CommandBuilder::new(&command.program);
        if command.login {
            // Every common shell takes this to mean "behave as a login shell".
            builder.arg("-l");
        }
        builder.args(&command.args);
        builder.cwd(&cwd);
        builder.env("TERM", TERM);
        for (name, value) in IDENTITY_ENV {
            builder.env(name, value);
        }
        for name in &command.env_remove {
            builder.env_remove(name);
        }
        for (name, value) in &command.env {
            builder.env(name, value);
        }

        let pair = native_pty_system()
            .openpty(portable_size(size))
            .map_err(other)?;
        let child = pair.slave.spawn_command(builder).map_err(other)?;
        // Only the child may hold the other end, or the terminal would never report that
        // the child has closed it.
        drop(pair.slave);

        let pid = child
            .process_id()
            .and_then(|pid| libc::pid_t::try_from(pid).ok())
            .filter(|&pid| pid > 1)
            .ok_or_else(|| io::Error::other("the child has no process id"))?;
        let master = MasterPty::as_raw_fd(&*pair.master)
            .ok_or_else(|| io::Error::other("the terminal has no descriptor"))?;
        let exited = Arc::new(AtomicBool::new(false));

        let pty = UnixPty {
            master: pair.master,
            reader: Some(duplicate(master)?),
            writer: Some(duplicate(master)?),
            pid,
            exited: Arc::clone(&exited),
        };
        Ok((Box::new(pty), Box::new(UnixChild { child, pid, exited })))
    }

    fn resize_pulls_scrollback(&self) -> bool {
        true
    }
}

/// Whether `program` names an executable file: at its path relative to `cwd` if it has a
/// directory part, otherwise in one of the directories of `path`.
fn program_exists(program: &OsStr, cwd: &Path, path: Option<&OsStr>) -> bool {
    let executable = |candidate: &Path| {
        std::fs::metadata(candidate)
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    };
    let given = Path::new(program);
    if given.components().count() > 1 {
        return executable(&cwd.join(given));
    }
    path.is_some_and(|path| {
        std::env::split_paths(path).any(|dir| executable(&cwd.join(dir).join(given)))
    })
}

/// A second descriptor for the same terminal, closed on exec so children never get it.
fn duplicate(fd: RawFd) -> io::Result<OwnedFd> {
    // SAFETY: `fd` is an open descriptor for the duration of the call.
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if duplicate == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the call succeeded, so this is an open descriptor that nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(duplicate) })
}

/// Sends `signal` to every process in the group the child leads. The child was started
/// as the leader of a new session, so its process id is also its group's id, and the
/// programs it started without job control are in that group with it.
fn signal_group(pid: libc::pid_t, signal: libc::c_int) -> io::Result<()> {
    // SAFETY: takes no pointers. `pid` is greater than 1, so the negated value names
    // one process group and never "every process".
    if unsafe { libc::kill(-pid, signal) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    // No such group: everything in it has already ended.
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

struct UnixPty {
    master: Box<dyn MasterPty + Send>,
    reader: Option<OwnedFd>,
    writer: Option<OwnedFd>,
    pid: libc::pid_t,
    exited: Arc<AtomicBool>,
}

fn taken(what: &str) -> io::Error {
    io::Error::other(format!("the terminal's {what} was already taken"))
}

impl Pty for UnixPty {
    fn take_reader(&mut self) -> io::Result<Box<dyn PtyRead>> {
        let fd = self.reader.take().ok_or_else(|| taken("reader"))?;
        Ok(Box::new(UnixReader {
            shared: Arc::new(ReadShared {
                fd,
                stopped: AtomicBool::new(false),
            }),
        }))
    }

    fn take_writer(&mut self) -> io::Result<Box<dyn Write + Send>> {
        let fd = self.writer.take().ok_or_else(|| taken("writer"))?;
        Ok(Box::new(File::from(fd)))
    }

    fn resize(&self, size: PtySize) -> io::Result<()> {
        if size.cols == 0 || size.rows == 0 {
            return Err(invalid(format!(
                "a terminal cannot be {}x{} cells",
                size.cols, size.rows
            )));
        }
        self.master.resize(portable_size(size)).map_err(other)
    }
}

impl Drop for UnixPty {
    fn drop(&mut self) {
        // The kernel hangs up a terminal's programs when the last descriptor for the
        // master closes. A reader or writer taken from this terminal may still hold one,
        // so the hang-up is sent here instead of being left to that.
        //
        // Not after the child has been waited for: its process id may by then belong to
        // an unrelated process.
        if !self.exited.load(Ordering::SeqCst) {
            let _ = signal_group(self.pid, libc::SIGHUP);
        }
    }
}

struct ReadShared {
    fd: OwnedFd,
    stopped: AtomicBool,
}

struct UnixReader {
    shared: Arc<ReadShared>,
}

impl Read for UnixReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let fd = self.shared.fd.as_raw_fd();
        loop {
            if self.shared.stopped.load(Ordering::SeqCst) {
                return Ok(0);
            }
            let mut poll = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: `poll` is one valid entry, and the descriptor in it is open for as
            // long as `self.shared` is alive.
            let ready = unsafe { libc::poll(&mut poll, 1, READ_POLL_MS) };
            if ready == 0 {
                continue;
            }
            if ready < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }

            // SAFETY: the descriptor is open and `buf` is valid for `buf.len()` bytes.
            let read = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
            if read >= 0 {
                return Ok(read as usize);
            }
            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                // Linux reports a terminal that every program has closed as an I/O
                // error rather than as end-of-file.
                Some(libc::EIO) => return Ok(0),
                Some(libc::EINTR | libc::EAGAIN) => {}
                _ => return Err(error),
            }
        }
    }
}

impl PtyRead for UnixReader {
    fn stopper(&self) -> Box<dyn ReadStop> {
        Box::new(UnixReadStop {
            shared: Arc::clone(&self.shared),
        })
    }
}

struct UnixReadStop {
    shared: Arc<ReadShared>,
}

impl ReadStop for UnixReadStop {
    fn stop(&self) {
        // The reader sees this the next time its wait times out.
        self.shared.stopped.store(true, Ordering::SeqCst);
    }
}

struct UnixChild {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    pid: libc::pid_t,
    /// Set once the child has been waited for. From then on its process id is free for
    /// the system to give to another process, so nothing may signal it.
    exited: Arc<AtomicBool>,
}

impl Child for UnixChild {
    fn pid(&self) -> u32 {
        self.pid as u32
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait()?;
        self.exited.store(true, Ordering::SeqCst);
        Ok(match status.signal() {
            Some(name) => signal_number(name).map_or(ExitStatus::Unknown, ExitStatus::Signal),
            None => ExitStatus::Code(status.exit_code()),
        })
    }

    fn killer(&self) -> Box<dyn ChildKiller> {
        Box::new(UnixKiller {
            pid: self.pid,
            exited: Arc::clone(&self.exited),
        })
    }
}

/// The number of the signal `portable-pty` reports by description. It keeps only the
/// text the system gives for a signal, so the number is found by asking the system for
/// the text of each signal in turn.
fn signal_number(description: &str) -> Option<u32> {
    if let Some(number) = description.strip_prefix("Signal ") {
        return number.parse().ok();
    }
    (1..=64)
        .find(|&signal| {
            // SAFETY: takes no pointers. The result is null or a NUL-terminated string that
            // stays valid until the next call on this thread.
            let text = unsafe { libc::strsignal(signal) };
            // SAFETY: checked non-null just before, and NUL-terminated as described above.
            !text.is_null()
                && unsafe { std::ffi::CStr::from_ptr(text) }.to_string_lossy() == description
        })
        .map(|signal| signal as u32)
}

struct UnixKiller {
    pid: libc::pid_t,
    exited: Arc<AtomicBool>,
}

impl ChildKiller for UnixKiller {
    fn kill(&self) -> io::Result<()> {
        if self.exited.load(Ordering::SeqCst) {
            return Ok(());
        }
        // What a program receives when its terminal goes away.
        signal_group(self.pid, libc::SIGHUP)?;

        let (pid, exited) = (self.pid, Arc::clone(&self.exited));
        let watchdog = move || {
            let deadline = Instant::now() + KILL_WAIT;
            while Instant::now() < deadline {
                if exited.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            if !exited.load(Ordering::SeqCst) {
                let _ = signal_group(pid, libc::SIGKILL);
            }
        };
        std::thread::Builder::new()
            .name("pty-kill".to_owned())
            .spawn(watchdog)?;
        Ok(())
    }
}
