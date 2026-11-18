//! The host's behaviour, as cases that run unchanged against any way of running it.
//!
//! Each case is written once, against [`Harness`]. [`host_suite!`] turns the list of
//! cases into tests for one harness, so the same cases prove the host both as a task in
//! the test's process and as a process of its own.
//!
//! Every case ends by shutting the host down, which is itself part of what is tested:
//! the host must end every program a connection started before it lets go of the
//! connection.

use std::time::{Duration, Instant};

use lume_proto::tuning::MAX_PAYLOAD;
use lume_proto::{
    Body, CellPx, ClientKind, CommandState, ErrorCode, ExitStatus, HydrateMode, HydratePayload,
    Message, Notification, PROTO_VERSION, ProgramState, ProtoError, RequestBody, Responder,
    ResponseBody, SessionEvent, SessionId,
};

use crate::support::{self, Harness, MARK_OUTPUT, PRINT_MARK, PRINT_SIZE, STEP, size};

/// Declares one test per suite case for the harness `$harness`.
macro_rules! host_suite {
    ($harness:ty) => {
        $crate::suite::host_suite!(@cases $harness;
            hello_must_come_first,
            a_client_with_no_common_version_is_refused,
            spawn_then_list,
            a_missing_program_is_an_invalid_spec,
            output_before_attach_is_on_the_screen_a_client_starts_from,
            a_client_of_another_engine_starts_from_replayed_output,
            a_session_with_no_client_runs_on,
            the_screen_answers_the_programs_questions,
            the_screen_can_be_read_and_waited_on,
            a_client_can_start_again_from_the_screen_in_the_middle_of_the_flow,
            input_is_echoed,
            resize_is_marked_in_the_output_and_reaches_the_shell,
            detach_keeps_the_session_and_a_new_stream_continues,
            a_second_attach_is_refused,
            frames_for_a_closed_stream_are_ignored,
            output_then_exit_arrive_in_order,
            kill_yields_exit_then_removal,
            an_unknown_session_is_not_found,
            a_malformed_frame_closes_the_connection,
            a_closed_connection_leaves_no_program_running,
            a_flood_with_no_credit_leaves_memory_flat,
            what_a_program_reports_arrives_as_events_behind_its_output,
            a_client_that_attaches_is_told_what_was_reported_before,
            the_program_that_has_the_terminal_is_reported,
            a_session_nobody_sees_is_looked_at_when_it_prints,
            output_nobody_is_watching_is_reported_now_and_then,
        );
    };
    (@cases $harness:ty; $($case:ident),* $(,)?) => {
        $(
            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn $case() {
                $crate::suite::$case::<$harness>().await;
            }
        )*
    };
}
pub(crate) use host_suite;

fn not_a_session() -> SessionId {
    SessionId::new(0x0bad_0bad_0bad_0bad).unwrap()
}

fn code(result: Result<ResponseBody, ProtoError>) -> Option<ErrorCode> {
    result.err().map(|error| error.code)
}

pub async fn hello_must_come_first<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;

    assert_eq!(
        code(client.request(RequestBody::List {}).await),
        Some(ErrorCode::HelloRequired)
    );

    let hello = RequestBody::Hello {
        proto: PROTO_VERSION,
        proto_min: PROTO_VERSION,
        vt_version: Some(lume_host::vt_version().to_owned()),
        client_kind: ClientKind::Test,
    };
    match client.request(hello.clone()).await {
        Ok(ResponseBody::HelloAck {
            proto,
            caps,
            max_payload,
            pid,
            vt_version,
            ..
        }) => {
            assert_eq!(proto, PROTO_VERSION);
            assert_eq!(max_payload as usize, MAX_PAYLOAD);
            assert_ne!(pid, 0);
            // The host keeps every session's screen, and that screen answers.
            assert_eq!(vt_version.as_deref(), Some(lume_host::vt_version()));
            assert_eq!(caps.responder, Responder::Backend);
            assert!(!caps.persistent && !caps.multi_client && !caps.server_layout);
            assert!(caps.hydrate.snapshot && caps.hydrate.vt_replay);
        }
        other => panic!("hello was answered with {other:?}"),
    }

    assert_eq!(
        code(client.request(hello).await),
        Some(ErrorCode::BadRequest)
    );
    assert!(client.list().await.is_empty());
    host.shutdown().await;
}

