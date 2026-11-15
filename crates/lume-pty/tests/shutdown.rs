//! Taking a terminal down never hangs, whatever the program in it is doing.

mod common;

use std::io::Write;
use std::time::{Duration, Instant};

use common::{Output, Running, STEP_TIMEOUT, wait_with_timeout};
use lume_pty::{Pty, PtySize};

/// Generous for a loaded machine, and still far below a hang.
const PROMPT: Duration = Duration::from_secs(10);

/// Lets go of a terminal whose reader has been stopped. With nobody reading, a program
/// that has printed something is not let go, on some systems, for as long as anything
/// still holds its terminal; whoever stops reading must let go of it too.
fn release(pty: Box<dyn Pty>, writer: Box<dyn Write + Send>, output: Output) {
    drop(output);
    drop(writer);
    drop(pty);
}

#[test]
fn kill_ends_a_long_running_child() {
    let session = Running::start(&common::quick_shell(), PtySize::cells(80, 24));
    assert!(session.output.wait_for_any(), "the shell printed nothing");

    let started = Instant::now();
    session.child.killer().kill().unwrap();
    assert!(
        wait_with_timeout(session.child, PROMPT).is_some(),
        "the child outlived a kill"
    );
    assert!(
        session.output.wait_for_end(PROMPT),
        "no end-of-file after a kill"
    );
    println!("kill to end-of-file: {:?}", started.elapsed());
}

#[test]
fn kill_ends_a_child_that_floods_while_its_output_is_read() {
    let session = Running::start(&common::flood(), PtySize::cells(80, 24));
    assert!(
        session.output.wait_for("0123456789"),
        "the flood never started"
    );

    session.child.killer().kill().unwrap();
    assert!(
        wait_with_timeout(session.child, PROMPT).is_some(),
        "the child outlived a kill"
    );
    assert!(
        session.output.wait_for_end(PROMPT),
        "no end-of-file after a kill"
    );
}

#[test]
fn dropping_a_flooding_pty_without_reading_returns_promptly() {
    let (pty, child) = lume_pty::native()
        .spawn(&common::flood(), PtySize::cells(80, 24))
        .unwrap();
    // Let the program fill the terminal's buffers and block on a full pipe.
    std::thread::sleep(Duration::from_millis(500));

    let started = Instant::now();
    drop(pty);
    let dropped_in = started.elapsed();
    assert!(
        dropped_in < Duration::from_secs(2),
        "dropping the terminal took {dropped_in:?}"
    );

    // With its terminal gone the program ends without being killed.
    assert!(
        wait_with_timeout(child, PROMPT).is_some(),
        "the child outlived its terminal"
    );
}

#[test]
fn reader_stop_unblocks_a_blocked_read() {
    let (mut pty, child) = lume_pty::native()
        .spawn(&common::quick_shell(), PtySize::cells(80, 24))
        .unwrap();
    let writer = pty.take_writer().unwrap();
    let output = Output::collect(pty.take_reader().unwrap());
    assert!(output.wait_for_any(), "the shell printed nothing");
    // The shell now sits at its prompt, so the reader is blocked with nothing to read.
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !output.wait_for_end(Duration::from_millis(100)),
        "the reader ended by itself"
    );

    let started = Instant::now();
    output.stop();
    assert!(
        output.wait_for_end(Duration::from_secs(2)),
        "stop did not end the read"
    );
    println!("stop to end of read: {:?}", started.elapsed());

    child.killer().kill().unwrap();
    release(pty, writer, output);
    assert!(wait_with_timeout(child, PROMPT).is_some());
}

/// A program whose terminal is closed in its first milliseconds can miss the close
/// altogether. It must still end soon, not after the wait a settled program is given
/// to save its state, and it must not be left running.
#[test]
fn a_terminal_closed_right_after_starting_its_program_ends_it_soon() {
    const SOON: Duration = Duration::from_millis(1500);

    for round in 0..6 {
        let (mut pty, child) = lume_pty::native()
            .spawn(&common::quick_shell(), PtySize::cells(80, 24))
            .unwrap();
        let output = Output::collect(pty.take_reader().unwrap());
        let _writer = pty.take_writer().unwrap();

        let started = Instant::now();
        child.killer().kill().unwrap();
        assert!(
            wait_with_timeout(child, SOON).is_some(),
            "round {round}: the program was still running {:?} after the kill",
            started.elapsed()
        );
        assert!(
            output.wait_for_end(PROMPT),
            "round {round}: no end-of-file after the kill"
        );
    }

    // The same when the terminal is simply dropped, with nobody reading it.
    for round in 0..6 {
        let (pty, child) = lume_pty::native()
            .spawn(&common::quick_shell(), PtySize::cells(80, 24))
            .unwrap();
        drop(pty);
        assert!(
            wait_with_timeout(child, SOON).is_some(),
            "round {round}: the program outlived its dropped terminal"
        );
    }
}

#[test]
fn stop_before_the_first_read_ends_it_at_once() {
    let (mut pty, child) = lume_pty::native()
        .spawn(&common::quick_shell(), PtySize::cells(80, 24))
        .unwrap();
    let writer = pty.take_writer().unwrap();
    let reader = pty.take_reader().unwrap();
    reader.stopper().stop();
    let output = Output::collect(reader);
    assert!(
        output.wait_for_end(Duration::from_secs(2)),
        "a stopped reader still blocked"
    );
    assert_eq!(output.len(), 0);

    child.killer().kill().unwrap();
    release(pty, writer, output);
    assert!(wait_with_timeout(child, PROMPT).is_some());
}

/// On Windows the terminal ends with the program that was started in it, even when a
/// program it left behind still holds the console. Elsewhere a terminal stays readable
/// while any program holds it, and the caller stops the reader instead.
#[test]
fn a_program_left_behind_does_not_keep_the_terminal_open() {
    #[cfg(windows)]
    let command = lume_pty::CommandSpec::new("cmd.exe").args([
        "/d",
        "/c",
        "start /b ping -n 30 127.0.0.1 >nul & exit 5",
    ]);
    #[cfg(unix)]
    let command = lume_pty::CommandSpec::new("/bin/sh").args(["-c", "sleep 30 & exit 5"]);

    let session = Running::start(&command, PtySize::cells(80, 24));
    let started = Instant::now();
    assert_eq!(
        wait_with_timeout(session.child, STEP_TIMEOUT),
        Some(lume_pty::ExitStatus::Code(5))
    );
    if cfg!(windows) {
        assert!(
            session.output.wait_for_end(PROMPT),
            "the left-behind program kept the terminal open"
        );
    } else {
        session.output.stop();
        assert!(
            session.output.wait_for_end(Duration::from_secs(2)),
            "stop did not end the read"
        );
    }
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "waited for the left-behind program"
    );
}
