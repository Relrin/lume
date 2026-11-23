//! A server to test against: the server's side of a control connection, over a stream
//! in memory, and what a real server answered when it was recorded.
//!
//! `fixtures/local.json` holds every message this crate's own recorder exchanged with a
//! Rex server on the machine it was made on (`examples/record.rs`). The pretend server
//! here answers with those payloads, so what the client reads in tests is what a server
//! sends.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use lume_rex::{ClientInfo, Dialer, Io, RexError};
use serde_json::{Value, json};
use tokio::io::DuplexStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::{WebSocketStream, accept_hdr_async};

/// Room in a stream for more than any test sends before the other side reads.
const STREAM_BYTES: usize = 8 * 1024 * 1024;

pub fn client() -> ClientInfo {
    ClientInfo::app("lume-test", "0.0.0")
}

/// The messages that were recorded.
pub struct Recorded {
    messages: Vec<Value>,
}

impl Recorded {
    pub fn load() -> Self {
        let text = include_str!("../fixtures/local.json");
        let record: Value = serde_json::from_str(text).unwrap();
        Self {
            messages: record["messages"].as_array().unwrap().clone(),
        }
    }

    /// Every message the server sent, in order.
    pub fn received(&self) -> impl Iterator<Item = &Value> {
        self.messages
            .iter()
            .filter(|entry| entry["way"] == "received")
            .map(|entry| &entry["message"])
    }

    /// Each request that was sent and the response it got, in order.
    pub fn calls(&self) -> Vec<(&Value, &Value)> {
        let sent = self
            .messages
            .iter()
            .filter(|entry| entry["way"] == "sent" && entry["message"]["type"] == "request")
            .map(|entry| &entry["message"]);
        sent.map(|request| {
            let response = self
                .received()
                .find(|message| message["type"] == "response" && message["id"] == request["id"])
                .unwrap();
            (request, response)
        })
        .collect()
    }

    /// What the server answered the `nth` call of a method with, from 0.
    pub fn response(&self, method: &str, nth: usize) -> &Value {
        self.calls()
            .into_iter()
            .filter(|(request, _)| request["method"] == method)
            .nth(nth)
            .unwrap_or_else(|| panic!("{method} was not recorded {} times", nth + 1))
            .1
    }

    /// The result of the `nth` call of a method.
    pub fn result(&self, method: &str, nth: usize) -> Value {
        self.response(method, nth)["payload"].clone()
    }

    /// The welcome the server sent.
    pub fn welcome(&self) -> Value {
        self.received()
            .find(|message| message["type"] == "welcome")
            .unwrap()
            .clone()
    }

    /// Every event the server sent, as its name and fields.
    pub fn events(&self) -> Vec<(&str, &Value)> {
        self.received()
            .filter(|message| message["type"] == "event")
            .map(|message| {
                (
                    message["payload"]["type"].as_str().unwrap(),
                    &message["payload"]["payload"],
                )
            })
            .collect()
    }
}

/// Hands the client's side of a new stream to a backend, and the server's side to the
/// test.
pub struct Streams {
    servers: mpsc::UnboundedSender<DuplexStream>,
    /// How many streams were asked for.
    pub dialled: Arc<AtomicUsize>,
    /// While set, there is no server to reach.
    pub down: Arc<Mutex<bool>>,
}

/// The server's sides of the streams a backend opened, in order.
pub struct Listener {
    servers: mpsc::UnboundedReceiver<DuplexStream>,
}

pub fn streams() -> (Streams, Listener) {
    let (servers, taken) = mpsc::unbounded_channel();
    (
        Streams {
            servers,
            dialled: Arc::new(AtomicUsize::new(0)),
            down: Arc::new(Mutex::new(false)),
        },
        Listener { servers: taken },
    )
}

#[async_trait]
impl Dialer for Streams {
    async fn dial(&self) -> Result<Box<dyn Io>, RexError> {
        self.dialled.fetch_add(1, Ordering::Relaxed);
        if *self.down.lock().unwrap() {
            return Err(RexError::Unreachable {
                endpoint: "test".to_owned(),
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            });
        }
        let (client, server) = tokio::io::duplex(STREAM_BYTES);
        let _ = self.servers.send(server);
        Ok(Box::new(client))
    }
}

impl Listener {
    /// The server's side of the next stream.
    pub async fn stream(&mut self) -> DuplexStream {
        tokio::time::timeout(std::time::Duration::from_secs(20), self.servers.recv())
            .await
            .expect("no connection was made")
            .expect("the backend is gone")
    }

