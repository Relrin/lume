//! The program that has a terminal is found: the shell, then what the shell runs, then
//! the shell again.

mod common;

use std::io::Write;
use std::thread;
use std::time::{Duration, Instant};

use common::{Running, STEP_TIMEOUT, wait_with_timeout};
use lume_pty::{ForegroundProbe, ForegroundProcess, PtySize};

/// How often a test asks again while it waits for the answer to change.
const ASK_INTERVAL: Duration = Duration::from_millis(25);

/// How many times the cost of one question is measured.
const TIMED_QUESTIONS: usize = 20;

/// A program that stays in the foreground until it is given its input, the name it is
/// found under, and the input that ends it.
///
/// It is ended by input and not by an interrupt on purpose. Whether a program can be
/// interrupted from its terminal is decided in part by whoever started the process
/// that opened the terminal: one that had interrupts turned off passes that on, and a
/// test runner may be started that way.
#[cfg(windows)]
const STAYS: (&str, &str, &[u8]) = ("choice /c y /n >nul", "choice", b"y");
#[cfg(unix)]
const STAYS: (&str, &str, &[u8]) = ("cat >/dev/null", "cat", END_OF_INPUT);

/// The character a Unix terminal takes as the end of input at the start of a line.
#[cfg(unix)]
const END_OF_INPUT: &[u8] = b"\x04";

/// Asks until the answer satisfies `wanted`, and returns that answer.
fn until(
    probe: &dyn ForegroundProbe,
    what: &str,
    wanted: impl Fn(&ForegroundProcess) -> bool,
) -> ForegroundProcess {
    let deadline = Instant::now() + STEP_TIMEOUT;
    let mut last = None;
    while Instant::now() < deadline {
        last = probe.foreground();
        if let Some(found) = last.as_ref().filter(|found| wanted(found)) {
            return found.clone();
        }
        thread::sleep(ASK_INTERVAL);
    }
    panic!("{what}: the last answer was {last:?}");
}

#[test]
fn the_foreground_is_the_shell_then_its_program_then_the_shell_again() {
    let mut session = Running::start(&common::quick_shell(), PtySize::cells(80, 24));
    assert!(session.output.wait_for_any(), "the shell printed nothing");
    let shell_pid = session.child.pid();
    let probe = session
        .pty
        .foreground_probe()
        .expect("this platform can say which program has a terminal");

    let shell = until(&*probe, "the shell is not in the foreground", |found| {
        found.pid == shell_pid
    });
    assert!(!shell.name.is_empty());
    #[cfg(windows)]
    assert!(shell.name.eq_ignore_ascii_case("cmd"), "{shell:?}");

    let (command, name, ending) = STAYS;
    session.type_line(command);
    let program = until(
        &*probe,
        "the program never came to the foreground",
        |found| found.name.eq_ignore_ascii_case(name),
    );
    assert_ne!(program.pid, shell_pid);

    let mut costs: Vec<Duration> = (0..TIMED_QUESTIONS)
        .map(|_| {
            let asked = Instant::now();
            let _ = probe.foreground();
            asked.elapsed()
        })
        .collect();
    costs.sort();
    println!(
        "one question: fastest {:?}, median {:?}, slowest {:?}",
        costs[0],
        costs[costs.len() / 2],
        costs[costs.len() - 1]
    );

    session.writer.write_all(ending).unwrap();
    session.writer.flush().unwrap();
    let back = until(&*probe, "the shell did not come back", |found| {
        found.pid == shell_pid
    });
    assert_eq!(back.name, shell.name);

    session.type_line("exit");
    assert!(
        wait_with_timeout(session.child, STEP_TIMEOUT).is_some(),
        "the shell did not end"
    );
    // The probe outlives the program it asked about, and then has nothing to say.
    assert_eq!(probe.foreground(), None);
    assert!(
        session.output.wait_for_end(STEP_TIMEOUT),
        "no end-of-file after exit"
    );
}

#[test]
fn a_probe_kept_after_its_terminal_is_gone_says_nothing() {
    let session = Running::start(&common::quick_shell(), PtySize::cells(80, 24));
    assert!(session.output.wait_for_any(), "the shell printed nothing");
    let probe = session.pty.foreground_probe().unwrap();
    assert!(probe.foreground().is_some());

    session.child.killer().kill().unwrap();
    assert!(
        wait_with_timeout(session.child, STEP_TIMEOUT).is_some(),
        "the child outlived a kill"
    );
    assert!(session.output.wait_for_end(STEP_TIMEOUT));
    drop(session.writer);
    drop(session.pty);
    assert_eq!(probe.foreground(), None);
}
