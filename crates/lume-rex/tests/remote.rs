//! A server reached over a network: the pretend server behind a real socket on this
//! machine, and what is not dialled at all.

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use common::{DataSide, SESSION, Tell, World, client, door, serve};
use lume_proto::{CellPx, ClientFacts, ColorScheme, Palette, TermSize};
use lume_rex::{Endpoint, Link, Options, RexBackend, RexError, dial_with};
use lume_session::{Hydrate, SessionBackend};

fn facts() -> ClientFacts {
    ClientFacts {
        size: TermSize::new(80, 24).unwrap(),
        cell_px: CellPx { w: 8, h: 16 },
        palette: Palette::default(),
        scheme: ColorScheme::Dark,
        focused: true,
        visible: true,
    }
}

fn connect(address: &str) -> Arc<RexBackend> {
    let endpoint = Endpoint::parse(address).unwrap();
    RexBackend::connect(common::backend_id(), endpoint, client())
}

/// Waits until the backend stands with its server as `wanted` says, and gives how.
async fn link(backend: &RexBackend, wanted: impl Fn(&Link) -> bool) -> Link {
    let mut link = backend.link();
    let reached = tokio::time::timeout(Duration::from_secs(20), link.wait_for(|link| wanted(link)))
        .await
        .expect("the backend never stood so with its server")
        .unwrap();
    reached.clone()
}

/// An address on this machine that nothing listens on.
async fn nowhere() -> SocketAddr {
    let door = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    door.local_addr().unwrap()
}

#[tokio::test]
async fn a_server_on_a_port_of_this_machine_is_reached_and_its_terminal_shown() {
    let (address, mut listener) = door().await;
    let backend = connect(&format!("http://{address}"));
    let conn = listener.accept().await;
    // The server is asked for by the address it was found at.
    assert_eq!(conn.host.as_deref(), Some(address.to_string().as_str()));
    let served = serve(conn, World::recorded());

    let version = backend.ready().await.unwrap();
    assert_eq!(version.api_revision, 19);
    assert_eq!(backend.sessions()[0].id, SESSION);
    assert_eq!(served.called(), ["server.version", "session.attach_all"]);

    // A terminal's stream is a connection of its own to the same address.
    backend.open_session(SESSION).await.unwrap();
    let id = backend.list().await.unwrap()[0].id;
    let attaching = {
        let backend = Arc::clone(&backend);
        tokio::spawn(async move { backend.attach(id, facts()).await })
    };
    let mut side = DataSide::accept(listener.stream().await).await;
    assert_eq!(side.host.as_deref(), Some(address.to_string().as_str()));
    let mut terminal = lume_vt::Terminal::new(80, 24).unwrap();
    terminal.write(b"over the network");
    side.bytes(&terminal.snapshot().unwrap()).await;
    side.size(80, 24).await;
    let attachment = attaching.await.unwrap().unwrap();
    assert!(matches!(attachment.hydrate, Hydrate::Snapshot { .. }));
}

#[tokio::test]
async fn a_server_found_by_name_is_asked_for_by_that_name() {
    let (address, mut listener) = door().await;
    let backend = connect(&format!("http://LocalHost:{}", address.port()));
    let conn = listener.accept().await;
    assert_eq!(
        conn.host.as_deref(),
        Some(format!("localhost:{}", address.port()).as_str())
    );
    let _served = serve(conn, World::recorded());
    backend.ready().await.unwrap();
}

#[tokio::test]
async fn a_connection_over_a_network_that_is_lost_is_made_again() {
    let (address, mut listener) = door().await;
    let backend = connect(&format!("http://{address}"));
    let served = serve(listener.accept().await, World::recorded());
    backend.ready().await.unwrap();

    served.tell(Tell::Drop);
    let again = serve(listener.accept().await, World::recorded());
    link(&backend, |link| matches!(link, Link::Connected(_))).await;
    assert_eq!(again.called()[0], "server.version");
}

#[tokio::test]
async fn a_port_nothing_listens_on_is_tried_again_and_not_given_up() {
    let address = nowhere().await;
    let backend = connect(&format!("http://{address}"));
    let waiting = link(
        &backend,
        |link| matches!(link, Link::Waiting { attempts, .. } if *attempts >= 2),
    )
    .await;
    let Link::Waiting { error, .. } = waiting else {
        unreachable!()
    };
    assert!(error.contains(&format!("http://{address}")), "{error}");
}

#[tokio::test]
async fn nothing_unencrypted_is_dialled_elsewhere_until_the_user_agrees() {
    // An address that is not this machine's. Nothing is sent to it: the refusal comes
    // before a stream is opened.
    let endpoint = Endpoint::parse("http://192.0.2.1:7297").unwrap();
    let started = std::time::Instant::now();
    let Err(refused) = dial_with(&endpoint, &Options::new()).await else {
        panic!("a stream was opened");
    };
    assert!(matches!(refused, RexError::Unconfirmed { .. }), "{refused}");
    assert_eq!(refused.code(), "confirmation_required");
    assert!(!refused.is_transient());
    assert!(started.elapsed() < Duration::from_secs(2));

    // A backend for it stops there, and does not try again.
    let backend = RexBackend::connect(common::backend_id(), endpoint, client());
    let stood = link(&backend, |link| matches!(link, Link::Refused { .. })).await;
    let Link::Refused { code, error } = stood else {
        unreachable!()
    };
    assert_eq!(code, "confirmation_required");
    assert!(error.contains("http://192.0.2.1:7297"), "{error}");
    assert!(backend.ready().await.is_err());
}

#[tokio::test]
async fn what_the_user_agreed_to_is_dialled() {
    // This machine's own address needs no agreement; that the agreement is carried to
    // the dial is seen by the same server being reached either way.
    let (address, mut listener) = door().await;
    let endpoint = Endpoint::parse(&format!("http://{address}")).unwrap();
    let options = Options::new().plain_http_agreed(true);
    let backend = RexBackend::connect_with(common::backend_id(), endpoint, client(), options);
    let _served = serve(listener.accept().await, World::recorded());
    backend.ready().await.unwrap();
}

#[tokio::test]
async fn a_tailnet_address_that_is_not_on_a_tailnet_is_not_dialled() {
    let (address, _listener) = door().await;
    for address in [
        format!("tailnet://{address}"),
        "tailnet://192.0.2.1:7297".to_owned(),
    ] {
        let endpoint = Endpoint::parse(&address).unwrap();
        // The user's agreement to an unencrypted connection is not what is missing.
        let options = Options::new().plain_http_agreed(true);
        let Err(refused) = dial_with(&endpoint, &options).await else {
            panic!("a stream was opened to {address}");
        };
        assert_eq!(refused.code(), "unsupported", "{address}");
        assert!(!refused.is_transient());
    }
}
