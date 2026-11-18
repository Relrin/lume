//! Turning received bytes back into messages, whatever sizes the bytes arrive in.

use std::collections::HashMap;

use crate::error::DecodeError;
use crate::frame::{Body, Header, Kind, Message, Role};
use crate::payload;
use crate::tuning::{MAX_CONTROL_MESSAGE, MAX_HYDRATE};

/// An incremental frame decoder.
///
/// Feed it bytes with [`push`](Self::push) as they arrive and take messages with
/// [`next`](Self::next) until it returns `Ok(None)`. Chunk boundaries carry no meaning: a
/// header or payload may be split anywhere.
///
/// The first error is final. A framing error leaves the position of the next frame
/// unknown, so every later call returns the same error and the connection must be closed.
#[derive(Debug)]
pub struct FrameDecoder {
    role: Role,
    buffer: Vec<u8>,
    /// How much of `buffer` has been consumed.
    read: usize,
    /// Messages whose frames have not all arrived, by stream. Streams are independent, so
    /// frames of other streams may come between two frames of one message.
    unfinished: HashMap<u32, Unfinished>,
    failed: Option<DecodeError>,
}

#[derive(Debug)]
struct Unfinished {
    kind: Kind,
    bytes: Vec<u8>,
}

impl FrameDecoder {
    pub fn new(role: Role) -> Self {
        Self {
            role,
            buffer: Vec::new(),
            read: 0,
            unfinished: HashMap::new(),
            failed: None,
        }
    }

    /// Adds received bytes. After an error they are discarded.
    pub fn push(&mut self, bytes: &[u8]) {
        if self.failed.is_some() {
            return;
        }
        if self.read > 0 {
            self.buffer.drain(..self.read);
            self.read = 0;
        }
        self.buffer.extend_from_slice(bytes);
    }

    /// The next complete message, or `Ok(None)` when more bytes are needed.
    #[allow(clippy::should_implement_trait)] // Fallible, and not an iterator: more may come.
    pub fn next(&mut self) -> Result<Option<Message>, DecodeError> {
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        match self.advance() {
            Ok(message) => Ok(message),
            Err(error) => {
                self.failed = Some(error.clone());
                self.buffer = Vec::new();
                self.read = 0;
                self.unfinished = HashMap::new();
                Err(error)
            }
        }
    }

    fn advance(&mut self) -> Result<Option<Message>, DecodeError> {
        loop {
            let available = &self.buffer[self.read..];
            let Some(header_bytes) = available.first_chunk::<{ Header::LEN }>() else {
                return Ok(None);
            };
            // The header is checked before its payload arrives, so a bad frame is refused
            // without waiting for bytes that may never come.
            let header = Header::parse(header_bytes)?;
            if !self.role.accepts(header.kind) {
                return Err(DecodeError::WrongDirection(header.kind));
            }
            let end = Header::LEN + header.len as usize;
            let Some(frame) = available.get(..end) else {
                return Ok(None);
            };
            let payload = &frame[Header::LEN..];

            let Header {
                stream, kind, more, ..
            } = header;
            let complete = match self.unfinished.get_mut(&stream) {
                Some(unfinished) => {
                    if unfinished.kind != kind {
                        return Err(DecodeError::Interrupted {
                            stream,
                            unfinished: unfinished.kind,
                            got: kind,
                        });
                    }
                    if unfinished.bytes.len() + payload.len() > message_limit(kind) {
                        return Err(DecodeError::MessageTooLarge(kind));
                    }
                    unfinished.bytes.extend_from_slice(payload);
                    if more {
                        None
                    } else {
                        self.unfinished.remove(&stream).map(|done| done.bytes)
                    }
                }
                None if more => {
                    self.unfinished.insert(
                        stream,
                        Unfinished {
                            kind,
                            bytes: payload.to_vec(),
                        },
                    );
                    None
                }
                None => Some(payload.to_vec()),
            };
            self.read += end;

            if let Some(bytes) = complete {
                let body = decode_body(kind, bytes)?;
                return Ok(Some(Message { stream, body }));
            }
        }
    }
}

