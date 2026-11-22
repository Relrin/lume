//! The control connection: one WebSocket that carries every method call and every
//! event.
//!
//! Every message, either way, is one JSON object in one text message. A client says
//! hello, the server answers with a welcome, and from then on the client sends requests
//! and the server sends a response to each, and events when it has them.
//!
//! Events are numbered from 1 in the order they are sent, with no number left out. A
//! response carries the number of the last event that was queued before its request
//! began, so whoever applies a result knows which events it already holds: those up to
//! that number. An event out of turn means some were lost, and the connection is ended
//! so that everything is read again on a new one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::error::ProtocolError;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

use crate::api::{ApiRevisions, ClientInfo, Hello, Secret, Welcome, WireWelcome, cut};
use crate::error::RexError;
use crate::transport::Io;
use crate::tuning::{
    API_MIN_REVISION, API_REVISION, CALL_TIMEOUT, CONTROL_PATH, CONTROL_SUBPROTOCOL, EVENT_BACKLOG,
    HELLO_MAX_BYTES, MESSAGE_FLOOR_BYTES, MESSAGE_MAX_BYTES, REQUEST_BACKLOG, TEXT_MAX_BYTES,
    WELCOME_TIMEOUT,
};
use crate::ws;

/// The answer to a request.
#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    /// The number of the last event queued before the request began, or 0. Events with
    /// a higher number are to be applied on top of the result.
    pub seq: u64,
    /// The method's result. Null for a method that returns nothing.
    pub payload: Value,
}

/// Something the server reported without being asked.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// The event's place among the events of its connection, from 1.
    pub seq: u64,
    /// The event's name, such as `session_created`.
    pub name: String,
    /// The event's fields.
    pub payload: Value,
}

/// The events of a control connection, in the order the server sent them.
pub struct Events {
    events: mpsc::Receiver<Event>,
}

impl Events {
    /// The next event, or `None` once the connection has ended.
    pub async fn next(&mut self) -> Option<Event> {
        self.events.recv().await
    }
}

/// Why a connection ended, kept so that everyone who asks is told the same.
#[derive(Clone, Debug)]
enum Ended {
    Closed { code: u16, reason: String },
    Protocol(String),
    Lost,
}

impl Ended {
    fn error(&self) -> RexError {
        match self {
            Ended::Closed { code, reason } => RexError::Closed {
                code: *code,
                reason: reason.clone(),
            },
            Ended::Protocol(what) => RexError::Protocol(what.clone()),
            Ended::Lost => RexError::Disconnected,
        }
    }
}

struct Request {
    method: String,
    payload: Value,
    reply: oneshot::Sender<Result<Response, RexError>>,
}

