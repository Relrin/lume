//! What can be said of an address before the server is listed: how it is read, whether
//! the machine answers, and whether a Rex server does.
//!
//! Nothing here is kept. A connection made to try an address is this module's own, is in
//! no list, and ends with the attempt.

use std::sync::Arc;
use std::time::Instant;

use lume_proto::BackendId;
use lume_rex::{ClientInfo, Endpoint, Link, Options, RexBackend};
use lume_session::Launcher;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::task::JoinSet;

use super::hosts::command_error;
use crate::error::CommandError;
use crate::host_probe::{self, Probe};
use crate::tuning::{HOST_PROBE_TIMEOUT, REX_CONNECT_WAIT};

/// The ports a host alone is looked for on, as the client tries them.
const HOST_PORTS: [u16; 2] = [lume_rex::HTTPS_PORT, lume_rex::REX_PORT];

/// The parts of an address, as a page shows how it is read.
pub fn parts(endpoint: &Endpoint) -> Value {
    let part = |kind: &str, host: &str, port: Option<u16>| json!({ "kind": kind, "host": host, "port": port });
    match endpoint {
        Endpoint::Local => json!({ "kind": "local" }),
        Endpoint::Pipe(name) => json!({ "kind": "pipe", "path": name }),
        Endpoint::Unix(path) => json!({ "kind": "unix", "path": path.display().to_string() }),
        Endpoint::Https { host, port } => part("https", host, Some(*port)),
        Endpoint::Http { host, port } => part("http", host, Some(*port)),
        Endpoint::Tailnet { host, port } => part("tailnet", host, Some(*port)),
        Endpoint::Host { host, port } => part("host", host, *port),
        Endpoint::Ssh {
            user,
            host,
            port,
            socket,
        } => json!({
            "kind": "ssh",
            "user": user,
            "host": host,
            "port": port,
            "path": socket,
        }),
    }
}

/// How long the machine at an address takes to answer, or why that is not known. Whether
/// it has a Rex server is not asked.
pub async fn reach(endpoint: &Endpoint) -> Probe {
    let ports: Vec<u16> = match endpoint {
        Endpoint::Local | Endpoint::Pipe(_) | Endpoint::Unix(_) => {
            return if lume_rex::listens(endpoint).await {
                Probe::within(0)
            } else {
                Probe::without("nothing listens there")
            };
        }
        Endpoint::Ssh {
            user, host, port, ..
        } => {
            return host_probe::probe(Launcher::Ssh {
                host: host.clone(),
                user: user.clone(),
                port: *port,
                extra_args: Vec::new(),
                client: None,
            })
            .await;
        }
        Endpoint::Https { port, .. }
        | Endpoint::Http { port, .. }
        | Endpoint::Tailnet { port, .. } => vec![*port],
        Endpoint::Host { port, .. } => port.map_or_else(|| HOST_PORTS.to_vec(), |port| vec![port]),
    };
    let (Endpoint::Https { host, .. }
    | Endpoint::Http { host, .. }
    | Endpoint::Tailnet { host, .. }
    | Endpoint::Host { host, .. }) = endpoint
    else {
        return Probe::without("the address names no machine");
    };
    let began = Instant::now();
    // The first that opens is enough; the others end with the set.
    let mut tried = JoinSet::new();
    for port in ports {
        let host = host.clone();
        tried.spawn(async move { TcpStream::connect((host.as_str(), port)).await.is_ok() });
    }
    let any = async {
        while let Some(opened) = tried.join_next().await {
            if matches!(opened, Ok(true)) {
                return true;
            }
        }
        false
    };
    match tokio::time::timeout(HOST_PROBE_TIMEOUT, any).await {
        Ok(true) => Probe::within(u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX)),
        Ok(false) => Probe::without("the machine did not take the connection"),
        Err(_) => Probe::without("the machine did not answer in time"),
    }
}

/// Ends the connection of an attempt, however it came out.
struct Ends(Arc<RexBackend>);

impl Drop for Ends {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

/// Connects to the server at an address, to learn what it is, and lets go of it again.
/// `agreed` is whether the user agreed to a connection that is not encrypted.
pub async fn attempt(endpoint: Endpoint, agreed: bool) -> Result<Value, CommandError> {
    let backend = RexBackend::connect_with(
        BackendId(format!("rex-attempt:{endpoint}")),
        endpoint,
        ClientInfo::app("Lume", env!("CARGO_PKG_VERSION")),
        Options::new().plain_http_agreed(agreed),
    );
    let ends = Ends(Arc::clone(&backend));
    let mut link = backend.link();
    let standing = async {
        loop {
            let stands = link.borrow_and_update().clone();
            match stands {
                Link::Connected(version) => return Ok(version),
                // One that was never made is not waited for.
                Link::Waiting { error, .. } => {
                    return Err(CommandError::new("unreachable", error));
                }
                Link::Refused { code, error } => return Err(CommandError::new(code, error)),
                Link::Closed => return Err(CommandError::new("disconnected", "it was ended")),
                Link::Connecting => {}
            }
            if link.changed().await.is_err() {
                return Err(CommandError::new("disconnected", "it was ended"));
            }
        }
    };
    let version = tokio::time::timeout(REX_CONNECT_WAIT, standing)
        .await
        .unwrap_or_else(|_| {
            Err(CommandError::new(
                "timeout",
                "the server did not answer in time",
            ))
        })?;
    let took = backend
        .round_trip()
        .await
        .map_err(|error| command_error(&error))?;
    drop(ends);
    Ok(json!({
        "version": version.rex_version,
        "platform": version.platform,
        "ms": u64::try_from(took.as_millis()).unwrap_or(u64::MAX),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(address: &str) -> Value {
        parts(&Endpoint::parse(address).unwrap())
    }

    #[test]
    fn an_address_is_shown_in_its_parts() {
        assert_eq!(
            read("ssh://me@Build:2222/run/rex.sock"),
            json!({ "kind": "ssh", "user": "me", "host": "build", "port": 2222, "path": "/run/rex.sock" })
        );
        assert_eq!(
            read("https://build"),
            json!({ "kind": "https", "host": "build", "port": 443 })
        );
        assert_eq!(
            read("build"),
            json!({ "kind": "host", "host": "build", "port": null })
        );
        assert_eq!(read("local"), json!({ "kind": "local" }));
    }

    #[tokio::test]
    async fn a_machine_that_answers_is_timed_and_one_that_does_not_is_not() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let address = format!("http://127.0.0.1:{port}");
        let there = reach(&Endpoint::parse(&address).unwrap()).await;
        assert!(there.ms.is_some(), "{there:?}");
        drop(listener);
        let gone = reach(&Endpoint::parse(&address).unwrap()).await;
        assert!(gone.ms.is_none() && gone.note.is_some(), "{gone:?}");
        let gone = reach(&Endpoint::Pipe(r"\\.\pipe\lume-no-such-pipe".to_owned())).await;
        assert_eq!(gone.ms, None);
    }
}