pub async fn a_client_with_no_common_version_is_refused<H: Harness>() {
    let mut host = H::start().await;
    let hello = RequestBody::Hello {
        proto: PROTO_VERSION + 5,
        proto_min: PROTO_VERSION + 1,
        vt_version: None,
        client_kind: ClientKind::Test,
    };
    assert_eq!(
        code(host.client.request(hello).await),
        Some(ErrorCode::VersionMismatch)
    );
    assert!(
        host.client.closes().await,
        "the host kept the connection open"
    );
    host.shutdown().await;
}

pub async fn spawn_then_list<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    let session = client.spawn(support::shell()).await;
    let info = client.only_session().await;
    assert_eq!(info.id, session);
    assert_eq!(info.size, size(80, 24));
    assert!(info.pid.is_some_and(|pid| pid != 0));
    assert!(!info.attached);
    assert_eq!(info.exit, None);
    assert!(info.created_unix_ms > 1_600_000_000_000);

    let second = client.spawn(support::shell()).await;
    assert_ne!(second, session);
    assert_eq!(client.list().await.len(), 2);
    host.shutdown().await;
}

pub async fn a_missing_program_is_an_invalid_spec<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    let spawn = RequestBody::Spawn {
        spec: support::missing_program(),
    };
    assert_eq!(
        code(client.request(spawn).await),
        Some(ErrorCode::InvalidSpec)
    );

    let mut elsewhere = support::shell();
    elsewhere.cwd = Some("/no/such/directory/here".to_owned());
    let spawn = RequestBody::Spawn { spec: elsewhere };
    assert_eq!(
        code(client.request(spawn).await),
        Some(ErrorCode::InvalidSpec)
    );

    assert!(client.list().await.is_empty());
    host.shutdown().await;
}

pub async fn output_before_attach_is_on_the_screen_a_client_starts_from<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    let session = client
        .spawn(support::prints_then_waits("printed-before-attach"))
        .await;
    // Let the program print before anyone is attached. The terminal's own start-up
    // sequences come before the program's line, so the wait is for the line itself.
    let printed = client
        .request(RequestBody::WaitText {
            session,
            text: "printed-before-attach".to_owned(),
            timeout_ms: u64::try_from(STEP.as_millis()).unwrap(),
        })
        .await;
    assert!(
        matches!(printed, Ok(ResponseBody::Matched { .. })),
        "{printed:?}"
    );

    match client.attach(session, 1).await {
        Ok(ResponseBody::Attached {
            size: attached,
            hydrate,
            resize_pulls_scrollback,
        }) => {
            assert_eq!(attached, size(80, 24));
            // The client runs the host's own engine, so it gets a snapshot.
            assert_eq!(hydrate, HydrateMode::Snapshot);
            // A pseudoconsole keeps its own copy of the screen; a Unix terminal does not.
            assert_eq!(resize_pulls_scrollback, cfg!(unix));
        }
        other => panic!("attach was answered with {other:?}"),
    }
    client
        .list_until("the session shows no client", |sessions| {
            sessions[0].attached
        })
        .await;

    // What the program printed is on the screen the client starts from; the output
    // that follows starts after it.
    let screen = client.hydrated_text(1);
    assert!(screen.contains("printed-before-attach"), "{screen:?}");
    assert!(matches!(
        client.hydrated(1),
        Some(HydratePayload::Snapshot { vt_version, .. }) if vt_version == lume_host::vt_version()
    ));

    // Credit brings no copy of what is already on that screen.
    client.credit(1, 1024 * 1024).await;
    let mut printed = String::new();
    while let Some(bytes) = client.data(1, Duration::from_millis(500)).await {
        printed.push_str(&String::from_utf8_lossy(&bytes));
    }
    assert!(
        !printed.contains("printed-before-attach"),
        "output on the screen came again: {printed:?}"
    );
    host.shutdown().await;
}

