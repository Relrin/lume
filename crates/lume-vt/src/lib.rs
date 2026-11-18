//! A safe wrapper over libghostty-vt.
//!
//! Current scope: create a terminal, write bytes to it, resize it, read back text, cell
//! attributes, the title, the working directory and modes, collect what the terminal
//! answered to the program's questions, and save a terminal as a snapshot or as output
//! that rebuilds it. The colours, cell size and colour scheme a program is told about
//! can be set. Render state, input encoding and selection are not wrapped.

// bindgen gives C enums a different integer type on each platform (`i32` with MSVC, `u32`
// elsewhere), so a cast that is needed on one platform is a no-op on another.
#![allow(clippy::unnecessary_cast)]

pub mod corpus;
mod saved;

pub use saved::{Format, Formatted, color_scheme_report, focus_report};

use std::cell::{Cell as StdCell, RefCell};
use std::ffi::{CStr, c_void};
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

/// Light or dark: what a program learns when it asks which the view uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorScheme {
    Light,
    Dark,
}

impl ColorScheme {
    fn raw(self) -> sys::GhosttyColorScheme {
        match self {
            Self::Light => sys::GHOSTTY_COLOR_SCHEME_LIGHT,
            Self::Dark => sys::GHOSTTY_COLOR_SCHEME_DARK,
        }
    }
}

/// The colours a program is told the view has, and that it draws with when the program
/// has set none: foreground, background, cursor and the 256-colour palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Colors {
    pub foreground: [u8; 3],
    pub background: [u8; 3],
    pub cursor: [u8; 3],
    pub palette: [[u8; 3]; 256],
}

/// The six levels each channel takes in the 6x6x6 colour cube of the 256-colour palette.
const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// The 256-colour palette built from sixteen ANSI colours: those sixteen, the colour
/// cube, then a ramp of 24 greys. The same palette the view builds.
pub fn palette_256(ansi: &[[u8; 3]; 16]) -> [[u8; 3]; 256] {
    let mut palette = [[0u8; 3]; 256];
    palette[..16].copy_from_slice(ansi);
    let mut i = 16;
    for r in CUBE_LEVELS {
        for g in CUBE_LEVELS {
            for b in CUBE_LEVELS {
                palette[i] = [r, g, b];
                i += 1;
            }
        }
    }
    for step in 0..24u8 {
        let level = 8 + step * 10;
        palette[i] = [level, level, level];
        i += 1;
    }
    palette
}

/// What the library's callbacks reach while it runs: the size and colour scheme to
/// report, and where the terminal's answers go.
struct Host {
    cols: StdCell<u16>,
    rows: StdCell<u16>,
    /// The size of one cell in pixels, zero while it is not known.
    cell_width: StdCell<u32>,
    cell_height: StdCell<u32>,
    /// `None` leaves a program's question about the colour scheme unanswered.
    scheme: StdCell<Option<ColorScheme>>,
    /// Everything the terminal wrote back to the program and nobody has taken yet.
    replies: RefCell<Vec<u8>>,
}

impl Host {
    fn new(cols: u16, rows: u16) -> Box<Self> {
        Box::new(Self {
            cols: StdCell::new(cols),
            rows: StdCell::new(rows),
            cell_width: StdCell::new(0),
            cell_height: StdCell::new(0),
            scheme: StdCell::new(None),
            replies: RefCell::new(Vec::new()),
        })
    }
}

/// Collects what the terminal writes back to the program: answers to its questions.
///
/// # Safety
///
/// Called by the library only, with the userdata this crate set: a live [`Host`].
unsafe extern "C" fn on_write_pty(
    _terminal: sys::GhosttyTerminal,
    userdata: *mut c_void,
    data: *const u8,
    len: usize,
) {
    if userdata.is_null() || data.is_null() || len == 0 {
        return;
    }
    // SAFETY: the userdata is the `Host` of the terminal that is running, which outlives
    // the call, and the library hands over `len` readable bytes at `data`.
    let (host, bytes) = unsafe {
        (
            &*userdata.cast::<Host>(),
            std::slice::from_raw_parts(data, len),
        )
    };
    host.replies.borrow_mut().extend_from_slice(bytes);
}