    /// The next connection, shaken, greeted and welcomed as a server would.
    pub async fn accept(&mut self) -> Conn {
        let stream = self.stream().await;
        Conn::accept(stream, Recorded::load().welcome()).await
    }
}

/// A stream to a pretend server and the server's side of it.
pub fn pair() -> (DuplexStream, DuplexStream) {
    tokio::io::duplex(STREAM_BYTES)
}

/// The server's side of one control connection.
pub struct Conn {
    pub socket: WebSocketStream<DuplexStream>,
    /// The hello the client sent.
    pub hello: Value,
    /// The subprotocol the client asked for.
    pub asked: Option<String>,
    /// The number of the last event sent.
    pub seq: u64,
}

/// One request a client made.
pub struct Asked {
    pub id: String,
    pub method: String,
    pub payload: Value,
}

impl Conn {
    /// Shakes hands as a server does and takes the hello, without answering it.
    // The library that shakes hands says what a refusal is, and it is not small.
    #[allow(clippy::result_large_err)]
    pub async fn shake(stream: DuplexStream) -> Self {
        let asked = Arc::new(Mutex::new(None));
        let noted = Arc::clone(&asked);
        let socket = accept_hdr_async(
            stream,
            move |request: &Request, mut response: Response| -> Result<Response, ErrorResponse> {
                assert_eq!(request.uri().path(), "/api/control");
                let protocol = request.headers().get("Sec-WebSocket-Protocol").cloned();
                if let Some(protocol) = protocol {
                    *noted.lock().unwrap() = protocol.to_str().ok().map(str::to_owned);
                    response
                        .headers_mut()
                        .insert("Sec-WebSocket-Protocol", protocol);
                }
                Ok(response)
            },
        )
        .await
        .unwrap();
        let mut conn = Self {
            socket,
            hello: Value::Null,
            asked: asked.lock().unwrap().clone(),
            seq: 0,
        };
        conn.hello = conn.text().await.expect("no hello");
        conn
    }

    /// Shakes hands, takes the hello and answers it with `welcome`.
    pub async fn accept(stream: DuplexStream, welcome: Value) -> Self {
        let mut conn = Self::shake(stream).await;
        conn.send(welcome).await;
        conn
    }

