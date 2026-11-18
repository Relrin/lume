//! The host as it runs for the application: a process of its own that serves many
//! clients, keeps sessions when their clients go, stops when asked to and exits when it
//! has nothing to do.

mod support;

use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use lume_proto::{ClientKind, ErrorCode, PROTO_VERSION, RequestBody, ResponseBody};

use support::{Client, MARK_OUTPUT, PRINT_MARK, STEP};

/// A `lume-host serve` of a test's own, on an endpoint of its own.
struct Daemon {
    child: Child,
    name: String,
}

impl Daemon {
    fn start(idle_exit_s: u64) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let name = format!(
            "daemon-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let child = Self::serve(&name, idle_exit_s);
        Self { child, name }
    }

    fn serve(name: &str, idle_exit_s: u64) -> Child {
        Command::new(env!("CARGO_BIN_EXE_lume-host"))
            .args(["serve", "--endpoint", name, "--idle-exit"])
            .arg(idle_exit_s.to_string())
            .stdin(Stdio::null())
            .spawn()
            .expect("the host binary starts")
    }

    /// A new client, connected once the host serves its endpoint, that has said hello.
    async fn client(&self) -> Client {
        let endpoint = lume_ipc::Endpoint::named(&self.name).unwrap();
        let deadline = Instant::now() + STEP;
        let stream = loop {
            match lume_ipc::connect(&endpoint).await {
                Ok(stream) => break stream,
                Err(error) => {
                    assert!(
                        Instant::now() < deadline,
                        "the host never served {endpoint}: {error}"
                    );
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        };
        let mut client = Client::new(stream);
        client.hello().await;
        client
    }

    fn exited(&mut self) -> Option<std::process::ExitStatus> {
        self.child.try_wait().ok().flatten()
    }

    /// Waits for the host to exit by itself, for `limit` at most.
    async fn exits_within(&mut self, limit: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if let Some(status) = self.exited() {
                return Some(status);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        None
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        // A test that failed may have left the host running with its sessions. It is
        // asked to end them and stop, and only then ended by force: a host killed with
        // its sessions would leave their programs behind.
        if self.exited().is_none() {
            let name = self.name.clone();
            let stopped = std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                runtime.block_on(async {
                    let endpoint = lume_ipc::Endpoint::named(&name).unwrap();
                    if let Ok(stream) = lume_ipc::connect(&endpoint).await {
                        let mut client = Client::new(stream);
                        client.hello().await;
                        let _ = client
                            .request(RequestBody::Shutdown { end_sessions: true })
                            .await;
                    }
                });
            });
            let _ = stopped.join();
            let deadline = Instant::now() + Duration::from_secs(10);
            while self.exited().is_none() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn status(client: &mut Client) -> (u32, u32, bool) {
    match client.request(RequestBody::HostStatus {}).await {
        Ok(ResponseBody::HostStatus {
            sessions,
            clients,
            persistent,
            ..
        }) => (sessions, clients, persistent),
        other => panic!("not a status: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sessions_outlive_their_client_and_the_next_starts_from_the_same_screen() {
    let mut daemon = Daemon::start(0);
    let mut first = daemon.client().await;
    let session = first.spawn(support::shell()).await;
    first.attach_with_credit(session, 1).await;
    first.type_line(1, PRINT_MARK).await;
    first.read_until(1, MARK_OUTPUT).await;
    let pid = first.only_session().await.pid.expect("a running program");
    // The client goes without a word, as an application that is killed does.
    drop(first);

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        support::process_alive(pid),
        "the program ended with its client"
    );
    let mut second = daemon.client().await;
    let (sessions, clients, persistent) = status(&mut second).await;
    assert_eq!((sessions, clients, persistent), (1, 1, true));
    let listed = second.only_session().await;
    assert_eq!(listed.id, session);
    assert!(!listed.attached, "the first client's attachment is gone");

    // The new client starts from the screen the first one saw, and goes on from there.
    second.attach_with_credit(session, 1).await;
    assert!(
        second.hydrated_text(1).contains(MARK_OUTPUT),
        "{}",
        second.hydrated_text(1)
    );
    second.type_line(1, "echo again-k7q").await;
    second.read_until(1, "again-k7q").await;

    // It stops only when told to end what runs in it.
    let refused = second
        .request(RequestBody::Shutdown {
            end_sessions: false,
        })
        .await;
    assert!(
        matches!(&refused, Err(error) if error.code == ErrorCode::BadRequest),
        "{refused:?}"
    );
    let stopping = second
        .request(RequestBody::Shutdown { end_sessions: true })
        .await;
    assert!(
        matches!(stopping, Ok(ResponseBody::Stopping { ended: 1 })),
        "{stopping:?}"
    );
    assert!(second.closes().await, "the host kept the connection open");
    let exited = daemon.exits_within(STEP).await;
    assert!(exited.is_some_and(|status| status.success()), "{exited:?}");
    assert!(
        !support::process_alive(pid),
        "the program outlived the host"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clients_are_served_side_by_side() {
    let daemon = Daemon::start(0);
    let mut one = daemon.client().await;
    let mut other = daemon.client().await;
    let session = one.spawn(support::shell()).await;
    assert_eq!(other.only_session().await.id, session);
    let (_, clients, _) = status(&mut other).await;
    assert_eq!(clients, 2);
    drop(one);
    // The count follows clients that go.
    let deadline = Instant::now() + STEP;
    while status(&mut other).await.1 != 1 {
        assert!(
            Instant::now() < deadline,
            "the client that went is still counted"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_host_with_nothing_to_do_exits_and_a_host_with_a_program_does_not() {
    // Nothing to do from the start.
    let mut idle = Daemon::start(1);
    drop(idle.client().await);
    let exited = idle.exits_within(STEP).await;
    assert!(exited.is_some_and(|status| status.success()), "{exited:?}");

    // A program runs with no client.
    let mut busy = Daemon::start(1);
    let mut client = busy.client().await;
    let session = client.spawn(support::shell()).await;
    drop(client);
    assert!(
        busy.exits_within(Duration::from_secs(3)).await.is_none(),
        "the host exited with a program running in it"
    );
    // Once it has ended, the host goes.
    let mut client = busy.client().await;
    let killed = client.request(RequestBody::Kill { session }).await;
    assert!(matches!(killed, Ok(ResponseBody::Killed {})), "{killed:?}");
    drop(client);
    let exited = busy.exits_within(STEP).await;
    assert!(exited.is_some_and(|status| status.success()), "{exited:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_command_line_tool_asking_questions_does_not_keep_the_host() {
    let mut daemon = Daemon::start(1);
    let endpoint = lume_ipc::Endpoint::named(&daemon.name).unwrap();
    // Asked what it is doing, again and again, faster than its idle time.
    let started = Instant::now();
    let exited = loop {
        if let Some(status) = daemon.exited() {
            break Some(status);
        }
        if started.elapsed() > STEP {
            break None;
        }
        if let Ok(stream) = lume_ipc::connect(&endpoint).await {
            let mut client = Client::new(stream);
            let hello = RequestBody::Hello {
                proto: PROTO_VERSION,
                proto_min: PROTO_VERSION,
                vt_version: None,
                client_kind: ClientKind::Ctl,
            };
            if client.request(hello).await.is_ok() {
                let _ = client.request(RequestBody::HostStatus {}).await;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert!(exited.is_some_and(|status| status.success()), "{exited:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_host_on_a_served_endpoint_exits_at_once() {
    let daemon = Daemon::start(0);
    drop(daemon.client().await);
    let mut second = Daemon::serve(&daemon.name, 0);
    let deadline = Instant::now() + STEP;
    let status = loop {
        if let Some(status) = second.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "the second host kept running");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(status.code(), Some(3));
    // The first is not disturbed.
    let mut client = daemon.client().await;
    assert_eq!(status_pid(&mut client).await, daemon.pid());
}

async fn status_pid(client: &mut Client) -> u32 {
    match client.request(RequestBody::HostStatus {}).await {
        Ok(ResponseBody::HostStatus { pid, .. }) => pid,
        other => panic!("not a status: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_connection_that_is_not_read_does_not_hold_up_another() {
    /// Far more than a connection and the host hold for a client that reads nothing.
    const READ: usize = 8 * 1024 * 1024;
    let daemon = Daemon::start(0);
    let mut reader = daemon.client().await;
    let session = reader.spawn(support::flood()).await;
    reader.attach_with_credit(session, 1).await;

    // Another client attaches, grants all the credit a stream may hold, and then reads
    // nothing more from its connection.
    let mut stuck = daemon.client().await;
    stuck.attach_with_credit(session, 1).await;
    stuck.credit(1, 15 * 1024 * 1024).await;
    stuck.stop_reading();

    let mut received = 0;
    while received < READ {
        let bytes = reader
            .data(1, STEP)
            .await
            .expect("output goes on for the client that reads it");
        received += bytes.len();
        reader.credit(1, bytes.len() as u32).await;
    }
    drop(stuck);
    assert_eq!(
        reader.request(RequestBody::Kill { session }).await,
        Ok(ResponseBody::Killed {})
    );
}
