//! Frames: the header, the kinds, a decoded message, and encoding.
//!
//! A frame is a nine-byte header, `[u32 stream][u8 kind][u32 len]` in little-endian order,
//! followed by `len` payload bytes. The top bit of `kind` says the message continues in the
//! next frame of the same stream and kind; only `Control` and `Hydrate` messages may be
//! split that way. Terminal bytes (`Data`, `Input`) are a stream, so a long run is simply
//! sent as several frames.

use crate::control::Control;
use crate::error::{DecodeError, EncodeError};
use crate::payload::{self, HydratePayload};
use crate::tuning::{MAX_CONTROL_MESSAGE, MAX_HYDRATE, MAX_PAYLOAD};
use crate::types::{CellPx, ClientFacts, ExitStatus, SessionEvent, TermSize};

/// The stream that carries the control plane.
pub const CONTROL_STREAM: u32 = 0;

/// What a frame carries. The values are the wire encoding and never change meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Kind {
    /// One JSON control-plane message. Both directions, stream 0 only.
    Control = 0x01,
    /// The state a newly attached client starts from. Host to client.
    Hydrate = 0x02,
    /// Terminal output bytes. Host to client; counted against credit.
    Data = 0x03,
    /// The terminal was resized at this point of the output. Host to client.
    Resized = 0x04,
    /// A session event. Host to client.
    Event = 0x05,
    /// The program ended. Host to client.
    Exit = 0x06,
    /// Bytes to write to the terminal. Client to host.
    Input = 0x10,
    /// The size the client wants the terminal to have. Client to host.
    Resize = 0x11,
    /// The client's view of the session. Client to host.
    Facts = 0x12,
    /// More output bytes the client is ready to receive. Client to host.
    Credit = 0x13,
    /// The end of this attachment. Client to host.
    Detach = 0x14,
}

impl Kind {
    /// The bit of the kind byte that says the message continues in the next frame.
    pub const MORE: u8 = 0x80;

    /// Reads a kind from the kind byte with the continuation bit already removed.
    pub fn from_wire(byte: u8) -> Option<Self> {
        Some(match byte {
            0x01 => Self::Control,
            0x02 => Self::Hydrate,
            0x03 => Self::Data,
            0x04 => Self::Resized,
            0x05 => Self::Event,
            0x06 => Self::Exit,
            0x10 => Self::Input,
            0x11 => Self::Resize,
            0x12 => Self::Facts,
            0x13 => Self::Credit,
            0x14 => Self::Detach,
            _ => return None,
        })
    }

    /// Whether a message of this kind may span several frames.
    pub fn fragmentable(self) -> bool {
        matches!(self, Self::Control | Self::Hydrate)
    }

    /// Whether the host sends this kind.
    pub fn from_host(self) -> bool {
        matches!(
            self,
            Self::Control | Self::Hydrate | Self::Data | Self::Resized | Self::Event | Self::Exit
        )
    }

    /// Whether a client sends this kind.
    pub fn from_client(self) -> bool {
        matches!(
            self,
            Self::Control | Self::Input | Self::Resize | Self::Facts | Self::Credit | Self::Detach
        )
    }
}

/// Which end of the connection is decoding. Each end accepts only what the other may send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The host, reading what a client sent.
    Host,
    /// A client, reading what the host sent.
    Client,
}

impl Role {
    pub(crate) fn accepts(self, kind: Kind) -> bool {
        match self {
            Self::Host => kind.from_client(),
            Self::Client => kind.from_host(),
        }
    }
}

/// A frame header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub stream: u32,
    pub kind: Kind,
    /// The message continues in the next frame of this stream and kind.
    pub more: bool,
    pub len: u32,
}

impl Header {
    /// The encoded size of a header.
    pub const LEN: usize = 9;

    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.stream.to_le_bytes());
        out.push(self.kind as u8 | if self.more { Kind::MORE } else { 0 });
        out.extend_from_slice(&self.len.to_le_bytes());
    }

    /// Reads a header and checks the rules that depend on nothing else: a known kind, a
    /// payload within the limit, the right plane for the kind, and a continuation bit only
    /// where a message may be split.
    pub fn parse(bytes: &[u8; Self::LEN]) -> Result<Self, DecodeError> {
        let [s0, s1, s2, s3, kind_byte, l0, l1, l2, l3] = *bytes;
        let stream = u32::from_le_bytes([s0, s1, s2, s3]);
        let len = u32::from_le_bytes([l0, l1, l2, l3]);
        let more = kind_byte & Kind::MORE != 0;
        let kind =
            Kind::from_wire(kind_byte & !Kind::MORE).ok_or(DecodeError::UnknownKind(kind_byte))?;

        if len as usize > MAX_PAYLOAD {
            return Err(DecodeError::PayloadTooLarge(len));
        }
        if (kind == Kind::Control) != (stream == CONTROL_STREAM) {
            return Err(DecodeError::WrongPlane { kind, stream });
        }
        if more && !kind.fragmentable() {
            return Err(DecodeError::NotFragmentable(kind));
        }
        if more && len == 0 {
            return Err(DecodeError::EmptyFragment);
        }
        Ok(Self {
            stream,
            kind,
            more,
            len,
        })
    }
}

/// A decoded message and the stream it arrived on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub stream: u32,
    pub body: Body,
}

impl Message {
    pub fn new(stream: u32, body: Body) -> Self {
        Self { stream, body }
    }

    /// A control-plane message, which always travels on stream 0.
    pub fn control(control: Control) -> Self {
        Self {
            stream: CONTROL_STREAM,
            body: Body::Control(control),
        }
    }
}

