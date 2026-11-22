//! The data connection against a pretend server: the handshake, the stream, and
//! everything a server can do wrong on the way.

mod common;

use std::future::Future;
use std::time::Duration;

use common::{DATA_TOKEN, DataSide, NONCE, TICKET, pair};
use futures_util::SinkExt;
use lume_rex::tuning::DATA_MESSAGE_MAX_BYTES;
use lume_rex::{
    Answer, DataConn, DataEnd, DataOpen, Note, Opening, Piece, Reject, RexError, Splitter, Ticket,
    read_answer,
};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio_tungstenite::tungstenite::Message;

fn ticket() -> Ticket {
    Ticket::read(TICKET).unwrap()
}

/// Opens a data connection against a server that does `server` with its side.
async fn open_against<F, Fut>(server: F) -> Result<DataConn, RexError>
where
    F: FnOnce(DuplexStream) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (near, far) = pair();
    tokio::spawn(server(far));
    let ticket = ticket();
    DataConn::open(near, "test", DataOpen::Start(&ticket)).await
}

/// A connection that was welcomed, and the server's side of it.
async fn welcomed() -> (DataConn, DataSide) {
    let (near, far) = pair();
    let server = tokio::spawn(DataSide::accept(far));
    let ticket = ticket();
    let conn = DataConn::open(near, "test", DataOpen::Start(&ticket))
        .await
        .unwrap();
    (conn, server.await.unwrap())
}

#[tokio::test]
async fn a_stream_is_started_with_its_ticket_and_read_piece_by_piece() {
    let (mut conn, mut side) = welcomed().await;
    assert_eq!(side.asked.as_deref(), Some("rex.data.terminal.v1"));
    let mut hello = vec![0x01];
    hello.extend(0u8..16);
    assert_eq!(side.first, hello);
    assert_eq!(conn.welcome().nonce, NONCE);
    assert_eq!(conn.welcome().keep, Duration::from_secs(60));

    side.bytes(b"a copy of the terminal").await;
    side.note(NONCE, json!({ "type": "resize", "cols": 80, "rows": 24 }))
        .await;
    side.bytes(b"hello").await;
    // A message that holds nothing says nothing.
    side.bytes(b"").await;
    side.bytes(b" world").await;
    // The close is finished only once the client has read it.
    tokio::spawn(side.close(1000, ""));

    let mut opening = Opening::new(conn.welcome().nonce);
    let mut splitter = Splitter::new(conn.welcome().nonce);
    let mut opened = None;
    let mut output = Vec::new();
    let mut sizes = Vec::new();
    let mut received = 0;
    let end = loop {
        let bytes = match conn.next().await {
            Ok(bytes) => bytes,
            Err(end) => break end,
        };
        received += bytes.len();
        let rest = match &opened {
            Some(_) => bytes,
            None => match opening.feed(&bytes) {
                Some(found) => {
                    opened = Some(found.snapshot);
                    found.rest
                }
                None => continue,
            },
        };
        for piece in splitter.feed(&rest) {
            match piece {
                Piece::Output(bytes) => output.extend(bytes),
                Piece::Note(Note::Resize { size, .. }) => sizes.push((size.cols(), size.rows())),
                Piece::Note(other) => panic!("{other:?}"),
            }
        }
    };
    assert!(matches!(end, DataEnd::Ended), "{end:?}");
    assert_eq!(
        opened.unwrap().as_deref(),
        Some(&b"a copy of the terminal"[..])
    );
    assert_eq!(sizes, [(80, 24)]);
    assert_eq!(output, b"hello world");
    assert!(received > output.len());
}

#[tokio::test]
async fn a_stream_is_continued_with_its_token_and_what_was_received() {
    let (conn, side) = welcomed().await;
    let token = conn.welcome().token.clone();
    drop(side);
    drop(conn);

    let (near, far) = pair();
    let server = tokio::spawn(async move {
        let mut side = DataSide::shake(far).await;
        side.welcome([0x5a; 16], 60_000, NONCE).await;
        side
    });
    let conn = DataConn::open(
        near,
        "test",
        DataOpen::Resume {
            token: &token,
            received: 70_000,
        },
    )
    .await
    .unwrap();
    let side = server.await.unwrap();
    let mut resume = vec![0x02];
    resume.extend(DATA_TOKEN);
    resume.extend(70_000u64.to_le_bytes());
    assert_eq!(side.first, resume);
    // The welcome's token is the one to continue with next time.
    assert_ne!(conn.welcome().token, token);
}

