//! Records what a Rex server on this machine says, for the tests to be checked against.
//!
//! It speaks to the user's own server as this crate does, makes a session, asks what
//! the tests need answered, destroys the session again, and writes every message sent
//! and received to the file named on the command line. What names this machine or its
//! user is replaced before anything is written, and the token a server hands out is
//! never kept.
//!
//! cargo run -p lume-rex --example record -- crates/lume-rex/tests/fixtures/local.json

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use lume_rex::tuning::{API_REVISION, CONTROL_PATH, CONTROL_SUBPROTOCOL};
use lume_rex::{Endpoint, Io, dial};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{WebSocketStream, client_async};

type Failure = Box<dyn std::error::Error>;

struct Recorder {
    socket: WebSocketStream<Box<dyn Io>>,
    messages: Vec<Value>,
    next_id: u64,
}

impl Recorder {
    fn note(&mut self, way: &str, message: &Value) {
        self.messages
            .push(json!({ "way": way, "message": message }));
    }

    async fn send(&mut self, message: Value) -> Result<(), Failure> {
        self.note("sent", &message);
        self.socket.send(Message::text(message.to_string())).await?;
        Ok(())
    }

    async fn receive(&mut self, wait: Duration) -> Result<Option<Value>, Failure> {
        loop {
            match tokio::time::timeout(wait, self.socket.next()).await {
                Err(_) => return Ok(None),
                Ok(None) => return Err("the server closed the connection".into()),
                Ok(Some(message)) => {
                    if let Message::Text(text) = message? {
                        let message: Value = serde_json::from_str(&text)?;
                        self.note("received", &message);
                        return Ok(Some(message));
                    }
                }
            }
        }
    }

    /// Calls a method and takes messages until its response.
    async fn call(&mut self, method: &str, payload: Value) -> Result<Value, Failure> {
        let id = self.next_id.to_string();
        self.next_id += 1;
        let mut request = json!({ "type": "request", "id": id, "method": method });
        if !payload.is_null() {
            request["payload"] = payload;
        }
        self.send(request).await?;
        loop {
            let message = self
                .receive(Duration::from_secs(10))
                .await?
                .ok_or("no response")?;
            if message["type"] == "response" && message["id"] == id.as_str() {
                return Ok(message);
            }
        }
    }

    /// Takes what else arrives for a moment: the events a call set off.
    async fn settle(&mut self) -> Result<(), Failure> {
        while self.receive(Duration::from_millis(400)).await?.is_some() {}
        Ok(())
    }
}

/// Replaces what names this machine or its user, wherever it appears in a text.
fn scrub_text(text: &str, named: &[(String, &str)]) -> String {
    let mut text = text.to_owned();
    for (name, with) in named {
        if !name.is_empty() {
            text = text.replace(name.as_str(), with);
            text = text.replace(&name.replace('\\', "/"), with);
        }
    }
    text
}