/// What a message carries, by kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Control(Control),
    Hydrate(HydratePayload),
    Data(Vec<u8>),
    Resized(TermSize),
    Event(SessionEvent),
    Exit(ExitStatus),
    Input(Vec<u8>),
    Resize { size: TermSize, cell_px: CellPx },
    Facts(ClientFacts),
    Credit(u32),
    Detach,
}

impl Body {
    pub fn kind(&self) -> Kind {
        match self {
            Self::Control(_) => Kind::Control,
            Self::Hydrate(_) => Kind::Hydrate,
            Self::Data(_) => Kind::Data,
            Self::Resized(_) => Kind::Resized,
            Self::Event(_) => Kind::Event,
            Self::Exit(_) => Kind::Exit,
            Self::Input(_) => Kind::Input,
            Self::Resize { .. } => Kind::Resize,
            Self::Facts(_) => Kind::Facts,
            Self::Credit(_) => Kind::Credit,
            Self::Detach => Kind::Detach,
        }
    }
}

/// The two kinds that are plain runs of terminal bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BytesKind {
    Data,
    Input,
}

/// Appends the frames of one message to `out`.
///
/// `Data` and `Input` longer than one payload become several frames; the receiver sees
/// several messages whose bytes join to the original. `Control` and `Hydrate` longer than
/// one payload are split with the continuation bit and arrive as one message.
pub fn encode(message: &Message, out: &mut Vec<u8>) -> Result<(), EncodeError> {
    let stream = message.stream;
    let kind = message.body.kind();
    if (kind == Kind::Control) != (stream == CONTROL_STREAM) {
        return Err(EncodeError::WrongPlane { kind, stream });
    }

    match &message.body {
        Body::Control(control) => {
            let json = serde_json::to_vec(control).map_err(|e| EncodeError::Unrepresentable {
                kind,
                reason: e.to_string(),
            })?;
            if json.len() > MAX_CONTROL_MESSAGE {
                return Err(EncodeError::MessageTooLarge(kind));
            }
            write_split(stream, kind, &json, true, out);
        }
        Body::Hydrate(hydrate) => {
            let bytes = payload::encode_hydrate(hydrate)?;
            if bytes.len() > MAX_HYDRATE {
                return Err(EncodeError::MessageTooLarge(kind));
            }
            write_split(stream, kind, &bytes, true, out);
        }
        Body::Data(bytes) | Body::Input(bytes) => write_split(stream, kind, bytes, false, out),
        Body::Resized(size) => write_one(stream, kind, &payload::encode_resized(*size), out),
        Body::Event(event) => {
            let json = serde_json::to_vec(event).map_err(|e| EncodeError::Unrepresentable {
                kind,
                reason: e.to_string(),
            })?;
            if json.len() > MAX_PAYLOAD {
                return Err(EncodeError::MessageTooLarge(kind));
            }
            write_one(stream, kind, &json, out);
        }
        Body::Exit(status) => write_one(stream, kind, &payload::encode_exit(*status), out),
        Body::Resize { size, cell_px } => {
            write_one(stream, kind, &payload::encode_resize(*size, *cell_px), out);
        }
        Body::Facts(facts) => write_one(stream, kind, &payload::encode_facts(facts), out),
        Body::Credit(bytes) => write_one(stream, kind, &bytes.to_le_bytes(), out),
        Body::Detach => write_one(stream, kind, &[], out),
    }
    Ok(())
}

/// Appends terminal bytes as `Data` or `Input` frames without building a [`Message`].
/// This is the path output and input take, so it copies the bytes once and allocates
/// nothing. `stream` must be a data-plane stream.
pub fn encode_bytes(stream: u32, kind: BytesKind, bytes: &[u8], out: &mut Vec<u8>) {
    debug_assert_ne!(
        stream, CONTROL_STREAM,
        "terminal bytes never travel on the control stream"
    );
    let kind = match kind {
        BytesKind::Data => Kind::Data,
        BytesKind::Input => Kind::Input,
    };
    write_split(stream, kind, bytes, false, out);
}

/// Appends one control-plane message that is already JSON, split across frames where it
/// is longer than one payload.
///
/// The control plane of a connection is JSON whoever speaks on it; which messages those
/// are is the business of the two ends. This is the framing alone, for ends whose
/// messages are not the session host's.
pub fn encode_control_json(json: &[u8], out: &mut Vec<u8>) -> Result<(), EncodeError> {
    if json.len() > MAX_CONTROL_MESSAGE {
        return Err(EncodeError::MessageTooLarge(Kind::Control));
    }
    write_split(CONTROL_STREAM, Kind::Control, json, true, out);
    Ok(())
}

fn write_one(stream: u32, kind: Kind, payload: &[u8], out: &mut Vec<u8>) {
    debug_assert!(payload.len() <= MAX_PAYLOAD);
    Header {
        stream,
        kind,
        more: false,
        len: payload.len() as u32,
    }
    .write(out);
    out.extend_from_slice(payload);
}

/// Writes `bytes` as as many frames as it takes. With `continued`, every frame but the
/// last carries the continuation bit, so the receiver joins them into one message.
fn write_split(stream: u32, kind: Kind, bytes: &[u8], continued: bool, out: &mut Vec<u8>) {
    if bytes.is_empty() {
        write_one(stream, kind, bytes, out);
        return;
    }
    out.reserve(bytes.len() + bytes.len().div_ceil(MAX_PAYLOAD) * Header::LEN);
    let mut chunks = bytes.chunks(MAX_PAYLOAD).peekable();
    while let Some(chunk) = chunks.next() {
        let more = continued && chunks.peek().is_some();
        Header {
            stream,
            kind,
            more,
            len: chunk.len() as u32,
        }
        .write(out);
        out.extend_from_slice(chunk);
    }
}