pub async fn a_client_can_start_again_from_the_screen_in_the_middle_of_the_flow<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    let session = client.spawn(support::shell()).await;
    client.attach_with_credit(session, 1).await;
    client.type_line(1, PRINT_MARK).await;
    client.read_until(1, MARK_OUTPUT).await;
    client.discard_output(1).await;

    client.send(Message::new(1, Body::Rehydrate)).await;
    let is_hydrate =
        |message: &Message| message.stream == 1 && matches!(message.body, Body::Hydrate(_));
    let Body::Hydrate(payload) = client
        .expect("the copy of the screen", is_hydrate)
        .await
        .body
    else {
        unreachable!("matched as a hydrate");
    };
    let HydratePayload::Snapshot { bytes, .. } = payload else {
        panic!("a client of the host's own engine is sent a snapshot");
    };
    let screen = lume_vt::Terminal::from_snapshot(&bytes)
        .unwrap()
        .screen_text()
        .unwrap()
        .join("\n");
    assert!(screen.contains(MARK_OUTPUT), "{screen:?}");

    // The flow goes on behind it.
    client.type_line(1, "echo after-the-copy").await;
    client.read_until(1, "after-the-copy").await;
    host.shutdown().await;
}

pub async fn a_client_of_another_engine_starts_from_replayed_output<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello_as(Some("0.0.0-another".to_owned())).await;

    let session = client
        .spawn(support::prints_then_waits("printed-before-attach"))
        .await;
    // The terminal's own start-up sequences come before the program's line, so the
    // wait is for the line itself.
    let printed = client
        .request(RequestBody::WaitText {
            session,
            text: "printed-before-attach".to_owned(),
            timeout_ms: u64::try_from(STEP.as_millis()).unwrap(),
        })
        .await;
    assert!(
        matches!(printed, Ok(ResponseBody::Matched { .. })),
        "{printed:?}"
    );
    match client.attach(session, 1).await {
        Ok(ResponseBody::Attached { hydrate, .. }) => assert_eq!(hydrate, HydrateMode::VtReplay),
        other => panic!("attach was answered with {other:?}"),
    }
    let screen = client.hydrated_text(1);
    assert!(screen.contains("printed-before-attach"), "{screen:?}");
    host.shutdown().await;
}

pub async fn a_session_with_no_client_runs_on<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    // A program that prints far more than any buffer holds and then waits.
    let session = client.spawn(support::prints_lines_then_waits(6_000)).await;
    let wanted = support::line_number(6_000);
    client
        .request(RequestBody::WaitText {
            session,
            text: wanted.clone(),
            timeout_ms: STEP.as_millis() as u64,
        })
        .await
        .expect("the program ran to its end with nobody attached");

    client.attach_with_credit(session, 1).await;
    let screen = client.hydrated_text(1);
    assert!(screen.contains(&wanted), "{screen:?}");
    host.shutdown().await;
}

pub async fn the_screen_answers_the_programs_questions<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    // Nobody is attached: the question is answered all the same, once, and the
    // program prints the answer it read.
    let session = client.spawn(support::asks_for_the_cursor()).await;
    client
        .request(RequestBody::WaitText {
            session,
            text: support::CURSOR_ANSWERED.to_owned(),
            timeout_ms: STEP.as_millis() as u64,
        })
        .await
        .expect("the program got an answer");
    host.shutdown().await;
}

