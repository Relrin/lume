//! A terminal saved and brought back: as a snapshot, which only the same version of the
//! library can load, or as output that rebuilds the screen in any terminal. Also the
//! short reports a terminal sends a program when its view gains focus or changes scheme.

use lume_vt_sys as sys;

use crate::{ColorScheme, Error, Result, Terminal, check};

/// What a terminal is turned into by [`Terminal::format`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// The text alone, one line per row, trailing blanks removed.
    Plain,
    /// Output that rebuilds the screen when written to a fresh terminal of the same
    /// size: the text with its colours, attributes and links, then the modes, the
    /// scroll region, the tab stops, the character sets, the keyboard modes, the cursor,
    /// the title, and the start of a sequence left unfinished.
    ///
    /// Each row comes back as it is, so a line that was wrapped onto several rows is
    /// several lines in the copy and does not flow again when the copy is resized.
    Replay,
}

/// The result of [`Terminal::format`]: the bytes and what they cover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Formatted {
    pub bytes: Vec<u8>,
    /// The screen formatted was the alternate one. Output rebuilds only the screen in
    /// use, so the primary screen under an alternate one is not in it.
    pub alternate: bool,
}

/// Bytes the library allocated, freed when dropped.
struct Allocated {
    ptr: *mut u8,
    len: usize,
}

impl Allocated {
    fn to_vec(&self) -> Vec<u8> {
        if self.ptr.is_null() || self.len == 0 {
            return Vec::new();
        }
        // SAFETY: the library handed over `len` readable bytes at `ptr`.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }.to_vec()
    }
}

impl Drop for Allocated {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: the bytes came from the default allocator, which a NULL allocator
            // names, and are freed once.
            unsafe { sys::ghostty_free(std::ptr::null(), self.ptr, self.len) };
        }
    }
}

/// A formatter of one terminal, freed when dropped.
struct Formatter(sys::GhosttyFormatter);

impl Drop for Formatter {
    fn drop(&mut self) {
        // SAFETY: the handle came from ghostty_formatter_terminal_new and is freed once.
        unsafe { sys::ghostty_formatter_free(self.0) };
    }
}

/// A snapshot decoder, freed when dropped.
struct Decoder(sys::GhosttySnapshotDecoder);

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: the handle came from ghostty_snapshot_decoder_new_buf and is freed once.
        unsafe { sys::ghostty_snapshot_decoder_free(self.0) };
    }
}

impl Terminal {
    /// Saves the whole terminal: both screens, scrollback, modes, colours and the
    /// unfinished sequence it is in the middle of, if any. Only a terminal of this
    /// library version can load the result.
    ///
    /// Fails when the terminal is inside a sequence longer than
    /// [`CONTINUATION_MAX_BYTES`](crate::CONTINUATION_MAX_BYTES).
    pub fn snapshot(&self) -> Result<Vec<u8>> {
        let mut out = Allocated {
            ptr: std::ptr::null_mut(),
            len: 0,
        };
        // SAFETY: the handle is live; the out pointers are valid; nothing else touches
        // the terminal during the call.
        check(unsafe {
            sys::ghostty_snapshot_encode_alloc(
                self.raw,
                std::ptr::null(),
                &mut out.ptr,
                &mut out.len,
            )
        })?;
        Ok(out.to_vec())
    }

