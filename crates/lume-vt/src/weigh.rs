//! How much loading a snapshot would take.
//!
//! A snapshot says how large a terminal is and, for each of its pages, how much room to
//! keep for what the page may come to hold. Nothing in the format ties either to the
//! snapshot's own length, and the library takes both at their word: a few kilobytes can
//! ask for gigabytes. A snapshot from another machine is weighed before it is loaded.

use lume_vt_sys as sys;

use crate::{Error, Result, Terminal};

/// The most a snapshot may take to be loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The cells of the terminal's screen.
    pub cells: u32,
    /// The memory all its pages ask for, in bytes.
    pub bytes: u64,
}

/// What a snapshot begins with, and the bytes of its version after that.
const MAGIC: &[u8] = b"GHOSTSNP";
const ENVELOPE: usize = MAGIC.len() + 2;
/// How each of its records begins: a tag, how long the rest is, and a checksum.
const HEADER: usize = 10;

/// The records that say how large something is, and the one that ends a snapshot.
const TERMINAL: u64 = 1;
const PAGE: u64 = 3;
const FINISH: u64 = 6;

/// What a page keeps for a cell, a row, a style and a byte of its links, at most.
const CELL_BYTES: u64 = 8;
const ROW_BYTES: u64 = 16;
const STYLE_BYTES: u64 = 48;
const LINK_FACTOR: u64 = 16;

fn u16_at(bytes: &[u8], at: usize) -> Option<u64> {
    let two = bytes.get(at..at + 2)?;
    Some(u64::from(u16::from_le_bytes([two[0], two[1]])))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u64> {
    let four = bytes.get(at..at + 4)?;
    Some(u64::from(u32::from_le_bytes([
        four[0], four[1], four[2], four[3],
    ])))
}

/// What a page that is described this way takes, about, and never much more.
fn page(described: &[u8]) -> Option<u64> {
    let rows = u16_at(described, 2)?;
    let cells = u16_at(described, 0)? * rows;
    let styles = u16_at(described, 8)?.next_power_of_two();
    let links = u16_at(described, 10)?;
    let kept = u32_at(described, 12)? + u32_at(described, 16)?;
    Some(
        cells * CELL_BYTES
            + rows * ROW_BYTES
            + styles * STYLE_BYTES
            + links * LINK_FACTOR
            // With a map of what is free in it.
            + kept
            + kept / 8,
    )
}

/// Whether a snapshot, which other bytes may follow, is within the limits. `None` for
/// what does not have the form of one.
fn fits(bytes: &[u8], limits: Limits) -> Option<bool> {
    if !bytes.starts_with(MAGIC) {
        return None;
    }
    let mut at = ENVELOPE;
    let mut total = 0u64;
    loop {
        let tag = u16_at(bytes, at)?;
        let length = usize::try_from(u32_at(bytes, at + 2)?).ok()?;
        let from = at + HEADER;
        let described = bytes.get(from..from.checked_add(length)?)?;
        match tag {
            TERMINAL => {
                let cells = u16_at(described, 0)? * u16_at(described, 2)?;
                if cells > u64::from(limits.cells) {
                    return Some(false);
                }
            }
            PAGE => total += page(described)?,
            FINISH => return Some(true),
            _ => {}
        }
        if total > limits.bytes {
            return Some(false);
        }
        at = from + length;
    }
}

impl Terminal {
    /// Loads a terminal as [`from_snapshot_prefix`](Self::from_snapshot_prefix) does, from
    /// a snapshot that was not made here. One that asks for more than the limits is
    /// refused before anything is kept for it.
    pub fn from_foreign_snapshot(bytes: &[u8], limits: Limits) -> Result<(Self, usize)> {
        match fits(bytes, limits) {
            Some(true) => Self::from_snapshot_prefix(bytes),
            Some(false) => Err(Error(sys::GHOSTTY_OUT_OF_MEMORY as i32)),
            None => Err(Error(sys::GHOSTTY_INVALID_VALUE as i32)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: Limits = Limits {
        cells: 10_000,
        bytes: 8 * 1024 * 1024,
    };

    fn made() -> Vec<u8> {
        let mut terminal = Terminal::new(80, 24).unwrap();
        for line in 0..500 {
            terminal.write(format!("\x1b[3{}mline {line}\r\n", line % 8).as_bytes());
        }
        terminal.snapshot().unwrap()
    }

    /// Where the first record of a kind says what it holds.
    fn described(snapshot: &[u8], kind: u64) -> usize {
        let mut at = ENVELOPE;
        while u16_at(snapshot, at) != Some(kind) {
            at += HEADER + u32_at(snapshot, at + 2).unwrap() as usize;
        }
        at + HEADER
    }

    #[test]
    fn a_snapshot_is_weighed_against_the_limits_and_bytes_may_follow_it() {
        let mut snapshot = made();
        let length = snapshot.len();
        assert_eq!(fits(&snapshot, LIMITS), Some(true));
        snapshot.extend_from_slice(b"\xff\xff\xff output");
        let (terminal, consumed) = Terminal::from_foreign_snapshot(&snapshot, LIMITS).unwrap();
        assert_eq!((terminal.cols(), terminal.rows()), (80, 24));
        assert_eq!(consumed, length);

        let light = Limits {
            bytes: 1024,
            ..LIMITS
        };
        assert_eq!(fits(&snapshot, light), Some(false));
        let small = Limits {
            cells: 80 * 24 - 1,
            ..LIMITS
        };
        assert_eq!(fits(&snapshot, small), Some(false));
    }

    #[test]
    fn a_snapshot_that_asks_for_far_more_than_it_holds_is_refused() {
        // What a page is to keep for text and for what its cells combine.
        for offset in [12, 16] {
            let mut snapshot = made();
            let at = described(&snapshot, PAGE) + offset;
            snapshot[at..at + 4].copy_from_slice(&(1u32 << 30).to_le_bytes());
            assert_eq!(fits(&snapshot, LIMITS), Some(false));
            assert!(Terminal::from_foreign_snapshot(&snapshot, LIMITS).is_err());
        }
        // The largest terminal there is, and the largest page.
        for kind in [TERMINAL, PAGE] {
            let mut snapshot = made();
            let at = described(&snapshot, kind);
            snapshot[at..at + 4].fill(0xff);
            assert_eq!(fits(&snapshot, LIMITS), Some(false));
        }
    }

    #[test]
    fn what_does_not_have_the_form_of_a_snapshot_is_refused() {
        let snapshot = made();
        for bytes in [
            &b""[..],
            b"GHOSTSNP",
            b"not a snapshot at all",
            &snapshot[..snapshot.len() - 1],
            &snapshot[..40],
        ] {
            assert_eq!(fits(bytes, LIMITS), None);
            assert!(Terminal::from_foreign_snapshot(bytes, LIMITS).is_err());
        }
    }
}
