//! The binary payload layouts. All integers are little-endian.
//!
//! | Kind      | Layout                                                                    |
//! |-----------|---------------------------------------------------------------------------|
//! | `Resized` | `u16 cols, u16 rows`                                                      |
//! | `Exit`    | `u8 how` (0 code, 1 signal, 2 unknown), `u32 value` (zero when unknown)   |
//! | `Resize`  | `u16 cols, u16 rows, u16 cell_width_px, u16 cell_height_px`               |
//! | `Facts`   | `u8 flags, u8 0, u16 cols, u16 rows, u16 cell_width_px,`                  |
//! |           | `u16 cell_height_px`, then 19 colours of three bytes: foreground,         |
//! |           | background, cursor, ANSI 0 to 15                                          |
//! | `Credit`  | `u32 bytes`                                                               |
//! | `Detach`  | empty                                                                     |
//! | `Hydrate` | `u8 form` (1 snapshot, 2 replay); a snapshot adds `u8 n` and `n` bytes    |
//! |           | naming the engine version that wrote it; then the bytes                   |

use crate::error::{DecodeError, EncodeError};
use crate::frame::Kind;
use crate::types::{CellPx, ClientFacts, ColorScheme, ExitStatus, Palette, Rgb, TermSize};

pub(crate) const RESIZED_LEN: usize = 4;
pub(crate) const EXIT_LEN: usize = 5;
pub(crate) const RESIZE_LEN: usize = 8;
pub(crate) const FACTS_LEN: usize = 67;
pub(crate) const CREDIT_LEN: usize = 4;

const FACTS_FOCUSED: u8 = 1 << 0;
const FACTS_VISIBLE: u8 = 1 << 1;
const FACTS_DARK: u8 = 1 << 2;
const FACTS_KNOWN: u8 = FACTS_FOCUSED | FACTS_VISIBLE | FACTS_DARK;

const EXIT_CODE: u8 = 0;
const EXIT_SIGNAL: u8 = 1;
const EXIT_UNKNOWN: u8 = 2;

const HYDRATE_SNAPSHOT: u8 = 1;
const HYDRATE_REPLAY: u8 = 2;

/// The state a newly attached client starts from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HydratePayload {
    /// A serialised terminal. Only an engine of the same version can load it.
    Snapshot { vt_version: String, bytes: Vec<u8> },
    /// Terminal output that rebuilds the screen when it is fed to any engine.
    VtReplay(Vec<u8>),
}

pub(crate) fn encode_resized(size: TermSize) -> [u8; RESIZED_LEN] {
    let [c0, c1] = size.cols().to_le_bytes();
    let [r0, r1] = size.rows().to_le_bytes();
    [c0, c1, r0, r1]
}

pub(crate) fn decode_resized(payload: &[u8]) -> Result<TermSize, DecodeError> {
    let bytes: &[u8; RESIZED_LEN] = fixed(Kind::Resized, payload)?;
    size(Kind::Resized, u16_at(bytes, 0), u16_at(bytes, 2))
}

pub(crate) fn encode_exit(status: ExitStatus) -> [u8; EXIT_LEN] {
    let (how, value) = match status {
        ExitStatus::Code(code) => (EXIT_CODE, code),
        ExitStatus::Signal(signal) => (EXIT_SIGNAL, signal),
        ExitStatus::Unknown => (EXIT_UNKNOWN, 0),
    };
    let [v0, v1, v2, v3] = value.to_le_bytes();
    [how, v0, v1, v2, v3]
}

pub(crate) fn decode_exit(payload: &[u8]) -> Result<ExitStatus, DecodeError> {
    let bytes: &[u8; EXIT_LEN] = fixed(Kind::Exit, payload)?;
    let value = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
    match (bytes[0], value) {
        (EXIT_CODE, code) => Ok(ExitStatus::Code(code)),
        (EXIT_SIGNAL, signal) => Ok(ExitStatus::Signal(signal)),
        (EXIT_UNKNOWN, 0) => Ok(ExitStatus::Unknown),
        (EXIT_UNKNOWN, _) => Err(bad(Kind::Exit, "an unknown status carries a value")),
        _ => Err(bad(Kind::Exit, "unknown status form")),
    }
}

pub(crate) fn encode_resize(size: TermSize, cell_px: CellPx) -> [u8; RESIZE_LEN] {
    let mut out = [0; RESIZE_LEN];
    put_u16(&mut out, 0, size.cols());
    put_u16(&mut out, 2, size.rows());
    put_u16(&mut out, 4, cell_px.w);
    put_u16(&mut out, 6, cell_px.h);
    out
}

pub(crate) fn decode_resize(payload: &[u8]) -> Result<(TermSize, CellPx), DecodeError> {
    let bytes: &[u8; RESIZE_LEN] = fixed(Kind::Resize, payload)?;
    let size = size(Kind::Resize, u16_at(bytes, 0), u16_at(bytes, 2))?;
    Ok((
        size,
        CellPx {
            w: u16_at(bytes, 4),
            h: u16_at(bytes, 6),
        },
    ))
}

