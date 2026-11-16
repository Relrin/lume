//! Terminals on Windows: pseudoconsoles.

mod api;
mod child;
mod cmdline;
mod conpty;
pub(crate) mod console;
mod env;
mod foreground;
mod loader;

use std::io;

use crate::{Child, CommandSpec, Pty, PtySize, PtySystem};

pub(crate) struct WinPtySystem;

impl PtySystem for WinPtySystem {
    fn spawn(
        &self,
        command: &CommandSpec,
        size: PtySize,
    ) -> io::Result<(Box<dyn Pty>, Box<dyn Child>)> {
        let (pty, child) = conpty::spawn(api::get(), command, size)?;
        Ok((Box::new(pty), Box::new(child)))
    }

    fn resize_pulls_scrollback(&self) -> bool {
        false
    }
}

/// Turns the result of a call that reports failure as `FALSE` into an `io::Result`.
pub(crate) fn check(ok: windows_sys::core::BOOL) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Turns an `HRESULT` into an `io::Result`, keeping the Win32 error code when the
/// `HRESULT` wraps one so the message is the system's own.
pub(crate) fn check_hresult(hr: windows_sys::core::HRESULT, what: &str) -> io::Result<()> {
    const FACILITY_WIN32: u32 = 0x8007;
    if hr >= 0 {
        return Ok(());
    }
    let bits = hr as u32;
    if bits >> 16 == FACILITY_WIN32 {
        let os = io::Error::from_raw_os_error((bits & 0xffff) as i32);
        Err(io::Error::new(os.kind(), format!("{what}: {os}")))
    } else {
        Err(io::Error::other(format!("{what} failed with {bits:#010x}")))
    }
}