pub async fn the_screen_can_be_read_and_waited_on<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    let session = client
        .spawn(support::prints_then_waits("on-the-screen"))
        .await;
    let matched = client
        .request(RequestBody::WaitText {
            session,
            text: "on-the-screen".to_owned(),
            timeout_ms: STEP.as_millis() as u64,
        })
        .await;
    let Ok(ResponseBody::Matched { row, line }) = matched else {
        panic!("wait was answered with {matched:?}");
    };
    assert!(line.contains("on-the-screen"), "{line:?}");

    match client.request(RequestBody::Screen { session }).await {
        Ok(ResponseBody::Screen {
            size: shown,
            lines,
            alternate,
            ..
        }) => {
            assert_eq!(shown, size(80, 24));
            assert_eq!(lines.len(), 24);
            assert!(!alternate);
            assert_eq!(lines[usize::from(row)], line);
        }
        other => panic!("screen was answered with {other:?}"),
    }

    let missing = client
        .request(RequestBody::WaitText {
            session,
            text: "never-printed".to_owned(),
            timeout_ms: 300,
        })
        .await;
    assert_eq!(code(missing), Some(ErrorCode::Timeout));
    assert_eq!(
        code(
            client
                .request(RequestBody::Screen {
                    session: not_a_session()
                })
                .await
        ),
        Some(ErrorCode::NotFound)
    );
    host.shutdown().await;
}

pub async fn input_is_echoed<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    let session = client.spawn(support::shell()).await;
    client.attach_with_credit(session, 1).await;
    client.type_line(1, PRINT_MARK).await;
    client.read_until(1, MARK_OUTPUT).await;

    let stats = client.only_session().await.stats;
    assert!(stats.read_bytes > 0 && stats.sent_bytes > 0);
    assert!(stats.sent_bytes <= stats.read_bytes);
    host.shutdown().await;
}

pub async fn resize_is_marked_in_the_output_and_reaches_the_shell<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    let session = client.spawn(support::shell()).await;
    client.attach_with_credit(session, 1).await;
    client.type_line(1, PRINT_MARK).await;
    client.read_until(1, MARK_OUTPUT).await;

    let resize = Body::Resize {
        size: size(97, 31),
        cell_px: CellPx { w: 9, h: 18 },
    };
    client.send(Message::new(1, resize)).await;
    assert_eq!(client.marker(1).await, Body::Resized(size(97, 31)));
    client
        .list_until("the session does not show its new size", |sessions| {
            sessions[0].size == size(97, 31)
        })
        .await;

    // Whatever the terminal repainted for the resize is not what is being looked for.
    client.discard_output(1).await;
    client.type_line(1, PRINT_SIZE).await;
    let printed = client.read_until(1, "97").await;
    assert!(
        printed.contains("31"),
        "the shell does not report 31 rows: {printed:?}"
    );
    host.shutdown().await;
}

pub async fn detach_keeps_the_session_and_a_new_stream_continues<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    let session = client.spawn(support::shell()).await;
    client.attach_with_credit(session, 1).await;
    client.type_line(1, PRINT_MARK).await;
    client.read_until(1, MARK_OUTPUT).await;

    client.send(Message::new(1, Body::Detach)).await;
    client
        .list_until("the session still shows a client", |sessions| {
            !sessions[0].attached
        })
        .await;
    assert_eq!(
        client.only_session().await.exit,
        None,
        "detaching ended the program"
    );

    // A stream id is never used twice on a connection.
    assert_eq!(
        code(client.attach(session, 1).await),
        Some(ErrorCode::StreamInUse)
    );
    assert_eq!(
        code(client.attach(session, 0).await),
        Some(ErrorCode::InvalidStream)
    );

    client.attach_with_credit(session, 2).await;
    client.discard_output(2).await;
    client.type_line(2, PRINT_MARK).await;
    client.read_until(2, MARK_OUTPUT).await;
    host.shutdown().await;
}

