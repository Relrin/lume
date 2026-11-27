//! Starting the program that serves an endpoint so that it outlives whoever started it.
//!
//! A server started on demand by a client must not end with that client: not when the
//! client exits, not when the console it was typed in closes, and not when the group of
//! processes it belongs to is ended. So the server is given nothing of the client's: no
//! console or terminal, no standard streams, no working directory to hold on to, and,
//! where the system allows it, no share in the client's job or session.

use std::io;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::{Endpoint, Stream};

/// How often the endpoint is tried while the server starts.
const START_POLL: Duration = Duration::from_millis(20);

/// Whether a failed connection means that nothing serves the endpoint.
pub fn not_served(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

/// Starts `command` detached, as [`spawn_detached`] does, and connects to `endpoint`
/// once it serves it, for `timeout` at most.
///
/// A server that finds the endpoint served already exits with `in_use`: another start
/// got there first, and its server is the one connected to.
pub async fn start_and_connect(
    endpoint: &Endpoint,
    command: &mut Command,
    in_use: i32,
    timeout: Duration,
) -> io::Result<Stream> {
    let mut child = spawn_detached(command)?;
    let deadline = Instant::now() + timeout;
    let mut lost_race = false;
    loop {
        match crate::connect(endpoint).await {
            Ok(stream) => {
                reap(child);
                return Ok(stream);
            }
            Err(error) if not_served(&error) && Instant::now() < deadline => {}
            Err(error) => return Err(error),
        }
        if !lost_race && let Some(status) = child.try_wait()? {
            if status.code() == Some(in_use) {
                lost_race = true;
            } else {
                return Err(io::Error::other(format!(
                    "the server exited as it started ({status})"
                )));
            }
        }
        tokio::time::sleep(START_POLL).await;
    }
}

/// Collects the server's status when it exits, so that it leaves nothing behind in
/// this process's table of children. The server normally outlives this process, and
/// then nothing is collected.
fn reap(mut child: Child) {
    let _ = std::thread::Builder::new()
        .name("detached-reaper".to_owned())
        .spawn(move || {
            let _ = child.wait();
        });
}

/// Starts `command` detached from this process. The caller should not wait for it,
/// except to collect its status once it has exited by itself.
///
/// On Windows it has no console, a process group of its own (so the Ctrl+C meant for
/// the starter passes it by) and leaves the starter's job when the job allows it; when
/// the job does not, it is started inside it and ends when the job does. It runs in
/// the program's own folder unless `command` names another. Elsewhere it runs in a
/// session of its own, with no controlling terminal, in `/`.
pub fn spawn_detached(command: &mut Command) -> io::Result<Child> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    keep_standard_handles();
    platform(command)
}

/// Keeps a program started from here from holding this process's own standard input,
/// output and error.
///
/// On Windows a child is handed every handle of its parent that may be inherited, and
/// not only those it is given as its own. A caller that reads what this process prints
/// through a pipe waits for the pipe's end, which does not come while a program started
/// from here holds a copy of it: a server started this way kept `lume host start | ...`
/// waiting for as long as it ran. Nothing is lost by it: a child that is to share them
/// is given copies of its own, as `Stdio::inherit` makes. Elsewhere a child's standard
/// streams are replaced, and this does nothing.
pub fn keep_standard_handles() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{
            HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation,
        };
        use windows_sys::Win32::System::Console::{
            GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        };

        for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            // SAFETY: `GetStdHandle` only reads this process's table of standard handles.
            let handle = unsafe { GetStdHandle(which) };
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                continue;
            }
            // SAFETY: `handle` is one of this process's standard handles, open for as
            // long as the process runs; only whether children inherit it is changed. A
            // failure leaves it as it was, which is no worse than before.
            unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
        }
    }
}

#[cfg(windows)]
fn platform(command: &mut Command) -> io::Result<Child> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;
    use windows_sys::Win32::System::Threading::{
        CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
    };

    // The folder it is started in stays in use while it runs, so it is the program's
    // own, which is in use anyway, unless the caller chose one.
    if command.get_current_dir().is_none()
        && let Some(own) = std::path::Path::new(command.get_program()).parent()
        && own.is_dir()
    {
        let own = own.to_owned();
        command.current_dir(own);
    }
    let detached = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    command.creation_flags(detached | CREATE_BREAKAWAY_FROM_JOB);
    match command.spawn() {
        // A job that does not allow its processes to leave refuses the start outright.
        Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
            command.creation_flags(detached);
            command.spawn()
        }
        started => started,
    }
}

#[cfg(unix)]
fn platform(command: &mut Command) -> io::Result<Child> {
    use std::os::unix::process::CommandExt;

    command.current_dir("/");
    // SAFETY: the closure runs in the child between fork and exec, and calls only
    // `setsid`, which is async-signal-safe and touches no memory of the parent's.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn()
}