    /// Loads a terminal saved with [`snapshot`](Self::snapshot) by this library
    /// version. Its colours, cell size and colour scheme start unset, as in a new
    /// terminal, and are set again by whoever shows it.
    pub fn from_snapshot(bytes: &[u8]) -> Result<Self> {
        let mut decoder = Decoder(std::ptr::null_mut());
        // SAFETY: the pointer and length describe `bytes`, which outlives the decoder.
        check(unsafe {
            sys::ghostty_snapshot_decoder_new_buf(
                std::ptr::null(),
                &mut decoder.0,
                bytes.as_ptr(),
                bytes.len(),
            )
        })?;
        if decoder.0.is_null() {
            return Err(Error(sys::GHOSTTY_OUT_OF_MEMORY as i32));
        }
        let mut raw: sys::GhosttyTerminal = std::ptr::null_mut();
        // SAFETY: the decoder is live and has not started; `raw` is a valid out pointer.
        check(unsafe { sys::ghostty_snapshot_decoder_decode(decoder.0, &mut raw) })?;
        drop(decoder);
        if raw.is_null() {
            return Err(Error(sys::GHOSTTY_OUT_OF_MEMORY as i32));
        }
        // The size is read before the handle is adopted; if reading fails the handle
        // is freed here.
        let read = |key| {
            let mut value = 0u16;
            // SAFETY: the handle is live and the key's output type is u16.
            let result = unsafe { sys::ghostty_terminal_get(raw, key, (&raw mut value).cast()) };
            check(result).map(|()| value)
        };
        let size = read(sys::GHOSTTY_TERMINAL_DATA_COLS)
            .and_then(|cols| read(sys::GHOSTTY_TERMINAL_DATA_ROWS).map(|rows| (cols, rows)));
        match size {
            Ok((cols, rows)) => Self::adopt(raw, cols, rows),
            Err(error) => {
                // SAFETY: the handle came from the decoder, is owned here, and is freed once.
                unsafe { sys::ghostty_terminal_free(raw) };
                Err(error)
            }
        }
    }

    /// Turns the screen in use, with its scrollback, into text or into output that
    /// rebuilds it.
    pub fn format(&self, format: Format) -> Result<Formatted> {
        let mut formatter = Formatter(std::ptr::null_mut());
        // SAFETY: the handle is live and outlives the formatter, which is dropped here.
        check(unsafe {
            sys::ghostty_formatter_terminal_new(std::ptr::null(), &mut formatter.0, self.raw)
        })?;
        let set = |option, value: *const std::ffi::c_void| {
            // SAFETY: the formatter is live; each value has the type its option takes.
            check(unsafe { sys::ghostty_formatter_set(formatter.0, option, value) })
        };
        match format {
            Format::Plain => {}
            Format::Replay => {
                let emit = sys::GHOSTTY_FORMATTER_FORMAT_VT;
                set(sys::GHOSTTY_FORMATTER_OPT_EMIT, (&raw const emit).cast())?;
                // Not the palette: the copy draws with its own view's colours, and the
                // palette here is only what that view, or another, last said it had.
                let extras: sys::GhosttyFormatterExtras =
                    sys::GHOSTTY_FORMATTER_EXTRAS_ALL & !sys::GHOSTTY_FORMATTER_EXTRA_PALETTE;
                set(
                    sys::GHOSTTY_FORMATTER_OPT_EXTRAS,
                    (&raw const extras).cast(),
                )?;
                // Blank rows at the end are part of the screen; without them the
                // cursor would be placed on rows that do not exist yet.
                let trim = false;
                set(sys::GHOSTTY_FORMATTER_OPT_TRIM, (&raw const trim).cast())?;
            }
        }
        let mut out = Allocated {
            ptr: std::ptr::null_mut(),
            len: 0,
        };
        // SAFETY: the formatter is live and the out pointers are valid.
        check(unsafe {
            sys::ghostty_formatter_format_alloc(
                formatter.0,
                std::ptr::null(),
                &mut out.ptr,
                &mut out.len,
            )
        })?;
        let mut bytes = out.to_vec();
        drop(out);
        if format == Format::Replay {
            // The formatter leaves out the title.
            let title = self.title()?;
            if !title.is_empty() {
                bytes.extend_from_slice(b"\x1b]2;");
                bytes.extend(
                    title
                        .chars()
                        .filter(|c| !c.is_control())
                        .collect::<String>()
                        .bytes(),
                );
                bytes.extend_from_slice(b"\x1b\\");
            }
            // Last, the start of a sequence the terminal is in the middle of, so that
            // the output that follows finishes it in the copy as it does here.
            bytes.extend(self.continuation()?);
        }
        Ok(Formatted {
            bytes,
            alternate: self.alternate_screen()?,
        })
    }

