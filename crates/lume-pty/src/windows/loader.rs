//! Choosing between a pseudoconsole library shipped beside the executable and the one
//! built into Windows.
//!
//! The shipped library (`conpty.dll`) starts its own console host (`OpenConsole.exe`)
//! from the same directory. The two are built together and talk a private protocol, so a
//! library and a host from different builds crash the shells started under them. The
//! pair is therefore used only when both files are present and carry the same version;
//! anything else falls back to the system's implementation, which always works.
//!
//! The library is loaded by its full path, with the search for its own dependencies
//! limited to its directory and the system directory. A bare name would let a file
//! planted in the working directory or on `PATH` be loaded into the process.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
};
use windows_sys::Win32::System::LibraryLoader::{
    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};

use super::api::ConPtyApi;

const LIBRARY: &str = "conpty.dll";
const HOST: &str = "OpenConsole.exe";

/// A file version: major, minor, build, revision.
pub(crate) type Version = [u16; 4];

/// What was found where one file of the pair should be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Probe {
    Absent,
    /// The file exists but has no readable version.
    Unreadable,
    Version(Version),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Use the shipped pair.
    Bundled,
    /// Use the system's implementation; nothing was shipped.
    Inbox,
    /// Use the system's implementation because the shipped pair cannot be trusted.
    InboxBecause(&'static str),
}

/// Decides which implementation to use from what was found beside the executable.
pub(crate) fn decide(library: Probe, host: Probe) -> Decision {
    match (library, host) {
        (Probe::Absent, Probe::Absent) => Decision::Inbox,
        (Probe::Absent, _) | (_, Probe::Absent) => {
            Decision::InboxBecause("only one of the library and its console host is present")
        }
        (Probe::Unreadable, _) | (_, Probe::Unreadable) => {
            Decision::InboxBecause("the version of the library or its console host cannot be read")
        }
        (Probe::Version(library), Probe::Version(host)) if library == host => Decision::Bundled,
        (Probe::Version(_), Probe::Version(_)) => {
            Decision::InboxBecause("the library and its console host have different versions")
        }
    }
}

/// Picks and loads the implementation for this process.
pub(crate) fn load() -> ConPtyApi {
    let Some(dir) = executable_dir() else {
        return ConPtyApi::inbox();
    };
    let library = dir.join(LIBRARY);
    match decide(probe(&library), probe(&dir.join(HOST))) {
        Decision::Inbox => ConPtyApi::inbox(),
        Decision::InboxBecause(reason) => {
            tracing::warn!(
                reason,
                "ignoring the pseudoconsole shipped beside the executable"
            );
            ConPtyApi::inbox()
        }
        Decision::Bundled => load_library(library).unwrap_or_else(|reason| {
            tracing::warn!(
                reason,
                "ignoring the pseudoconsole shipped beside the executable"
            );
            ConPtyApi::inbox()
        }),
    }
}

/// The directory of the running executable, as an absolute path.
fn executable_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    dir.is_absolute().then(|| dir.to_path_buf())
}

fn load_library(path: PathBuf) -> Result<ConPtyApi, &'static str> {
    if !path.is_absolute() {
        return Err("its path is not absolute");
    }
    let wide = wide(&path).ok_or("its path contains a NUL")?;
    // SAFETY: `wide` is a NUL-terminated absolute path, which the load-directory flag
    // requires. The file handle argument is reserved and must be null.
    let module = unsafe {
        LoadLibraryExW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if module.is_null() {
        return Err("the library could not be loaded");
    }
    // SAFETY: the library was just loaded and is never freed. It is the pseudoconsole
    // library, whose exports of these names have the system functions' signatures.
    unsafe { ConPtyApi::from_module(module, path) }
        .ok_or("the library lacks a pseudoconsole function")
}

fn probe(path: &Path) -> Probe {
    if !path.is_file() {
        return Probe::Absent;
    }
    file_version(path).map_or(Probe::Unreadable, Probe::Version)
}