/// Tells the library the terminal's size when a program asks for it, in cells and, once
/// it has been set, in pixels.
///
/// # Safety
///
/// Called by the library only, with the userdata this crate set: a live [`Host`].
unsafe extern "C" fn on_size(
    _terminal: sys::GhosttyTerminal,
    userdata: *mut c_void,
    out_size: *mut sys::GhosttySizeReportSize,
) -> bool {
    if userdata.is_null() || out_size.is_null() {
        return false;
    }
    // SAFETY: as in `on_write_pty`; `out_size` is the library's own zeroed struct.
    unsafe {
        let host = &*userdata.cast::<Host>();
        (*out_size).rows = host.rows.get();
        (*out_size).columns = host.cols.get();
        (*out_size).cell_width = host.cell_width.get();
        (*out_size).cell_height = host.cell_height.get();
    }
    true
}

/// Tells the library the colour scheme when a program asks, if one has been set.
///
/// # Safety
///
/// Called by the library only, with the userdata this crate set: a live [`Host`].
unsafe extern "C" fn on_color_scheme(
    _terminal: sys::GhosttyTerminal,
    userdata: *mut c_void,
    out_scheme: *mut sys::GhosttyColorScheme,
) -> bool {
    if userdata.is_null() || out_scheme.is_null() {
        return false;
    }
    // SAFETY: as in `on_write_pty`; `out_scheme` is the library's own value.
    unsafe {
        let host = &*userdata.cast::<Host>();
        match host.scheme.get() {
            Some(scheme) => {
                *out_scheme = scheme.raw();
                true
            }
            None => false,
        }
    }
}

/// The most of an unfinished escape sequence a terminal keeps so that it can be saved
/// in the middle of one. A longer one makes saving it as a snapshot fail, and the
/// terminal is then saved as output instead.
pub const CONTINUATION_MAX_BYTES: usize = 64 * 1024;

/// A terminal emulator: screen, scrollback, cursor, styles and modes.
///
/// libghostty-vt is not thread-safe, so a `Terminal` is neither `Send` nor `Sync`. Each
/// terminal lives on one thread.
pub struct Terminal {
    raw: sys::GhosttyTerminal,
    cols: u16,
    rows: u16,
    /// Boxed so that its address, which the library holds, never changes.
    host: Box<Host>,
    _single_thread: PhantomData<*mut ()>,
}

impl Terminal {
    /// Create a terminal with the given grid size.
    pub fn new(cols: u16, rows: u16) -> Result<Self> {
        let mut raw: sys::GhosttyTerminal = std::ptr::null_mut();
        // SAFETY: a NULL allocator selects the library's default; `raw` is a valid place
        // for the handle.
        check(unsafe { sys::ghostty_terminal_new(std::ptr::null(), &mut raw, cols, rows) })?;
        Self::adopt(raw, cols, rows)
    }

    /// Takes ownership of a terminal the library made and connects its callbacks. The
    /// handle is freed if anything fails.
    fn adopt(raw: sys::GhosttyTerminal, cols: u16, rows: u16) -> Result<Self> {
        if raw.is_null() {
            return Err(Error(sys::GHOSTTY_OUT_OF_MEMORY as i32));
        }
        let term = Self {
            raw,
            cols,
            rows,
            host: Host::new(cols, rows),
            _single_thread: PhantomData,
        };
        // From here on dropping `term` frees the handle, whatever fails.
        term.set(
            sys::GHOSTTY_TERMINAL_OPT_USERDATA,
            (&raw const *term.host).cast(),
        )?;
        term.set(
            sys::GHOSTTY_TERMINAL_OPT_WRITE_PTY,
            on_write_pty as *const c_void,
        )?;
        term.set(sys::GHOSTTY_TERMINAL_OPT_SIZE, on_size as *const c_void)?;
        term.set(
            sys::GHOSTTY_TERMINAL_OPT_COLOR_SCHEME,
            on_color_scheme as *const c_void,
        )?;
        // Reporting the title back lets output inject text into the program's input.
        let no = false;
        term.set(
            sys::GHOSTTY_TERMINAL_OPT_TITLE_REPORT,
            (&raw const no).cast(),
        )?;
        // Inline images are not drawn anywhere this terminal is shown from, so none are
        // kept. A build without image support does not know the option, which amounts
        // to the same thing.
        let none: u64 = 0;
        let stored = term.set(
            sys::GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_STORAGE_LIMIT,
            (&raw const none).cast(),
        );
        if let Err(error) = stored
            && error.0 != sys::GHOSTTY_UNSUPPORTED as i32
        {
            return Err(error);
        }
        // Keep the start of an unfinished sequence, so the terminal can be saved
        // between two writes that split one.
        let keep = CONTINUATION_MAX_BYTES;
        term.set(
            sys::GHOSTTY_TERMINAL_OPT_CONTINUATION_MAX_BYTES,
            (&raw const keep).cast(),
        )?;
        Ok(term)
    }

