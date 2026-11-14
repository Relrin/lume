//! Wire types and framing between the session host and its clients. No I/O.
//!
//! One connection carries two planes. The control plane is stream 0: JSON requests,
//! responses and notifications in `Control` frames. The data plane is every other stream,
//! one per attachment, carrying binary frames: terminal output and its markers one way,
//! input, sizes, client facts and credit the other way.
//!
//! This crate only turns messages into bytes and back, and keeps the credit arithmetic.
//! Whoever owns the connection does the reading and writing.

mod control;
mod decode;
mod error;
mod flow;
mod frame;
mod payload;
pub mod tuning;
mod types;

pub use control::{
    ClientKind, Control, HydrateMode, Notification, Request, RequestBody, Response, ResponseBody,
    negotiate,
};
pub use decode::FrameDecoder;
pub use error::{DecodeError, EncodeError, ErrorCode, FlowError, ProtoError};
pub use flow::{Granter, ReceiveWindow, SendWindow};
pub use frame::{
    Body, BytesKind, CONTROL_STREAM, Header, Kind, Message, Role, encode, encode_bytes,
};
pub use payload::HydratePayload;
pub use types::{
    BackendCaps, BackendId, CellPx, ClientFacts, ColorScheme, ExitStatus, HydrateKinds,
    InvalidSize, Launcher, Palette, Responder, Rgb, SessionEvent, SessionId, SessionInfo,
    SessionStats, SpawnSpec, StreamId, TermSize,
};

/// The protocol version this crate speaks.
pub const PROTO_VERSION: u32 = 1;
