//! A data connection: one WebSocket that carries the bytes one block produces.
//!
//! A client asks for a ticket on its control connection, opens a data connection and
//! sends the ticket in its first message. The server answers with a welcome or a
//! rejection, and after a welcome every message is a piece of the block's stream. Bytes
//! flow from the server only.
//!
//! A stream outlives its connection for a while. When a connection drops, a new one
//! that names the welcome's token and the number of bytes taken so far continues the
//! stream where it stopped.

use std::fmt;
use std::time::Duration;

use data_encoding::BASE64URL_NOPAD;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::error::ProtocolError;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

use crate::error::RexError;
use crate::pulse::{Beat, Pulse};
use crate::text::plain;
use crate::transport::{Io, LOCAL_AUTHORITY};
use crate::tuning::{
    DATA_ANSWER_TIMEOUT, DATA_CLOSE_TIMEOUT, DATA_MESSAGE_MAX_BYTES, DATA_PATH,
    TERMINAL_SUBPROTOCOL,
};
use crate::ws;

const HELLO: u8 = 0x01;
const RESUME: u8 = 0x02;
const WELCOME: u8 = 0x81;
const REJECT: u8 = 0x82;

/// A welcome is always exactly this long.
const WELCOME_BYTES: usize = 32;
/// A ticket as text: 16 bytes in base64url, without padding.
const TICKET_CHARS: usize = 22;

fn protocol(what: impl Into<String>) -> RexError {
    RexError::Protocol(what.into())
}

/// What opens a data connection, once. Never written anywhere.
#[derive(Clone, PartialEq, Eq)]
pub struct Ticket([u8; 16]);

impl Ticket {
    /// Reads a ticket as `data.connect` returns it.
    pub fn read(text: &str) -> Result<Self, RexError> {
        let not_a_ticket = || protocol("the server gave a ticket that is not one");
        if text.len() != TICKET_CHARS {
            return Err(not_a_ticket());
        }
        let bytes = BASE64URL_NOPAD
            .decode(text.as_bytes())
            .map_err(|_| not_a_ticket())?;
        <[u8; 16]>::try_from(bytes)
            .map(Self)
            .map_err(|_| not_a_ticket())
    }
}

impl fmt::Debug for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Ticket(…)")
    }
}

/// What continues a stream on a new connection. Never written anywhere.
#[derive(Clone, PartialEq, Eq)]
pub struct ResumeToken([u8; 16]);

impl fmt::Debug for ResumeToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ResumeToken(…)")
    }
}

/// Why a server refused a data connection at its handshake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    /// The ticket or token is unknown, has expired, was used before or is someone
    /// else's, or its session is gone.
    BadTicket,
    /// This client already has a connection to that stream.
    AlreadyConnected,
    /// The stream is not there: its block has closed, or fewer bytes were counted than
    /// the server sent.
    Unavailable,
    /// The server has as many connections as it takes.
    Limit,
    /// The first message was not as it should be or came too late, or more bytes were
    /// counted than the server sent.
    Malformed,
    /// The server failed.
    ServerFailed,
    /// The ticket or token was given for another subprotocol.
    Subprotocol,
    /// A reason this client does not know.
    Other(u8),
}

impl Reject {
    fn read(reason: u8) -> Self {
        match reason {
            0x01 => Reject::BadTicket,
            0x02 => Reject::AlreadyConnected,
            0x03 => Reject::Unavailable,
            0x04 => Reject::Limit,
            0x05 => Reject::Malformed,
            0x06 => Reject::ServerFailed,
            0x07 => Reject::Subprotocol,
            other => Reject::Other(other),
        }
    }
}

