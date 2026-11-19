//! Opening what a checked path names, and asking the system where the opened thing
//! really is.
//!
//! A path is checked before it is opened, and between the two something could put a link
//! where a folder was. So once a file or folder is open, the system is asked for the
//! final path of the open handle (Windows `GetFinalPathNameByHandleW`, Linux
//! `/proc/self/fd`, macOS `F_GETPATH`) and that is checked again. On Linux and macOS the
//! last name of a path is opened without following a link, and a folder's entries are
//! read through its open handle rather than by its path again.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

/// How a checked path is opened.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Open {
    /// A file, to read.
    Read,
    /// A folder, to read its entries.
    Dir,
    /// A file or a folder, to read what the system says about it.
    Meta,
    /// A file that exists, to change. With `append` every write goes to its end.
    Write { append: bool },
    /// A file that does not exist yet, made by opening it.
    Create { append: bool },
}

/// One entry of a folder, as read through the folder's handle.
pub(crate) struct RawEntry {
    pub name: std::ffi::OsString,
    pub kind: RawKind,
    pub size: u64,
}

/// What an entry is, without following a link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RawKind {
    File,
    Dir,
    /// A link, a junction, a device or anything else.
    Other,
}

#[cfg(windows)]
pub(crate) use windows::{final_path, links, list, open, unblock};

#[cfg(unix)]
pub(crate) use unix::{final_path, links, list, open, unblock};

/// How many names a path has to the thing it names, a file's hard links. Read without
/// opening the file to change it.
pub(crate) fn links_at(path: &Path) -> io::Result<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::symlink_metadata(path).map(|metadata| metadata.nlink())
    }
    #[cfg(windows)]
    {
        links(&open(path, Open::Meta)?)
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Foundation::{ERROR_NO_MORE_FILES, HANDLE};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_BOTH_DIR_INFO, FILE_NAME_NORMALIZED,
        FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdBothDirectoryInfo,
        FileIdBothDirectoryRestartInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
        GetFinalPathNameByHandleW, VOLUME_NAME_DOS,
    };

    use super::{File, Open, OpenOptions, Path, PathBuf, RawEntry, RawKind, io};
    use crate::paths::strip_verbatim;

    fn raw(file: &File) -> HANDLE {
        file.as_raw_handle()
    }

    pub fn open(path: &Path, how: Open) -> io::Result<File> {
        let mut options = OpenOptions::new();
        match how {
            Open::Read => {
                options.read(true);
            }
            // Without sharing deletion, nothing can rename or remove the folder while its
            // entries are read.
            Open::Dir => {
                options
                    .read(true)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
            }
            Open::Meta => {
                options
                    .access_mode(FILE_READ_ATTRIBUTES)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
            }
            Open::Write { append } => {
                options.write(true).append(append);
            }
            Open::Create { append } => {
                options.write(true).append(append).create_new(true);
            }
        }
        options.open(path)
    }

    pub fn final_path(file: &File) -> io::Result<PathBuf> {
        let mut buffer = vec![0u16; 512];
        loop {
            let capacity = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
            // SAFETY: a handle this file owns, and a buffer of the length given.
            let written = unsafe {
                GetFinalPathNameByHandleW(
                    raw(file),
                    buffer.as_mut_ptr(),
                    capacity,
                    FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
                )
            };
            let written = written as usize;
            if written == 0 {
                return Err(io::Error::last_os_error());
            }
            if written < buffer.len() {
                buffer.truncate(written);
                return Ok(strip_verbatim(PathBuf::from(OsString::from_wide(&buffer))));
            }
            // Too small: the call said how much it needs.
            buffer.resize(written + 1, 0);
        }
    }

    pub fn links(file: &File) -> io::Result<u64> {
        // SAFETY: a plain structure the call fills.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: a handle this file owns.
        if unsafe { GetFileInformationByHandle(raw(file), &raw mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(u64::from(info.nNumberOfLinks))
    }

    pub fn unblock(_file: &File) -> io::Result<()> {
        Ok(())
    }

    /// The buffer one listing call fills, in eight-byte words for the records' alignment.
    const LIST_WORDS: usize = 8 * 1024;

    pub fn list(dir: &File) -> io::Result<Vec<RawEntry>> {
        let mut buffer = vec![0u64; LIST_WORDS];
        let bytes = u32::try_from(LIST_WORDS * 8).unwrap_or(u32::MAX);
        let mut entries = Vec::new();
        let mut class = FileIdBothDirectoryRestartInfo;
        loop {
            // SAFETY: a folder handle this file owns, and a buffer of the size given.
            let done = unsafe {
                GetFileInformationByHandleEx(raw(dir), class, buffer.as_mut_ptr().cast(), bytes)
            };
            if done == 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == i32::try_from(ERROR_NO_MORE_FILES).ok() {
                    return Ok(entries);
                }
                return Err(error);
            }
            class = FileIdBothDirectoryInfo;
            let start = buffer.as_ptr().cast::<u8>();
            let mut offset = 0usize;
            loop {
                // SAFETY: the call wrote whole records into the buffer, each at the offset
                // the one before gives; the name follows its record's fixed part.
                let (record, name) = unsafe {
                    let at = start.add(offset);
                    let record = at.cast::<FILE_ID_BOTH_DIR_INFO>().read_unaligned();
                    let name = at
                        .add(std::mem::offset_of!(FILE_ID_BOTH_DIR_INFO, FileName))
                        .cast::<u16>();
                    let length = record.FileNameLength as usize / 2;
                    let mut units = vec![0u16; length];
                    std::ptr::copy_nonoverlapping(
                        name.cast::<u8>(),
                        units.as_mut_ptr().cast(),
                        length * 2,
                    );
                    (record, units)
                };
                let name = OsString::from_wide(&name);
                if name != "." && name != ".." {
                    let attributes = record.FileAttributes;
                    let kind = if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                        RawKind::Other
                    } else if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                        RawKind::Dir
                    } else {
                        RawKind::File
                    };
                    let size = if kind == RawKind::File {
                        u64::try_from(record.EndOfFile).unwrap_or(0)
                    } else {
                        0
                    };
                    entries.push(RawEntry { name, kind, size });
                }
                if record.NextEntryOffset == 0 {
                    break;
                }
                offset += record.NextEntryOffset as usize;
            }
        }
    }
}