fn scrub(value: &mut Value, named: &[(String, &str)]) {
    match value {
        Value::String(text) => *text = scrub_text(text, named),
        Value::Array(items) => items.iter_mut().for_each(|item| scrub(item, named)),
        Value::Object(fields) => {
            for (key, field) in fields.iter_mut() {
                if key == "reconnect_token" {
                    *field = json!("TOKEN");
                } else if matches!(key.as_str(), "pid" | "user_id") && field.is_number() {
                    *field = json!(1000);
                } else {
                    scrub(field, named);
                }
            }
        }
        _ => {}
    }
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let out = std::env::args().nth(1).ok_or("where to write the record")?;
    let io = dial(&Endpoint::Local).await?;
    let mut request = format!("ws://localhost{CONTROL_PATH}").into_client_request()?;
    request
        .headers_mut()
        .insert("Sec-WebSocket-Protocol", CONTROL_SUBPROTOCOL.parse()?);
    let (socket, _) = client_async(request, io).await?;
    let mut recorder = Recorder {
        socket,
        messages: Vec::new(),
        next_id: 1,
    };

    recorder
        .send(json!({
            "type": "hello",
            "api_revision": API_REVISION,
            "api_min_revision": API_REVISION,
            "client": {"kind": "other", "name": "lume-rex-record", "version": "0"},
        }))
        .await?;
    recorder
        .receive(Duration::from_secs(10))
        .await?
        .ok_or("no welcome")?;

    recorder.call("server.version", Value::Null).await?;
    recorder
        .call("session.attach_all", json!({ "event_scope": "session" }))
        .await?;
    let created = recorder
        .call(
            "session.create",
            json!({
                "label": "lume-fixture",
                "initial_windows": [
                    {"layout": {"block": {"flavor": "com.superlogical.terminal.shell"}}}
                ],
            }),
        )
        .await?;
    let session = created["payload"]["session_id"]
        .as_str()
        .ok_or("no session")?
        .to_owned();
    let block = created["payload"]["initial_windows"][0]["block_ids"][0]
        .as_str()
        .ok_or("no block")?
        .to_owned();
    recorder.settle().await?;

    let of_session = || json!({ "session_id": session });
    let of_block = || json!({ "session_id": session, "block_id": block });
    recorder.call("session.attach", of_session()).await?;
    recorder.settle().await?;
    // The shell has started by now and says what it is.
    tokio::time::sleep(Duration::from_secs(2)).await;
    recorder.settle().await?;
    recorder
        .call("com.superlogical.terminal.size", of_block())
        .await?;
    recorder
        .call("com.superlogical.terminal.process", of_block())
        .await?;
    recorder
        .call("com.superlogical.terminal.title", of_block())
        .await?;
    recorder
        .call(
            "session.new_split",
            json!({
                "session_id": session, "anchor_id": block,
                "direction": "horizontal", "side": "after",
                "layout": {"block": {"flavor": "com.superlogical.terminal.shell"}},
            }),
        )
        .await?;
    recorder.settle().await?;
    recorder.call("session.view", of_session()).await?;
    recorder
        .call(
            "session.set_label",
            json!({ "session_id": session, "label": "lume-fixture-renamed" }),
        )
        .await?;
    recorder.settle().await?;
    recorder
        .call(
            "session.view",
            json!({ "session_id": "session:00000000-0000-7000-8000-000000000000" }),
        )
        .await?;
    recorder
        .call(
            "com.superlogical.terminal.size",
            json!({ "session_id": session, "block_id": "block:00000000-0000-7000-8000-000000000000" }),
        )
        .await?;
    recorder.call("no.such_method", Value::Null).await?;
    recorder.call("session.destroy", of_session()).await?;
    recorder.settle().await?;
    recorder.socket.close(None).await.ok();

    let named: Vec<(String, &str)> = [
        ("USERPROFILE", "/home/user"),
        ("HOME", "/home/user"),
        ("USERDOMAIN", "MACHINE"),
        ("COMPUTERNAME", "MACHINE"),
        ("HOSTNAME", "MACHINE"),
        ("USERNAME", "user"),
        ("USER", "user"),
    ]
    .into_iter()
    .filter_map(|(variable, with)| Some((std::env::var(variable).ok()?, with)))
    .chain(
        #[cfg(windows)]
        lume_ipc_sid(),
        #[cfg(not(windows))]
        None,
    )
    .collect();
    let mut record = json!({ "api_revision": API_REVISION, "messages": recorder.messages });
    scrub(&mut record, &named);
    std::fs::write(&out, serde_json::to_string_pretty(&record)? + "\n")?;
    println!(
        "{} messages written to {out}",
        record["messages"].as_array().map_or(0, Vec::len)
    );
    Ok(())
}

/// The user's security identifier, which names the server's pipe and the caller.
#[cfg(windows)]
fn lume_ipc_sid() -> Option<(String, &'static str)> {
    let whoami = std::process::Command::new("whoami")
        .arg("/user")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&whoami.stdout);
    let sid = text
        .split_whitespace()
        .find(|word| word.starts_with("S-1-"))?;
    Some((sid.to_owned(), "S-1-5-21-0-0-0-1000"))
}