#[tokio::test]
async fn every_rejection_is_told_apart() {
    let reasons = [
        (0x01, Reject::BadTicket),
        (0x02, Reject::AlreadyConnected),
        (0x03, Reject::Unavailable),
        (0x04, Reject::Limit),
        (0x05, Reject::Malformed),
        (0x06, Reject::ServerFailed),
        (0x07, Reject::Subprotocol),
        (0x7f, Reject::Other(0x7f)),
    ];
    for (byte, reason) in reasons {
        let opened =
            open_against(move |far| async move { DataSide::shake(far).await.reject(byte).await })
                .await;
        match opened {
            Err(RexError::Rejected(said)) => assert_eq!(said, reason),
            Err(other) => panic!("{byte:#x}: {other:?}"),
            Ok(_) => panic!("{byte:#x}: taken"),
        }
    }
}

#[tokio::test]
async fn a_close_after_the_welcome_says_what_became_of_the_stream() {
    type Is = fn(&DataEnd) -> bool;
    let cases: [(u16, &str, Is); 8] = [
        (1000, "", |end| matches!(end, DataEnd::Ended)),
        (1001, "keepalive timeout", |end| {
            matches!(end, DataEnd::Dropped)
        }),
        (1001, "data provider canceled", |end| {
            matches!(end, DataEnd::Restart)
        }),
        (1013, "data reconnect required", |end| {
            matches!(end, DataEnd::Restart)
        }),
        (4000, "block closed", |end| {
            matches!(end, DataEnd::BlockClosed)
        }),
        (1008, "data provider unavailable", |end| {
            matches!(end, DataEnd::Failed(RexError::Closed { code: 1008, .. }))
        }),
        (1011, "data provider failed", |end| {
            matches!(end, DataEnd::Failed(RexError::Closed { code: 1011, .. }))
        }),
        (4321, "something new", |end| {
            matches!(end, DataEnd::Failed(RexError::Closed { code: 4321, .. }))
        }),
    ];
    for (code, reason, is) in cases {
        let (mut conn, mut side) = welcomed().await;
        side.bytes(b"some output").await;
        tokio::spawn(side.close(code, reason));
        assert_eq!(conn.next().await.unwrap(), b"some output");
        let end = conn.next().await.unwrap_err();
        assert!(is(&end), "{code} {reason}: {end:?}");
    }
}

#[tokio::test]
async fn a_connection_that_drops_leaves_its_stream_to_be_continued() {
    let (mut conn, mut side) = welcomed().await;
    side.bytes(b"so far").await;
    assert_eq!(conn.next().await.unwrap(), b"so far");
    drop(side);
    assert!(matches!(conn.next().await, Err(DataEnd::Dropped)));
    // And goes on saying so.
    assert!(matches!(conn.next().await, Err(DataEnd::Dropped)));
}

#[tokio::test]
async fn closing_a_connection_ends_its_stream_properly() {
    let (conn, mut side) = welcomed().await;
    let closed = tokio::spawn(async move { side.closed_by_client().await });
    conn.close().await;
    assert_eq!(closed.await.unwrap(), Some(1000));
}

