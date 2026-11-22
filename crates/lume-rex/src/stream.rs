//! A terminal's stream, taken apart.
//!
//! The stream opens with a copy of the terminal as it stood, in the form its engine
//! saves one. Everything the terminal's program writes follows, and among it the
//! server's own messages, each written as `ESC _ rex1;<nonce>;<json> ESC \` where the
//! nonce is the one the data connection was welcomed with. A program cannot know the
//! nonce, so what carries another one was printed by a program and is output like any
//! other.
//!
//! Nothing here reads from a connection: bytes are fed in as they arrive, in pieces of
//! any size, and what they hold comes out.

use lume_proto::{CellPx, ColorScheme, Rgb, TermSize};
use serde::Deserialize;

use crate::api::cut;
use crate::tuning::{SNAPSHOT_MAX_BYTES, STREAM_NOTE_MAX_BYTES, TEXT_MAX_BYTES};

const ESC: u8 = 0x1b;

/// How every message of the server's own begins in a stream marked with `nonce`.
fn marker(nonce: u64) -> Vec<u8> {
    format!("\u{1b}_rex1;{nonce};").into_bytes()
}

/// Something the server says in a terminal's stream, at the place in the output where
/// it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// The terminal has this size from here on.
    Resize {
        size: TermSize,
        /// The size of a cell, or zeros where the server does not know it.
        cell_px: CellPx,
        /// The client that owned the block when the size was set.
        owner: Option<String>,
    },
    /// The terminal's default colours, in place of any said before.
    Theme(Theme),
    /// The terminal's program ended. No more output of it follows.
    ChildExited {
        /// The program's exit code, or -1 if it has none.
        exit_code: i64,
    },
}

/// A terminal's default colours. What is left out is the terminal's built-in colour.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Theme {
    pub foreground: Option<Rgb>,
    pub background: Option<Rgb>,
    pub cursor: Option<Rgb>,
    /// Changes to the palette of 256 colours, by index.
    pub palette: Vec<(u8, Rgb)>,
    /// Whether the theme is light or dark, where the server says.
    pub scheme: Option<ColorScheme>,
}

/// A part of a terminal's stream, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece {
    /// What the terminal's program wrote.
    Output(Vec<u8>),
    Note(Note),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireNote {
    Resize {
        cols: u64,
        rows: u64,
        #[serde(default)]
        cell_width_px: u64,
        #[serde(default)]
        cell_height_px: u64,
        #[serde(default)]
        owner: Option<String>,
    },
    Theme {
        #[serde(default)]
        foreground: Option<String>,
        #[serde(default)]
        background: Option<String>,
        #[serde(default)]
        cursor: Option<String>,
        #[serde(default)]
        palette: Option<Vec<String>>,
        #[serde(default)]
        scheme: Option<String>,
    },
    ChildExited {
        #[serde(default = "no_code")]
        exit_code: i64,
    },
    /// A newer server may say things this client does not know.
    #[serde(other)]
    Other,
}

fn no_code() -> i64 {
    -1
}

/// A colour written as `RRGGBB` or `#RRGGBB`.
fn colour(text: &str) -> Option<Rgb> {
    let hex = text.strip_prefix('#').unwrap_or(text);
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let part = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    Some(Rgb([part(0)?, part(2)?, part(4)?]))
}