    /// The next text message, or `None` once the client has gone.
    pub async fn text(&mut self) -> Option<Value> {
        loop {
            match self.socket.next().await? {
                Ok(Message::Text(text)) => return Some(serde_json::from_str(&text).unwrap()),
                Ok(Message::Close(_)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
    }

    /// The next request, or `None` once the client has gone.
    pub async fn asked(&mut self) -> Option<Asked> {
        let message = self.text().await?;
        assert_eq!(message["type"], "request", "{message}");
        Some(Asked {
            id: message["id"]
                .as_str()
                .expect("an id that is a string")
                .to_owned(),
            method: message["method"].as_str().unwrap().to_owned(),
            payload: message.get("payload").cloned().unwrap_or(Value::Null),
        })
    }

    pub async fn send(&mut self, message: Value) {
        self.raw(message.to_string()).await;
    }

    pub async fn raw(&mut self, text: String) {
        let _ = self.socket.send(Message::text(text)).await;
    }

    /// Answers a request with a result.
    pub async fn respond(&mut self, id: &str, payload: Value) {
        let seq = self.seq;
        self.send(json!({ "type": "response", "id": id, "seq": seq, "payload": payload }))
            .await;
    }

    /// Answers a request with an error.
    pub async fn fail(&mut self, id: &str, code: &str, status: u16, message: &str) {
        let seq = self.seq;
        self.send(json!({
            "type": "response", "id": id, "seq": seq,
            "error": { "code": code, "message": message, "status": status },
        }))
        .await;
    }

    /// Sends an event, numbered after the last.
    pub async fn event(&mut self, name: &str, payload: Value) {
        self.seq += 1;
        let seq = self.seq;
        self.send(json!({
            "type": "event", "seq": seq,
            "payload": { "type": name, "payload": payload },
        }))
        .await;
    }

    /// Sends an event of a session.
    pub async fn session_event(&mut self, session_id: &str, name: &str, payload: Value) {
        self.event(
            "session_event",
            json!({ "session_id": session_id, "event": { "type": name, "payload": payload } }),
        )
        .await;
    }

    /// Closes the connection as a server does, with a status and a reason.
    pub async fn close(mut self, code: u16, reason: &str) {
        let _ = self
            .socket
            .close(Some(CloseFrame {
                code: CloseCode::from(code),
                reason: reason.to_owned().into(),
            }))
            .await;
        // The close is finished when the client has answered it.
        while let Some(Ok(_)) = self.socket.next().await {}
    }
}

/// The server's side of one data connection.
pub struct DataSide {
    pub socket: WebSocketStream<DuplexStream>,
    /// The first message the client sent.
    pub first: Vec<u8>,
    /// The subprotocol the client asked for.
    pub asked: Option<String>,
    /// How many bytes of the stream were sent.
    pub sent: u64,
}

/// A message of the server's own, as it stands in a terminal's stream.
pub fn note_bytes(nonce: u64, json: &Value) -> Vec<u8> {
    format!("\u{1b}_rex1;{nonce};{json}\u{1b}\\").into_bytes()
}

/// The ticket a pretend server gives for a data connection.
pub const TICKET: &str = "AAECAwQFBgcICQoLDA0ODw";
/// The token a pretend server welcomes a data connection with.
pub const DATA_TOKEN: [u8; 16] = [0xa5; 16];
/// The nonce a pretend server marks its streams with.
pub const NONCE: u64 = 12_491_831_766_469_670_801;

impl DataSide {
    /// Shakes hands as a server does and takes the first message, without answering it.
    #[allow(clippy::result_large_err)]
    pub async fn shake(stream: DuplexStream) -> Self {
        let asked = Arc::new(Mutex::new(None));
        let noted = Arc::clone(&asked);
        let socket = accept_hdr_async(
            stream,
            move |request: &Request, mut response: Response| -> Result<Response, ErrorResponse> {
                assert_eq!(request.uri().path(), "/api/data");
                let protocol = request.headers().get("Sec-WebSocket-Protocol").cloned();
                if let Some(protocol) = protocol {
                    *noted.lock().unwrap() = protocol.to_str().ok().map(str::to_owned);
                    response
                        .headers_mut()
                        .insert("Sec-WebSocket-Protocol", protocol);
                }
                Ok(response)
            },
        )
        .await
        .unwrap();
        let mut side = Self {
            socket,
            first: Vec::new(),
            asked: asked.lock().unwrap().clone(),
            sent: 0,
        };
        side.first = match side.socket.next().await {
            Some(Ok(Message::Binary(first))) => first.to_vec(),
            other => panic!("no first message: {other:?}"),
        };
        side
    }

    /// Shakes hands, takes the first message and welcomes the connection.
    pub async fn accept(stream: DuplexStream) -> Self {
        let mut side = Self::shake(stream).await;
        side.welcome(DATA_TOKEN, 60_000, NONCE).await;
        side
    }

    /// Answers the first message with a welcome.
    pub async fn welcome(&mut self, token: [u8; 16], keep_ms: u32, nonce: u64) {
        let mut message = vec![0x81];
        message.extend(token);
        message.extend([0; 3]);
        message.extend(keep_ms.to_le_bytes());
        message.extend(nonce.to_le_bytes());
        let _ = self.socket.send(Message::binary(message)).await;
    }

    /// Answers the first message with a rejection, and closes as a server does then.
    pub async fn reject(mut self, reason: u8) {
        let _ = self.socket.send(Message::binary(vec![0x82, reason])).await;
        self.close(1008, "data handshake rejected").await;
    }

    /// Sends one binary message of the stream.
    pub async fn bytes(&mut self, bytes: &[u8]) {
        self.sent += bytes.len() as u64;
        let _ = self.socket.send(Message::binary(bytes.to_vec())).await;
    }

    /// Says the terminal's size, as a server does in a stream.
    pub async fn size(&mut self, cols: u16, rows: u16) {
        self.note(
            NONCE,
            json!({
                "type": "resize", "cols": cols, "rows": rows,
                "cell_width_px": 8, "cell_height_px": 16, "owner": null,
            }),
        )
        .await;
    }

    /// Sends a message of the server's own, as it stands in a terminal's stream.
    pub async fn note(&mut self, nonce: u64, json: Value) {
        self.bytes(&note_bytes(nonce, &json)).await;
    }

    /// Closes the connection as a server does, with a status and a reason.
    pub async fn close(mut self, code: u16, reason: &str) {
        let _ = self
            .socket
            .close(Some(CloseFrame {
                code: CloseCode::from(code),
                reason: reason.to_owned().into(),
            }))
            .await;
        while let Some(Ok(_)) = self.socket.next().await {}
    }

    /// Waits for the client to close the connection, and says with what status.
    pub async fn closed_by_client(&mut self) -> Option<u16> {
        loop {
            match self.socket.next().await {
                Some(Ok(Message::Close(frame))) => {
                    return frame.map(|frame| u16::from(frame.code));
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => return None,
            }
        }
    }
}

/// What a pretend server holds: its sessions in order, and each one's view.
#[derive(Clone)]
pub struct World {
    pub sessions: Vec<(String, String)>,
    pub views: HashMap<String, Value>,
    /// A terminal's screen written out as output that rebuilds it, in the parts it is
    /// given in.
    pub screen: Vec<String>,
    /// Whether a terminal shows its alternate screen.
    pub alternate: bool,
}

/// The session and the block that were recorded.
pub const SESSION: &str = "session:recorded";

impl World {
    /// A server with the one session that was recorded, as it stood after its split.
    pub fn recorded() -> Self {
        let recorded = Recorded::load();
        let mut view = recorded.result("session.view", 0);
        view["session_id"] = json!(SESSION);
        Self {
            sessions: vec![(SESSION.to_owned(), "lume-fixture".to_owned())],
            views: HashMap::from([(SESSION.to_owned(), view)]),
            screen: vec![String::new()],
            alternate: false,
        }
    }

    fn listed(&self) -> Value {
        let sessions: Vec<Value> = self
            .sessions
            .iter()
            .map(|(id, label)| json!({ "session_id": id, "label": label }))
            .collect();
        json!({ "sessions": sessions })
    }

    /// Answers one request as a server holding this would.
    pub async fn answer(&self, conn: &mut Conn, asked: &Asked) {
        let recorded = Recorded::load();
        let session = asked.payload.get("session_id").and_then(Value::as_str);
        let view = session.and_then(|session| self.views.get(session));
        // A server takes a block method's own arguments as an object, or not at all.
        if asked.payload.get("args").is_some_and(Value::is_null) {
            conn.fail(&asked.id, "bad_request", 400, "validating input: type null")
                .await;
            return;
        }
        let reply = match asked.method.as_str() {
            "server.version" => Some(recorded.result("server.version", 0)),
            "session.attach_all" | "session.list" => Some(self.listed()),
            "session.attach" => {
                view.map(|view| json!({ "client_id": "client:test", "view": view }))
            }
            "session.view" => view.cloned(),
            "session.detach" => view.map(|_| json!({})),
            "com.superlogical.terminal.write" => view.map(|_| {
                let typed = asked.payload["args"]["data"].as_str().unwrap();
                json!({ "bytes_written": typed.len() })
            }),
            "com.superlogical.terminal.resize" | "block.event" => view.map(|_| json!({})),
            "com.superlogical.terminal.modes" => view.map(|_| {
                let screen = if self.alternate {
                    "alternate"
                } else {
                    "primary"
                };
                json!({ "screen": screen, "modes": [] })
            }),
            "session.own_block" | "session.disown_block" => view.map(|_| {
                let owner = (asked.method == "session.own_block").then_some("client:test");
                let blocks: Vec<Value> = asked.payload["block_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|block| json!({ "block_id": block, "owner": owner }))
                    .collect();
                json!({ "blocks": blocks })
            }),
            "block.close" => {
                view.map(|_| json!({ "block_id": asked.payload["block_id"], "revision": 9 }))
            }
            "com.superlogical.terminal.pwd" => {
                view.map(|_| json!({ "pwd": "file://recorded/home/someone" }))
            }
            "com.superlogical.terminal.program_status" => view.map(|_| {
                json!({ "records": [
                    { "state": "working", "app": "cargo", "updated_at": "2026-11-23T10:00:00Z" },
                ] })
            }),
            "data.connect" => view.map(|_| json!({ "ticket": TICKET, "expires_in_ms": 60_000 })),
            "com.superlogical.terminal.format" => view.map(|_| {
                let mut reply = json!({ "content": self.screen[0] });
                if self.screen.len() > 1 {
                    reply["id"] = json!(7);
                }
                reply
            }),
            "com.superlogical.terminal.blob_read" => view.map(|_| {
                // The part that begins where the caller says it has read to.
                let offset = asked.payload["args"]["offset"].as_u64().unwrap();
                let mut begins = 0;
                let mut found = json!({ "id": 7, "data": "", "total": 0, "eof": true });
                for (nth, part) in self.screen.iter().enumerate() {
                    if begins == offset {
                        found["data"] = json!(part);
                        found["eof"] = json!(nth + 1 == self.screen.len());
                        break;
                    }
                    begins += part.len() as u64;
                }
                let total: usize = self.screen.iter().map(String::len).sum();
                found["total"] = json!(total);
                found
            }),
            "com.superlogical.terminal.size"
            | "com.superlogical.terminal.process"
            | "com.superlogical.terminal.title" => view.map(|_| recorded.result(&asked.method, 0)),
            _ => {
                conn.fail(&asked.id, "method_not_found", 404, "no such method")
                    .await;
                return;
            }
        };
        match reply {
            Some(payload) => conn.respond(&asked.id, payload).await,
            None => {
                conn.fail(&asked.id, "session_not_found", 404, "session not found")
                    .await;
            }
        }
    }
}

/// What a test tells a server that is being run for it.
pub enum Tell {
    Event(String, Value),
    SessionEvent(String, String, Value),
    /// Replaces what the server holds.
    World(World),
    /// Closes the connection with a status and a reason.
    Close(u16, String),
    /// Lets the connection drop without a word.
    Drop,
}

/// A server that answers on one connection until told to stop or the client goes.
pub struct Served {
    tell: mpsc::UnboundedSender<Tell>,
    /// The methods the client called, in order.
    pub called: Arc<Mutex<Vec<String>>>,
    /// What each was called with.
    pub payloads: Arc<Mutex<Vec<(String, Value)>>>,
    pub hello: Value,
    done: tokio::task::JoinHandle<()>,
}

impl Served {
    pub fn tell(&self, what: Tell) {
        let _ = self.tell.send(what);
    }

    pub fn called(&self) -> Vec<String> {
        self.called.lock().unwrap().clone()
    }

    /// What a method was called with, each time, in order.
    pub fn asked(&self, method: &str) -> Vec<Value> {
        let payloads = self.payloads.lock().unwrap();
        payloads
            .iter()
            .filter(|(called, _)| called == method)
            .map(|(_, payload)| payload.clone())
            .collect()
    }

    /// The methods called whose names begin with one of `prefixes`, in order.
    pub fn called_of(&self, prefixes: &[&str]) -> Vec<String> {
        self.called()
            .into_iter()
            .filter(|method| prefixes.iter().any(|prefix| method.starts_with(prefix)))
            .collect()
    }

    /// Waits for the connection to be over.
    pub async fn over(self) {
        let _ = self.done.await;
    }
}

/// Runs a pretend server on a connection.
pub fn serve(mut conn: Conn, mut world: World) -> Served {
    let (tell, mut told) = mpsc::unbounded_channel();
    let called = Arc::new(Mutex::new(Vec::new()));
    let noted = Arc::clone(&called);
    let payloads = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&payloads);
    let hello = conn.hello.clone();
    let done = tokio::spawn(async move {
        loop {
            tokio::select! {
                asked = conn.asked() => {
                    let Some(asked) = asked else { return };
                    noted.lock().unwrap().push(asked.method.clone());
                    kept.lock()
                        .unwrap()
                        .push((asked.method.clone(), asked.payload.clone()));
                    world.answer(&mut conn, &asked).await;
                }
                what = told.recv() => match what {
                    Some(Tell::Event(name, payload)) => conn.event(&name, payload).await,
                    Some(Tell::SessionEvent(session, name, payload)) => {
                        conn.session_event(&session, &name, payload).await;
                    }
                    Some(Tell::World(next)) => world = next,
                    Some(Tell::Close(code, reason)) => {
                        conn.close(code, &reason).await;
                        return;
                    }
                    Some(Tell::Drop) | None => return,
                },
            }
        }
    });
    Served {
        tell,
        called,
        payloads,
        hello,
        done,
    }
}

/// Waits until `done` says so, for at most a while.
pub async fn until(what: &str, mut done: impl FnMut() -> bool) {
    let waiting = async {
        while !done() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    };
    if tokio::time::timeout(std::time::Duration::from_secs(20), waiting)
        .await
        .is_err()
    {
        panic!("{what} never happened");
    }
}
