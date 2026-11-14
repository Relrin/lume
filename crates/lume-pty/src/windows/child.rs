//! The process started in a pseudoconsole: waiting for it and ending it.

use std::io;
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, INFINITE, TerminateProcess, WaitForSingleObject,
};

use super::check;
use super::conpty::Console;
use crate::{Child, ChildKiller, ExitStatus};

/// How long a child has to end by itself after its console is closed. Programs use this
/// time to save their state; one that is still running afterwards is ended by force.
const KILL_WAIT: Duration = Duration::from_secs(3);

/// A program started this recently is taken to be still starting.
const STARTING_WINDOW: Duration = Duration::from_secs(1);

/// How long a program that is still starting has to end after its console is closed.
///
/// A program whose console is closed in its first few milliseconds never learns of it:
/// the close is announced before the program is able to receive the announcement, and
/// it then waits for input on a console that no longer has anyone behind it. Left to
/// the full wait, closing a terminal straight after opening it would take seconds. A
/// program that young has had no time to make anything worth saving, so it is given
/// only long enough to show that it did notice.
const STARTING_KILL_WAIT: Duration = Duration::from_millis(250);

/// The exit code given to a child that had to be ended by force.
const FORCED_EXIT_CODE: u32 = 1;

/// The means of ending a child: its console, to ask, and its process, to insist.
/// Shared by the terminal, which ends the child when it is dropped, and by the child's
/// killers.
#[derive(Clone)]
pub(crate) struct ChildEnd {
    process: Arc<OwnedHandle>,
    console: Arc<Console>,
    started: Instant,
}

impl ChildEnd {
    pub(crate) fn new(process: OwnedHandle, console: Arc<Console>) -> Self {
        Self {
            process: Arc::new(process),
            console,
            started: Instant::now(),
        }
    }

    fn has_exited(&self) -> bool {
        // SAFETY: the handle is open for as long as `self.process` is alive. A zero
        // timeout only asks whether the process has ended.
        unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) == WAIT_OBJECT_0 }
    }

    /// Closes the console, which asks every program attached to it to exit, and ends
    /// the child by force if it is still running after its wait. Returns at once.
    pub(crate) fn end(&self) -> io::Result<()> {
        self.console.close();
        if self.has_exited() {
            return Ok(());
        }

        let wait = if self.started.elapsed() < STARTING_WINDOW {
            STARTING_KILL_WAIT
        } else {
            KILL_WAIT
        };
        let process = Arc::clone(&self.process);
        let watchdog = move || {
            let millis = wait.as_millis() as u32;
            // SAFETY: the handle is open for as long as `process` is alive.
            if unsafe { WaitForSingleObject(process.as_raw_handle(), millis) } == WAIT_TIMEOUT {
                // SAFETY: as above. Failure means the process ended in the meantime.
                unsafe { TerminateProcess(process.as_raw_handle(), FORCED_EXIT_CODE) };
            }
        };
        std::thread::Builder::new()
            .name("pty-kill".to_owned())
            .spawn(watchdog)?;
        Ok(())
    }
}

pub(crate) struct WinChild {
    end: ChildEnd,
    pid: u32,
    /// What was started, for log messages.
    program: PathBuf,
}

impl WinChild {
    pub(crate) fn new(end: ChildEnd, pid: u32, program: PathBuf) -> Self {
        Self { end, pid, program }
    }
}

impl Child for WinChild {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        let process = self.end.process.as_raw_handle();
        // SAFETY: the handle is open for as long as `self.end` is alive.
        let waited = unsafe { WaitForSingleObject(process, INFINITE) };
        if waited != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        let mut code = 0;
        // SAFETY: the handle is open and `code` is a valid out pointer.
        check(unsafe { GetExitCodeProcess(process, &mut code) })?;

        // The program that was started has ended, so the terminal is over even if
        // programs it started are still attached. Closing the console lets the reader
        // reach end-of-file; without it the console host would wait for those programs.
        self.end.console.close();
        tracing::debug!(pid = self.pid, program = %self.program.display(), code, "child exited");
        Ok(ExitStatus::Code(code))
    }

    fn killer(&self) -> Box<dyn ChildKiller> {
        Box::new(self.end.clone())
    }
}

impl ChildKiller for ChildEnd {
    fn kill(&self) -> io::Result<()> {
        self.end()
    }
}