/// Reads the JSON of one message. `None` for one this client does not know or cannot
/// make sense of: it is passed over.
fn read_note(json: &[u8]) -> Option<Note> {
    let wire: WireNote = match serde_json::from_slice(json) {
        Ok(wire) => wire,
        Err(error) => {
            tracing::debug!(%error, "a message in a terminal's stream is not as described");
            return None;
        }
    };
    match wire {
        WireNote::Resize {
            cols,
            rows,
            cell_width_px,
            cell_height_px,
            owner,
        } => {
            // A terminal larger than this application shows is shown as large as it can.
            let fits = |count: u64| u16::try_from(count).unwrap_or(u16::MAX).min(TermSize::MAX);
            let size = TermSize::new(fits(cols), fits(rows)).ok()?;
            let px = |count: u64| u16::try_from(count).unwrap_or(0);
            Some(Note::Resize {
                size,
                cell_px: CellPx {
                    w: px(cell_width_px),
                    h: px(cell_height_px),
                },
                owner: owner.map(|mut owner| {
                    cut(&mut owner, TEXT_MAX_BYTES);
                    owner
                }),
            })
        }
        WireNote::Theme {
            foreground,
            background,
            cursor,
            palette,
            scheme,
        } => {
            let named = |text: Option<String>| text.as_deref().and_then(colour);
            let mut changes: Vec<(u8, Rgb)> = Vec::new();
            for entry in palette.unwrap_or_default() {
                let Some((index, hex)) = entry.split_once('=') else {
                    continue;
                };
                let (Ok(index), Some(rgb)) = (index.parse::<u8>(), colour(hex)) else {
                    continue;
                };
                // The last word on an index is the one that holds.
                match changes.iter_mut().find(|(known, _)| *known == index) {
                    Some(change) => change.1 = rgb,
                    None => changes.push((index, rgb)),
                }
            }
            Some(Note::Theme(Theme {
                foreground: named(foreground),
                background: named(background),
                cursor: named(cursor),
                palette: changes,
                scheme: match scheme.as_deref() {
                    Some("light") => Some(ColorScheme::Light),
                    Some("dark") => Some(ColorScheme::Dark),
                    _ => None,
                },
            }))
        }
        WireNote::ChildExited { exit_code } => Some(Note::ChildExited { exit_code }),
        WireNote::Other => None,
    }
}

/// What a stream opened with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    /// The copy of the terminal the stream began with. `None` when it was larger than
    /// this client takes and was let go.
    pub snapshot: Option<Vec<u8>>,
    /// What was fed after it: the stream from the server's first message on, to be
    /// given to a [`Splitter`].
    pub rest: Vec<u8>,
}

/// Collects the copy of the terminal a stream opens with.
///
/// The copy does not say how long it is. The server's first message follows it
/// directly, so everything before that message is taken as the copy. The engine that
/// loads it knows where it truly ends, and is the one to check.
#[derive(Debug)]
pub struct Opening {
    marker: Vec<u8>,
    taken: Vec<u8>,
    /// The most that is kept.
    max: usize,
    /// More than that came: nothing but the last few bytes is kept, to find the first
    /// message with.
    over: bool,
}

impl Opening {
    /// For a stream whose data connection was welcomed with `nonce`.
    pub fn new(nonce: u64) -> Self {
        Self::keeping(nonce, SNAPSHOT_MAX_BYTES)
    }

    fn keeping(nonce: u64, max: usize) -> Self {
        Self {
            marker: marker(nonce),
            taken: Vec::new(),
            max,
            over: false,
        }
    }

    /// Takes the next bytes of the stream. Returns what the stream opened with once
    /// the server's first message has begun, and must not be fed again after that.
    pub fn feed(&mut self, bytes: &[u8]) -> Option<Opened> {
        // The message may have begun in what was taken before.
        let from = self.taken.len().saturating_sub(self.marker.len() - 1);
        self.taken.extend_from_slice(bytes);
        let found = self.taken[from..]
            .windows(self.marker.len())
            .position(|window| window == self.marker)
            .map(|at| from + at);
        match found {
            Some(at) => {
                let rest = self.taken.split_off(at);
                let snapshot = std::mem::take(&mut self.taken);
                Some(Opened {
                    snapshot: (!self.over).then_some(snapshot),
                    rest,
                })
            }
            None => {
                if self.over || self.taken.len() > self.max {
                    self.over = true;
                    let keep = self.taken.len().min(self.marker.len() - 1);
                    self.taken.drain(..self.taken.len() - keep);
                    self.taken.shrink_to_fit();
                }
                None
            }
        }
    }
}

/// Takes the server's own messages out of a terminal's output.
///
/// It keeps its place between pieces, and between the connections of one stream: a
/// stream that is continued on a new connection goes on from the very byte it stopped
/// at.
#[derive(Debug)]
pub struct Splitter {
    marker: Vec<u8>,
    /// What may be a message of the server's and is not yet known to be: it begins
    /// with an escape, and so far reads as a message does.
    held: Vec<u8>,
}

impl Splitter {
    /// For a stream whose data connection was welcomed with `nonce`.
    pub fn new(nonce: u64) -> Self {
        Self {
            marker: marker(nonce),
            held: Vec::new(),
        }
    }

    /// What is held turned out to be output after all. An escape at its end may still
    /// begin a message, and stays held.
    fn let_go(&mut self, output: &mut Vec<u8>) {
        let escape = self.held.len() > 1 && self.held.last() == Some(&ESC);
        if escape {
            self.held.pop();
        }
        output.append(&mut self.held);
        if escape {
            self.held.push(ESC);
        }
    }