impl fmt::Display for Reject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reject::BadTicket => f.write_str("the ticket is not valid, or no longer"),
            Reject::AlreadyConnected => f.write_str("the stream is already connected"),
            Reject::Unavailable => f.write_str("the stream is not available"),
            Reject::Limit => f.write_str("the server has too many connections"),
            Reject::Malformed => f.write_str("the handshake was not as it should be"),
            Reject::ServerFailed => f.write_str("the server failed"),
            Reject::Subprotocol => f.write_str("the ticket is for another subprotocol"),
            Reject::Other(reason) => write!(f, "reason {reason:#04x}"),
        }
    }
}

/// What a server answers the first message with when it takes the connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataWelcome {
    /// What continues the stream on a new connection.
    pub token: ResumeToken,
    /// For how long after a connection drops the server keeps the stream.
    pub keep: Duration,
    /// The number the stream's own messages are marked with. Never zero, and the same
    /// for as long as the stream lives.
    pub nonce: u64,
}

/// A server's answer to the first message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Welcome(DataWelcome),
    Reject(Reject),
}

/// The first message of a connection that starts a stream.
pub fn hello(ticket: &Ticket) -> [u8; 17] {
    let mut message = [0; 17];
    message[0] = HELLO;
    message[1..].copy_from_slice(&ticket.0);
    message
}

/// The first message of a connection that continues a stream of which `received` bytes
/// were taken.
pub fn resume(token: &ResumeToken, received: u64) -> [u8; 25] {
    let mut message = [0; 25];
    message[0] = RESUME;
    message[1..17].copy_from_slice(&token.0);
    message[17..].copy_from_slice(&received.to_le_bytes());
    message
}

/// Reads a server's answer to the first message.
pub fn read_answer(message: &[u8]) -> Result<Answer, RexError> {
    match message {
        [WELCOME, rest @ ..] => {
            if message.len() != WELCOME_BYTES {
                return Err(protocol("the welcome of a data connection is not 32 bytes"));
            }
            let (token, rest) = rest.split_at(16);
            // Three bytes that say nothing yet.
            let (keep, nonce) = rest[3..].split_at(4);
            let read = || protocol("the welcome of a data connection is not as described");
            let token = <[u8; 16]>::try_from(token).map_err(|_| read())?;
            let keep = u32::from_le_bytes(keep.try_into().map_err(|_| read())?);
            let nonce = u64::from_le_bytes(nonce.try_into().map_err(|_| read())?);
            if nonce == 0 {
                return Err(protocol("the welcome of a data connection marks nothing"));
            }
            Ok(Answer::Welcome(DataWelcome {
                token: ResumeToken(token),
                keep: Duration::from_millis(u64::from(keep)),
                nonce,
            }))
        }
        [REJECT, reason, ..] => Ok(Answer::Reject(Reject::read(*reason))),
        _ => Err(protocol(
            "the server answered a data connection with neither a welcome nor a rejection",
        )),
    }
}

/// How a data connection is opened.
#[derive(Clone, Copy, Debug)]
pub enum DataOpen<'a> {
    /// A new stream, with a ticket from `data.connect`.
    Start(&'a Ticket),
    /// The stream of a connection that dropped, of which `received` bytes were taken.
    /// The count must be exact: the server ends the stream otherwise.
    Resume {
        token: &'a ResumeToken,
        received: u64,
    },
}

/// Why a data connection gave no more of its stream.
#[derive(Debug)]
pub enum DataEnd {
    /// The stream is over: nothing more will come of it.
    Ended,
    /// The connection was lost. The server keeps the stream for a while, to be
    /// continued on a new connection.
    Dropped,
    /// The server let go of the stream. A new one can be started with a new ticket.
    Restart,
    /// The block closed. There is nothing to connect to any more.
    BlockClosed,
    /// The stream cannot be had.
    Failed(RexError),
}

/// A data connection to a Rex server, past its handshake.
///
/// Dropping it leaves the stream waiting on the server to be continued.
/// [`close`](Self::close) ends the stream.
pub struct DataConn {
    socket: WebSocketStream<Box<dyn Io>>,
    welcome: DataWelcome,
    pulse: Pulse,
}