/// A control connection to a Rex server.
///
/// Dropping it, or calling [`close`](Self::close), ends the client on the server at
/// once. A connection that is lost instead is one the server keeps a client for, for
/// about a minute, to be continued with [`reconnect_token`](Self::reconnect_token).
pub struct Control {
    requests: mpsc::Sender<Request>,
    welcome: Welcome,
    token: Option<Secret>,
    ended: watch::Receiver<Option<Ended>>,
    /// Told to close the connection properly.
    closing: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

/// What every message has: the name of its kind.
#[derive(Deserialize)]
struct Incoming {
    #[serde(rename = "type")]
    kind: String,
}

/// A text field of a message, cut to size, or nothing if it is not there as text.
fn text_of(message: &Value, field: &str) -> String {
    let mut text = message
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    cut(&mut text, TEXT_MAX_BYTES);
    text
}

fn protocol(what: impl Into<String>) -> RexError {
    RexError::Protocol(what.into())
}

/// What a close frame says, cut to size.
fn closed(code: CloseCode, reason: &str) -> Ended {
    let mut reason = reason.to_owned();
    cut(&mut reason, TEXT_MAX_BYTES);
    Ended::Closed {
        code: u16::from(code),
        reason,
    }
}

/// A close before the welcome is how a server refuses a client it cannot serve.
fn refused_at_hello(code: u16, reason: String) -> RexError {
    if reason.starts_with("no common API revision") {
        RexError::Revision(reason)
    } else {
        RexError::Closed { code, reason }
    }
}

/// The hello, short enough to be taken: what a client says of itself is left out piece
/// by piece until it is.
fn hello_text(client: &ClientInfo, token: Option<&str>) -> Result<String, RexError> {
    let mut client = client.clone();
    client.cut();
    loop {
        let hello = Hello {
            kind: "hello",
            api_revision: API_REVISION,
            api_min_revision: API_MIN_REVISION,
            client: &client,
            reconnect_token: token,
        };
        let text = serde_json::to_string(&hello).map_err(|error| protocol(error.to_string()))?;
        if text.len() < HELLO_MAX_BYTES {
            return Ok(text);
        }
        if client.label.take().is_some() {
            continue;
        }
        if !client.version.is_empty() {
            client.version.clear();
            continue;
        }
        if !client.name.is_empty() {
            client.name.clear();
            continue;
        }
        return Err(protocol("the hello cannot be made short enough to send"));
    }
}

impl Control {
    /// Opens a control connection over a stream to a server: the WebSocket handshake,
    /// the hello and the welcome. `endpoint` names the server in what is said of
    /// failures. `token` continues a client whose connection was lost.
    ///
    /// A server that speaks no revision of the API this client was written for is
    /// refused.
    pub async fn open(
        io: impl Io,
        endpoint: &str,
        client: &ClientInfo,
        token: Option<&str>,
    ) -> Result<(Self, Events), RexError> {
        let hello = hello_text(client, token)?;
        let mut socket = ws::open(
            io,
            endpoint,
            CONTROL_PATH,
            CONTROL_SUBPROTOCOL,
            MESSAGE_MAX_BYTES,
        )
        .await?;

        socket
            .send(Message::text(hello))
            .await
            .map_err(|error| ended_by(&error).error())?;
        let first = tokio::time::timeout(WELCOME_TIMEOUT, socket.next())
            .await
            .map_err(|_| RexError::Timeout("hello".to_owned()))?;
        let wire: WireWelcome = match first {
            Some(Ok(Message::Text(text))) => {
                let named: Incoming = serde_json::from_str(&text)
                    .map_err(|error| protocol(format!("the welcome is not a message: {error}")))?;
                if named.kind != "welcome" {
                    return Err(protocol("the first message was not a welcome"));
                }
                serde_json::from_str(&text).map_err(|error| {
                    protocol(format!("the welcome is not as described: {error}"))
                })?
            }
            Some(Ok(Message::Close(frame))) => {
                let (code, reason) = match frame {
                    Some(frame) => match closed(frame.code, &frame.reason) {
                        Ended::Closed { code, reason } => (code, reason),
                        _ => (1005, String::new()),
                    },
                    None => (1005, String::new()),
                };
                return Err(refused_at_hello(code, reason));
            }
            Some(Ok(_)) => return Err(protocol("the first message was not a welcome")),
            Some(Err(error)) => return Err(ended_by(&error).error()),
            None => return Err(RexError::Disconnected),
        };
        if wire.protocol != CONTROL_SUBPROTOCOL {
            return Err(protocol("the welcome names another protocol"));
        }
        if !(API_MIN_REVISION..=API_REVISION).contains(&wire.api_revision) {
            let update = if wire.api_server_revision < API_MIN_REVISION {
                "update the Rex server"
            } else {
                "update this application"
            };
            return Err(RexError::Revision(format!(
                "the server speaks {} to {} and chose {}, this application speaks {API_MIN_REVISION} to {API_REVISION}; {update}",
                wire.api_min_revision, wire.api_server_revision, wire.api_revision,
            )));
        }
        let named = usize::try_from(wire.limits.max_message_bytes).unwrap_or(usize::MAX);
        if named < MESSAGE_FLOOR_BYTES {
            return Err(protocol(
                "the welcome names a message limit below the least there is",
            ));
        }
        let (client_id, token, resumed) = match wire.client {
            Some(mut client) => {
                cut(&mut client.id, TEXT_MAX_BYTES);
                (
                    Some(client.id),
                    // A token too long to send back in a hello is of no use.
                    Some(client.reconnect_token)
                        .filter(|token| token.len() < HELLO_MAX_BYTES / 2)
                        .map(Secret),
                    client.resumed,
                )
            }
            None => (None, None, false),
        };
        let welcome = Welcome {
            client_id,
            resumed,
            max_message_bytes: named.min(MESSAGE_MAX_BYTES),
            revisions: ApiRevisions {
                used: wire.api_revision,
                server: wire.api_server_revision,
                server_min: wire.api_min_revision,
            },
        };

        let (requests, queued) = mpsc::channel(REQUEST_BACKLOG);
        let (events, taken) = mpsc::channel(EVENT_BACKLOG);
        let (ended, watched) = watch::channel(None);
        let (closing, close) = oneshot::channel();
        tokio::spawn(run(
            socket,
            queued,
            events,
            ended,
            close,
            welcome.max_message_bytes,
        ));
        Ok((
            Self {
                requests,
                welcome,
                token,
                ended: watched,
                closing: Arc::new(Mutex::new(Some(closing))),
            },
            Events { events: taken },
        ))
    }

