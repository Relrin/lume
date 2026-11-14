//! A real shell in a terminal: it starts, echoes, follows a resize, and reports its exit.

mod common;

use std::time::Instant;

use common::{Running, STEP_TIMEOUT, shells, wait_with_timeout};
use lume_pty::{CommandSpec, ExitStatus, PtySize};

#[test]
fn interactive_shell_echoes_resizes_and_reports_exit_status() {
    for shell in shells() {
        let name = shell.name;
        let command = shell.command.clone().env("LUME_MARK", "k7q");
        let mut session = Running::start(&command, PtySize::cells(80, 24));

        session.type_line(shell.print_marker);
        assert!(
            session.output.wait_for("lume-k7q-done"),
            "{name}: the command's output never arrived; got {:?}",
            session.output.text()
        );

        session.pty.resize(PtySize::cells(97, 31)).unwrap();
        let before = session.output.len();
        session.type_line(shell.print_size);
        for dimension in ["97", "31"] {
            assert!(
                session.output.wait_for_after(before, dimension),
                "{name}: the shell does not report {dimension} after the resize; got {:?}",
                session.output.text()
            );
        }

        session.type_line("exit 7");
        let status = wait_with_timeout(session.child, STEP_TIMEOUT);
        assert_eq!(status, Some(ExitStatus::Code(7)), "{name}");
        assert!(
            session.output.wait_for_end(STEP_TIMEOUT),
            "{name}: no end-of-file after exit"
        );
    }
}

#[test]
fn environment_and_working_directory_reach_the_child() {
    let dir = std::env::temp_dir().canonicalize().unwrap();
    // The last component is enough to recognise the directory in the shell's output,
    // however the shell chooses to spell the rest of the path.
    let leaf = dir.file_name().unwrap().to_string_lossy().into_owned();

    for shell in shells() {
        let name = shell.name;
        let command = shell.command.clone().env("LUME_MARK", "env-ok").cwd(&dir);
        let mut session = Running::start(&command, PtySize::cells(120, 30));

        session.type_line(shell.print_marker);
        assert!(
            session.output.wait_for("lume-env-ok-done"),
            "{name}: {:?}",
            session.output.text()
        );

        let before = session.output.len();
        session.type_line(shell.print_cwd);
        assert!(
            session.output.wait_for_after(before, &leaf),
            "{name}: the working directory is not {leaf}; got {:?}",
            session.output.text()
        );

        let before = session.output.len();
        session.type_line(shell.print_term);
        let expected = if cfg!(windows) {
            "term=["
        } else {
            "term=[xterm-256color]"
        };
        assert!(
            session.output.wait_for_after(before, expected),
            "{name}: {:?}",
            session.output.text()
        );
        if cfg!(windows) {
            // Windows programs are not told a terminal type, whatever the caller's own is.
            let printed = session.output.text();
            assert!(
                !printed[before..].contains("term=[xterm"),
                "{name}: {printed:?}"
            );
        }

        session.type_line("exit 0");
        assert_eq!(
            wait_with_timeout(session.child, STEP_TIMEOUT),
            Some(ExitStatus::Code(0)),
            "{name}"
        );
    }
}

#[test]
fn a_program_that_prints_and_exits_delivers_its_output_and_status() {
    let started = Instant::now();
    let session = Running::start(&common::print_and_exit_3(), PtySize::cells(80, 24));
    assert_eq!(
        wait_with_timeout(session.child, STEP_TIMEOUT),
        Some(ExitStatus::Code(3))
    );
    assert!(
        session.output.wait_for_end(STEP_TIMEOUT),
        "no end-of-file after exit"
    );
    assert!(
        session.output.text().contains("short-lived"),
        "{:?}",
        session.output.text()
    );
    // Recorded so a slow start shows up in the test log.
    println!("start to end-of-file: {:?}", started.elapsed());
}

#[test]
fn a_program_that_does_not_exist_is_reported_and_nothing_is_left_running() {
    let error = lume_pty::native()
        .spawn(
            &CommandSpec::new("no-such-program-anywhere"),
            PtySize::cells(80, 24),
        )
        .err()
        .expect("spawning a missing program fails");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound, "{error}");

    let missing_dir =
        CommandSpec::new(common::quick_shell().program).cwd("/no/such/directory/here");
    let error = lume_pty::native()
        .spawn(&missing_dir, PtySize::cells(80, 24))
        .err();
    assert!(error.is_some(), "a missing working directory is refused");
}

#[test]
fn a_terminal_without_cells_is_refused() {
    let error = lume_pty::native()
        .spawn(&common::quick_shell(), PtySize::cells(0, 24))
        .err();
    assert!(error.is_some());
}
