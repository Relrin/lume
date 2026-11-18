//! The local backend with the session host as a process of its own: started on
//! demand, reached again after the backend that started it has gone, restarted on
//! request; and, with `LUME_COMPAT_HOST` naming another build's host, against that
//! host.
//!
//! The host program is the one beside these tests (`target/<profile>/lume-host`), which
//! a build of the whole workspace makes. Without it the cases say so and pass.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use lume_proto::{ErrorCode, PROTO_VERSION};
use lume_session::{
    Attachment, CellPx, ClientFacts, ColorScheme, Daemon, ExitStatus, Hydrate, Launcher,
    LocalBackend, OutputFrame, Palette, SessionBackend, SessionError, SpawnSpec, TermSize,
};
use tokio::time::timeout;

/// How long any single step of a test may take before the test fails.
const STEP: Duration = Duration::from_secs(20);

const CREDIT: u32 = 1024 * 1024;

/// The variable that names another build's host to test against.
const COMPAT_HOST: &str = "LUME_COMPAT_HOST";

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
        scrollback_lines: Some(2000),
    }
}

const PRINT_MARK: &[u8] = if cfg!(windows) {
    b"echo lume-%LUME_MARK%-done\r"
} else {
    b"echo \"lume-${LUME_MARK}-done\"\r"
};

const MARK_OUTPUT: &str = "lume-k7q-done";

fn endpoint_name(what: &str) -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    format!(
        "{what}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// The host built with these tests, if it has been: in the folder of the build's
/// profile, which holds the folder these tests run from.
fn built_host() -> Option<PathBuf> {
    let tests = std::env::current_exe().ok()?;
    tests
        .ancestors()
        .skip(1)
        .map(|dir| dir.join(lume_session::daemon::HOST_PROGRAM))
        .find(|host| host.is_file())
}

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

async fn read_to_exit(attachment: &mut Attachment) -> ExitStatus {
    loop {
        match timeout(STEP, attachment.output.next()).await {
            Ok(Some(OutputFrame::Exit(status))) => return status,
            Ok(Some(_)) => {}
            other => panic!("the session's exit never arrived ({other:?})"),
        }
    }
}

/// The text of the screen an attachment started from.
fn hydrated_text(hydrate: &Hydrate) -> String {
    let terminal = match hydrate {
        Hydrate::Snapshot { bytes, .. } => lume_vt::Terminal::from_snapshot(bytes).unwrap(),
        Hydrate::VtReplay(bytes) => {
            let mut terminal = lume_vt::Terminal::new(80, 24).unwrap();
            terminal.write(bytes);
            terminal
        }
        Hydrate::None => return String::new(),
    };
    terminal.screen_text().unwrap().join("\n")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_host_is_started_reached_again_and_restarted() {
    let Some(program) = built_host() else {
        eprintln!(
            "skipped: the host is not built beside these tests ({:?})",
            std::env::current_exe()
        );
        return;
    };
    let daemon = Daemon {
        endpoint: endpoint_name("backend"),
        program,
        // Once the last backend has gone and nothing runs, the host goes too.
        idle_exit: Duration::from_secs(1),
        log: None,
        state: None,
    };

    let first = LocalBackend::daemon(daemon.clone()).await.unwrap();
    assert!(first.is_daemon());
    assert!(first.caps().persistent);
    let host_pid = first.hello().pid;
    let session = first.spawn(shell()).await.unwrap();
    let mut attachment = first.attach(session, facts()).await.unwrap();
    attachment.input.credit(CREDIT).await.unwrap();
    attachment.input.write(PRINT_MARK).await.unwrap();
    read_until(&mut attachment, MARK_OUTPUT).await;
    let status = first.host_status().await.unwrap();
    assert_eq!((status.sessions, status.clients), (1, 1));
    assert_eq!(status.idle_exit, Some(Duration::from_secs(1)));
    first.shutdown().await;
    drop(attachment);

    // Another backend finds the same host, and the session as it was.
    let second = LocalBackend::daemon(daemon.clone()).await.unwrap();
    assert_eq!(second.hello().pid, host_pid);
    let listed = second.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, session);
    let mut attachment = second.attach(session, facts()).await.unwrap();
    assert!(
        hydrated_text(&attachment.hydrate).contains(MARK_OUTPUT),
        "{}",
        hydrated_text(&attachment.hydrate)
    );
    attachment.input.credit(CREDIT).await.unwrap();

    // Restarting ends the session and leaves a new host.
    let ended = second.restart_host().await.unwrap();
    assert_eq!(ended, 1);
    assert_ne!(second.hello().pid, host_pid);
    assert!(second.list().await.unwrap().is_empty());
    // The old attachment has ended with the connection it came through.
    let end = timeout(STEP, async {
        while attachment.output.next().await.is_some() {}
    })
    .await;
    assert!(end.is_ok(), "the attachment outlived its host");

    // The new host serves sessions as the old one did.
    let session = second.spawn(shell()).await.unwrap();
    let mut attachment = second.attach(session, facts()).await.unwrap();
    attachment.input.credit(CREDIT).await.unwrap();
    attachment.input.write(PRINT_MARK).await.unwrap();
    read_until(&mut attachment, MARK_OUTPUT).await;
    second.kill(session).await.unwrap();
    read_to_exit(&mut attachment).await;
    second.shutdown().await;
}

/// A host of another build, serving one client on an endpoint of its own.
struct OtherHost(Child);

impl Drop for OtherHost {
    fn drop(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(8);
        while matches!(self.0.try_wait(), Ok(None)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs another build's host in LUME_COMPAT_HOST; run with `cargo xtask compat`"]
async fn the_backend_works_with_the_host_another_build_left_running() {
    let Some(program) = std::env::var_os(COMPAT_HOST).map(PathBuf::from) else {
        eprintln!("skipped: {COMPAT_HOST} names no host");
        return;
    };
    let name = endpoint_name("compat");
    // Every host there has been serves one client this way.
    let child = Command::new(&program)
        .args(["--endpoint", &name])
        .stdin(Stdio::null())
        .spawn()
        .expect("the other host starts");
    let _host = OtherHost(child);
    let endpoint = lume_ipc::Endpoint::named(&name).unwrap();
    let deadline = Instant::now() + STEP;
    let stream = loop {
        match lume_ipc::connect(&endpoint).await {
            Ok(stream) => break stream,
            Err(error) => {
                assert!(
                    Instant::now() < deadline,
                    "the other host never served {endpoint}: {error}"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    };

    let backend = LocalBackend::connect(stream).await.unwrap();
    let hello = backend.hello();
    println!(
        "the other host is version {}, speaking protocol {} (this build speaks {PROTO_VERSION})",
        hello.version, hello.proto
    );
    let session = backend.spawn(shell()).await.unwrap();
    let mut attachment = backend.attach(session, facts()).await.unwrap();
    attachment.input.credit(CREDIT).await.unwrap();
    attachment.input.write(PRINT_MARK).await.unwrap();
    read_until(&mut attachment, MARK_OUTPUT).await;
    attachment
        .input
        .resize(size(100, 30), CellPx { w: 9, h: 18 })
        .await
        .unwrap();

    // What the older version does not have is refused here, as unsupported.
    match backend.screen(session).await {
        Ok(screen) => assert!(hello.proto >= 2, "{screen:?}"),
        Err(SessionError::Refused(error)) => {
            assert!(hello.proto < 2);
            assert_eq!(error.code, ErrorCode::Unsupported);
        }
        Err(error) => panic!("{error}"),
    }

    backend.kill(session).await.unwrap();
    read_to_exit(&mut attachment).await;
    backend.shutdown().await;
}
