//! The console `lume` runs in, for `lume ctl attach --raw`: raw mode, its size, and
//! writing terminal output to it as it is.
//!
//! In raw mode every key reaches the session as the bytes a terminal sends for it,
//! Ctrl+C included, and nothing is echoed or edited on the way. The mode the console
//! was in is put back when the [`Raw`] guard goes, however the command ends.

use std::io;

use lume_proto::TermSize;

/// The console held in raw mode. Dropping it puts the console back as it was.
pub struct Raw {
    #[cfg(windows)]
    saved: windows::Saved,
    #[cfg(unix)]
    saved: unix::Saved,
}

impl Raw {
    /// Puts the console in raw mode. Fails when standard input or output is not a
    /// console.
    pub fn enter() -> io::Result<Self> {
        #[cfg(windows)]
        let saved = windows::enter()?;
        #[cfg(unix)]
        let saved = unix::enter()?;
        Ok(Self { saved })
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        #[cfg(windows)]
        windows::leave(&self.saved);
        #[cfg(unix)]
        unix::leave(&self.saved);
    }
}

/// The size of the console's window, if standard output is a console.
pub fn size() -> Option<TermSize> {
    #[cfg(windows)]
    let (cols, rows) = windows::size()?;
    #[cfg(unix)]
    let (cols, rows) = unix::size()?;
    TermSize::new(cols, rows).ok()
}

/// Writes terminal output to the console exactly as it is, bytes that are not text
/// included.
pub fn write(bytes: &[u8]) -> io::Result<()> {
    #[cfg(windows)]
    return windows::write(bytes);
    #[cfg(unix)]
    return unix::write(bytes);
}

#[cfg(windows)]
mod windows {
    use std::io;

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Globalization::CP_UTF8;
    use windows_sys::Win32::Storage::FileSystem::WriteFile;
    use windows_sys::Win32::System::Console::{
        CONSOLE_MODE, CONSOLE_SCREEN_BUFFER_INFO, DISABLE_NEWLINE_AUTO_RETURN, ENABLE_ECHO_INPUT,
        ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT, ENABLE_VIRTUAL_TERMINAL_INPUT,
        ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetConsoleOutputCP,
        GetConsoleScreenBufferInfo, GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        SetConsoleMode, SetConsoleOutputCP,
    };

    pub struct Saved {
        input: CONSOLE_MODE,
        output: CONSOLE_MODE,
        code_page: u32,
    }

    fn handle(which: u32) -> HANDLE {
        // SAFETY: GetStdHandle only reads the process's standard handles.
        unsafe { GetStdHandle(which) }
    }

    fn mode(handle: HANDLE) -> io::Result<CONSOLE_MODE> {
        let mut mode = 0;
        // SAFETY: `mode` is a valid place for the call to write to.
        if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
            return Err(io::Error::other(
                "standard input and output must be a console",
            ));
        }
        Ok(mode)
    }

    fn set_mode(handle: HANDLE, mode: CONSOLE_MODE) -> io::Result<()> {
        // SAFETY: setting a console mode reads nothing of this process's memory.
        if unsafe { SetConsoleMode(handle, mode) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn enter() -> io::Result<Saved> {
        let (input, output) = (handle(STD_INPUT_HANDLE), handle(STD_OUTPUT_HANDLE));
        let saved = Saved {
            input: mode(input)?,
            output: mode(output)?,
            // SAFETY: reads the console's code page.
            code_page: unsafe { GetConsoleOutputCP() },
        };
        let raw_input = (saved.input
            & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT))
            | ENABLE_VIRTUAL_TERMINAL_INPUT;
        let raw_output =
            saved.output | ENABLE_VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN;
        set_mode(input, raw_input)?;
        if let Err(error) = set_mode(output, raw_output) {
            let _ = set_mode(input, saved.input);
            return Err(error);
        }
        // Output is UTF-8, as terminals speak it.
        // SAFETY: sets the console's code page.
        unsafe { SetConsoleOutputCP(CP_UTF8) };
        Ok(saved)
    }

    pub fn leave(saved: &Saved) {
        let _ = set_mode(handle(STD_INPUT_HANDLE), saved.input);
        let _ = set_mode(handle(STD_OUTPUT_HANDLE), saved.output);
        // SAFETY: sets the console's code page.
        unsafe { SetConsoleOutputCP(saved.code_page) };
    }

    pub fn size() -> Option<(u16, u16)> {
        // SAFETY: the structure is plain data, zero is a valid value for it, and the
        // call writes into it.
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
        // SAFETY: as above.
        if unsafe { GetConsoleScreenBufferInfo(handle(STD_OUTPUT_HANDLE), &mut info) } == 0 {
            return None;
        }
        let window = info.srWindow;
        let cols = u16::try_from(window.Right - window.Left + 1).ok()?;
        let rows = u16::try_from(window.Bottom - window.Top + 1).ok()?;
        Some((cols, rows))
    }

    pub fn write(mut bytes: &[u8]) -> io::Result<()> {
        let output = handle(STD_OUTPUT_HANDLE);
        while !bytes.is_empty() {
            let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
            let mut written = 0;
            // SAFETY: `bytes` holds `len` readable bytes and `written` is a valid place
            // for the count; no overlapped I/O is asked for.
            let ok = unsafe {
                WriteFile(
                    output,
                    bytes.as_ptr(),
                    len,
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            bytes = &bytes[written as usize..];
        }
        Ok(())
    }
}

#[cfg(unix)]
mod unix {
    use std::io::{self, Write};

    pub struct Saved(libc::termios);

    pub fn enter() -> io::Result<Saved> {
        // SAFETY: termios is plain data and the call fills it in.
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: as above, for standard input.
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut saved) } != 0 {
            return Err(io::Error::other("standard input must be a terminal"));
        }
        let mut raw = saved;
        // SAFETY: changes the copy in place.
        unsafe { libc::cfmakeraw(&mut raw) };
        // SAFETY: sets standard input's mode from a valid termios.
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Saved(saved))
    }

    pub fn leave(saved: &Saved) {
        // SAFETY: puts back the mode read before.
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &saved.0) };
    }

    pub fn size() -> Option<(u16, u16)> {
        // SAFETY: winsize is plain data and the call fills it in.
        let mut size: libc::winsize = unsafe { std::mem::zeroed() };
        // SAFETY: as above, for standard output.
        if unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) } != 0 {
            return None;
        }
        Some((size.ws_col, size.ws_row))
    }

    pub fn write(bytes: &[u8]) -> io::Result<()> {
        let mut out = io::stdout().lock();
        out.write_all(bytes)?;
        out.flush()
    }
}
