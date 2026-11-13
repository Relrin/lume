//! A safe wrapper over libghostty-vt.
//!
//! Current scope: create a terminal, write bytes to it, resize it, and read back text and cell
//! attributes. That is enough for the smoke test and the golden corpus. Render state,
//! input encoding, selection and snapshots are not wrapped yet.

// bindgen gives C enums a different integer type on each platform (`i32` with MSVC, `u32`
// elsewhere), so a cast that is needed on one platform is a no-op on another.
#![allow(clippy::unnecessary_cast)]

pub mod corpus;

use std::ffi::CStr;
use std::fmt;
use std::marker::PhantomData;

use lume_vt_sys as sys;

/// An error returned by libghostty-vt, carrying its `GhosttyResult` code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error(pub i32);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self.0 {
            x if x == sys::GHOSTTY_OUT_OF_MEMORY as i32 => "out of memory",
            x if x == sys::GHOSTTY_INVALID_VALUE as i32 => "invalid value",
            x if x == sys::GHOSTTY_OUT_OF_SPACE as i32 => "buffer too small",
            x if x == sys::GHOSTTY_NO_VALUE as i32 => "no value",
            _ => "error",
        };
        write!(f, "libghostty-vt: {name} (code {})", self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn check(result: sys::GhosttyResult) -> Result<()> {
    if result == sys::GHOSTTY_SUCCESS {
        Ok(())
    } else {
        Err(Error(result as i32))
    }
}

/// The version of the libghostty-vt library that is linked, for example `"1.0.1"`.
pub fn library_version() -> &'static str {
    // SAFETY: the function returns a NUL-terminated string that is never NULL and lives as
    // long as the library is loaded.
    let text = unsafe { CStr::from_ptr(sys::ghostty_version()) };
    text.to_str().unwrap_or("")
}

/// The version of the headers these bindings were generated from.
pub fn header_version() -> &'static str {
    CStr::from_bytes_with_nul(sys::GHOSTTY_VT_VERSION)
        .ok()
        .and_then(|s| s.to_str().ok())
        .unwrap_or("")
}

/// A cell's foreground or background colour as the program set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    /// The terminal's default colour.
    Default,
    /// An index into the 256-colour palette.
    Palette(u8),
    /// A direct RGB colour.
    Rgb(u8, u8, u8),
}

impl Color {
    fn from_raw(raw: &sys::GhosttyStyleColor) -> Self {
        let tag = u32::from(raw.tag);
        if tag == sys::GHOSTTY_STYLE_COLOR_PALETTE as u32 {
            // SAFETY: the tag says the palette member of the union is the active one.
            Color::Palette(unsafe { raw.value.palette })
        } else if tag == sys::GHOSTTY_STYLE_COLOR_RGB as u32 {
            // SAFETY: the tag says the rgb member of the union is the active one.
            let rgb = unsafe { raw.value.rgb };
            Color::Rgb(rgb.r, rgb.g, rgb.b)
        } else {
            Color::Default
        }
    }
}

/// Text attributes of a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Attrs {
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub inverse: bool,
    pub strikethrough: bool,
    /// The underline style number libghostty-vt reports; zero means none.
    pub underline: u8,
}

/// How a cell takes part in a wide (two-column) character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wide {
    Narrow,
    /// The first column of a wide character.
    Wide,
    /// The second column of a wide character; it holds no text of its own.
    SpacerTail,
    /// A spacer at the end of a line that a wide character wrapped past.
    SpacerHead,
}

/// One cell of the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// The grapheme cluster in the cell. Empty for a blank cell.
    pub text: String,
    pub fg: Color,
    pub bg: Color,
    pub attrs: Attrs,
    pub wide: Wide,
}

/// The cursor of the active area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub x: u16,
    pub y: u16,
    pub visible: bool,
    /// The cursor sits past the last column and the next character will wrap.
    pub pending_wrap: bool,
}

/// A terminal emulator: screen, scrollback, cursor, styles and modes.
///
/// libghostty-vt is not thread-safe, so a `Terminal` is neither `Send` nor `Sync`. Each
/// terminal lives on one thread.
pub struct Terminal {
    raw: sys::GhosttyTerminal,
    cols: u16,
    rows: u16,
    _single_thread: PhantomData<*mut ()>,
}

