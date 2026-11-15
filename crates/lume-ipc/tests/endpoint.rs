//! An endpoint carries bytes between a server and its clients, and exists only once.

use std::io::ErrorKind;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use lume_ipc::{Endpoint, Listener, connect};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

const STEP: Duration = Duration::from_secs(10);

/// An endpoint name no other test or process uses.
fn unique_endpoint() -> Endpoint {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let name = format!(
        "test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    Endpoint::named(&name).unwrap()
}

#[tokio::test]
async fn a_client_reaches_the_server_and_bytes_flow_both_ways() {
    let endpoint = unique_endpoint();
    let mut listener = Listener::bind(&endpoint).unwrap();

    let server = tokio::spawn(async move {
        let mut stream = listener.accept().await.unwrap();
        let mut request = [0u8; 5];
        stream.read_exact(&mut request).await.unwrap();
        assert_eq!(&request, b"hello");
        stream.write_all(b"world").await.unwrap();
        stream.flush().await.unwrap();
        // Dropping the stream ends it for the client.
    });

    let mut client = timeout(STEP, connect(&endpoint)).await.unwrap().unwrap();
    client.write_all(b"hello").await.unwrap();
    client.flush().await.unwrap();
    let mut reply = Vec::new();
    timeout(STEP, client.read_to_end(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply, b"world");
    timeout(STEP, server).await.unwrap().unwrap();
}

#[tokio::test]
async fn clients_are_served_one_after_another_and_together() {
    let endpoint = unique_endpoint();
    let mut listener = Listener::bind(&endpoint).unwrap();

    let server = tokio::spawn(async move {
        for _ in 0..4 {
            let mut stream = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut byte = [0u8; 1];
                stream.read_exact(&mut byte).await.unwrap();
                stream.write_all(&[byte[0] + 1]).await.unwrap();
                stream.flush().await.unwrap();
            });
        }
    });

    // Two clients at once, then two more in turn.
    let ask = |byte: u8| {
        let endpoint = endpoint.clone();
        async move {
            let mut client = connect(&endpoint).await.unwrap();
            client.write_all(&[byte]).await.unwrap();
            client.flush().await.unwrap();
            let mut reply = [0u8; 1];
            client.read_exact(&mut reply).await.unwrap();
            reply[0]
        }
    };
    let (first, second) = timeout(STEP, async { tokio::join!(ask(10), ask(20)) })
        .await
        .unwrap();
    assert_eq!((first, second), (11, 21));
    assert_eq!(timeout(STEP, ask(30)).await.unwrap(), 31);
    assert_eq!(timeout(STEP, ask(40)).await.unwrap(), 41);
    timeout(STEP, server).await.unwrap().unwrap();
}

#[tokio::test]
async fn an_endpoint_that_is_being_served_cannot_be_bound_again() {
    let endpoint = unique_endpoint();
    let _listener = Listener::bind(&endpoint).unwrap();
    let error = Listener::bind(&endpoint)
        .err()
        .expect("a second bind is refused");
    assert_eq!(error.kind(), ErrorKind::AddrInUse, "{error}");
}

#[tokio::test]
async fn an_endpoint_can_be_bound_again_after_its_server_is_gone() {
    let endpoint = unique_endpoint();
    drop(Listener::bind(&endpoint).unwrap());
    let mut listener = Listener::bind(&endpoint).unwrap();

    let server = tokio::spawn(async move { listener.accept().await.map(|_| ()) });
    // Kept open until the server has it: a client that is gone before it is accepted is
    // not served.
    let client = timeout(STEP, connect(&endpoint)).await.unwrap().unwrap();
    timeout(STEP, server).await.unwrap().unwrap().unwrap();
    drop(client);
}

#[tokio::test]
async fn connecting_to_an_endpoint_nobody_serves_fails() {
    let endpoint = unique_endpoint();
    let error = timeout(STEP, connect(&endpoint))
        .await
        .unwrap()
        .err()
        .expect("nothing is there");
    assert!(
        matches!(
            error.kind(),
            ErrorKind::NotFound | ErrorKind::ConnectionRefused
        ),
        "{error}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn the_socket_and_its_directory_are_closed_to_other_users() {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use std::path::PathBuf;

    let endpoint = unique_endpoint();
    let _listener = Listener::bind(&endpoint).unwrap();
    let path = PathBuf::from(endpoint.to_string());

    let socket = std::fs::symlink_metadata(&path).unwrap();
    assert!(socket.file_type().is_socket());
    assert_eq!(socket.permissions().mode() & 0o777, 0o600);

    let dir = std::fs::symlink_metadata(path.parent().unwrap()).unwrap();
    assert!(dir.file_type().is_dir());
    assert_eq!(dir.permissions().mode() & 0o077, 0);
}

#[cfg(unix)]
#[tokio::test]
async fn a_socket_left_behind_by_a_dead_server_is_replaced() {
    use std::path::PathBuf;

    let endpoint = unique_endpoint();
    let path = PathBuf::from(endpoint.to_string());
    // Make sure the directory exists, then leave a socket nobody listens on.
    drop(Listener::bind(&endpoint).unwrap());
    let stale = std::os::unix::net::UnixListener::bind(&path).unwrap();
    drop(stale);
    assert!(
        path.exists(),
        "the standard library leaves the socket file behind"
    );

    let _listener = Listener::bind(&endpoint).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn the_socket_is_removed_when_the_server_goes() {
    use std::path::PathBuf;

    let endpoint = unique_endpoint();
    let path = PathBuf::from(endpoint.to_string());
    let listener = Listener::bind(&endpoint).unwrap();
    assert!(path.exists());
    drop(listener);
    assert!(!path.exists());
}
