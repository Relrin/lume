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
    platform(command)
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