    /// Takes the next bytes of the stream and returns what they hold, in order. Output
    /// that may yet turn out to be a message is kept back until it is known.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Piece> {
        let mut pieces = Vec::new();
        let mut output = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            if self.held.is_empty() {
                let Some(escape) = bytes[at..].iter().position(|&byte| byte == ESC) else {
                    output.extend_from_slice(&bytes[at..]);
                    break;
                };
                output.extend_from_slice(&bytes[at..at + escape]);
                self.held.push(ESC);
                at += escape + 1;
                continue;
            }
            let byte = bytes[at];
            if self.held.len() < self.marker.len() {
                if byte == self.marker[self.held.len()] {
                    self.held.push(byte);
                    at += 1;
                } else {
                    // It begins something else. This byte is looked at again: it may
                    // be an escape itself.
                    output.append(&mut self.held);
                }
                continue;
            }
            // Past the marker: the message's JSON, up to `ESC \`.
            let ending = self.held.len() > self.marker.len() && self.held.last() == Some(&ESC);
            if ending && byte == b'\\' {
                let json = &self.held[self.marker.len()..self.held.len() - 1];
                if let Some(note) = read_note(json) {
                    if !output.is_empty() {
                        pieces.push(Piece::Output(std::mem::take(&mut output)));
                    }
                    pieces.push(Piece::Note(note));
                }
                self.held.clear();
                at += 1;
            } else if ending {
                // No message has an escape inside it: this was output, and the escape
                // may begin a message of its own.
                self.let_go(&mut output);
            } else {
                self.held.push(byte);
                at += 1;
                if self.held.len() > STREAM_NOTE_MAX_BYTES {
                    self.let_go(&mut output);
                }
            }
        }
        if !output.is_empty() {
            pieces.push(Piece::Output(output));
        }
        pieces
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONCE: u64 = 12_491_831_766_469_670_801;

    fn note(nonce: u64, json: &str) -> Vec<u8> {
        format!("\u{1b}_rex1;{nonce};{json}\u{1b}\\").into_bytes()
    }

    fn resize(cols: u16, rows: u16) -> Piece {
        Piece::Note(Note::Resize {
            size: TermSize::new(cols, rows).unwrap(),
            cell_px: CellPx { w: 8, h: 16 },
            owner: None,
        })
    }

    const RESIZE: &str = r#"{"type":"resize","cols":100,"rows":30,"cell_width_px":8,"cell_height_px":16,"owner":null}"#;

    /// Joins output that came apart only because of how the bytes were fed.
    fn joined(pieces: Vec<Piece>) -> Vec<Piece> {
        let mut all: Vec<Piece> = Vec::new();
        for piece in pieces {
            match (all.last_mut(), piece) {
                (Some(Piece::Output(before)), Piece::Output(more)) => before.extend(more),
                (_, piece) => all.push(piece),
            }
        }
        all
    }

    /// What a stream holds when fed whole, and when fed in pieces of every size up to
    /// a few bytes: the two must agree.
    fn split(stream: &[u8]) -> Vec<Piece> {
        let whole = joined(Splitter::new(NONCE).feed(stream));
        for size in 1..=7 {
            let mut splitter = Splitter::new(NONCE);
            let pieces: Vec<Piece> = stream
                .chunks(size)
                .flat_map(|chunk| splitter.feed(chunk))
                .collect();
            assert_eq!(joined(pieces), whole, "fed {size} bytes at a time");
        }
        whole
    }

    #[test]
    fn messages_are_taken_out_of_the_output_where_they_stand() {
        let mut stream = b"before".to_vec();
        stream.extend(note(NONCE, RESIZE));
        stream.extend(b"between\x1b[1mbold\x1b[0m");
        stream.extend(note(NONCE, r#"{"type":"child_exited","exit_code":3}"#));
        stream.extend(b"after");
        assert_eq!(
            split(&stream),
            [
                Piece::Output(b"before".to_vec()),
                resize(100, 30),
                Piece::Output(b"between\x1b[1mbold\x1b[0m".to_vec()),
                Piece::Note(Note::ChildExited { exit_code: 3 }),
                Piece::Output(b"after".to_vec()),
            ]
        );
    }

    #[test]
    fn what_a_program_printed_stays_output() {
        // Another nonce, a nonce this one begins, no nonce, and other sequences that
        // begin the same way.
        let mut stream = note(NONCE - 1, RESIZE);
        stream.extend(note(1249, RESIZE));
        stream.extend(b"\x1b_rex1;;{}\x1b\\");
        stream.extend(b"\x1b_rex2;1;{}\x1b\\\x1b_Gf=100;AAAA\x1b\\\x1b]0;title\x1b\\\x1b\x1b\x1b_");
        let printed = stream.clone();
        stream.extend(note(NONCE, RESIZE));
        assert_eq!(split(&stream), [Piece::Output(printed), resize(100, 30)]);
    }

    #[test]
    fn a_message_that_never_ends_or_holds_an_escape_is_output() {
        // An escape inside what began as a message: it was not one, and a real one may
        // begin at that very escape.
        let mut stream = format!("\u{1b}_rex1;{NONCE};{{\"type\":").into_bytes();
        let begun = stream.clone();
        stream.extend(note(NONCE, RESIZE));
        stream.extend(b"x");
        assert_eq!(
            split(&stream),
            [
                Piece::Output(begun),
                resize(100, 30),
                Piece::Output(b"x".to_vec())
            ]
        );

        // Longer than any message is: let go as output, so that nothing is held
        // without end.
        let mut endless = format!("\u{1b}_rex1;{NONCE};").into_bytes();
        endless.extend(std::iter::repeat_n(b'a', STREAM_NOTE_MAX_BYTES * 2));
        let mut splitter = Splitter::new(NONCE);
        let pieces = joined(splitter.feed(&endless));
        assert_eq!(pieces, [Piece::Output(endless.clone())]);
        assert!(splitter.held.is_empty());
        // And a message is found again after it.
        assert_eq!(
            joined(splitter.feed(&note(NONCE, RESIZE))),
            [resize(100, 30)]
        );
    }

    #[test]
    fn what_may_be_a_message_is_kept_back_until_it_is_known() {
        let mut splitter = Splitter::new(NONCE);
        assert_eq!(
            splitter.feed(b"text\x1b_rex1;1249"),
            [Piece::Output(b"text".to_vec())]
        );
        assert_eq!(
            splitter.feed(b"0 and more"),
            [Piece::Output(b"\x1b_rex1;12490 and more".to_vec())]
        );
        // A lone escape at the end of a piece waits for what follows it.
        assert_eq!(splitter.feed(b"a\x1b"), [Piece::Output(b"a".to_vec())]);
        assert_eq!(splitter.feed(b"[0m"), [Piece::Output(b"\x1b[0m".to_vec())]);
    }

    #[test]
    fn messages_this_client_cannot_read_are_passed_over() {
        let mut stream = b"a".to_vec();
        stream.extend(note(NONCE, r#"{"type":"something_new","value":1}"#));
        stream.extend(note(NONCE, "not json"));
        stream.extend(note(NONCE, r#"{"no":"type"}"#));
        stream.extend(note(NONCE, r#"{"type":"resize","cols":"wide","rows":1}"#));
        stream.extend(note(NONCE, r#"{"type":"resize","cols":0,"rows":0}"#));
        stream.extend(note(NONCE, ""));
        stream.extend(b"b");
        assert_eq!(split(&stream), [Piece::Output(b"ab".to_vec())]);
    }

    #[test]
    fn a_size_is_read_with_its_owner_and_kept_within_what_is_shown() {
        let said = r#"{"type":"resize","cols":80,"rows":24,"cell_width_px":0,"cell_height_px":0,"owner":"client:1","more":true}"#;
        assert_eq!(
            split(&note(NONCE, said)),
            [Piece::Note(Note::Resize {
                size: TermSize::new(80, 24).unwrap(),
                cell_px: CellPx { w: 0, h: 0 },
                owner: Some("client:1".to_owned()),
            })]
        );

        let huge = format!(
            r#"{{"type":"resize","cols":65535,"rows":18446744073709551615,"owner":"{}"}}"#,
            "c".repeat(5000)
        );
        let [Piece::Note(Note::Resize { size, owner, .. })] = &split(&note(NONCE, &huge))[..]
        else {
            panic!("not a size");
        };
        assert_eq!((size.cols(), size.rows()), (TermSize::MAX, TermSize::MAX));
        assert_eq!(owner.as_ref().unwrap().len(), TEXT_MAX_BYTES);
    }

    #[test]
    fn a_theme_is_read_colour_by_colour() {
        let said = r##"{"type":"theme","foreground":"#d6deeb","background":"011627","cursor":"blue","palette":["1=#ef5350","2=00ff00","300=ffffff","x","1=000001"],"scheme":"dark"}"##;
        assert_eq!(
            split(&note(NONCE, said)),
            [Piece::Note(Note::Theme(Theme {
                foreground: Some(Rgb([0xd6, 0xde, 0xeb])),
                background: Some(Rgb([0x01, 0x16, 0x27])),
                cursor: None,
                palette: vec![(1, Rgb([0, 0, 1])), (2, Rgb([0, 0xff, 0]))],
                scheme: Some(ColorScheme::Dark),
            }))]
        );
        assert_eq!(
            split(&note(NONCE, r#"{"type":"theme","palette":null}"#)),
            [Piece::Note(Note::Theme(Theme::default()))]
        );
        assert_eq!(colour("#12345"), None);
        assert_eq!(colour("+12345"), None);
        assert_eq!(colour("ééé"), None);
    }

    #[test]
    fn the_copy_of_the_terminal_ends_where_the_first_message_begins() {
        let mut stream = b"GHOSTSNP and the rest of a copy \x1b_rex1;1;\x1b".to_vec();
        let copy = stream.clone();
        let mut rest = note(NONCE, RESIZE);
        rest.extend(b"output");
        stream.extend(&rest);

        for size in [1, 2, 5, 19, stream.len()] {
            let mut opening = Opening::new(NONCE);
            let mut chunks = stream.chunks(size);
            let mut opened = None;
            let mut after = Vec::new();
            for chunk in chunks.by_ref() {
                if let Some(found) = opening.feed(chunk) {
                    opened = Some(found);
                    break;
                }
            }
            for chunk in chunks {
                after.extend_from_slice(chunk);
            }
            let opened = opened.unwrap_or_else(|| panic!("never opened, fed {size} at a time"));
            assert_eq!(opened.snapshot.as_deref(), Some(&copy[..]));
            let mut all = opened.rest;
            all.extend(after);
            assert_eq!(all, rest, "fed {size} at a time");
        }
    }

    #[test]
    fn a_copy_larger_than_is_taken_is_let_go_and_the_stream_still_found() {
        let mut stream = vec![b'x'; 1000];
        let rest = note(NONCE, RESIZE);
        stream.extend(&rest);
        for size in [1, 7, 64, stream.len()] {
            let mut opening = Opening::keeping(NONCE, 100);
            let mut opened = None;
            for chunk in stream.chunks(size) {
                assert!(opening.taken.len() <= 100 + size, "fed {size} at a time");
                if let Some(found) = opening.feed(chunk) {
                    opened = Some(found);
                }
            }
            let opened = opened.unwrap();
            // Fed whole, the message is found before the size is looked at.
            if size < stream.len() {
                assert_eq!(opened.snapshot, None, "fed {size} at a time");
            }
            assert!(rest.starts_with(&opened.rest) || opened.rest.starts_with(&rest));
        }
    }

    #[test]
    fn a_stream_a_server_sent_comes_apart_as_it_should() {
        // The first bytes of a shell's stream, as a server on this machine sent them:
        // the copy of an empty terminal, its size, what the shell printed, a size
        // again after another client changed it, and the redraw.
        let stream = include_bytes!("../tests/fixtures/stream.bin");
        let mut opening = Opening::new(NONCE);
        let opened = opening.feed(stream).unwrap();
        let snapshot = opened.snapshot.unwrap();
        assert!(snapshot.starts_with(b"GHOSTSNP"));
        assert_eq!(snapshot.len(), 1157);

        let pieces = split(&opened.rest);
        let sizes: Vec<(u16, u16)> = pieces
            .iter()
            .filter_map(|piece| match piece {
                Piece::Note(Note::Resize {
                    size,
                    cell_px,
                    owner,
                }) => {
                    assert_eq!(*cell_px, CellPx { w: 8, h: 16 });
                    assert!(owner.as_ref().unwrap().starts_with("client:"));
                    Some((size.cols(), size.rows()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(sizes, [(80, 24), (100, 30)]);
        assert!(matches!(pieces[0], Piece::Note(Note::Resize { .. })));
        let output: usize = pieces
            .iter()
            .map(|piece| match piece {
                Piece::Output(bytes) => {
                    assert!(!bytes.windows(6).any(|window| window == b"_rex1;"));
                    bytes.len()
                }
                Piece::Note(_) => 0,
            })
            .sum();
        // Everything that is not the copy or a message is output, to the byte.
        assert_eq!(output, stream.len() - 1157 - 159 - 160);
    }
}
