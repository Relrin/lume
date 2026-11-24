//! Wire types and framing between the session host and its clients. No I/O.
//!
//! One connection carries two planes. The control plane is stream 0: JSON requests,
//! responses and notifications in `Control` frames. The data plane is every other stream,
//! one per attachment, carrying binary frames: terminal output and its markers one way,
//! input, sizes, client facts and credit the other way.
//!
//! The application's own control endpoint uses the control plane alone, with the
//! messages in [`app`].
//!
//! This crate only turns messages into bytes and back, and keeps the credit arithmetic.
//! Whoever owns the connection does the reading and writing.

pub mod app;
mod control;
mod decode;
mod error;
mod event;
mod flow;
mod frame;
mod payload;
mod secret;
pub mod tuning;
mod types;

pub use control::{
    ClientKind, Control, HydrateMode, Notification, Request, RequestBody, Response, ResponseBody,
    negotiate,
};
pub use decode::{ControlDecoder, FrameDecoder};
pub use error::{DecodeError, EncodeError, ErrorCode, FlowError, ProtoError};
pub use event::{
    BlockKind, CommandState, DisconnectReason, Disconnected, Foreground, ProgramState,
    ProgramStatus, SessionEvent, SessionState,
};
pub use flow::{Granter, ReceiveWindow, SendWindow};
pub use frame::{
    Body, BytesKind, CONTROL_STREAM, Header, Kind, Message, Role, encode, encode_bytes,
    encode_control_json,
};
pub use payload::HydratePayload;
pub use secret::Secret;
pub use types::{
    BackendCaps, BackendId, CellPx, ClientFacts, ColorScheme, ExitStatus, HydrateKinds,
    InvalidSize, Launcher, Palette, Responder, Rgb, SessionId, SessionInfo, SessionStats,
    SpawnSpec, StreamId, TermSize,
};

/// The protocol version this crate speaks.
pub const PROTO_VERSION: u32 = 2;

/// The oldest protocol version a client of this crate still speaks: a newer application
/// can use a host that an older one left running.
pub const PROTO_VERSION_MIN: u32 = 1;

/// The name the session host is served under when it runs on its own, for an
/// application that serves its control endpoint under the default name.
///
/// A build made for development has a name of its own, so that it neither uses nor
/// stops the host of an installed copy.
pub const HOST_ENDPOINT_NAME: &str = if cfg!(debug_assertions) {
    "host-dev"
} else {
    "host"
};

/// The name the session host is served under for an application whose control endpoint
/// is called `app_endpoint`. An application started under a name of its own, as tests
/// and trials are, has a host of its own too.
pub fn host_endpoint_name(app_endpoint: Option<&str>) -> String {
    match app_endpoint {
        None => HOST_ENDPOINT_NAME.to_owned(),
        Some(name) if name == app::ENDPOINT_NAME => HOST_ENDPOINT_NAME.to_owned(),
        Some(name) => format!("{name}-host"),
    }
}