/// The version in a file's version resource.
fn file_version(path: &Path) -> Option<Version> {
    const SIGNATURE: u32 = 0xFEEF_04BD;

    let wide = wide(path)?;
    let mut ignored = 0;
    // SAFETY: `wide` is NUL-terminated and `ignored` is a valid place for the unused out
    // value.
    let size = unsafe { GetFileVersionInfoSizeW(wide.as_ptr(), &mut ignored) };
    if size == 0 {
        return None;
    }
    // Words, not bytes: the structure read out of this buffer below needs their alignment.
    let mut buffer = vec![0u32; (size as usize).div_ceil(4)];
    // SAFETY: the buffer holds at least `size` bytes.
    let ok = unsafe { GetFileVersionInfoW(wide.as_ptr(), 0, size, buffer.as_mut_ptr().cast()) };
    if ok == 0 {
        return None;
    }

    let root = [u16::from(b'\\'), 0];
    let mut info: *mut core::ffi::c_void = std::ptr::null_mut();
    let mut len = 0;
    // SAFETY: `buffer` was filled by `GetFileVersionInfoW`, `root` is NUL-terminated,
    // and the two out pointers are valid.
    let ok = unsafe { VerQueryValueW(buffer.as_ptr().cast(), root.as_ptr(), &mut info, &mut len) };
    if ok == 0 || info.is_null() || (len as usize) < size_of::<VS_FIXEDFILEINFO>() {
        return None;
    }
    // SAFETY: on success `info` points at a `VS_FIXEDFILEINFO` of at least `len` bytes
    // inside `buffer`, which is alive and aligned for it.
    let info = unsafe { &*info.cast::<VS_FIXEDFILEINFO>() };
    if info.dwSignature != SIGNATURE {
        return None;
    }
    let (high, low) = (info.dwFileVersionMS, info.dwFileVersionLS);
    Some([
        (high >> 16) as u16,
        high as u16,
        (low >> 16) as u16,
        low as u16,
    ])
}

/// A path as a NUL-terminated wide string, or `None` if it contains a NUL.
fn wide(path: &Path) -> Option<Vec<u16>> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return None;
    }
    wide.push(0);
    Some(wide)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: Probe = Probe::Version([1, 22, 2502, 0]);
    const V2: Probe = Probe::Version([1, 23, 2510, 0]);

    #[test]
    fn nothing_shipped_selects_the_system_implementation_quietly() {
        assert_eq!(decide(Probe::Absent, Probe::Absent), Decision::Inbox);
    }

    #[test]
    fn a_matched_pair_is_used() {
        assert_eq!(decide(V1, V1), Decision::Bundled);
    }

    #[test]
    fn mismatched_versions_select_the_system_implementation() {
        assert!(matches!(decide(V1, V2), Decision::InboxBecause(_)));
        assert!(matches!(decide(V2, V1), Decision::InboxBecause(_)));
    }

    #[test]
    fn half_a_pair_or_an_unreadable_version_selects_the_system_implementation() {
        for (library, host) in [
            (V1, Probe::Absent),
            (Probe::Absent, V1),
            (V1, Probe::Unreadable),
            (Probe::Unreadable, V1),
            (Probe::Unreadable, Probe::Unreadable),
            (Probe::Unreadable, Probe::Absent),
        ] {
            assert!(
                matches!(decide(library, host), Decision::InboxBecause(_)),
                "{library:?} with {host:?}"
            );
        }
    }

    #[test]
    fn a_system_file_has_a_readable_version_and_a_missing_file_is_absent() {
        let system = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .expect("SystemRoot is set");
        let kernel = system.join("System32").join("kernel32.dll");
        assert!(matches!(probe(&kernel), Probe::Version([major, ..]) if major >= 6));
        assert_eq!(probe(&system.join("no-such-file.dll")), Probe::Absent);
    }

    #[test]
    fn a_library_is_only_ever_loaded_by_an_absolute_path() {
        assert_eq!(
            load_library(PathBuf::from("conpty.dll")).err(),
            Some("its path is not absolute")
        );
        assert_eq!(
            load_library(PathBuf::from(r"sub\conpty.dll")).err(),
            Some("its path is not absolute")
        );
    }

    #[test]
    fn the_test_process_uses_the_system_implementation() {
        // Nothing is shipped beside a test executable.
        assert_eq!(
            super::super::api::get().source,
            super::super::api::Source::Inbox
        );
    }
}
