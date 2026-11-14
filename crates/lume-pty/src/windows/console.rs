//! Giving a windowless process a console that nobody sees.
//!
//! A program built for the GUI subsystem starts without a console. A console program it
//! then starts gets a new console of its own, shown as a window for as long as that
//! program runs. A process that owns one hidden console lets such children share it
//! instead, so nothing is shown.
//!
//! Programs started in a pseudoconsole attach to the pseudoconsole and are not affected
//! either way. This matters for the helper programs a terminal application runs outside
//! any terminal.

use std::io;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Console::{AllocConsole, GetConsoleCP, GetConsoleWindow};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};
use windows_sys::core::{BOOL, HRESULT};

use super::api::ExportedFn;
use super::check;

/// `ALLOC_CONSOLE_OPTIONS` from `consoleapi.h`.
#[repr(C)]
struct AllocConsoleOptions {
    mode: i32,
    use_show_window: BOOL,
    show_window: u16,
}

/// `ALLOC_CONSOLE_MODE_NO_WINDOW`: a console with no window at all.
const MODE_NO_WINDOW: i32 = 2;

type AllocConsoleWithOptionsFn =
    unsafe extern "system" fn(*const AllocConsoleOptions, *mut i32) -> HRESULT;

/// Makes sure this process has a console, creating a hidden one if it has none.
///
/// A process that already has a console (one started from a terminal, for instance) is
/// left alone. On Windows builds that can create a console without a window, none is ever
/// created. On older builds the window exists for a moment before it is hidden, which can
/// show as a brief flash.
pub fn ensure_hidden_console() -> io::Result<()> {
    // SAFETY: takes no arguments and only reads process state. It returns zero when the
    // process has no console.
    if unsafe { GetConsoleCP() } != 0 {
        return Ok(());
    }

    if let Some(alloc) = alloc_console_with_options() {
        let options = AllocConsoleOptions {
            mode: MODE_NO_WINDOW,
            use_show_window: 0,
            show_window: 0,
        };
        let mut result = 0;
        // SAFETY: both pointers are valid for the call, and `options` has the layout the
        // function expects.
        if unsafe { alloc(&options, &mut result) } >= 0 {
            return Ok(());
        }
        // Fall through: an older console host may refuse the mode.
    }

    // SAFETY: takes no arguments.
    check(unsafe { AllocConsole() })?;
    // SAFETY: takes no arguments.
    let window: HWND = unsafe { GetConsoleWindow() };
    if !window.is_null() {
        // SAFETY: `window` is the console window this process now owns.
        unsafe { ShowWindow(window, SW_HIDE) };
    }
    Ok(())
}

/// `AllocConsoleWithOptions`, where this build of Windows has it.
fn alloc_console_with_options() -> Option<AllocConsoleWithOptionsFn> {
    let kernel32: Vec<u16> = "kernel32.dll\0".encode_utf16().collect();
    // SAFETY: the name is NUL-terminated. kernel32 is loaded in every process.
    let module = unsafe { GetModuleHandleW(kernel32.as_ptr()) };
    if module.is_null() {
        return None;
    }
    // SAFETY: `module` is a loaded library and the name is NUL-terminated.
    let found = unsafe { GetProcAddress(module, c"AllocConsoleWithOptions".as_ptr().cast()) }?;
    // SAFETY: the export of this name has this signature, as declared in the Windows SDK.
    Some(unsafe { std::mem::transmute::<ExportedFn, AllocConsoleWithOptionsFn>(found) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_that_has_a_console_keeps_it() {
        // Tests run from a terminal or a build tool, both of which provide a console.
        // SAFETY: takes no arguments.
        if unsafe { GetConsoleCP() } == 0 {
            return;
        }
        // SAFETY: takes no arguments.
        let before = unsafe { GetConsoleWindow() };
        ensure_hidden_console().unwrap();
        // SAFETY: takes no arguments.
        assert_eq!(unsafe { GetConsoleWindow() }, before);
    }
}