impl Terminal {
    /// Create a terminal with the given grid size.
    pub fn new(cols: u16, rows: u16) -> Result<Self> {
        let mut raw: sys::GhosttyTerminal = std::ptr::null_mut();
        // SAFETY: a NULL allocator selects the library's default; `raw` is a valid place
        // for the handle.
        check(unsafe { sys::ghostty_terminal_new(std::ptr::null(), &mut raw, cols, rows) })?;
        if raw.is_null() {
            return Err(Error(sys::GHOSTTY_OUT_OF_MEMORY as i32));
        }
        Ok(Self {
            raw,
            cols,
            rows,
            _single_thread: PhantomData,
        })
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Feed terminal output (text and escape sequences) into the terminal.
    pub fn write(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        // SAFETY: the handle is live and the pointer and length describe `bytes`.
        unsafe { sys::ghostty_terminal_vt_write(self.raw, bytes.as_ptr(), bytes.len()) };
    }

    /// Resize the grid. Cell pixel sizes are not known to this wrapper yet and are passed
    /// as zero.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        // SAFETY: the handle is live.
        check(unsafe { sys::ghostty_terminal_resize(self.raw, cols, rows, 0, 0) })?;
        self.cols = cols;
        self.rows = rows;
        Ok(())
    }

    /// The cell at `col`, `row` of the active area (the screen without scrollback).
    pub fn cell(&self, col: u16, row: u16) -> Result<Cell> {
        let grid_ref = self.grid_ref(col, row)?;

        let mut raw_cell: sys::GhosttyCell = 0;
        // SAFETY: `grid_ref` was just produced by the library and the terminal has not
        // changed since; the out pointer is valid.
        check(unsafe { sys::ghostty_grid_ref_cell(&grid_ref, &mut raw_cell) })?;

        let mut wide_raw: sys::GhosttyCellWide = sys::GHOSTTY_CELL_WIDE_NARROW;
        // SAFETY: the out pointer has the type the header documents for this key.
        check(unsafe {
            sys::ghostty_cell_get(
                raw_cell,
                sys::GHOSTTY_CELL_DATA_WIDE,
                (&raw mut wide_raw).cast(),
            )
        })?;
        let wide = match wide_raw {
            x if x == sys::GHOSTTY_CELL_WIDE_WIDE => Wide::Wide,
            x if x == sys::GHOSTTY_CELL_WIDE_SPACER_TAIL => Wide::SpacerTail,
            x if x == sys::GHOSTTY_CELL_WIDE_SPACER_HEAD => Wide::SpacerHead,
            _ => Wide::Narrow,
        };

        let mut style = std::mem::MaybeUninit::<sys::GhosttyStyle>::zeroed();
        // SAFETY: as above; the library fills `style` on success.
        check(unsafe { sys::ghostty_grid_ref_style(&grid_ref, style.as_mut_ptr()) })?;
        // SAFETY: the call succeeded, and a zeroed GhosttyStyle is a valid value anyway.
        let style = unsafe { style.assume_init() };

        let flags = u32::from(style.flags);
        let attrs = Attrs {
            bold: flags & (1 << 0) != 0,
            italic: flags & (1 << 1) != 0,
            faint: flags & (1 << 2) != 0,
            inverse: flags & (1 << 4) != 0,
            strikethrough: flags & (1 << 6) != 0,
            underline: ((flags >> sys::GHOSTTY_STYLE_UNDERLINE_SHIFT) & 0x7) as u8,
        };

        Ok(Cell {
            text: self.graphemes(&grid_ref)?,
            fg: Color::from_raw(&style.fg_color),
            bg: Color::from_raw(&style.bg_color),
            attrs,
            wide,
        })
    }

    /// The text of one row of the active area, with trailing blanks removed.
    pub fn row_text(&self, row: u16) -> Result<String> {
        let mut line = String::new();
        for col in 0..self.cols {
            let cell = self.cell(col, row)?;
            match cell.wide {
                Wide::SpacerTail | Wide::SpacerHead => {}
                _ if cell.text.is_empty() => line.push(' '),
                _ => line.push_str(&cell.text),
            }
        }
        line.truncate(line.trim_end_matches(' ').len());
        Ok(line)
    }

    /// The text of every row of the active area.
    pub fn screen_text(&self) -> Result<Vec<String>> {
        (0..self.rows).map(|row| self.row_text(row)).collect()
    }

    /// The cursor's position in the active area and whether it is shown.
    pub fn cursor(&self) -> Result<Cursor> {
        Ok(Cursor {
            x: self.get::<u16>(sys::GHOSTTY_TERMINAL_DATA_CURSOR_X)?,
            y: self.get::<u16>(sys::GHOSTTY_TERMINAL_DATA_CURSOR_Y)?,
            visible: self.get::<bool>(sys::GHOSTTY_TERMINAL_DATA_CURSOR_VISIBLE)?,
            pending_wrap: self.get::<bool>(sys::GHOSTTY_TERMINAL_DATA_CURSOR_PENDING_WRAP)?,
        })
    }

    /// Whether the alternate screen (used by full-screen programs) is active.
    pub fn alternate_screen(&self) -> Result<bool> {
        let screen =
            self.get::<sys::GhosttyTerminalScreen>(sys::GHOSTTY_TERMINAL_DATA_ACTIVE_SCREEN)?;
        Ok(screen == sys::GHOSTTY_TERMINAL_SCREEN_ALTERNATE)
    }