pub async fn a_second_attach_is_refused<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    let session = client.spawn(support::shell()).await;
    client.attach_with_credit(session, 1).await;
    assert_eq!(
        code(client.attach(session, 2).await),
        Some(ErrorCode::AlreadyAttached)
    );

    // The refused stream was not opened, and the first is unaffected.
    client.type_line(1, PRINT_MARK).await;
    client.read_until(1, MARK_OUTPUT).await;
    host.shutdown().await;
}

pub async fn frames_for_a_closed_stream_are_ignored<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    let session = client.spawn(support::shell()).await;
    client.attach_with_credit(session, 1).await;
    client.send(Message::new(1, Body::Detach)).await;

    // Frames a client sent before it knew the stream was closed, and frames for a
    // stream that never existed, are not errors.
    client.type_line(1, PRINT_MARK).await;
    client.credit(1, 10).await;
    client.send(Message::new(1, Body::Detach)).await;
    client.type_line(77, PRINT_MARK).await;

    // The connection is still in order, and the session is there without a client.
    let sessions = client
        .list_until("the session still shows a client", |sessions| {
            !sessions[0].attached
        })
        .await;
    assert_eq!(sessions[0].id, session);
    assert_eq!(sessions[0].exit, None);
    host.shutdown().await;
}

pub async fn output_then_exit_arrive_in_order<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    let session = client
        .spawn(support::prints_then_exits("last-words", 4))
        .await;
    // The program has ended before anyone attached. The session waits for a client.
    assert_eq!(
        client.notification().await,
        Notification::SessionExited {
            session,
            status: ExitStatus::Code(4)
        }
    );
    let info = client.only_session().await;
    assert_eq!(info.exit, Some(ExitStatus::Code(4)));

    client.attach_with_credit(session, 1).await;
    // The screen it ended on, then everything on the stream in the order it arrives,
    // up to the exit.
    let mut printed = client.hydrated_text(1);
    loop {
        let message = client
            .expect("the rest of the stream", |message| message.stream == 1)
            .await;
        match message.body {
            Body::Data(bytes) => printed.push_str(&String::from_utf8_lossy(&bytes)),
            Body::Exit(status) => {
                assert_eq!(status, ExitStatus::Code(4));
                break;
            }
            // What the session reported (the terminal sets a title, for one) is not
            // what is being looked at here.
            Body::Event(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(
        printed.contains("last-words"),
        "the output did not precede the exit: {printed:?}"
    );

    // The session stays listed while a client shows it, and goes when the client does.
    assert_eq!(client.only_session().await.exit, Some(ExitStatus::Code(4)));
    client.send(Message::new(1, Body::Detach)).await;
    client
        .list_until("the exited session was not removed", |sessions| {
            sessions.is_empty()
        })
        .await;
    host.shutdown().await;
}

pub async fn kill_yields_exit_then_removal<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    let session = client.spawn(support::shell()).await;
    client.attach_with_credit(session, 1).await;
    client.type_line(1, PRINT_MARK).await;
    client.read_until(1, MARK_OUTPUT).await;

    assert_eq!(
        client.request(RequestBody::Kill { session }).await,
        Ok(ResponseBody::Killed {})
    );
    assert!(matches!(client.marker(1).await, Body::Exit(_)));
    assert!(matches!(
        client.notification().await,
        Notification::SessionExited { session: exited, .. } if exited == session
    ));
    client
        .list_until("the killed session was not removed", |sessions| {
            sessions.is_empty()
        })
        .await;

    assert_eq!(
        code(client.request(RequestBody::Kill { session }).await),
        Some(ErrorCode::NotFound)
    );
    host.shutdown().await;
}

pub async fn an_unknown_session_is_not_found<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    assert_eq!(
        code(client.attach(not_a_session(), 1).await),
        Some(ErrorCode::NotFound)
    );
    let kill = RequestBody::Kill {
        session: not_a_session(),
    };
    assert_eq!(code(client.request(kill).await), Some(ErrorCode::NotFound));
    // The refused attach did not use up the stream id.
    let session = client.spawn(support::shell()).await;
    client.attach_with_credit(session, 1).await;
    host.shutdown().await;
}