#[tokio::test]
async fn what_is_not_a_server_or_does_not_agree_is_refused() {
    // Not a WebSocket at all.
    let opened = open_against(|mut far| async move {
        let mut asked = [0; 16];
        let _ = far.read(&mut asked).await;
        let _ = far.write_all(b"SSH-2.0-OpenSSH_9.9\r\n").await;
    })
    .await;
    assert!(matches!(opened, Err(RexError::Protocol(_))));

    // A server that does not let this user in.
    let opened = open_against(|mut far| async move {
        let mut asked = [0; 1024];
        let _ = far.read(&mut asked).await;
        let body = r#"{"error":"connection was not classified by its listener"}"#;
        let answer = format!(
            "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let _ = far.write_all(answer.as_bytes()).await;
    })
    .await;
    match opened {
        Err(RexError::Refused { reason, .. }) => assert!(reason.contains("classified")),
        Err(other) => panic!("{other:?}"),
        Ok(_) => panic!("taken"),
    }

    // A WebSocket that speaks something else.
    let opened = open_against(|far| async move {
        let _ = tokio_tungstenite::accept_async(far).await;
    })
    .await;
    assert!(matches!(opened, Err(RexError::Protocol(_))));
}

#[tokio::test]
async fn an_answer_that_is_not_one_is_refused() {
    let answers: [&[u8]; 6] = [
        // Nothing, a welcome cut short, one too long, one that marks nothing, a
        // message of another kind, and a rejection without a reason.
        b"",
        &[0x81; 31],
        &[0x81; 33],
        &[
            0x81, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0x60, 0xea, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0,
        ],
        &[0x01; 32],
        &[0x82],
    ];
    for answer in answers {
        assert!(read_answer(answer).is_err());
        let opened = open_against(move |far| async move {
            let mut side = DataSide::shake(far).await;
            side.bytes(answer).await;
            let _ = side.closed_by_client().await;
        })
        .await;
        assert!(
            matches!(opened, Err(RexError::Protocol(_))),
            "{answer:?}: {:?}",
            opened.err()
        );
    }

    // Text where the answer should be.
    let opened = open_against(|far| async move {
        let mut side = DataSide::shake(far).await;
        let _ = side.socket.send(Message::text("welcome")).await;
        let _ = side.closed_by_client().await;
    })
    .await;
    assert!(matches!(opened, Err(RexError::Protocol(_))));

    // A close with no answer, and a server that is simply gone.
    let opened = open_against(|far| async move {
        DataSide::shake(far)
            .await
            .close(1011, "data provider failed")
            .await;
    })
    .await;
    assert!(matches!(opened, Err(RexError::Closed { code: 1011, .. })));
    let opened = open_against(|far| async move { drop(DataSide::shake(far).await) }).await;
    assert!(matches!(opened, Err(RexError::Disconnected)));
}

#[tokio::test(start_paused = true)]
async fn a_server_that_never_answers_is_not_waited_for_forever() {
    let opened = open_against(|far| async move {
        let mut side = DataSide::shake(far).await;
        let _ = side.closed_by_client().await;
    })
    .await;
    assert!(matches!(opened, Err(RexError::Timeout(_))));
}

#[tokio::test]
async fn what_a_stream_may_not_hold_ends_the_connection() {
    // Text among the stream.
    let (mut conn, mut side) = welcomed().await;
    let _ = side.socket.send(Message::text("output")).await;
    assert!(matches!(
        conn.next().await,
        Err(DataEnd::Failed(RexError::Protocol(_)))
    ));

    // A piece as large as a piece may be is taken, a larger one is not.
    let (mut conn, mut side) = welcomed().await;
    side.bytes(&vec![b'x'; DATA_MESSAGE_MAX_BYTES]).await;
    side.bytes(&vec![b'x'; DATA_MESSAGE_MAX_BYTES + 1]).await;
    assert_eq!(conn.next().await.unwrap().len(), DATA_MESSAGE_MAX_BYTES);
    assert!(matches!(
        conn.next().await,
        Err(DataEnd::Failed(RexError::Protocol(_)))
    ));

    // An answer that is a welcome is still read as one afterwards: nothing after the
    // first is taken for the handshake.
    let (mut conn, mut side) = welcomed().await;
    let mut again = vec![0x81];
    again.extend([0; 31]);
    side.bytes(&again).await;
    assert_eq!(conn.next().await.unwrap(), again);
    assert!(matches!(
        read_answer(&[0x82, 0x01]),
        Ok(Answer::Reject(Reject::BadTicket))
    ));
}