/// An incremental decoder for a connection that carries the control plane and nothing
/// else. It yields each message's JSON as it was sent, for the caller to read as its own
/// kind of message.
///
/// It follows the same rules as [`FrameDecoder`]: chunk boundaries carry no meaning, a
/// long message arrives in several frames and is joined, and the first error is final.
#[derive(Debug, Default)]
pub struct ControlDecoder {
    buffer: Vec<u8>,
    /// How much of `buffer` has been consumed.
    read: usize,
    /// The frames so far of a message that is not complete.
    unfinished: Option<Vec<u8>>,
    failed: Option<DecodeError>,
}

impl ControlDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds received bytes. After an error they are discarded.
    pub fn push(&mut self, bytes: &[u8]) {
        if self.failed.is_some() {
            return;
        }
        if self.read > 0 {
            self.buffer.drain(..self.read);
            self.read = 0;
        }
        self.buffer.extend_from_slice(bytes);
    }

    /// The bytes received after the last complete message, for a connection that goes
    /// on with frames of other kinds, read by a [`FrameDecoder`] from here.
    pub fn into_rest(mut self) -> Vec<u8> {
        self.buffer.split_off(self.read)
    }

    /// The JSON of the next complete message, or `Ok(None)` when more bytes are needed.
    #[allow(clippy::should_implement_trait)] // Fallible, and not an iterator: more may come.
    pub fn next(&mut self) -> Result<Option<Vec<u8>>, DecodeError> {
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        match self.advance() {
            Ok(message) => Ok(message),
            Err(error) => {
                self.failed = Some(error.clone());
                self.buffer = Vec::new();
                self.read = 0;
                self.unfinished = None;
                Err(error)
            }
        }
    }

    fn advance(&mut self) -> Result<Option<Vec<u8>>, DecodeError> {
        loop {
            let available = &self.buffer[self.read..];
            let Some(header_bytes) = available.first_chunk::<{ Header::LEN }>() else {
                return Ok(None);
            };
            let header = Header::parse(header_bytes)?;
            if header.kind != Kind::Control {
                return Err(DecodeError::NotControl(header.kind));
            }
            let end = Header::LEN + header.len as usize;
            let Some(frame) = available.get(..end) else {
                return Ok(None);
            };
            let payload = &frame[Header::LEN..];

            let mut bytes = self.unfinished.take().unwrap_or_default();
            if bytes.len() + payload.len() > MAX_CONTROL_MESSAGE {
                return Err(DecodeError::MessageTooLarge(Kind::Control));
            }
            bytes.extend_from_slice(payload);
            self.read += end;
            if header.more {
                self.unfinished = Some(bytes);
            } else {
                return Ok(Some(bytes));
            }
        }
    }
}

/// The most bytes a message of this kind may have once its frames are joined.
fn message_limit(kind: Kind) -> usize {
    match kind {
        Kind::Hydrate => MAX_HYDRATE,
        _ => MAX_CONTROL_MESSAGE,
    }
}

fn decode_body(kind: Kind, bytes: Vec<u8>) -> Result<Body, DecodeError> {
    let malformed = |e: serde_json::Error| DecodeError::BadPayload {
        kind,
        reason: e.to_string(),
    };
    Ok(match kind {
        Kind::Control => Body::Control(serde_json::from_slice(&bytes).map_err(malformed)?),
        Kind::Hydrate => Body::Hydrate(payload::decode_hydrate(&bytes)?),
        Kind::Data => Body::Data(bytes),
        Kind::Resized => Body::Resized(payload::decode_resized(&bytes)?),
        Kind::Event => Body::Event(serde_json::from_slice(&bytes).map_err(malformed)?),
        Kind::Exit => Body::Exit(payload::decode_exit(&bytes)?),
        Kind::Input => Body::Input(bytes),
        Kind::Resize => {
            let (size, cell_px) = payload::decode_resize(&bytes)?;
            Body::Resize { size, cell_px }
        }
        Kind::Facts => Body::Facts(payload::decode_facts(&bytes)?),
        Kind::Credit => Body::Credit(payload::decode_credit(&bytes)?),
        Kind::Detach => {
            payload::decode_empty(Kind::Detach, &bytes)?;
            Body::Detach
        }
        Kind::Rehydrate => {
            payload::decode_empty(Kind::Rehydrate, &bytes)?;
            Body::Rehydrate
        }
        Kind::Release => {
            payload::decode_empty(Kind::Release, &bytes)?;
            Body::Release
        }
    })
}