pub async fn a_malformed_frame_closes_the_connection<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;
    // A session, to see that it does not outlive the connection the host closes.
    client.spawn(support::shell()).await;
    let pid = client
        .only_session()
        .await
        .pid
        .expect("the session has a process");

    // A header with a frame kind that does not exist.
    client.send_raw(&[1, 0, 0, 0, 0x7f, 0, 0, 0, 0]).await;
    match client.notification().await {
        Notification::Fatal { code, .. } => assert_eq!(code, ErrorCode::ProtocolError),
        other => panic!("unexpected {other:?}"),
    }
    assert!(client.closes().await, "the host kept the connection open");
    host.shutdown().await;
    assert_gone(&[pid]).await;
}

pub async fn a_closed_connection_leaves_no_program_running<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;
    // One session nobody attached to, one with a client, and one flooding a client
    // that takes nothing: the three states a session can be in when its client goes.
    client.spawn(support::shell()).await;
    let attached = client.spawn(support::shell()).await;
    client.attach_with_credit(attached, 1).await;
    let flooding = client.spawn(support::flood()).await;
    let stalled = client.attach(flooding, 2).await;
    assert!(
        matches!(stalled, Ok(ResponseBody::Attached { .. })),
        "{stalled:?}"
    );

    let pids: Vec<u32> = client
        .list()
        .await
        .iter()
        .filter_map(|info| info.pid)
        .collect();
    assert_eq!(pids.len(), 3);
    assert!(pids.iter().all(|&pid| support::process_alive(pid)));

    // The client goes away without a word.
    let started = Instant::now();
    host.shutdown().await;
    println!(
        "closing a connection with three sessions took {:?}",
        started.elapsed()
    );
    assert_gone(&pids).await;
}