    /// Set one option with `ghostty_terminal_set`. `value` must be what the header
    /// documents for `option`.
    fn set(&self, option: sys::GhosttyTerminalOption, value: *const c_void) -> Result<()> {
        // SAFETY: the handle is live; the callers pass the pointer kind the option takes,
        // and what it points to lives as long as the terminal.
        check(unsafe { sys::ghostty_terminal_set(self.raw, option, value) })
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

    /// Resize the grid. The cell size in pixels is the one last set with
    /// [`set_cell_px`](Self::set_cell_px), zero until then.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        // The size a program is told is the one the grid is about to have: the resize
        // itself may report it.
        self.host.cols.set(cols);
        self.host.rows.set(rows);
        // SAFETY: the handle is live.
        let resized = check(unsafe {
            sys::ghostty_terminal_resize(
                self.raw,
                cols,
                rows,
                self.host.cell_width.get(),
                self.host.cell_height.get(),
            )
        });
        if resized.is_err() {
            self.host.cols.set(self.cols);
            self.host.rows.set(self.rows);
            return resized;
        }
        self.cols = cols;
        self.rows = rows;
        Ok(())
    }

    /// Everything the terminal has written back to the program and nobody has taken:
    /// its answers to device, cursor, size and colour queries, in the order it gave them.
    pub fn replies(&self) -> Vec<u8> {
        self.host.replies.borrow().clone()
    }

    /// Takes what [`replies`](Self::replies) would return, so that the next call starts
    /// empty.
    pub fn take_replies(&self) -> Vec<u8> {
        std::mem::take(&mut *self.host.replies.borrow_mut())
    }

    /// Sets the size of one cell in pixels, as a program learns it when it asks.
    pub fn set_cell_px(&mut self, width: u32, height: u32) -> Result<()> {
        if (width, height) == (self.host.cell_width.get(), self.host.cell_height.get()) {
            return Ok(());
        }
        self.host.cell_width.set(width);
        self.host.cell_height.set(height);
        // SAFETY: the handle is live.
        check(unsafe {
            sys::ghostty_terminal_resize(self.raw, self.cols, self.rows, width, height)
        })
    }

    /// Sets the colour scheme a program is told about. `None` leaves its question
    /// unanswered.
    pub fn set_color_scheme(&mut self, scheme: Option<ColorScheme>) {
        self.host.scheme.set(scheme);
    }

    /// Sets the colours a program is told the view has and that cells without a colour
    /// of their own take.
    pub fn set_colors(&mut self, colors: &Colors) -> Result<()> {
        let rgb = |[r, g, b]: [u8; 3]| sys::GhosttyColorRgb { r, g, b };
        let fg = rgb(colors.foreground);
        let bg = rgb(colors.background);
        let cursor = rgb(colors.cursor);
        let palette: Vec<sys::GhosttyColorRgb> = colors.palette.iter().map(|&c| rgb(c)).collect();
        self.set(
            sys::GHOSTTY_TERMINAL_OPT_COLOR_FOREGROUND,
            (&raw const fg).cast(),
        )?;
        self.set(
            sys::GHOSTTY_TERMINAL_OPT_COLOR_BACKGROUND,
            (&raw const bg).cast(),
        )?;
        self.set(
            sys::GHOSTTY_TERMINAL_OPT_COLOR_CURSOR,
            (&raw const cursor).cast(),
        )?;
        self.set(
            sys::GHOSTTY_TERMINAL_OPT_COLOR_PALETTE,
            palette.as_ptr().cast(),
        )
    }

    /// Caps scrollback at about `lines` lines and `bytes` bytes, whichever is reached
    /// first. The library keeps whole pages, so a few dozen more lines may be kept.
    pub fn set_scrollback_limits(&mut self, lines: usize, bytes: usize) -> Result<()> {
        self.set(
            sys::GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_BYTES,
            (&raw const bytes).cast(),
        )?;
        self.set(
            sys::GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_LINES,
            (&raw const lines).cast(),
        )
    }

