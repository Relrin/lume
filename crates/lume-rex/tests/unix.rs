//! A server on this machine's own Unix socket: named by its address, and found where a
//! server keeps it.

#![cfg(unix)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use common::{Conn, Recorded, SESSION, STREAM_BYTES, Served, World, client, serve};
use lume_rex::{Endpoint, RexBackend, dial, listens};
use lume_session::SessionBackend;
use tokio::net::UnixListener;
use tokio::sync::{Mutex, MutexGuard};
use tokio::time::timeout;

const SOON: Duration = Duration::from_secs(20);

/// The variables that say where a server's socket is belong to the whole process, so the
/// tests take turns.
static TURN: Mutex<()> = Mutex::const_new(());

/// Folders of a test's own in place of the user's, for as long as it runs.
struct Folders {
    root: PathBuf,
    _turn: MutexGuard<'static, ()>,
}

impl Folders {
    async fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(1);
        let turn = TURN.lock().await;
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        // A socket's path has to be short.
        let root = PathBuf::from(format!("/tmp/lume-rex-{}-{number}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // SAFETY: every test here holds the turn while it reads or writes the variables,
        // and nothing else runs in this process.
        unsafe {
            std::env::remove_var("REX_RUNTIME_DIR");
            std::env::remove_var("XDG_RUNTIME_DIR");
            std::env::set_var("HOME", &root);
        }
        Self { root, _turn: turn }
    }

    fn set(&self, name: &str, folder: &Path) {
        std::fs::create_dir_all(folder).unwrap();
        // SAFETY: as in `new`.
        unsafe { std::env::set_var(name, folder) };
    }
}

impl Drop for Folders {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A pretend server on a socket, which serves every connection made to it.
struct Server {
    path: PathBuf,
    taken: Arc<std::sync::Mutex<Vec<Served>>>,
    accepting: tokio::task::JoinHandle<()>,
}

impl Server {
    fn on(path: PathBuf) -> Self {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let door = UnixListener::bind(&path).unwrap();
        let taken = Arc::new(std::sync::Mutex::new(Vec::new()));
        let keep = Arc::clone(&taken);
        let accepting = tokio::spawn(async move {
            while let Ok((mut stream, _)) = door.accept().await {
                let keep = Arc::clone(&keep);
                tokio::spawn(async move {
                    let (mut near, far) = tokio::io::duplex(STREAM_BYTES);
                    tokio::spawn(async move {
                        tokio::io::copy_bidirectional(&mut stream, &mut near).await
                    });
                    // A connection that is only looked at says nothing.
                    if let Some((true, far)) = common::asks_for_control(far).await {
                        let conn = Conn::accept(far, Recorded::load().welcome()).await;
                        keep.lock().unwrap().push(serve(conn, World::recorded()));
                    }
                });
            }
        });
        Self {
            path,
            taken,
            accepting,
        }
    }

    fn connections(&self) -> usize {
        self.taken.lock().unwrap().len()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.accepting.abort();
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Connects to an address and opens the session there.
async fn opens(endpoint: Endpoint) {
    let backend = RexBackend::connect(common::backend_id(), endpoint, client());
    timeout(SOON, backend.ready()).await.unwrap().unwrap();
    backend.open_session(SESSION).await.unwrap();
    let listed = backend.list().await.unwrap();
    assert_eq!(listed.len(), 2);
    // A server on this machine names processes and folders of this machine.
    assert_eq!(listed[0].pid, Some(1000));
    backend.shutdown();
}

#[tokio::test]
async fn a_server_is_reached_at_the_socket_its_address_names() {
    let folders = Folders::new().await;
    let server = Server::on(folders.root.join("named.sock"));
    let address = format!("unix://{}", server.path.display());
    let endpoint = Endpoint::parse(&address).unwrap();
    assert!(listens(&endpoint).await);
    opens(endpoint.clone()).await;
    assert_eq!(server.connections(), 1);

    let path = server.path.clone();
    drop(server);
    assert!(!path.exists());
    assert!(!listens(&endpoint).await);
    let refused = dial(&endpoint).await.err().unwrap();
    assert_eq!(refused.code(), "unreachable");
}

#[tokio::test]
async fn the_users_own_server_is_found_where_it_keeps_its_socket() {
    let folders = Folders::new().await;
    assert!(!listens(&Endpoint::Local).await);

    // Where there is no runtime directory at all.
    let fallback = Server::on(folders.root.join(".local/state/rex/run/server.sock"));
    assert!(listens(&Endpoint::Local).await);
    opens(Endpoint::Local).await;
    assert_eq!(fallback.connections(), 1);

    // The user's runtime directory goes before that, and the one named for it before both.
    let runtime = folders.root.join("xdg");
    folders.set("XDG_RUNTIME_DIR", &runtime);
    let usual = Server::on(runtime.join("rex/server.sock"));
    opens(Endpoint::Local).await;
    assert_eq!((usual.connections(), fallback.connections()), (1, 1));

    let named = folders.root.join("named");
    folders.set("REX_RUNTIME_DIR", &named);
    let own = Server::on(named.join("server.sock"));
    opens(Endpoint::Local).await;
    assert_eq!((own.connections(), usual.connections()), (1, 1));
}

#[tokio::test]
async fn the_servers_own_socket_goes_before_others_beside_it() {
    let folders = Folders::new().await;
    let runtime = folders.root.join("run");
    folders.set("REX_RUNTIME_DIR", &runtime);
    let other = Server::on(runtime.join("login.sock"));
    let own = Server::on(runtime.join("server.sock"));
    std::fs::write(runtime.join("aaa.sock"), "no socket").unwrap();
    opens(Endpoint::Local).await;
    assert_eq!((own.connections(), other.connections()), (1, 0));
}

#[tokio::test]
async fn a_socket_nothing_listens_on_any_more_is_passed_over() {
    let folders = Folders::new().await;
    let runtime = folders.root.join("run");
    folders.set("REX_RUNTIME_DIR", &runtime);
    // Left behind by a server that was ended.
    drop(std::os::unix::net::UnixListener::bind(runtime.join("server.sock")).unwrap());
    assert!(runtime.join("server.sock").exists());
    assert!(!listens(&Endpoint::Local).await);

    let fallback = Server::on(folders.root.join(".local/state/rex/run/server.sock"));
    opens(Endpoint::Local).await;
    assert_eq!(fallback.connections(), 1);
}

#[tokio::test]
async fn a_socket_that_is_not_the_users_to_use_is_unreachable() {
    let folders = Folders::new().await;
    let server = Server::on(folders.root.join("theirs.sock"));
    let barred = || std::fs::Permissions::from_mode(0o000);
    std::fs::set_permissions(&server.path, barred()).unwrap();
    let file = folders.root.join("barred");
    std::fs::write(&file, "").unwrap();
    std::fs::set_permissions(&file, barred()).unwrap();
    if std::fs::File::open(&file).is_ok() {
        println!("skipped: run by the one account that nothing is barred to");
        return;
    }
    let endpoint = Endpoint::parse(&format!("unix://{}", server.path.display())).unwrap();
    let refused = dial(&endpoint).await.err().unwrap();
    assert_eq!(refused.code(), "unreachable");
}