    /// Read one value with `ghostty_terminal_get`.
    ///
    /// `T` must be the output type the header documents for `key`. Only called from this
    /// module, with keys whose types were checked against the header.
    fn get<T: Copy>(&self, key: sys::GhosttyTerminalData) -> Result<T> {
        let mut out = std::mem::MaybeUninit::<T>::zeroed();
        // SAFETY: the handle is live and `out` has room for the key's output type.
        check(unsafe { sys::ghostty_terminal_get(self.raw, key, out.as_mut_ptr().cast()) })?;
        // SAFETY: the call succeeded, so the library wrote a valid `T`.
        Ok(unsafe { out.assume_init() })
    }

    fn grid_ref(&self, col: u16, row: u16) -> Result<sys::GhosttyGridRef> {
        if col >= self.cols || row >= self.rows {
            return Err(Error(sys::GHOSTTY_INVALID_VALUE as i32));
        }
        // SAFETY: both types are plain C data for which all-zero bytes are valid.
        let mut point: sys::GhosttyPoint = unsafe { std::mem::zeroed() };
        point.tag = sys::GHOSTTY_POINT_TAG_ACTIVE;
        point.value.coordinate = sys::GhosttyPointCoordinate {
            x: col,
            y: u32::from(row),
        };
        // SAFETY: as above.
        let mut grid_ref: sys::GhosttyGridRef = unsafe { std::mem::zeroed() };
        // SAFETY: the handle is live and the out pointer is valid.
        check(unsafe { sys::ghostty_terminal_grid_ref(self.raw, point, &mut grid_ref) })?;
        Ok(grid_ref)
    }

    fn graphemes(&self, grid_ref: &sys::GhosttyGridRef) -> Result<String> {
        let mut buf = [0u32; 16];
        let mut written = 0usize;
        // SAFETY: the buffer pointer and length describe `buf`.
        let result = unsafe {
            sys::ghostty_grid_ref_graphemes(grid_ref, buf.as_mut_ptr(), buf.len(), &mut written)
        };
        let codepoints: Vec<u32> = if result == sys::GHOSTTY_OUT_OF_SPACE {
            // `written` holds the required length. Clusters this long are rare.
            let mut big = vec![0u32; written];
            // SAFETY: the buffer pointer and length describe `big`.
            check(unsafe {
                sys::ghostty_grid_ref_graphemes(grid_ref, big.as_mut_ptr(), big.len(), &mut written)
            })?;
            big.truncate(written);
            big
        } else {
            check(result)?;
            buf[..written.min(buf.len())].to_vec()
        };
        Ok(codepoints
            .into_iter()
            .filter(|&cp| cp != 0)
            .filter_map(char::from_u32)
            .collect())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // SAFETY: the handle came from ghostty_terminal_new and is freed exactly once.
        unsafe { sys::ghostty_terminal_free(self.raw) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_and_headers_are_the_same_version() {
        assert!(!library_version().is_empty());
        assert_eq!(library_version(), header_version());
    }

    /// The basic smoke test. The WebAssembly wrapper's test suite runs the same one.
    #[test]
    fn writes_text_and_reads_back_text_and_colour() {
        let mut term = Terminal::new(80, 24).expect("create terminal");
        term.write(b"hello\r\n\x1b[31mred");

        assert_eq!(term.row_text(0).unwrap(), "hello");
        assert_eq!(term.row_text(1).unwrap(), "red");

        let cell = term.cell(0, 1).unwrap();
        assert_eq!(cell.text, "r");
        assert_eq!(
            cell.fg,
            Color::Palette(1),
            "SGR 31 is palette colour 1 (red)"
        );

        let plain = term.cell(0, 0).unwrap();
        assert_eq!(plain.fg, Color::Default);
    }

    #[test]
    fn wide_characters_take_two_columns_and_one_place_in_text() {
        let mut term = Terminal::new(10, 2).expect("create terminal");
        term.write("a\u{4e2d}b".as_bytes());
        assert_eq!(term.row_text(0).unwrap(), "a\u{4e2d}b");
        assert_eq!(term.cell(1, 0).unwrap().wide, Wide::Wide);
        assert_eq!(term.cell(2, 0).unwrap().wide, Wide::SpacerTail);
    }

    #[test]
    fn resize_changes_the_grid() {
        let mut term = Terminal::new(10, 2).expect("create terminal");
        term.resize(20, 5).unwrap();
        assert_eq!((term.cols(), term.rows()), (20, 5));
        assert!(term.cell(19, 4).is_ok());
        assert!(term.cell(20, 0).is_err());
    }
}