/// What a close frame, or the lack of one, means for the stream.
fn end_of_close(frame: Option<CloseFrame>) -> DataEnd {
    let Some(frame) = frame else {
        return DataEnd::Dropped;
    };
    let code = u16::from(frame.code);
    let reason = frame.reason.as_str();
    match (code, reason) {
        (1000, _) => DataEnd::Ended,
        // The server stopped the stream: it is shutting down, or the client that opened
        // the stream has left it.
        (1001, "data provider canceled") => DataEnd::Restart,
        // The server heard nothing from this side for too long.
        (1001, _) => DataEnd::Dropped,
        (1013, _) => DataEnd::Restart,
        (4000, _) => DataEnd::BlockClosed,
        _ => {
            let reason = plain(reason);
            DataEnd::Failed(RexError::Closed { code, reason })
        }
    }
}

fn end_of_error(error: &WsError) -> DataEnd {
    match error {
        WsError::ConnectionClosed
        | WsError::AlreadyClosed
        | WsError::Io(_)
        | WsError::Protocol(ProtocolError::ResetWithoutClosingHandshake) => DataEnd::Dropped,
        WsError::Capacity(_) => DataEnd::Failed(protocol(
            "the server sent more of a stream at once than it may",
        )),
        _ => DataEnd::Failed(protocol("the server broke the WebSocket protocol")),
    }
}

impl DataConn {
    /// Opens a data connection for a terminal's stream over a stream to a server: the
    /// WebSocket handshake, the first message and the server's answer. `endpoint` names
    /// the server in what is said of failures.
    ///
    /// A ticket is used up by this, whether or not the server takes it.
    pub async fn open(io: impl Io, endpoint: &str, how: DataOpen<'_>) -> Result<Self, RexError> {
        Self::open_at(io, endpoint, LOCAL_AUTHORITY, how).await
    }