    /// How many rows are in scrollback, above the active area.
    pub fn scrollback_rows(&self) -> Result<usize> {
        self.get::<usize>(sys::GHOSTTY_TERMINAL_DATA_SCROLLBACK_ROWS)
    }

    /// The memory the terminal's screens hold, in bytes.
    pub fn memory_bytes(&self) -> Result<u64> {
        let mut usage = std::mem::MaybeUninit::<sys::GhosttyTerminalMemoryUsage>::zeroed();
        // SAFETY: the struct is plain C data; the library reads its `size` field first.
        unsafe {
            sys::ghostty_sized_init(
                usage.as_mut_ptr().cast(),
                std::mem::size_of::<sys::GhosttyTerminalMemoryUsage>(),
            );
        }
        // SAFETY: the handle is live and `usage` is the sized struct the key documents.
        check(unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_MEMORY_USAGE,
                usage.as_mut_ptr().cast(),
            )
        })?;
        // SAFETY: the call succeeded and filled the struct.
        let usage = unsafe { usage.assume_init() };
        Ok(usage.primary_resident_bytes + usage.alternate_resident_bytes)
    }

    /// The title the program last set, or the empty string.
    pub fn title(&self) -> Result<String> {
        self.string(sys::GHOSTTY_TERMINAL_DATA_TITLE)
    }

    /// The working directory the program last reported, or the empty string.
    pub fn pwd(&self) -> Result<String> {
        self.string(sys::GHOSTTY_TERMINAL_DATA_PWD)
    }

    /// Whether a mode is set. `mode` is the number a program uses for it; `ansi` selects
    /// the ANSI modes (`CSI n h`) instead of the DEC private ones (`CSI ? n h`).
    pub fn mode(&self, mode: u16, ansi: bool) -> Result<bool> {
        // The library packs the mode number and its kind into one 16-bit value.
        let mut config = sys::GhosttyTerminalModeConfig {
            mode: (mode & 0x7fff) | if ansi { 0x8000 } else { 0 },
            value: false,
        };
        // SAFETY: the handle is live and `config` is the type the header documents for
        // this key, with the mode to look up filled in.
        check(unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_MODE,
                (&raw mut config).cast(),
            )
        })?;
        Ok(config.value)
    }

    /// Read a string value. The library's string is borrowed until the terminal next
    /// changes, so it is copied at once.
    fn string(&self, key: sys::GhosttyTerminalData) -> Result<String> {
        let borrowed = self.get::<sys::GhosttyString>(key)?;
        if borrowed.ptr.is_null() || borrowed.len == 0 {
            return Ok(String::new());
        }
        // SAFETY: the library returned `len` readable bytes at `ptr`, valid until the
        // terminal is next written to, which cannot happen during this call.
        let bytes = unsafe { std::slice::from_raw_parts(borrowed.ptr, borrowed.len) };
        Ok(String::from_utf8_lossy(bytes).into_owned())
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
    fn answers_to_the_programs_questions_are_collected_in_order() {
        let mut term = Terminal::new(80, 24).expect("create terminal");
        assert!(term.replies().is_empty());
        // Where is the cursor, then how large is the text area.
        term.write(b"ab\x1b[6n\x1b[18t");
        assert_eq!(term.replies(), b"\x1b[1;3R\x1b[8;24;80t");

        // The size reported is the one the terminal has after a resize.
        term.resize(100, 30).unwrap();
        term.write(b"\x1b[18t");
        assert!(term.replies().ends_with(b"\x1b[8;30;100t"));
    }

    #[test]
    fn the_title_the_directory_and_modes_can_be_read() {
        let mut term = Terminal::new(80, 24).expect("create terminal");
        assert_eq!(term.title().unwrap(), "");
        assert_eq!(term.pwd().unwrap(), "");
        assert!(!term.mode(2004, false).unwrap());
        assert!(term.mode(7, false).unwrap(), "lines wrap by default");

        term.write(b"\x1b]2;a title\x07\x1b]7;file://host/tmp\x07\x1b[?2004h\x1b[4h");
        assert_eq!(term.title().unwrap(), "a title");
        assert_eq!(term.pwd().unwrap(), "file://host/tmp");
        assert!(term.mode(2004, false).unwrap());
        assert!(term.mode(4, true).unwrap());
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
