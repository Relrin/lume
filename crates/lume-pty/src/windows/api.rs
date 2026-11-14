//! The pseudoconsole functions, taken either from the operating system or from a library
//! shipped beside the executable.
//!
//! Windows carries a pseudoconsole implementation of whatever age the installed build
//! has. An application can ship a newer one as a library and its console host, and use
//! those instead. Both offer the same three functions, so the rest of this crate calls
//! them through this table and does not care which it got.

use std::path::PathBuf;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HANDLE, HMODULE};
use windows_sys::Win32::System::Console::{
    COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole,
};
use windows_sys::Win32::System::LibraryLoader::GetProcAddress;
use windows_sys::core::HRESULT;

use super::loader;

pub(crate) type CreateFn =
    unsafe extern "system" fn(COORD, HANDLE, HANDLE, u32, *mut HPCON) -> HRESULT;
pub(crate) type ResizeFn = unsafe extern "system" fn(HPCON, COORD) -> HRESULT;
pub(crate) type CloseFn = unsafe extern "system" fn(HPCON);

/// The type a library export has before its real signature is known.
pub(crate) type ExportedFn = unsafe extern "system" fn() -> isize;

/// Where the functions came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// The implementation built into Windows.
    Inbox,
    /// The library at this path, beside the executable.
    Bundled(PathBuf),
}

pub(crate) struct ConPtyApi {
    pub(crate) create: CreateFn,
    pub(crate) resize: ResizeFn,
    pub(crate) close: CloseFn,
    pub(crate) source: Source,
}

impl ConPtyApi {
    pub(crate) fn inbox() -> Self {
        Self {
            create: CreatePseudoConsole,
            resize: ResizePseudoConsole,
            close: ClosePseudoConsole,
            source: Source::Inbox,
        }
    }

    /// Reads the three functions from a loaded library, or `None` if any is missing.
    ///
    /// # Safety
    ///
    /// `module` must be a loaded library that is never unloaded, and its exports of
    /// these names must have the signatures of the system functions.
    pub(crate) unsafe fn from_module(module: HMODULE, path: PathBuf) -> Option<Self> {
        // The export names have not been checked against a shipped build of the library,
        // so both the prefixed and the plain spelling are tried.
        let find = |prefixed: &'static [u8], plain: &'static [u8]| {
            [prefixed, plain].into_iter().find_map(|name| {
                debug_assert_eq!(name.last(), Some(&0));
                // SAFETY: `module` is a loaded library and `name` is NUL-terminated.
                unsafe { GetProcAddress(module, name.as_ptr()) }
            })
        };
        let create = find(b"ConptyCreatePseudoConsole\0", b"CreatePseudoConsole\0")?;
        let resize = find(b"ConptyResizePseudoConsole\0", b"ResizePseudoConsole\0")?;
        let close = find(b"ConptyClosePseudoConsole\0", b"ClosePseudoConsole\0")?;

        // SAFETY: the caller guarantees these exports have the system functions'
        // signatures, and function pointers of any signature have the same size.
        unsafe {
            Some(Self {
                create: std::mem::transmute::<ExportedFn, CreateFn>(create),
                resize: std::mem::transmute::<ExportedFn, ResizeFn>(resize),
                close: std::mem::transmute::<ExportedFn, CloseFn>(close),
                source: Source::Bundled(path),
            })
        }
    }
}

/// The functions this process uses, chosen once.
pub(crate) fn get() -> &'static ConPtyApi {
    static API: OnceLock<ConPtyApi> = OnceLock::new();
    API.get_or_init(|| {
        let api = loader::load();
        tracing::debug!(source = ?api.source, "pseudoconsole implementation chosen");
        api
    })
}