    /// Opens a data connection as [`open`](Self::open) does, to a server that is asked
    /// for by `authority`: its host, and its port where that is not the usual one.
    pub async fn open_at(
        io: impl Io,
        endpoint: &str,
        authority: &str,
        how: DataOpen<'_>,
    ) -> Result<Self, RexError> {
        let io: Box<dyn Io> = Box::new(io);
        let mut socket = ws::open(
            io,
            endpoint,
            authority,
            DATA_PATH,
            TERMINAL_SUBPROTOCOL,
            DATA_MESSAGE_MAX_BYTES,
        )
        .await?;
        let first = match how {
            DataOpen::Start(ticket) => hello(ticket).to_vec(),
            DataOpen::Resume { token, received } => resume(token, received).to_vec(),
        };
        socket
            .send(Message::binary(first))
            .await
            .map_err(|_| RexError::Disconnected)?;
        let answer = tokio::time::timeout(DATA_ANSWER_TIMEOUT, async {
            loop {
                match socket.next().await {
                    Some(Ok(Message::Binary(message))) => break read_answer(&message),
                    Some(Ok(Message::Close(frame))) => {
                        break Err(match end_of_close(frame) {
                            DataEnd::Failed(error) => error,
                            _ => RexError::Disconnected,
                        });
                    }
                    Some(Ok(Message::Text(_))) => {
                        break Err(protocol("the server answered a data connection with text"));
                    }
                    // Pings are answered by the socket itself.
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        break Err(match end_of_error(&error) {
                            DataEnd::Failed(error) => error,
                            _ => RexError::Disconnected,
                        });
                    }
                    None => break Err(RexError::Disconnected),
                }
            }
        })
        .await
        .map_err(|_| RexError::Timeout("the data connection".to_owned()))??;
        match answer {
            Answer::Welcome(welcome) => Ok(Self {
                socket,
                welcome,
                pulse: Pulse::new(),
            }),
            Answer::Reject(reason) => Err(RexError::Rejected(reason)),
        }
    }

    /// What the server answered the first message with.
    pub fn welcome(&self) -> &DataWelcome {
        &self.welcome
    }

    /// The next piece of the stream. Where one piece ends and the next begins means
    /// nothing. Every byte returned counts towards what a later
    /// [`DataOpen::Resume`] says was received.
    ///
    /// A server that sends nothing for a while is asked whether it is there, and the
    /// stream is [`Dropped`](DataEnd::Dropped) when that brings nothing either. The
    /// connection has to be read for that to be noticed.
    pub async fn next(&mut self) -> Result<Vec<u8>, DataEnd> {
        loop {
            let message = tokio::select! {
                message = self.socket.next() => message,
                () = self.pulse.due() => {
                    match self.pulse.beat() {
                        Beat::Ping => {
                            let ping = Message::Ping(Vec::new().into());
                            if let Err(error) = self.socket.send(ping).await {
                                return Err(end_of_error(&error));
                            }
                        }
                        Beat::Lost => return Err(DataEnd::Dropped),
                    }
                    continue;
                }
            };
            if matches!(message, Some(Ok(_))) {
                self.pulse.heard();
            }
            match message {
                // A piece that holds nothing says nothing.
                Some(Ok(Message::Binary(piece))) if piece.is_empty() => {}
                Some(Ok(Message::Binary(piece))) => return Ok(Vec::from(piece)),
                Some(Ok(Message::Close(frame))) => return Err(end_of_close(frame)),
                Some(Ok(Message::Text(_))) => {
                    return Err(DataEnd::Failed(protocol(
                        "the server sent text on a data connection",
                    )));
                }
                // Pings are answered by the socket itself.
                Some(Ok(_)) => {}
                Some(Err(error)) => return Err(end_of_error(&error)),
                None => return Err(DataEnd::Dropped),
            }
        }
    }

    /// Closes the connection properly, which ends its stream on the server.
    pub async fn close(mut self) {
        let closing = async {
            let frame = CloseFrame {
                code: CloseCode::Normal,
                reason: "".into(),
            };
            if self.socket.close(Some(frame)).await.is_ok() {
                // The close is finished when the server has answered it.
                while let Some(Ok(_)) = self.socket.next().await {}
            }
        };
        let _ = tokio::time::timeout(DATA_CLOSE_TIMEOUT, closing).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ticket_is_sixteen_bytes_of_base64url() {
        let ticket = Ticket::read("AAECAwQFBgcICQoLDA0ODw").unwrap();
        assert_eq!(
            ticket.0,
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
        );
        assert!(Ticket::read("__-_AAAAAAAAAAAAAAAAAA").is_ok());

        for not_one in [
            "",
            "AAECAwQFBgcICQoLDA0OD",
            "AAECAwQFBgcICQoLDA0ODw==",
            "AAECAwQFBgcICQoLDA0ODwAA",
            "AAECAwQFBgcICQoLDA0OD+",
            "AAECAwQFBgcICQoLDA0OD ",
            "ééééééééééé",
        ] {
            assert!(Ticket::read(not_one).is_err(), "{not_one:?}");
        }
    }

    #[test]
    fn the_first_messages_are_laid_out_as_described() {
        let ticket = Ticket([7; 16]);
        let message = hello(&ticket);
        assert_eq!(message.len(), 17);
        assert_eq!(message[0], 0x01);
        assert_eq!(message[1..], [7; 16]);

        let message = resume(&ResumeToken([9; 16]), 0x0102_0304_0506_0708);
        assert_eq!(message.len(), 25);
        assert_eq!(message[0], 0x02);
        assert_eq!(message[1..17], [9; 16]);
        assert_eq!(message[17..], [8, 7, 6, 5, 4, 3, 2, 1]);
    }

    fn welcome(keep_ms: u32, nonce: u64) -> Vec<u8> {
        let mut message = vec![0x81];
        message.extend([5; 16]);
        message.extend([0; 3]);
        message.extend(keep_ms.to_le_bytes());
        message.extend(nonce.to_le_bytes());
        message
    }

    #[test]
    fn a_welcome_is_read_and_must_be_whole() {
        let Answer::Welcome(read) =
            read_answer(&welcome(60_000, 12_491_831_766_469_670_801)).unwrap()
        else {
            panic!("not a welcome");
        };
        assert_eq!(read.token, ResumeToken([5; 16]));
        assert_eq!(read.keep, Duration::from_secs(60));
        assert_eq!(read.nonce, 12_491_831_766_469_670_801);

        let whole = welcome(60_000, 1);
        assert!(read_answer(&whole[..31]).is_err());
        let mut longer = whole.clone();
        longer.push(0);
        assert!(read_answer(&longer).is_err());
        assert!(read_answer(&welcome(60_000, 0)).is_err());
        assert!(read_answer(&[]).is_err());
        assert!(read_answer(&[0x81]).is_err());
        assert!(read_answer(&[0x01; 32]).is_err());
    }

    #[test]
    fn every_rejection_has_its_reason() {
        let reasons = [
            (0x01, Reject::BadTicket),
            (0x02, Reject::AlreadyConnected),
            (0x03, Reject::Unavailable),
            (0x04, Reject::Limit),
            (0x05, Reject::Malformed),
            (0x06, Reject::ServerFailed),
            (0x07, Reject::Subprotocol),
            (0x08, Reject::Other(0x08)),
            (0xff, Reject::Other(0xff)),
        ];
        for (byte, reason) in reasons {
            assert_eq!(
                read_answer(&[0x82, byte]).unwrap(),
                Answer::Reject(reason),
                "{byte:#x}"
            );
            assert!(!reason.to_string().is_empty());
        }
        // A later server may say more after the reason.
        assert_eq!(
            read_answer(&[0x82, 0x03, 0xaa]).unwrap(),
            Answer::Reject(Reject::Unavailable)
        );
        assert!(read_answer(&[0x82]).is_err());
    }

    #[test]
    fn what_opens_or_continues_a_stream_is_not_written_out() {
        assert_eq!(format!("{:?}", Ticket([0x41; 16])), "Ticket(…)");
        let read = read_answer(&welcome(1, 1)).unwrap();
        let written = format!("{read:?}");
        assert!(written.contains("ResumeToken(…)"), "{written}");
        assert!(!written.contains("5, 5"), "{written}");
    }

    fn close(code: u16, reason: &str) -> Option<CloseFrame> {
        Some(CloseFrame {
            code: CloseCode::from(code),
            reason: reason.to_owned().into(),
        })
    }

    #[test]
    fn a_close_says_what_became_of_the_stream() {
        assert!(matches!(end_of_close(close(1000, "")), DataEnd::Ended));
        assert!(matches!(
            end_of_close(close(1001, "keepalive timeout")),
            DataEnd::Dropped
        ));
        assert!(matches!(
            end_of_close(close(1001, "data provider canceled")),
            DataEnd::Restart
        ));
        assert!(matches!(
            end_of_close(close(1013, "data reconnect required")),
            DataEnd::Restart
        ));
        assert!(matches!(
            end_of_close(close(4000, "block closed")),
            DataEnd::BlockClosed
        ));
        assert!(matches!(end_of_close(None), DataEnd::Dropped));
        for (code, reason) in [
            (1008, "data provider unavailable"),
            (1011, "data provider failed"),
            (1002, "data handshake rejected"),
            (4999, "something new"),
        ] {
            match end_of_close(close(code, reason)) {
                DataEnd::Failed(RexError::Closed {
                    code: said,
                    reason: why,
                }) => {
                    assert_eq!(said, code);
                    assert_eq!(why, reason);
                }
                other => panic!("{code}: {other:?}"),
            }
        }
    }
}
