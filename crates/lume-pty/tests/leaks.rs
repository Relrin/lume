//! Starting and ending many terminals leaves no handles and no processes behind.
//!
//! This file holds a single test so that nothing else runs in its process: the counts
//! below are of the whole process.

mod common;

use std::io::Read;
use std::time::{Duration, Instant};

use common::{STEP_TIMEOUT, wait_with_timeout};
use lume_pty::{ExitStatus, PtySize};

const WARM_UP: usize = 3;
const RUN_TO_EXIT: usize = 100;
const KILL_MID_OUTPUT: usize = 20;

/// Starts a program that prints a line and exits, reads all of it, and waits for it.
fn run_to_exit() {
    let (mut pty, child) = lume_pty::native()
        .spawn(&common::print_and_exit_3(), PtySize::cells(80, 24))
        .unwrap();
    let mut reader = pty.take_reader().unwrap();
    let writer = pty.take_writer().unwrap();
    // The output is far smaller than the terminal's buffer, so the program can finish
    // before anything is read.
    assert_eq!(
        wait_with_timeout(child, STEP_TIMEOUT),
        Some(ExitStatus::Code(3))
    );
    let mut printed = Vec::new();
    reader.read_to_end(&mut printed).unwrap();
    assert!(String::from_utf8_lossy(&printed).contains("short-lived"));
    drop(writer);
}

/// Starts a program that prints for ever, reads some of it, and kills it.
fn kill_mid_output() {
    let (mut pty, child) = lume_pty::native()
        .spawn(&common::flood(), PtySize::cells(80, 24))
        .unwrap();
    let mut reader = pty.take_reader().unwrap();
    let _writer = pty.take_writer().unwrap();
    let mut buf = [0u8; 4096];
    assert!(reader.read(&mut buf).unwrap() > 0);

    child.killer().kill().unwrap();
    // Keep reading, or the program cannot finish writing and end.
    let drain = std::thread::spawn(move || {
        let mut rest = Vec::new();
        let _ = reader.read_to_end(&mut rest);
    });
    assert!(
        wait_with_timeout(child, STEP_TIMEOUT).is_some(),
        "the child outlived a kill"
    );
    drain.join().unwrap();
}

/// Polls `settled` until it holds. Processes and their handles go away a moment after
/// the calls that end them return.
fn eventually(what: &str, settled: impl Fn() -> Result<(), String>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match settled() {
            Ok(()) => return,
            Err(state) if Instant::now() >= deadline => panic!("{what}: {state}"),
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

#[test]
fn no_handles_or_processes_leak_after_many_terminals() {
    for _ in 0..WARM_UP {
        run_to_exit();
        kill_mid_output();
    }
    eventually(
        "children of the warm-up",
        || match platform::child_processes() {
            0 => Ok(()),
            n => Err(format!("{n} child processes remain")),
        },
    );
    let baseline = platform::open_handles();

    for _ in 0..RUN_TO_EXIT {
        run_to_exit();
    }
    for _ in 0..KILL_MID_OUTPUT {
        kill_mid_output();
    }

    eventually("child processes", || match platform::child_processes() {
        0 => Ok(()),
        n => Err(format!("{n} child processes remain")),
    });
    eventually("open handles", || {
        let now = platform::open_handles();
        if now <= baseline + platform::HANDLE_SLACK {
            Ok(())
        } else {
            Err(format!(
                "{now} handles are open; there were {baseline} before"
            ))
        }
    });
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentProcessId, GetProcessHandleCount,
    };

    /// The runtime opens a few handles of its own as threads come and go.
    pub const HANDLE_SLACK: u32 = 4;

    pub fn open_handles() -> u32 {
        let mut count = 0;
        // SAFETY: the pseudo-handle for this process is always valid, and `count` is a
        // valid out pointer.
        let ok = unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) };
        assert_ne!(ok, 0, "GetProcessHandleCount failed");
        count
    }

    /// How many live processes name this process as their parent. Each pseudoconsole
    /// starts a console host as a child of ours, besides the program itself.
    pub fn child_processes() -> usize {
        // SAFETY: takes no pointers.
        let (snapshot, me) = unsafe {
            (
                CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0),
                GetCurrentProcessId(),
            )
        };
        assert_ne!(
            snapshot, INVALID_HANDLE_VALUE,
            "CreateToolhelp32Snapshot failed"
        );

        // SAFETY: plain data for which all-zero is a valid value.
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut children = 0;
        // SAFETY: `snapshot` is open and `entry` has its size set, as the calls require.
        let mut more = unsafe { Process32FirstW(snapshot, &mut entry) };
        while more != 0 {
            if entry.th32ParentProcessID == me {
                children += 1;
            }
            // SAFETY: as above.
            more = unsafe { Process32NextW(snapshot, &mut entry) };
        }
        // SAFETY: `snapshot` is open and owned here.
        unsafe { CloseHandle(snapshot) };
        children
    }
}

#[cfg(unix)]
mod platform {
    pub const HANDLE_SLACK: u32 = 0;

    /// How many file descriptors this process has open.
    pub fn open_handles() -> u32 {
        // Reading the directory opens one descriptor itself, the same one each time.
        std::fs::read_dir("/dev/fd")
            .expect("/dev/fd lists open descriptors")
            .count() as u32
    }

    /// How many children this process has that it has not waited for.
    pub fn child_processes() -> usize {
        // With no children at all the call fails with ECHILD; with children that are
        // still running, or ended and not yet collected, it does not.
        let mut status = 0;
        // SAFETY: `status` is a valid out pointer, and WNOHANG makes the call return at
        // once. A child collected here is one this test should not have left behind, so
        // collecting it does not hide a leak: it is counted.
        let result = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if result == -1 { 0 } else { 1 }
    }
}
