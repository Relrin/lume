//! A client of the Rex server.
//!
//! A Rex server runs terminals and keeps them running whether or not anything shows
//! them. Its clients connect to it to show and control them. This crate is such a
//! client, written from what a server says of itself: the page it serves at
//! `/llms.txt` and the method descriptions at `/api/help`.
//!
//! - [`Endpoint`] and [`dial`] reach a server: over its named pipe on Windows or a
//!   Unix socket elsewhere when it is on this machine, over a network when it is not.
//! - [`Control`] is the control connection: one WebSocket that carries every method
//!   call and every event.
//! - [`DataConn`] is a data connection: one WebSocket that carries what one terminal
//!   prints. [`Opening`] and [`Splitter`] take such a stream apart.
//! - [`RexBackend`] is a server as the application sees any place sessions run.
//!
//! What a server sends is not trusted. Everything read from it is bounded and checked
//! before it is used, and nothing it sends can do more than be shown.

mod api;
mod authority;
mod backend;
mod control;
mod data;
mod endpoint;
mod error;
mod events;
mod ids;
mod input;
mod intents;
pub mod layout;
mod pulse;
mod screen;
mod ssh;
mod stream;
mod terminal;
mod tls;
mod transport;
pub mod tuning;
mod ws;

pub use api::{
    ApiRevisions, ClientInfo, ServerVersion, SessionSummary, View, ViewBlock, ViewLayer, ViewNode,
    ViewSplit, ViewWindow, Welcome,
};
pub use backend::{Dialer, Link, RexBackend, RexSession};
pub use control::{Control, Event, Events, Response};
pub use data::{
    Answer, DataConn, DataEnd, DataOpen, DataWelcome, Reject, ResumeToken, Ticket, hello,
    read_answer, resume,
};
pub use endpoint::Endpoint;
pub use error::RexError;
pub use ssh::SshOptions;
pub use stream::{Note, Opened, Opening, Piece, Splitter, Theme};
pub use tls::Trust;
pub use transport::{Dialed, Io, LOCAL_AUTHORITY, Options, Reach, dial, dial_with, listens};

/// The result of something asked of a Rex server.
pub type Result<T> = std::result::Result<T, RexError>;