    /// The start of the escape sequence or character the last write left unfinished.
    fn continuation(&self) -> Result<Vec<u8>> {
        let mut out = Allocated {
            ptr: std::ptr::null_mut(),
            len: 0,
        };
        // SAFETY: the handle is live and the out pointers are valid.
        let result = unsafe {
            sys::ghostty_terminal_continuation_alloc(
                self.raw,
                std::ptr::null(),
                &mut out.ptr,
                &mut out.len,
            )
        };
        // No value means it could not be kept (too long); the copy then starts at the
        // next sequence, as a terminal does after one it cannot hold.
        if result == sys::GHOSTTY_NO_VALUE {
            return Ok(Vec::new());
        }
        check(result)?;
        Ok(out.to_vec())
    }

    /// Clears the terminal as a full reset (`ESC c`) would, keeping its size,
    /// callbacks and the colours set on it.
    pub fn reset(&mut self) {
        // SAFETY: the handle is live.
        unsafe { sys::ghostty_terminal_reset(self.raw) };
    }
}

/// Encodes a short report into a small buffer.
fn encode(write: impl FnOnce(*mut u8, usize, *mut usize) -> sys::GhosttyResult) -> Vec<u8> {
    let mut buf = [0u8; 32];
    let mut written = 0usize;
    match check(write(buf.as_mut_ptr(), buf.len(), &mut written)) {
        Ok(()) => buf[..written.min(buf.len())].to_vec(),
        Err(_) => Vec::new(),
    }
}

/// What a terminal sends a program that asked to hear of focus changes (mode 1004).
pub fn focus_report(focused: bool) -> Vec<u8> {
    let event = if focused {
        sys::GHOSTTY_FOCUS_GAINED
    } else {
        sys::GHOSTTY_FOCUS_LOST
    };
    // SAFETY: the buffer pointer and length describe the caller's buffer.
    encode(|buf, len, written| unsafe { sys::ghostty_focus_encode(event, buf, len, written) })
}

