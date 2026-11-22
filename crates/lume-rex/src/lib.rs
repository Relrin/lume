//! A client of the Rex server.
//!
//! A Rex server runs terminals and keeps them running whether or not anything shows
//! them. Its clients connect to it to show and control them. This crate is such a
//! client, written from what a server says of itself: the page it serves at
//! `/llms.txt` and the method descriptions at `/api/help`.
//!
//! - [`Endpoint`] and [`dial`] reach a server: over its named pipe on Windows, or a
//!   Unix socket elsewhere.
//! - [`Control`] is the control connection: one WebSocket that carries every method
//!   call and every event.
//! - [`RexBackend`] is a server as the application sees any place sessions run.
//!
//! What a server sends is not trusted. Everything read from it is bounded and checked
//! before it is used, and nothing it sends can do more than be shown.

mod api;
mod backend;
mod control;
mod endpoint;
mod error;
mod ids;
mod transport;
pub mod tuning;

pub use api::{
    ApiRevisions, ClientInfo, ServerVersion, SessionSummary, View, ViewBlock, ViewLayer, ViewNode,
    ViewSplit, ViewWindow, Welcome,
};
pub use backend::{Dialer, Link, RexBackend, RexSession};
pub use control::{Control, Event, Events, Response};
pub use endpoint::Endpoint;
pub use error::RexError;
pub use transport::{Io, dial};

/// The result of something asked of a Rex server.
pub type Result<T> = std::result::Result<T, RexError>;