    /// What the server answered the hello with.
    pub fn welcome(&self) -> &Welcome {
        &self.welcome
    }

    /// What continues this client on a new connection after this one is lost. Never
    /// written anywhere.
    pub(crate) fn reconnect_token(&self) -> Option<&str> {
        self.token.as_ref().map(|token| token.0.as_str())
    }

    /// Why the connection ended, once it has.
    pub fn ended(&self) -> Option<RexError> {
        self.ended.borrow().as_ref().map(Ended::error)
    }

    /// Waits for the connection to end, and says why.
    pub async fn until_ended(&self) -> RexError {
        let mut ended = self.ended.clone();
        loop {
            if let Some(why) = ended.borrow_and_update().as_ref() {
                return why.error();
            }
            if ended.changed().await.is_err() {
                return RexError::Disconnected;
            }
        }
    }

    fn why_ended(&self) -> RexError {
        self.ended().unwrap_or(RexError::Disconnected)
    }

    /// Calls a method and waits for its answer. `payload` holds the method's arguments,
    /// or is null for a method that takes none.
    pub async fn call(&self, method: &str, payload: Value) -> Result<Response, RexError> {
        let (reply, answer) = oneshot::channel();
        let request = Request {
            method: method.to_owned(),
            payload,
            reply,
        };
        if self.requests.send(request).await.is_err() {
            return Err(self.why_ended());
        }
        match tokio::time::timeout(CALL_TIMEOUT, answer).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => Err(self.why_ended()),
            Err(_) => Err(RexError::Timeout(method.to_owned())),
        }
    }

    /// Calls a method and reads its result as `T`. The number of the last event the
    /// result already holds comes with it.
    pub async fn call_as<T: DeserializeOwned>(
        &self,
        method: &str,
        payload: Value,
    ) -> Result<(T, u64), RexError> {
        let response = self.call(method, payload).await?;
        let result = serde_json::from_value(response.payload).map_err(|error| {
            protocol(format!(
                "the result of `{method}` is not as described: {error}"
            ))
        })?;
        Ok((result, response.seq))
    }

    /// Closes the connection properly, which ends this client on the server at once.
    pub fn close(&self) {
        let closing = self
            .closing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(closing) = closing {
            let _ = closing.send(());
        }
    }
}

/// What an error of the socket means for the connection.
fn ended_by(error: &WsError) -> Ended {
    match error {
        WsError::ConnectionClosed
        | WsError::AlreadyClosed
        | WsError::Io(_)
        // The stream ended with no close frame: the server, or the way to it, is gone.
        | WsError::Protocol(ProtocolError::ResetWithoutClosingHandshake) => Ended::Lost,
        WsError::Capacity(_) => Ended::Protocol("the server sent a message too large to take".to_owned()),
        WsError::Protocol(_) | WsError::Utf8(_) => {
            Ended::Protocol("the server broke the WebSocket protocol".to_owned())
        }
        other => Ended::Protocol(other.to_string()),
    }
}

struct Running {
    /// The requests that wait for their responses, by id.
    waiting: HashMap<String, (String, oneshot::Sender<Result<Response, RexError>>)>,
    next_id: u64,
    /// The number of the last event taken.
    last_seq: u64,
    events: mpsc::Sender<Event>,
}

impl Running {
    /// Acts on one message from the server. An error ends the connection.
    ///
    /// A text that is not a message at all ends it, and so does an event out of turn,
    /// since events must not be lost. Anything else that is not as it should be is
    /// passed over: it is no answer to anyone, and nothing depends on it.
    fn take(&mut self, text: &str) -> Result<(), Ended> {
        let not_a_message = || Ended::Protocol("the server sent what is not a message".to_owned());
        let mut message: Value = serde_json::from_str(text).map_err(|_| not_a_message())?;
        let kind = message
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(not_a_message)?;
        match kind {
            "response" => {
                let Some(id) = message.get("id").and_then(Value::as_str) else {
                    return Ok(());
                };
                // A response to a request nobody waits for any more is passed over.
                let Some((method, reply)) = self.waiting.remove(id) else {
                    return Ok(());
                };
                let answer = match message.get("error").filter(|error| !error.is_null()) {
                    Some(error) => Err(RexError::Call {
                        method,
                        code: text_of(error, "code"),
                        message: text_of(error, "message"),
                        status: error
                            .get("status")
                            .and_then(Value::as_u64)
                            .and_then(|status| u16::try_from(status).ok())
                            .unwrap_or(0),
                    }),
                    None => Ok(Response {
                        seq: message.get("seq").and_then(Value::as_u64).unwrap_or(0),
                        payload: message
                            .get_mut("payload")
                            .map(Value::take)
                            .unwrap_or(Value::Null),
                    }),
                };
                let _ = reply.send(answer);
                Ok(())
            }
            "event" => {
                let seq = message.get("seq").and_then(Value::as_u64).unwrap_or(0);
                if seq != self.last_seq + 1 {
                    return Err(Ended::Protocol(format!(
                        "event {seq} came after event {}",
                        self.last_seq
                    )));
                }
                self.last_seq = seq;
                let (name, payload) = match message.get_mut("payload") {
                    Some(event) => (
                        text_of(event, "type"),
                        event
                            .get_mut("payload")
                            .map(Value::take)
                            .unwrap_or(Value::Null),
                    ),
                    None => (String::new(), Value::Null),
                };
                let event = Event { seq, name, payload };
                match self.events.try_send(event) {
                    Ok(()) => Ok(()),
                    // Nobody follows the events: they are counted and let go.
                    Err(mpsc::error::TrySendError::Closed(_)) => Ok(()),
                    Err(mpsc::error::TrySendError::Full(_)) => Err(Ended::Protocol(
                        "events arrived faster than they were taken".to_owned(),
                    )),
                }
            }
            // A newer server may say things this client does not know.
            _ => Ok(()),
        }
    }

