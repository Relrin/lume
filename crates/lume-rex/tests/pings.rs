//! A server that goes without a word: what each connection does about a silence, and
//! how the backend and a terminal's stream come back from it.
//!
//! Time stands still in these tests and moves only when everything waits, so half a
//! minute of silence takes no time at all.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::{
    Conn, DATA_TOKEN, DataSide, Recorded, SESSION, Tell, World, client, pair, serve, streams,
};
use futures_util::StreamExt;
use lume_proto::{CellPx, ClientFacts, ColorScheme, Palette, TermSize};
use lume_rex::tuning::{PING_EVERY, PING_WAIT};
use lume_rex::{Control, DataConn, DataEnd, DataOpen, Link, RexBackend, RexError, Ticket};
use lume_session::SessionBackend;
use serde_json::json;
use tokio::io::DuplexStream;
use tokio::time::Instant;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

/// How long a server may say nothing before a connection gives it up.
const SILENCE: Duration = PING_EVERY.saturating_add(PING_WAIT);

/// Reads a server's side of a connection for as long as it lasts, which answers the
/// pings that arrive, and counts them.
fn answering(mut socket: WebSocketStream<DuplexStream>) -> Arc<AtomicUsize> {
    let pings = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&pings);
    tokio::spawn(async move {
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Ping(_) = message {
                counted.fetch_add(1, Ordering::Relaxed);
            }
        }
    });
    pings
}

async fn control() -> (Control, Conn) {
    let (near, far) = pair();
    let server = tokio::spawn(Conn::accept(far, Recorded::load().welcome()));
    let (control, _events) = Control::open(near, "test", &client(), None).await.unwrap();
    (control, server.await.unwrap())
}

async fn data() -> (DataConn, DataSide) {
    let (near, far) = pair();
    let server = tokio::spawn(DataSide::accept(far));
    let ticket = Ticket::read(common::TICKET).unwrap();
    let conn = DataConn::open(near, "test", DataOpen::Start(&ticket))
        .await
        .unwrap();
    (conn, server.await.unwrap())
}

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

#[tokio::test(start_paused = true)]
async fn a_quiet_server_that_answers_is_kept() {
    let (control, conn) = control().await;
    let pings = answering(conn.socket);

    tokio::time::sleep(Duration::from_secs(600)).await;
    assert!(control.ended().is_none());
    // It was asked about each silence, and no more often than that.
    let asked = pings.load(Ordering::Relaxed);
    assert!((15..=20).contains(&asked), "{asked} pings");
}

#[tokio::test(start_paused = true)]
async fn a_server_that_no_longer_answers_is_given_up() {
    // The server holds the connection and reads nothing of it.
    let (control, conn) = control().await;
    let start = Instant::now();
    while control.ended().is_none() {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let waited = start.elapsed();
    assert!(
        waited >= SILENCE && waited < SILENCE + Duration::from_secs(2),
        "{waited:?}"
    );
    assert!(matches!(control.ended(), Some(RexError::Disconnected)));
    let answer = control.call("server.version", json!(null)).await;
    assert!(matches!(answer, Err(RexError::Disconnected)), "{answer:?}");
    drop(conn);
}

#[tokio::test(start_paused = true)]
async fn what_a_server_says_puts_the_question_off() {
    let (control, mut conn) = control().await;
    // An event every twenty seconds: the server is plainly there, and is not asked.
    let saying = tokio::spawn(async move {
        let mut pings = 0;
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_secs(20)).await;
            conn.event("keymap_changed", json!({})).await;
            while let Ok(Some(Ok(message))) =
                tokio::time::timeout(Duration::from_millis(1), conn.socket.next()).await
            {
                if let Message::Ping(_) = message {
                    pings += 1;
                }
            }
        }
        (pings, conn)
    });
    let (pings, _conn) = saying.await.unwrap();
    assert_eq!(pings, 0);
    assert!(control.ended().is_none());
}

#[tokio::test(start_paused = true)]
async fn a_quiet_stream_whose_server_answers_is_kept() {
    let (mut conn, side) = data().await;
    let pings = answering(side.socket);

    // A terminal that prints nothing for ten minutes is still a terminal.
    let waited = tokio::time::timeout(Duration::from_secs(600), conn.next()).await;
    assert!(waited.is_err(), "{waited:?}");
    assert!(pings.load(Ordering::Relaxed) >= 15);
}

#[tokio::test(start_paused = true)]
async fn a_stream_whose_server_no_longer_answers_is_to_be_continued() {
    let (mut conn, side) = data().await;
    let start = Instant::now();
    let end = conn.next().await;
    assert!(matches!(end, Err(DataEnd::Dropped)), "{end:?}");
    let waited = start.elapsed();
    assert!(
        waited >= SILENCE && waited < SILENCE + Duration::from_secs(2),
        "{waited:?}"
    );
    drop(side);
}

#[tokio::test]
async fn a_backend_whose_server_goes_silent_connects_again() {
    let (dialer, mut listener) = streams();
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
    let served = serve(listener.accept().await, World::recorded());
    backend.ready().await.unwrap();

    tokio::time::pause();
    served.tell(Tell::Stall);
    tokio::time::sleep(SILENCE + Duration::from_secs(1)).await;
    assert!(
        !matches!(&*backend.link().borrow(), Link::Connected(_)),
        "the backend still thinks it is connected"
    );

    let again = serve(listener.accept().await, World::recorded());
    let mut link = backend.link();
    link.wait_for(|link| matches!(link, Link::Connected(_)))
        .await
        .unwrap();
    assert_eq!(again.called()[0], "server.version");
}

#[tokio::test]
async fn a_terminal_whose_stream_goes_silent_is_continued_on_a_new_connection() {
    let (dialer, mut listener) = streams();
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
    let _served = serve(listener.accept().await, World::recorded());
    backend.ready().await.unwrap();
    backend.open_session(SESSION).await.unwrap();
    let id = backend.list().await.unwrap()[0].id;
    let attaching = {
        let backend = Arc::clone(&backend);
        tokio::spawn(async move { backend.attach(id, facts()).await })
    };
    let mut side = DataSide::accept(listener.stream().await).await;
    let terminal = lume_vt::Terminal::new(80, 24).unwrap();
    side.bytes(&terminal.snapshot().unwrap()).await;
    side.size(80, 24).await;
    let _attachment = attaching.await.unwrap().unwrap();
    let sent = side.sent;

    // The control connection goes on being answered; only the stream's server stops.
    tokio::time::pause();
    tokio::time::sleep(SILENCE + Duration::from_secs(1)).await;
    let again = DataSide::shake(listener.stream().await).await;
    let mut resume = vec![0x02];
    resume.extend(DATA_TOKEN);
    resume.extend(sent.to_le_bytes());
    assert_eq!(again.first, resume);
    drop(side);
}
