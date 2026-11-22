//! A local endpoint that only the current user can reach.
//!
//! On Windows it is a named pipe whose access list names the user alone. Elsewhere it is
//! a Unix socket, readable and writable by its owner only, in a directory nobody else can
//! enter. Either way a connection is a byte stream in both directions.
//!
//! What this defends against is another user of the same machine. Another process of
//! the same user can connect, and that is accepted: such a process can already do
//! anything the user can.
//!
//! An endpoint is created exclusively. If something already serves the name, binding
//! fails instead of joining or replacing it, so a process that got there first cannot be
//! mistaken for the real server by the one that binds, and a client checks that what it
//! reached belongs to the same user.

use std::fmt;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

mod detach;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
use windows as platform;

pub use detach::{not_served, spawn_detached, start_and_connect};

/// The current user's security identifier in its textual form, `S-1-5-21-…`. Servers
/// that keep an endpoint per user on Windows name it after this.
#[cfg(windows)]
pub fn user_sid() -> io::Result<&'static str> {
    platform::user_sid()
}

/// The longest endpoint name. Socket paths are limited to about a hundred bytes on some
/// systems, directory included.
const MAX_NAME_LEN: usize = 40;

/// Where an endpoint lives for the current user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    name: String,
    address: platform::Address,
}

impl Endpoint {
    /// The endpoint called `name` for the current user. Two processes of the same user
    /// that use the same name get the same endpoint; different users never do.
    ///
    /// A name is one to forty characters from `a-z`, `0-9` and `-`.
    pub fn named(name: &str) -> io::Result<Self> {
        let well_formed = (1..=MAX_NAME_LEN).contains(&name.len())
            && name
                .bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'));
        if !well_formed {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("`{name}` is not a valid endpoint name"),
            ));
        }
        Ok(Self {
            name: name.to_owned(),
            address: platform::address(name)?,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for Endpoint {
    /// The pipe name or socket path.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        platform::display(&self.address, f)
    }
}

/// The serving side of an endpoint.
pub struct Listener(platform::Listener);

impl Listener {
    /// Creates the endpoint. Fails with [`io::ErrorKind::AddrInUse`] if it is already
    /// being served. Must be called from within a Tokio runtime.
    pub fn bind(endpoint: &Endpoint) -> io::Result<Self> {
        platform::Listener::bind(&endpoint.address).map(Self)
    }

    /// Waits for the next client. A connection from another user is dropped and the
    /// wait continues.
    pub async fn accept(&mut self) -> io::Result<Stream> {
        self.0.accept().await.map(Stream)
    }
}

/// Connects to an endpoint. Fails if nothing serves it, or if what serves it belongs to
/// another user.
pub async fn connect(endpoint: &Endpoint) -> io::Result<Stream> {
    platform::connect(&endpoint.address).await.map(Stream)
}

/// One connection: a byte stream in both directions.
pub struct Stream(platform::Stream);

impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_short_lowercase_words() {
        for name in ["app", "host", "test-01", "a"] {
            assert!(Endpoint::named(name).is_ok(), "{name}");
        }
        let too_long = "x".repeat(MAX_NAME_LEN + 1);
        for name in [
            "",
            "App",
            "a b",
            "a/b",
            r"a\b",
            "a.b",
            "..",
            too_long.as_str(),
        ] {
            assert!(Endpoint::named(name).is_err(), "{name:?} was accepted");
        }
    }

    #[test]
    fn the_same_name_gives_the_same_endpoint() {
        assert_eq!(
            Endpoint::named("app").unwrap(),
            Endpoint::named("app").unwrap()
        );
        assert_ne!(
            Endpoint::named("app").unwrap(),
            Endpoint::named("host").unwrap()
        );
    }
}