    fn fail_all(&mut self, why: &Ended) {
        for (_, (_, reply)) in self.waiting.drain() {
            let _ = reply.send(Err(why.error()));
        }
    }
}

async fn run<S: Io>(
    mut socket: WebSocketStream<S>,
    mut requests: mpsc::Receiver<Request>,
    events: mpsc::Sender<Event>,
    ended: watch::Sender<Option<Ended>>,
    mut close: oneshot::Receiver<()>,
    max_message_bytes: usize,
) {
    let mut running = Running {
        waiting: HashMap::new(),
        next_id: 1,
        last_seq: 0,
        events,
    };
    let why = loop {
        tokio::select! {
            asked = requests.recv() => {
                // Whoever held the connection let go of it.
                let Some(request) = asked else {
                    let _ = socket.close(None).await;
                    break Ended::Lost;
                };
                let id = running.next_id.to_string();
                running.next_id += 1;
                let mut message = json!({ "type": "request", "id": id, "method": request.method });
                if !request.payload.is_null() {
                    message["payload"] = request.payload;
                }
                let text = message.to_string();
                // Sending it would have the server close the connection.
                if text.len() > max_message_bytes {
                    let _ = request.reply.send(Err(RexError::Call {
                        method: request.method,
                        code: "too_large".to_owned(),
                        message: "the request is larger than the server takes".to_owned(),
                        status: 413,
                    }));
                    continue;
                }
                running.waiting.insert(id, (request.method, request.reply));
                if let Err(error) = socket.send(Message::text(text)).await {
                    break ended_by(&error);
                }
            }
            _ = &mut close => {
                let _ = socket.close(None).await;
                break Ended::Lost;
            }
            message = socket.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    if let Err(why) = running.take(&text) {
                        let _ = socket.close(None).await;
                        break why;
                    }
                }
                Some(Ok(Message::Close(frame))) => break match frame {
                    Some(frame) => closed(frame.code, &frame.reason),
                    None => Ended::Lost,
                },
                // Pings are answered by the socket itself. A server sends nothing else
                // that is not text.
                Some(Ok(_)) => {}
                Some(Err(error)) => break ended_by(&error),
                None => break Ended::Lost,
            },
        }
    };
    running.fail_all(&why);
    let _ = ended.send(Some(why));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hello_is_kept_short_enough_to_be_taken() {
        let mut client = ClientInfo::app("Lume", "1.2.3");
        client.label = Some("studio".to_owned());
        let text = hello_text(&client, Some("token")).unwrap();
        let said: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(said["type"], "hello");
        assert_eq!(said["api_revision"], API_REVISION);
        assert_eq!(said["api_min_revision"], API_MIN_REVISION);
        assert_eq!(said["client"]["name"], "Lume");
        assert_eq!(said["client"]["kind"], "app");
        assert_eq!(said["client"]["label"], "studio");
        assert_eq!(said["reconnect_token"], "token");

        // With a token that leaves little room, what the client says of itself goes.
        let long = "t".repeat(HELLO_MAX_BYTES - 150);
        let text = hello_text(&client, Some(&long)).unwrap();
        assert!(text.len() < HELLO_MAX_BYTES);
        let said: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(said["reconnect_token"], long.as_str());
        assert!(said["client"].get("label").is_none());

        let too_long = "t".repeat(HELLO_MAX_BYTES);
        assert!(hello_text(&client, Some(&too_long)).is_err());
    }
}