pub(crate) fn encode_facts(facts: &ClientFacts) -> [u8; FACTS_LEN] {
    let mut out = [0; FACTS_LEN];
    let mut flags = 0;
    if facts.focused {
        flags |= FACTS_FOCUSED;
    }
    if facts.visible {
        flags |= FACTS_VISIBLE;
    }
    if facts.scheme == ColorScheme::Dark {
        flags |= FACTS_DARK;
    }
    out[0] = flags;
    put_u16(&mut out, 2, facts.size.cols());
    put_u16(&mut out, 4, facts.size.rows());
    put_u16(&mut out, 6, facts.cell_px.w);
    put_u16(&mut out, 8, facts.cell_px.h);

    let palette = &facts.palette;
    let colours = [palette.fg, palette.bg, palette.cursor]
        .into_iter()
        .chain(palette.ansi);
    for (slot, colour) in out[10..].as_chunks_mut::<3>().0.iter_mut().zip(colours) {
        *slot = colour.0;
    }
    out
}

pub(crate) fn decode_facts(payload: &[u8]) -> Result<ClientFacts, DecodeError> {
    let bytes: &[u8; FACTS_LEN] = fixed(Kind::Facts, payload)?;
    let flags = bytes[0];
    if flags & !FACTS_KNOWN != 0 {
        return Err(bad(Kind::Facts, "unknown flag bits are set"));
    }
    if bytes[1] != 0 {
        return Err(bad(Kind::Facts, "the reserved byte is not zero"));
    }
    let size = size(Kind::Facts, u16_at(bytes, 2), u16_at(bytes, 4))?;
    let cell_px = CellPx {
        w: u16_at(bytes, 6),
        h: u16_at(bytes, 8),
    };

    let mut colours = bytes[10..]
        .as_chunks::<3>()
        .0
        .iter()
        .map(|colour| Rgb(*colour));
    let mut next = || colours.next().unwrap_or_default();
    let palette = Palette {
        fg: next(),
        bg: next(),
        cursor: next(),
        ansi: std::array::from_fn(|_| next()),
    };

    Ok(ClientFacts {
        size,
        cell_px,
        palette,
        scheme: if flags & FACTS_DARK != 0 {
            ColorScheme::Dark
        } else {
            ColorScheme::Light
        },
        focused: flags & FACTS_FOCUSED != 0,
        visible: flags & FACTS_VISIBLE != 0,
    })
}

pub(crate) fn decode_credit(payload: &[u8]) -> Result<u32, DecodeError> {
    let bytes: &[u8; CREDIT_LEN] = fixed(Kind::Credit, payload)?;
    Ok(u32::from_le_bytes(*bytes))
}

pub(crate) fn decode_detach(payload: &[u8]) -> Result<(), DecodeError> {
    let _: &[u8; 0] = fixed(Kind::Detach, payload)?;
    Ok(())
}

pub(crate) fn encode_hydrate(hydrate: &HydratePayload) -> Result<Vec<u8>, EncodeError> {
    match hydrate {
        HydratePayload::Snapshot { vt_version, bytes } => {
            let version_len =
                u8::try_from(vt_version.len()).map_err(|_| EncodeError::Unrepresentable {
                    kind: Kind::Hydrate,
                    reason: "the engine version is longer than 255 bytes".to_owned(),
                })?;
            let mut out = Vec::with_capacity(2 + vt_version.len() + bytes.len());
            out.push(HYDRATE_SNAPSHOT);
            out.push(version_len);
            out.extend_from_slice(vt_version.as_bytes());
            out.extend_from_slice(bytes);
            Ok(out)
        }
        HydratePayload::VtReplay(bytes) => {
            let mut out = Vec::with_capacity(1 + bytes.len());
            out.push(HYDRATE_REPLAY);
            out.extend_from_slice(bytes);
            Ok(out)
        }
    }
}

pub(crate) fn decode_hydrate(payload: &[u8]) -> Result<HydratePayload, DecodeError> {
    match payload.split_first() {
        Some((&HYDRATE_SNAPSHOT, rest)) => {
            let (&version_len, rest) = rest
                .split_first()
                .ok_or_else(|| bad(Kind::Hydrate, "a snapshot has no engine version"))?;
            let (version, bytes) = rest
                .split_at_checked(usize::from(version_len))
                .ok_or_else(|| bad(Kind::Hydrate, "the engine version is cut short"))?;
            let vt_version = std::str::from_utf8(version)
                .map_err(|_| bad(Kind::Hydrate, "the engine version is not UTF-8"))?
                .to_owned();
            Ok(HydratePayload::Snapshot {
                vt_version,
                bytes: bytes.to_vec(),
            })
        }
        Some((&HYDRATE_REPLAY, rest)) => Ok(HydratePayload::VtReplay(rest.to_vec())),
        Some(_) => Err(bad(Kind::Hydrate, "unknown form")),
        None => Err(bad(Kind::Hydrate, "empty")),
    }
}

fn fixed<const N: usize>(kind: Kind, payload: &[u8]) -> Result<&[u8; N], DecodeError> {
    payload.try_into().map_err(|_| DecodeError::BadLength {
        kind,
        len: payload.len(),
        expected: N,
    })
}

fn size(kind: Kind, cols: u16, rows: u16) -> Result<TermSize, DecodeError> {
    TermSize::new(cols, rows).map_err(|e| bad(kind, &e.to_string()))
}

fn bad(kind: Kind, reason: &str) -> DecodeError {
    DecodeError::BadPayload {
        kind,
        reason: reason.to_owned(),
    }
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn put_u16(out: &mut [u8], at: usize, value: u16) {
    out[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