#[cfg(unix)]
mod unix {
    use std::ffi::{CStr, OsStr, OsString};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;

    use super::{File, Open, OpenOptions, Path, PathBuf, RawEntry, RawKind, io};

    /// The last name is never followed if it is a link, and opening a pipe or a device
    /// does not wait for the other end.
    const FLAGS: libc::c_int = libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;

    /// Opening only to ask about the thing needs no permission to read it on Linux.
    #[cfg(target_os = "linux")]
    const META: libc::c_int = libc::O_PATH;
    #[cfg(not(target_os = "linux"))]
    const META: libc::c_int = 0;

    pub fn open(path: &Path, how: Open) -> io::Result<File> {
        let mut options = OpenOptions::new();
        match how {
            Open::Read => {
                options.read(true).custom_flags(FLAGS);
            }
            Open::Dir => {
                options.read(true).custom_flags(FLAGS | libc::O_DIRECTORY);
            }
            Open::Meta => {
                options.read(true).custom_flags(FLAGS | META);
            }
            Open::Write { append } => {
                options.write(true).append(append).custom_flags(FLAGS);
            }
            Open::Create { append } => {
                options
                    .write(true)
                    .append(append)
                    .create_new(true)
                    .mode(0o666)
                    .custom_flags(FLAGS);
            }
        }
        options.open(path)
    }

    #[cfg(target_os = "linux")]
    pub fn final_path(file: &File) -> io::Result<PathBuf> {
        std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
    }

    #[cfg(not(target_os = "linux"))]
    pub fn final_path(file: &File) -> io::Result<PathBuf> {
        let mut buffer = vec![0u8; libc::PATH_MAX as usize + 1];
        // SAFETY: a descriptor this file owns, and a buffer of the size the call fills.
        let done = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buffer.as_mut_ptr()) };
        if done == -1 {
            return Err(io::Error::last_os_error());
        }
        let path = CStr::from_bytes_until_nul(&buffer).map_err(io::Error::other)?;
        Ok(PathBuf::from(OsStr::from_bytes(path.to_bytes())))
    }

    pub fn links(file: &File) -> io::Result<u64> {
        use std::os::unix::fs::MetadataExt;
        file.metadata().map(|metadata| metadata.nlink())
    }

    /// Lets reads wait for data again, once the file is known to be a plain file.
    pub fn unblock(file: &File) -> io::Result<()> {
        let fd = file.as_raw_fd();
        // SAFETY: a descriptor this file owns.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: as above.
        if unsafe { libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn list(dir: &File) -> io::Result<Vec<RawEntry>> {
        // SAFETY: a descriptor this file owns; the copy belongs to the listing, which
        // closes it.
        let fd = unsafe { libc::dup(dir.as_raw_fd()) };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a descriptor of a folder, now owned by the stream.
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            let error = io::Error::last_os_error();
            // SAFETY: the copy made above, which the stream did not take.
            unsafe { libc::close(fd) };
            return Err(error);
        }
        // SAFETY: the stream opened above; the copy shares its place with the original.
        unsafe { libc::rewinddir(stream) };
        let mut entries = Vec::new();
        loop {
            // SAFETY: the stream opened above.
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                break;
            }
            // SAFETY: an entry the stream gave, valid until the next read.
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            let bytes = name.to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            // SAFETY: a plain structure the call fills.
            let mut status: libc::stat = unsafe { std::mem::zeroed() };
            // SAFETY: the stream's folder, a name from it and a structure to fill.
            let done = unsafe {
                libc::fstatat(
                    libc::dirfd(stream),
                    name.as_ptr(),
                    &raw mut status,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if done != 0 {
                continue;
            }
            let kind = match status.st_mode & libc::S_IFMT {
                libc::S_IFREG => RawKind::File,
                libc::S_IFDIR => RawKind::Dir,
                _ => RawKind::Other,
            };
            let size = if kind == RawKind::File {
                u64::try_from(status.st_size).unwrap_or(0)
            } else {
                0
            };
            entries.push(RawEntry {
                name: OsString::from(OsStr::from_bytes(bytes)),
                kind,
                size,
            });
        }
        // SAFETY: the stream opened above, closed once.
        unsafe { libc::closedir(stream) };
        Ok(entries)
    }
}