/// Checks that none of the processes is running. They were ended before this is
/// called; the wait only covers the moment the system takes to tear a process down.
async fn assert_gone(pids: &[u32]) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while pids.iter().any(|&pid| support::process_alive(pid)) {
        let alive: Vec<u32> = pids
            .iter()
            .copied()
            .filter(|&pid| support::process_alive(pid))
            .collect();
        assert!(
            Instant::now() < deadline,
            "programs outlived their connection: {alive:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Waits until the only session has stopped reading its terminal, and returns how many
/// bytes it had read by then.
async fn reading_stops(client: &mut support::Client) -> u64 {
    let deadline = Instant::now() + STEP;
    let mut last = u64::MAX;
    let mut unchanged = 0;
    loop {
        let read = client.only_session().await.stats.read_bytes;
        unchanged = if read == last { unchanged + 1 } else { 0 };
        last = read;
        if unchanged >= 3 && read > 0 {
            return read;
        }
        assert!(
            Instant::now() < deadline,
            "the host kept reading: {read} bytes"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub async fn a_flood_with_no_credit_leaves_memory_flat<H: Harness>() {
    const KIB: u64 = 1024;
    /// What a session may hold for a client that grants nothing: one send, and one
    /// read that was already under way when it filled.
    const HELD_MAX: u64 = (64 + 64) * KIB;

    let mut host = H::start().await;
    let client = &mut host.client;
    let host_pid = client.hello().await;

    let session = client.spawn(support::flood()).await;
    let attached = client.attach(session, 1).await;
    assert!(
        matches!(attached, Ok(ResponseBody::Attached { .. })),
        "{attached:?}"
    );

    // Without credit the host reads until its buffer is full and then stops.
    let read_at_stall = reading_stops(client).await;
    let stats = client.only_session().await.stats;
    assert!(stats.paused, "the host is not paused: {stats:?}");
    assert_eq!(stats.sent_bytes, 0);
    assert!(u64::from(stats.pending_bytes) <= HELD_MAX, "{stats:?}");
    assert_eq!(
        client.data(1, Duration::from_millis(300)).await,
        None,
        "output without credit"
    );

    // Held like this, nothing moves and nothing grows.
    let memory_before = support::resident_bytes(host_pid);
    tokio::time::sleep(Duration::from_secs(3)).await;
    let memory_after = support::resident_bytes(host_pid);
    let stats = client.only_session().await.stats;
    assert_eq!(
        stats.read_bytes, read_at_stall,
        "the host read on without credit"
    );
    let grown = memory_after.saturating_sub(memory_before);
    println!("host memory while stalled: {memory_before} -> {memory_after} bytes");
    assert!(
        grown < 8 * 1024 * KIB,
        "the host grew by {grown} bytes while stalled"
    );

    // Credit for one frame's worth releases exactly that much, and then it stalls again.
    let granted = MAX_PAYLOAD as u32;
    client.credit(1, granted).await;
    let mut received = 0;
    while received < MAX_PAYLOAD {
        let bytes = client
            .data(1, STEP)
            .await
            .expect("output arrives against credit");
        received += bytes.len();
    }
    assert_eq!(received, MAX_PAYLOAD, "more arrived than was granted");
    assert_eq!(
        client.data(1, Duration::from_millis(500)).await,
        None,
        "output beyond credit"
    );
    reading_stops(client).await;
    let stats = client.only_session().await.stats;
    assert_eq!(stats.sent_bytes, u64::from(granted));
    assert!(stats.paused, "the host is not paused again: {stats:?}");

    assert_eq!(
        client.request(RequestBody::Kill { session }).await,
        Ok(ResponseBody::Killed {})
    );
    assert!(matches!(client.marker(1).await, Body::Exit(_)));
    host.shutdown().await;
}

/// The reports [`support::reports_then_waits`] makes, as they should arrive.
fn expected_reports() -> Vec<SessionEvent> {
    vec![
        SessionEvent::Title {
            title: "lume-title-k7q".to_owned(),
        },
        SessionEvent::Cwd {
            url: "file://box/srv/www%20x".to_owned(),
        },
        SessionEvent::CommandStarted {
            command: Some("make test".to_owned()),
        },
        SessionEvent::CommandFinished { exit: Some(3) },
    ]
}

pub async fn what_a_program_reports_arrives_as_events_behind_its_output<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    let session = client.spawn(support::reports_then_waits(MARK_OUTPUT)).await;
    client.attach_with_credit(session, 1).await;
    client.read_until(1, MARK_OUTPUT).await;

    // In the order the program made them. A terminal may set a title of its own
    // before the program does, so other titles are passed over.
    for expected in expected_reports() {
        let same_kind = |event: &SessionEvent| match (&expected, event) {
            (SessionEvent::Title { title }, SessionEvent::Title { title: other }) => title == other,
            (wanted, event) => {
                std::mem::discriminant(wanted) == std::mem::discriminant(event)
                    && !matches!(event, SessionEvent::Title { .. })
            }
        };
        let event = client.event(1, "what the program said", same_kind).await;
        assert_eq!(event, expected);
    }
    let status = client
        .event(1, "the program's status", |event| {
            matches!(event, SessionEvent::ProgramStatus(_))
        })
        .await;
    let SessionEvent::ProgramStatus(status) = status else {
        unreachable!("matched as a status");
    };
    assert_eq!(status.state, ProgramState::Done);
    assert_eq!(status.app.as_deref(), Some("make"));
    host.shutdown().await;
}

pub async fn a_client_that_attaches_is_told_what_was_reported_before<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    let session = client.spawn(support::reports_then_waits(MARK_OUTPUT)).await;
    client.attach_with_credit(session, 1).await;
    client.read_until(1, MARK_OUTPUT).await;
    client
        .event(1, "the program's status", |event| {
            matches!(event, SessionEvent::ProgramStatus(_))
        })
        .await;
    client.send(Message::new(1, Body::Detach)).await;
    client
        .list_until("the session is not free", |sessions| !sessions[0].attached)
        .await;

    // The second attachment starts with everything the first was told one by one.
    let attached = client.attach(session, 2).await;
    assert!(
        matches!(attached, Ok(ResponseBody::Attached { .. })),
        "{attached:?}"
    );
    let whole = client
        .event(2, "the state so far", |event| {
            matches!(event, SessionEvent::State { .. })
        })
        .await;
    let SessionEvent::State { state } = whole else {
        unreachable!("matched as a state");
    };
    assert_eq!(state.title.as_deref(), Some("lume-title-k7q"));
    assert_eq!(state.cwd.as_deref(), Some("file://box/srv/www%20x"));
    assert_eq!(
        state.command,
        Some(CommandState::Finished { exit: Some(3) })
    );
    assert_eq!(state.statuses.len(), 1, "{:?}", state.statuses);
    assert_eq!(state.statuses[0].state, ProgramState::Done);
    host.shutdown().await;
}

pub async fn the_program_that_has_the_terminal_is_reported<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    let session = client.spawn(support::shell()).await;
    let pid = client.only_session().await.pid;
    client.attach_with_credit(session, 1).await;
    let event = client
        .event(1, "the program that has the terminal", |event| {
            matches!(event, SessionEvent::Foreground(_))
        })
        .await;
    let SessionEvent::Foreground(foreground) = event else {
        unreachable!("matched as a foreground program");
    };
    let shell = if cfg!(windows) { "cmd" } else { "sh" };
    assert_eq!(foreground.name, shell);
    assert_eq!(Some(foreground.pid), pid);
    host.shutdown().await;
}

pub async fn a_session_nobody_sees_is_looked_at_when_it_prints<H: Harness>() {
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    let session = client.spawn(support::shell()).await;
    let pid = client.only_session().await.pid;
    // Attached out of sight, as a pane in a tab that is not on show is: nothing has it
    // looked at now and then.
    let mut facts = support::facts();
    facts.focused = false;
    facts.visible = false;
    let attached = client
        .request(RequestBody::Attach {
            session,
            stream: 1,
            facts,
        })
        .await;
    assert!(
        matches!(attached, Ok(ResponseBody::Attached { .. })),
        "{attached:?}"
    );
    client.credit(1, 1024 * 1024).await;

    // A program the shell starts, which prints now and then.
    client.type_line(1, support::PRINTS_NOW_AND_THEN).await;
    let event = client
        .event(
            1,
            "the program the shell started",
            |event| matches!(event, SessionEvent::Foreground(found) if Some(found.pid) != pid),
        )
        .await;
    assert!(matches!(event, SessionEvent::Foreground(_)), "{event:?}");
    host.shutdown().await;
}

pub async fn output_nobody_is_watching_is_reported_now_and_then<H: Harness>() {
    let is_activity = |event: &SessionEvent| matches!(event, SessionEvent::Activity);
    let mut host = H::start().await;
    let client = &mut host.client;
    client.hello().await;

    let session = client.spawn(support::shell()).await;
    client.attach_with_credit(session, 1).await;
    client.type_line(1, PRINT_MARK).await;
    client.read_until(1, MARK_OUTPUT).await;
    // The view has the keyboard, so its output is being watched.
    assert_eq!(
        client
            .event_within(1, Duration::from_millis(300), is_activity)
            .await,
        None
    );

    let mut facts = support::facts();
    facts.focused = false;
    client.send(Message::new(1, Body::Facts(facts))).await;
    client.type_line(1, PRINT_MARK).await;
    client
        .event(1, "output behind the user's back", is_activity)
        .await;
    // Said once for a burst of output, not for every piece of it.
    client.read_until(1, MARK_OUTPUT).await;
    assert_eq!(
        client
            .event_within(1, Duration::from_millis(300), is_activity)
            .await,
        None
    );
    host.shutdown().await;
}