/// What a terminal sends a program that asked to hear of colour scheme changes
/// (mode 2031).
pub fn color_scheme_report(scheme: ColorScheme) -> Vec<u8> {
    let raw = scheme.raw();
    // SAFETY: as in `focus_report`.
    encode(|buf, len, written| unsafe {
        sys::ghostty_color_scheme_report_encode(raw, buf, len, written)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Colors, palette_256};

    /// Everything a person sees of the active area, and the modes that change what a
    /// program does next.
    fn view(
        term: &Terminal,
    ) -> (
        Vec<String>,
        crate::Cursor,
        bool,
        Vec<bool>,
        Vec<crate::Cell>,
    ) {
        let modes = [1, 6, 7, 25, 1000, 1002, 1006, 2004]
            .iter()
            .map(|&m| term.mode(m, false).unwrap())
            .collect();
        let mut cells = Vec::new();
        for row in 0..term.rows() {
            for col in 0..term.cols() {
                cells.push(term.cell(col, row).unwrap());
            }
        }
        (
            term.screen_text().unwrap(),
            term.cursor().unwrap(),
            term.alternate_screen().unwrap(),
            modes,
            cells,
        )
    }

    const SCENE: &[u8] = b"line one\r\n\x1b[1;31mbold red\x1b[0m \x1b]8;;https://example.org\x1b\\link\x1b]8;;\x1b\\\r\n\
        \x1b[?2004h\x1b[?1h\x1b[3;5r\x1b[2;4H\xe4\xb8\xad wide";

    #[test]
    fn a_snapshot_brings_back_the_same_terminal_and_it_stays_in_step() {
        let mut original = Terminal::new(40, 6).unwrap();
        for i in 0..30 {
            original.write(format!("history {i}\r\n").as_bytes());
        }
        original.write(SCENE);
        let mut restored = Terminal::from_snapshot(&original.snapshot().unwrap()).unwrap();
        assert_eq!((restored.cols(), restored.rows()), (40, 6));
        assert_eq!(view(&restored), view(&original));
        assert_eq!(
            restored.scrollback_rows().unwrap(),
            original.scrollback_rows().unwrap()
        );

        let more = b"\x1b[1;1Hagain\x1b[?1049hfull screen";
        original.write(more);
        restored.write(more);
        assert_eq!(view(&restored), view(&original));
    }

    #[test]
    fn a_snapshot_taken_inside_a_sequence_finishes_it_after_loading() {
        let mut original = Terminal::new(20, 3).unwrap();
        original.write(b"ab\x1b[3");
        let mut restored = Terminal::from_snapshot(&original.snapshot().unwrap()).unwrap();
        original.write(b"1mc");
        restored.write(b"1mc");
        assert_eq!(view(&restored), view(&original));
        assert_eq!(restored.cell(2, 0).unwrap().fg, crate::Color::Palette(1));
    }

    #[test]
    fn replayed_output_rebuilds_the_screen_and_the_modes() {
        let mut original = Terminal::new(40, 6).unwrap();
        original.write(SCENE);
        let replay = original.format(Format::Replay).unwrap();
        assert!(!replay.alternate);
        let mut rebuilt = Terminal::new(40, 6).unwrap();
        rebuilt.write(&replay.bytes);
        assert_eq!(view(&rebuilt), view(&original));
    }

    #[test]
    fn replaying_an_alternate_screen_leaves_out_the_primary_one_under_it() {
        let mut original = Terminal::new(20, 4).unwrap();
        original.write(b"primary text\r\n\x1b[?1049h\x1b[Hfull screen");
        let replay = original.format(Format::Replay).unwrap();
        assert!(replay.alternate);
        let mut copy = Terminal::new(20, 4).unwrap();
        copy.write(&replay.bytes);
        assert_eq!(view(&copy), view(&original));

        // Leaving the alternate screen shows what the replay could not carry.
        original.write(b"\x1b[?1049l");
        copy.write(b"\x1b[?1049l");
        assert_eq!(original.row_text(0).unwrap(), "primary text");
        assert_eq!(copy.row_text(0).unwrap(), "");
    }

    #[test]
    fn plain_text_has_the_scrollback_and_the_screen() {
        let mut term = Terminal::new(20, 3).unwrap();
        for i in 0..10 {
            term.write(format!("row {i}\r\n").as_bytes());
        }
        let text = String::from_utf8(term.format(Format::Plain).unwrap().bytes).unwrap();
        assert!(text.contains("row 0"), "{text:?}");
        assert!(text.contains("row 9"), "{text:?}");
    }

    #[test]
    fn colours_cell_size_and_scheme_are_what_a_program_is_told() {
        let mut term = Terminal::new(80, 24).unwrap();
        let mut ansi = [[0u8; 3]; 16];
        ansi[1] = [0xaa, 0x11, 0x22];
        term.set_colors(&Colors {
            foreground: [0x10, 0x20, 0x30],
            background: [0xf0, 0xe0, 0xd0],
            cursor: [1, 2, 3],
            palette: palette_256(&ansi),
        })
        .unwrap();
        term.set_cell_px(9, 18).unwrap();
        term.write(b"\x1b]10;?\x1b\\\x1b]4;1;?\x1b\\\x1b[16t\x1b[?996n");
        let replies = String::from_utf8(term.take_replies()).unwrap();
        assert!(replies.contains("rgb:1010/2020/3030"), "{replies:?}");
        assert!(replies.contains("rgb:aaaa/1111/2222"), "{replies:?}");
        assert!(replies.contains("\x1b[6;18;9t"), "{replies:?}");
        assert!(!replies.contains("?997"), "no scheme is set: {replies:?}");
        assert!(term.replies().is_empty());

        term.set_color_scheme(Some(ColorScheme::Dark));
        term.write(b"\x1b[?996n");
        assert_eq!(term.take_replies(), b"\x1b[?997;1n");
    }

    #[test]
    fn focus_and_scheme_reports_are_the_standard_ones() {
        assert_eq!(focus_report(true), b"\x1b[I");
        assert_eq!(focus_report(false), b"\x1b[O");
        assert_eq!(color_scheme_report(ColorScheme::Light), b"\x1b[?997;2n");
    }

    #[test]
    fn scrollback_can_be_capped_and_memory_read() {
        let mut term = Terminal::new(20, 3).unwrap();
        term.set_scrollback_limits(100, usize::MAX).unwrap();
        for i in 0..5000 {
            term.write(format!("{i}\r\n").as_bytes());
        }
        let kept = term.scrollback_rows().unwrap();
        assert!(kept < 5000, "{kept} rows kept");
        assert!(term.memory_bytes().unwrap() > 0);
    }
}
