//! Sessions through the router and the local backend, against a real host and shell.

use std::sync::Arc;
use std::time::Duration;

use lume_host::HostConfig;
use lume_proto::{
    Body, Control, ErrorCode, FrameDecoder, Message, PROTO_VERSION, RequestBody, Response,
    ResponseBody, Role, encode,
};
use lume_session::{
    Attachment, BackendId, CellPx, ClientFacts, ColorScheme, ExitStatus, Hydrate, HydrateKinds,
    Launcher, LocalBackend, OutputFrame, Palette, Responder, Router, SessionBackend, SessionError,
    SessionId, SpawnSpec, TermSize,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

/// How long any single step of a test may take before the test fails.
const STEP: Duration = Duration::from_secs(20);

const CREDIT: u32 = 1024 * 1024;

fn size(cols: u16, rows: u16) -> TermSize {
    TermSize::new(cols, rows).unwrap()
}

fn facts() -> ClientFacts {
    ClientFacts {
        size: size(80, 24),
        cell_px: CellPx { w: 9, h: 18 },
        palette: Palette::default(),
        scheme: ColorScheme::Dark,
        focused: true,
        visible: true,
    }
}

/// An interactive shell with `LUME_MARK` set, so a command can print something that
/// does not appear in its own echo.
fn shell() -> SpawnSpec {
    let (program, args) = if cfg!(windows) {
        ("cmd.exe", vec!["/d"])
    } else {
        ("/bin/sh", vec![])
    };
    SpawnSpec {
        launcher: Launcher::Shell {
            program: program.to_owned(),
            args: args.into_iter().map(str::to_owned).collect(),
            login: false,
        },
        cwd: None,
        env: vec![("LUME_MARK".to_owned(), "k7q".to_owned())],
        size: size(80, 24),
        shell_integration: false,
        scrollback_lines: None,
    }
}

const PRINT_MARK: &[u8] = if cfg!(windows) {
    b"echo lume-%LUME_MARK%-done\r"
} else {
    b"echo \"lume-${LUME_MARK}-done\"\r"
};

/// A router with one backend, the local one, on a host in this process.
async fn local_router() -> (Router, Arc<LocalBackend>) {
    let backend = Arc::new(
        LocalBackend::in_process(HostConfig::default())
            .await
            .unwrap(),
    );
    let router = Router::new();
    router.insert(Router::local_id(), backend.clone());
    (router, backend)
}

/// Reads output until it contains `needle`.
async fn read_until(attachment: &mut Attachment, needle: &str) -> String {
    let mut text = String::new();
    while !text.contains(needle) {
        match timeout(STEP, attachment.output.next()).await {
            Ok(Some(OutputFrame::Data(bytes))) => text.push_str(&String::from_utf8_lossy(&bytes)),
            Ok(Some(_)) => {}
            other => panic!("`{needle}` never appeared ({other:?}); got {text:?}"),
        }
    }
    text
}

/// Reads frames until the session's exit.
async fn read_to_exit(attachment: &mut Attachment) -> ExitStatus {
    loop {
        match timeout(STEP, attachment.output.next()).await {
            Ok(Some(OutputFrame::Exit(status))) => return status,
            Ok(Some(_)) => {}
            other => panic!("the session's exit never arrived ({other:?})"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn router_spawns_attaches_echoes_and_kills() {
    let (router, local) = local_router().await;
    let backend = router.get(&Router::local_id()).unwrap();

    let session = backend.spawn(shell()).await.unwrap();
    let listed = backend.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, session);

    let mut attachment = backend.attach(session, facts()).await.unwrap();
    assert_eq!(attachment.size, size(80, 24));
    // The backend says it runs the host's own engine, so it is sent a snapshot.
    assert!(
        matches!(&attachment.hydrate, Hydrate::Snapshot { vt_version, .. }
            if vt_version == lume_host::vt_version()),
        "{:?}",
        attachment.hydrate
    );
    assert_eq!(attachment.resize_pulls_scrollback, cfg!(unix));

    // The screen can be read and waited on without a view.
    let screen = backend.screen(session).await.unwrap();
    assert_eq!(screen.size, size(80, 24));
    assert_eq!(screen.lines.len(), 24);

    attachment.input.credit(CREDIT).await.unwrap();
    attachment.input.write(PRINT_MARK).await.unwrap();
    read_until(&mut attachment, "lume-k7q-done").await;
    let found = backend
        .wait_text(session, "lume-k7q-done", STEP)
        .await
        .unwrap();
    assert!(found.line.contains("lume-k7q-done"), "{found:?}");

    attachment
        .input
        .resize(size(100, 30), CellPx { w: 9, h: 18 })
        .await
        .unwrap();
    loop {
        match timeout(STEP, attachment.output.next()).await {
            Ok(Some(OutputFrame::Resized(resized))) => {
                assert_eq!(resized, size(100, 30));
                break;
            }
            Ok(Some(_)) => {}
            other => panic!("the resize was never marked ({other:?})"),
        }
    }
    attachment.input.facts(&facts()).await.unwrap();

    backend.kill(session).await.unwrap();
    read_to_exit(&mut attachment).await;
    // Nothing follows the exit, and the session is gone.
    let deadline = tokio::time::Instant::now() + STEP;
    while !backend.list().await.unwrap().is_empty() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the killed session is still listed"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    local.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn caps_describe_the_in_process_host() {
    let (router, local) = local_router().await;
    let caps = router.get(&Router::local_id()).unwrap().caps();
    assert!(!caps.persistent);
    assert!(!caps.multi_client);
    assert!(!caps.server_layout);
    assert_eq!(caps.responder, Responder::Backend);
    assert_eq!(
        caps.hydrate,
        HydrateKinds {
            snapshot: true,
            vt_replay: true
        }
    );
    // A backend that does not arrange panes itself has no layout of its own to offer.
    assert!(router.get(&Router::local_id()).unwrap().layout().is_none());
    local.shutdown().await;
}

#[tokio::test]
async fn unknown_backend_is_an_error() {
    let router = Router::new();
    assert!(router.ids().is_empty());
    let nowhere = BackendId("nowhere".to_owned());
    match router.get(&nowhere) {
        Err(SessionError::UnknownBackend(id)) => assert_eq!(id, nowhere),
        Err(error) => panic!("unexpected error: {error}"),
        Ok(_) => panic!("a backend nobody added was found"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_request_carries_the_hosts_reason() {
    let (router, local) = local_router().await;
    let backend = router.get(&Router::local_id()).unwrap();
    let nobody = SessionId::new(0x0bad_0bad).unwrap();

    match backend.kill(nobody).await {
        Err(SessionError::Refused(error)) => assert_eq!(error.code, ErrorCode::NotFound),
        other => panic!("unexpected {other:?}"),
    }
    match backend.attach(nobody, facts()).await {
        Err(SessionError::Refused(error)) => assert_eq!(error.code, ErrorCode::NotFound),
        Err(error) => panic!("unexpected error: {error}"),
        Ok(_) => panic!("attached to a session that does not exist"),
    }

    let mut missing = shell();
    missing.launcher = Launcher::Shell {
        program: "no-such-program-anywhere".into(),
        args: vec![],
        login: false,
    };
    match backend.spawn(missing).await {
        Err(SessionError::Refused(error)) => assert_eq!(error.code, ErrorCode::InvalidSpec),
        other => panic!("unexpected {other:?}"),
    }
    local.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_attachment_follows_a_detach_and_the_first_is_dead() {
    let (router, local) = local_router().await;
    let backend = router.get(&Router::local_id()).unwrap();
    let session = backend.spawn(shell()).await.unwrap();

    let mut first = backend.attach(session, facts()).await.unwrap();
    first.input.credit(CREDIT).await.unwrap();
    first.input.write(PRINT_MARK).await.unwrap();
    read_until(&mut first, "lume-k7q-done").await;

    first.input.detach().await.unwrap();
    // The first attachment's stream ends and its sink refuses everything. What the shell
    // printed before the detach took effect, such as its next prompt, may still be on
    // the way and arrives first; nothing but output does.
    loop {
        match timeout(STEP, first.output.next()).await.unwrap() {
            None => break,
            Some(OutputFrame::Data(_)) => {}
            Some(other) => panic!("unexpected {other:?} after a detach"),
        }
    }
    assert!(matches!(
        first.input.write(b"x").await,
        Err(SessionError::Detached)
    ));
    assert!(matches!(
        first.input.credit(1).await,
        Err(SessionError::Detached)
    ));
    assert!(matches!(
        first.input.detach().await,
        Err(SessionError::Detached)
    ));

    // The session is still there for a new attachment.
    let mut second = backend.attach(session, facts()).await.unwrap();
    second.input.credit(CREDIT).await.unwrap();
    second.input.write(PRINT_MARK).await.unwrap();
    read_until(&mut second, "lume-k7q-done").await;

    local.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_ends_sessions_and_everything_after_it_is_disconnected() {
    let (router, local) = local_router().await;
    let backend = router.get(&Router::local_id()).unwrap();
    let session = backend.spawn(shell()).await.unwrap();
    let pid = backend.list().await.unwrap()[0]
        .pid
        .expect("the session has a process");
    let mut attachment = backend.attach(session, facts()).await.unwrap();
    attachment.input.credit(CREDIT).await.unwrap();

    local.shutdown().await;

    assert!(matches!(
        backend.list().await,
        Err(SessionError::Disconnected)
    ));
    assert!(matches!(
        attachment.input.write(b"x").await,
        Err(SessionError::Disconnected)
    ));
    // The attachment's stream ends rather than waiting for ever.
    while timeout(STEP, attachment.output.next())
        .await
        .unwrap()
        .is_some()
    {}
    assert!(
        !process_alive(pid),
        "the session's program outlived the shutdown"
    );
}

/// The host behind a real endpoint rather than an in-memory pipe. A named pipe cannot
/// close one direction alone, so the host sees the connection end only when the backend
/// lets go of all of it. An in-memory pipe would hide a backend that did not.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_a_backend_over_an_endpoint_ends_the_connection_for_the_host() {
    for how in ["drop", "shutdown"] {
        let name = format!("session-{}-{how}", std::process::id());
        let endpoint = lume_ipc::Endpoint::named(&name).unwrap();
        let mut listener = lume_ipc::Listener::bind(&endpoint).unwrap();
        let serving = tokio::spawn(async move {
            let client = listener.accept().await.unwrap();
            lume_host::Host::new(HostConfig::default())
                .serve(client)
                .await
        });

        let stream = lume_ipc::connect(&endpoint).await.unwrap();
        let backend = LocalBackend::connect(stream).await.unwrap();
        let session = backend.spawn(shell()).await.unwrap();
        let pid = backend.list().await.unwrap()[0]
            .pid
            .expect("the session has a process");
        let attachment = backend.attach(session, facts()).await.unwrap();
        attachment.input.credit(CREDIT).await.unwrap();

        if how == "shutdown" {
            backend.shutdown().await;
        }
        // An attachment may outlive its backend; it must not keep the connection open.
        let closed_at = std::time::Instant::now();
        drop(backend);

        let served = timeout(STEP, serving).await;
        println!(
            "{how}: the host finished {:?} after the close",
            closed_at.elapsed()
        );
        let served = served.unwrap_or_else(|_| panic!("{how}: the host never saw the close"));
        assert!(
            served.unwrap().is_ok(),
            "{how}: serving ended with an error"
        );
        // The host ends a connection's sessions before it stops serving it.
        assert!(
            !process_alive(pid),
            "{how}: the session's program outlived the connection"
        );
        assert!(matches!(
            attachment.input.write(b"x").await,
            Err(SessionError::Disconnected)
        ));
    }
}

#[tokio::test]
async fn more_credit_than_a_stream_may_hold_is_refused_before_it_is_sent() {
    let (router, local) = local_router().await;
    let backend = router.get(&Router::local_id()).unwrap();
    let session = backend.spawn(shell()).await.unwrap();
    let attachment = backend.attach(session, facts()).await.unwrap();

    assert!(matches!(
        attachment.input.credit(u32::MAX).await,
        Err(SessionError::CreditOverflow)
    ));
    // The refused grant changed nothing: the connection is still in use.
    attachment.input.credit(CREDIT).await.unwrap();
    assert_eq!(backend.list().await.unwrap().len(), 1);
    local.shutdown().await;
}

/// A host that answers hello and one attach properly, then sends output although the
/// client granted no credit.
async fn host_that_ignores_credit(io: tokio::io::DuplexStream) {
    let (mut reader, mut writer) = tokio::io::split(io);
    let mut decoder = FrameDecoder::new(Role::Host);
    let mut buf = vec![0u8; 4096];

    loop {
        let read = match reader.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        decoder.push(&buf[..read]);
        while let Some(message) = decoder.next().unwrap() {
            let Body::Control(Control::Request(request)) = message.body else {
                continue;
            };
            let (answer, then_overrun) = match request.body {
                RequestBody::Hello { .. } => {
                    let ack = ResponseBody::HelloAck {
                        proto: PROTO_VERSION,
                        host_version: "test".to_owned(),
                        vt_version: None,
                        caps: lume_session::BackendCaps {
                            persistent: false,
                            multi_client: false,
                            server_layout: false,
                            responder: Responder::Client,
                            hydrate: HydrateKinds::default(),
                        },
                        max_payload: 65536,
                        pid: 1,
                    };
                    (ack, None)
                }
                RequestBody::Attach { stream, .. } => {
                    let attached = ResponseBody::Attached {
                        size: size(80, 24),
                        hydrate: lume_proto::HydrateMode::None,
                        resize_pulls_scrollback: true,
                    };
                    (attached, Some(stream))
                }
                // Anything else goes unanswered; the client should be gone by then.
                _ => continue,
            };

            let mut frames = Vec::new();
            let response = Response {
                id: request.id,
                result: Ok(answer),
            };
            encode(&Message::control(Control::Response(response)), &mut frames).unwrap();
            if let Some(stream) = then_overrun {
                let unasked = Body::Data(b"nobody asked for this".to_vec());
                encode(&Message::new(stream, unasked), &mut frames).unwrap();
            }
            if writer.write_all(&frames).await.is_err() {
                return;
            }
        }
    }
}

#[tokio::test]
async fn host_exceeding_its_grant_closes_the_connection() {
    let (client_end, host_end) = tokio::io::duplex(64 * 1024);
    tokio::spawn(host_that_ignores_credit(host_end));

    let backend = LocalBackend::connect(client_end).await.unwrap();
    let session = SessionId::new(7).unwrap();
    // Whether the attach itself succeeds depends on which the client sees first, the
    // response or the overrun. Either way the output must never be delivered and the
    // connection must end up closed.
    if let Ok(mut attachment) = backend.attach(session, facts()).await {
        let delivered = timeout(STEP, attachment.output.next()).await.unwrap();
        assert_eq!(delivered, None, "output sent without credit was delivered");
    }
    let deadline = tokio::time::Instant::now() + STEP;
    loop {
        match backend.list().await {
            Err(SessionError::Disconnected) => break,
            other => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "the connection stayed open: {other:?}"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    // No Windows API crate is a dependency here, and the task list answers the question.
    let output = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .expect("tasklist runs");
    String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""))
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}
