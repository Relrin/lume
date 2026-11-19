//! Bytes as a plugin's code sends and gets them: as text, or as base64 where they are
//! not text.

use crate::types::Encoding;
use crate::{ApiError, ErrorCode};

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Bytes written as base64, with padding.
pub fn to_base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (index, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if index <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> shift) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The bytes base64 text stands for. Padding may be left out; anything else that is not
/// base64 is refused.
pub fn from_base64(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=');
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in text.bytes() {
        let value = ALPHABET.iter().position(|letter| *letter == byte)?;
        buffer = (buffer << 6) | u32::try_from(value).ok()?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buffer >> bits) & 0xff).ok()?);
        }
    }
    // A last group of one character holds no whole byte.
    if text.len() % 4 == 1 {
        return None;
    }
    Some(out)
}

/// What a plugin's code sent, as bytes.
pub fn decode(text: &str, encoding: Encoding) -> Result<Vec<u8>, ApiError> {
    match encoding {
        Encoding::Utf8 => Ok(text.as_bytes().to_vec()),
        Encoding::Base64 => from_base64(text)
            .ok_or_else(|| ApiError::new(ErrorCode::Invalid, "what was sent is not base64")),
    }
}

/// Bytes as a plugin's code is given them. Text that is not UTF-8 is refused, so that
/// it can be asked for again as base64, rather than changed on the way.
pub fn encode(bytes: Vec<u8>, encoding: Encoding) -> Result<String, ApiError> {
    match encoding {
        Encoding::Utf8 => String::from_utf8(bytes).map_err(|_| {
            ApiError::new(
                ErrorCode::Invalid,
                "what was read is not UTF-8 text; ask for it as base64",
            )
        }),
        Encoding::Base64 => Ok(to_base64(&bytes)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_go_to_base64_and_back() {
        for bytes in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"foob",
            b"fooba",
            b"foobar",
            &[0, 255, 128, 7],
        ] {
            let written = to_base64(bytes);
            assert_eq!(from_base64(&written).as_deref(), Some(bytes), "{written}");
        }
        assert_eq!(to_base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(to_base64(b"fo"), "Zm8=");
        assert_eq!(from_base64("Zm8").as_deref(), Some(&b"fo"[..]));
        assert_eq!(from_base64("Zm9v!"), None);
        assert_eq!(from_base64("Z"), None);
    }

    #[test]
    fn text_that_is_not_utf8_is_refused_rather_than_changed() {
        assert!(encode(vec![0xff, 0xfe], Encoding::Utf8).is_err());
        assert_eq!(encode(b"ok".to_vec(), Encoding::Utf8).unwrap(), "ok");
        assert_eq!(decode("b2s=", Encoding::Base64).unwrap(), b"ok");
    }
}
