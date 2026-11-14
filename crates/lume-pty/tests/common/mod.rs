//! Helpers for driving a real shell in a terminal from a test.

// Each test file compiles this module separately and uses a different part of it.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use lume_pty::{Child, CommandSpec, ExitStatus, Pty, PtyRead, PtySize, ReadStop};

/// How long any single step of a test may take before the test fails.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(30);

/// Everything a terminal has printed so far, collected by a thread.
pub struct Output {
    shared: Arc<(Mutex<Collected>, Condvar)>,
    stopper: Box<dyn ReadStop>,
}

#[derive(Default)]
struct Collected {
    bytes: Vec<u8>,
    ended: bool,
}

impl Output {
    /// Starts collecting from `reader` until it reports end-of-file or fails.
    pub fn collect(mut reader: Box<dyn PtyRead>) -> Self {
        let shared = Arc::new((Mutex::new(Collected::default()), Condvar::new()));
        let stopper = reader.stopper();
        let sink = Arc::clone(&shared);
        thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            loop {
                let read = reader.read(&mut buf);
                let (collected, changed) = &*sink;
                let mut collected = collected.lock().unwrap();
                match read {
                    Ok(n) if n > 0 => collected.bytes.extend_from_slice(&buf[..n]),
                    _ => {
                        collected.ended = true;
                        changed.notify_all();
                        return;
                    }
                }
                changed.notify_all();
            }
        });
        Self { shared, stopper }
    }

    /// Blocks until `done` holds for the output so far, or the timeout passes.
    fn wait(&self, timeout: Duration, done: impl Fn(&Collected) -> bool) -> bool {
        let (collected, changed) = &*self.shared;
        let deadline = Instant::now() + timeout;
        let mut collected = collected.lock().unwrap();
        while !done(&collected) {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            collected = changed.wait_timeout(collected, left).unwrap().0;
        }
        true
    }

    /// Waits for `needle` to appear in what was printed after byte `from`.
    pub fn wait_for_after(&self, from: usize, needle: &str) -> bool {
        self.wait(STEP_TIMEOUT, |collected| {
            let tail = collected.bytes.get(from..).unwrap_or_default();
            String::from_utf8_lossy(tail).contains(needle)
        })
    }

    pub fn wait_for(&self, needle: &str) -> bool {
        self.wait_for_after(0, needle)
    }

    /// Waits for end-of-file.
    pub fn wait_for_end(&self, timeout: Duration) -> bool {
        self.wait(timeout, |collected| collected.ended)
    }

    /// Waits for at least one byte of output.
    pub fn wait_for_any(&self) -> bool {
        self.wait(STEP_TIMEOUT, |collected| !collected.bytes.is_empty())
    }

    pub fn len(&self) -> usize {
        self.shared.0.lock().unwrap().bytes.len()
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.shared.0.lock().unwrap().bytes).into_owned()
    }

    pub fn stop(&self) {
        self.stopper.stop();
    }
}

/// A program running in a terminal, with its output being collected.
pub struct Running {
    pub pty: Box<dyn Pty>,
    pub child: Box<dyn Child>,
    pub writer: Box<dyn Write + Send>,
    pub output: Output,
}

impl Running {
    pub fn start(command: &CommandSpec, size: PtySize) -> Self {
        let (mut pty, child) = lume_pty::native()
            .spawn(command, size)
            .expect("the program starts");
        let output = Output::collect(pty.take_reader().unwrap());
        let writer = pty.take_writer().unwrap();
        Self {
            pty,
            child,
            writer,
            output,
        }
    }

    /// Types `line` and presses Enter.
    pub fn type_line(&mut self, line: &str) {
        self.writer.write_all(line.as_bytes()).unwrap();
        self.writer.write_all(b"\r").unwrap();
        self.writer.flush().unwrap();
    }
}

/// Waits for a child on another thread, so a child that never ends fails the test
/// instead of hanging it.
pub fn wait_with_timeout(mut child: Box<dyn Child>, timeout: Duration) -> Option<ExitStatus> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(child.wait());
    });
    match receiver.recv_timeout(timeout) {
        Ok(status) => Some(status.expect("waiting for the child works")),
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => None,
    }
}

/// A shell this machine has, and how to ask it for things.
pub struct Shell {
    pub name: &'static str,
    pub command: CommandSpec,
    /// Prints `lume-<value of LUME_MARK>-done`. The marker comes from the environment so
    /// that it appears in the output only when the command ran, not when it was echoed.
    pub print_marker: &'static str,
    /// Prints the terminal size in a form that contains `<cols>` and `<rows>`.
    pub print_size: &'static str,
    /// Prints the working directory.
    pub print_cwd: &'static str,
    /// Prints `term=[<value of TERM>]`.
    pub print_term: &'static str,
}

/// The shells to test with: the command prompt, and each PowerShell that is installed.
#[cfg(windows)]
pub fn shells() -> Vec<Shell> {
    let on_path = |name: &str| {
        std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
    };
    let command_prompt = Shell {
        name: "cmd",
        command: CommandSpec::new("cmd.exe").arg("/d"),
        print_marker: "echo lume-%LUME_MARK%-done",
        print_size: "mode con",
        print_cwd: "cd",
        print_term: "echo term=[%TERM%]",
    };
    let powershells = ["powershell.exe", "pwsh.exe"].into_iter().filter(|name| on_path(name)).map(
        |name| Shell {
            name,
            command: CommandSpec::new(name).args(["-NoLogo", "-NoProfile"]),
            print_marker: "Write-Output \"lume-$($env:LUME_MARK)-done\"",
            print_size: "Write-Output \"size=$($Host.UI.RawUI.WindowSize.Width)x$($Host.UI.RawUI.WindowSize.Height)\"",
            print_cwd: "(Get-Location).Path",
            print_term: "Write-Output \"term=[$($env:TERM)]\"",
        },
    );
    std::iter::once(command_prompt).chain(powershells).collect()
}

/// The shells to test with: the one every Unix system has.
#[cfg(unix)]
pub fn shells() -> Vec<Shell> {
    vec![Shell {
        name: "sh",
        command: CommandSpec::new("/bin/sh"),
        print_marker: "echo \"lume-${LUME_MARK}-done\"",
        print_size: "stty size",
        print_cwd: "pwd",
        print_term: "echo \"term=[${TERM}]\"",
    }]
}

/// A command that prints lines for ever.
pub fn flood() -> CommandSpec {
    #[cfg(windows)]
    {
        CommandSpec::new("cmd.exe").args([
            "/d",
            "/c",
            "for /l %i in () do @echo 0123456789abcdefghijklmnopqrstuvwxyz",
        ])
    }
    #[cfg(unix)]
    {
        CommandSpec::new("/bin/sh").args(["-c", "yes 0123456789abcdefghijklmnopqrstuvwxyz"])
    }
}

/// A command that prints one line and exits with status 3.
pub fn print_and_exit_3() -> CommandSpec {
    #[cfg(windows)]
    {
        CommandSpec::new("cmd.exe").args(["/d", "/c", "echo short-lived & exit 3"])
    }
    #[cfg(unix)]
    {
        CommandSpec::new("/bin/sh").args(["-c", "echo short-lived; exit 3"])
    }
}

/// An interactive shell that starts quickly.
pub fn quick_shell() -> CommandSpec {
    #[cfg(windows)]
    {
        CommandSpec::new("cmd.exe").arg("/d")
    }
    #[cfg(unix)]
    {
        CommandSpec::new("/bin/sh")
    }
}
