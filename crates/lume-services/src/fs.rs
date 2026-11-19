//! Files: reading, writing, looking at and watching them, within what `fs.read` and
//! `fs.write` grant.
//!
//! A plugin names a path that is absolute or begins with `${workspace}`, `${home}` or
//! `${plugin-data}`. The path is made canonical, every link and junction on the way
//! resolved, and only then matched against the grant's patterns, which are made canonical
//! the same way. So a link inside the workspace that leads out of it leads nowhere. What
//! lies in Lume's own folders is refused whatever is granted, except the plugin's own
//! data folder.
//!
//! Between the check and the use something could change, so once a file or folder is
//! open the system is asked where the open handle really is, and that is checked again.
//! A file with more than one name (a hard link) is never written: the other names may lie
//! anywhere.
//!
//! Whether something outside the grants exists is not for a plugin to learn: such a path
//! is refused before anything says it is not there.

use std::fs::File;
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;

use crate::handle::{self, Open, RawKind};
use crate::paths::{self, Fence, IGNORE_CASE, Parts, Pattern};
use crate::scope::{Allowed, Places, ServiceError};

/// What a path is wanted for, and so which grant must cover it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Covered by `fs.read`. The path must exist.
    Read,
    /// Covered by `fs.write`. The path may be a file not made yet, in a folder that
    /// exists.
    Write,
}

impl Access {
    fn permission(self) -> &'static str {
        match self {
            Self::Read => "fs.read",
            Self::Write => "fs.write",
        }
    }
}

/// The grants of one kind, ready to match paths, and Lume's own folders.
struct Policy {
    fence: Fence,
    patterns: Vec<(Pattern, String)>,
}

impl Policy {
    fn new(places: &Places, granted: &[String]) -> Self {
        Self {
            fence: Fence::new(places),
            patterns: granted
                .iter()
                .filter_map(|grant| Some((Pattern::compile(places, grant)?, grant.clone())))
                .collect(),
        }
    }

    /// The grant that covers a canonical path, if it is reachable at all.
    fn grant_for(&self, path: &Parts) -> Option<&str> {
        if self.fence.shuts_out(path) {
            return None;
        }
        self.patterns
            .iter()
            .find(|(pattern, _)| pattern.matches(path))
            .map(|(_, grant)| grant.as_str())
    }

    /// The grant that covers a canonical path, or why there is none.
    fn allow(&self, path: &Path, asked: &str, access: Access) -> Result<String, ServiceError> {
        let parts = Parts::of(path)
            .ok_or_else(|| ServiceError::denied(format!("`{asked}` cannot be reached")))?;
        if self.fence.shuts_out(&parts) {
            return Err(ServiceError::denied(format!(
                "`{asked}` is in Lume's own folders, which plugins cannot reach"
            )));
        }
        self.patterns
            .iter()
            .find(|(pattern, _)| pattern.matches(&parts))
            .map(|(_, grant)| grant.clone())
            .ok_or_else(|| {
                ServiceError::denied(format!("no {} grant covers `{asked}`", access.permission()))
            })
    }
}

/// An error of the system's about a path, for the plugin.
fn system_error(error: &io::Error, asked: &str) -> ServiceError {
    // Opening without following a link fails this way when the last name has become one.
    #[cfg(unix)]
    if error.raw_os_error() == Some(libc::ELOOP) {
        return ServiceError::denied(format!("`{asked}` changed into a link while it was opened"));
    }
    match error.kind() {
        io::ErrorKind::NotFound => ServiceError::not_found(format!("`{asked}` does not exist")),
        io::ErrorKind::PermissionDenied => {
            ServiceError::failed(format!("the system refused access to `{asked}`"))
        }
        io::ErrorKind::IsADirectory => ServiceError::invalid(format!("`{asked}` is a folder")),
        io::ErrorKind::NotADirectory => ServiceError::invalid(format!("`{asked}` is not a folder")),
        _ => ServiceError::failed(format!("`{asked}` could not be used: {error}")),
    }
}

/// Where a checked path is, and the grant that covers it.
struct Located {
    path: PathBuf,
    grant: String,
    policy: Policy,
}

/// Finds and checks the path a plugin asked for.
fn locate(
    places: &Places,
    granted: &[String],
    asked: &str,
    access: Access,
) -> Result<Located, ServiceError> {
    let expanded = paths::expand(places, asked)?;
    let policy = Policy::new(places, granted);
    let path = match paths::canonical(&expanded) {
        Ok(path) => path,
        Err(error) => {
            // A path outside the grants is refused before anything says whether it is
            // there.
            policy.allow(&paths::canonical_lenient(&expanded), asked, access)?;
            if access == Access::Read || error.kind() != io::ErrorKind::NotFound {
                return Err(system_error(&error, asked));
            }
            new_file(&expanded, asked)?
        }
    };
    let grant = policy.allow(&path, asked, access)?;
    Ok(Located {
        path,
        grant,
        policy,
    })
}

/// Where a file that is not there yet would be made: its folder made canonical, and its
/// name as written.
fn new_file(expanded: &Path, asked: &str) -> Result<PathBuf, ServiceError> {
    let last = asked
        .rsplit(|c| c == '/' || (cfg!(windows) && c == '\\'))
        .next()
        .unwrap_or("");
    if last.is_empty() || last == "." || last.starts_with("${") {
        return Err(ServiceError::invalid(format!(
            "`{asked}` does not end in a file's name"
        )));
    }
    check_new_name(last, asked)?;
    let folder = expanded
        .parent()
        .ok_or_else(|| ServiceError::invalid(format!("`{asked}` does not end in a file's name")))?;
    let folder = paths::canonical(folder).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => {
            ServiceError::not_found(format!("the folder of `{asked}` does not exist"))
        }
        _ => system_error(&error, asked),
    })?;
    let path = folder.join(last);
    // Something is there that could not be made canonical: a link to nothing.
    if std::fs::symlink_metadata(&path).is_ok() {
        return Err(ServiceError::denied(format!(
            "`{asked}` is a link to something that is not there"
        )));
    }
    Ok(path)
}

/// Refuses a new name Windows would change or read as something else: one ending in a
/// dot or a space, one naming a stream of another file, or a device's name.
fn check_new_name(name: &str, asked: &str) -> Result<(), ServiceError> {
    if !cfg!(windows) {
        return Ok(());
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    let device = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ((stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.len() == 4
        && stem.as_bytes()[3].is_ascii_digit());
    if device || name.ends_with(['.', ' ']) || name.contains([':', '*', '?', '"', '<', '>', '|']) {
        return Err(ServiceError::invalid(format!(
            "`{asked}`: Windows does not allow that file name"
        )));
    }
    Ok(())
}

/// Checks a path the way every call here does, without opening it: the path is
/// canonical, covered by one of `granted` (the patterns of `fs.read` for reading, of
/// `fs.write` for writing) and not in Lume's own folders. For writing, a file not made
/// yet is checked by its folder, made canonical, and its name.
pub fn check(
    places: &Places,
    granted: &[String],
    path: &str,
    access: Access,
) -> Result<Allowed<PathBuf>, ServiceError> {
    let located = locate(places, granted, path, access)?;
    Ok(Allowed {
        value: located.path,
        grant: located.grant,
    })
}

/// Opens a located path and checks again, through the handle, that it is where it was
/// checked and still allowed.
fn open_checked(
    places: &Places,
    granted: &[String],
    located: &Located,
    asked: &str,
    access: Access,
    how: Open,
) -> Result<File, ServiceError> {
    let file = handle::open(&located.path, how).map_err(|error| system_error(&error, asked))?;
    verify(places, granted, &file, &located.path, asked, access)?;
    Ok(file)
}

/// Checks that an open handle is the path that was checked, and that the path is still
/// allowed.
fn verify(
    places: &Places,
    granted: &[String],
    file: &File,
    checked: &Path,
    asked: &str,
    access: Access,
) -> Result<(), ServiceError> {
    let actual = handle::final_path(file).map_err(|_| {
        ServiceError::failed(format!("where `{asked}` really lies could not be read"))
    })?;
    let moved = || ServiceError::denied(format!("`{asked}` changed while it was opened"));
    let (Some(actual_parts), Some(checked_parts)) = (Parts::of(&actual), Parts::of(checked)) else {
        return Err(moved());
    };
    if !actual_parts.same(&checked_parts, IGNORE_CASE) {
        return Err(moved());
    }
    Policy::new(places, granted).allow(&actual, asked, access)?;
    Ok(())
}

/// Opens a file to read, once it is checked against `granted` (the patterns of
/// `fs.read`). Only a plain file is opened, never a folder, a device or a pipe.
pub fn open_read(
    places: &Places,
    granted: &[String],
    path: &str,
) -> Result<Allowed<File>, ServiceError> {
    let located = locate(places, granted, path, Access::Read)?;
    // Windows refuses to open a folder as a file without saying why; say it here.
    if located.path.is_dir() {
        return Err(not_a_file(path, true));
    }
    let file = open_checked(places, granted, &located, path, Access::Read, Open::Read)?;
    let metadata = file
        .metadata()
        .map_err(|error| system_error(&error, path))?;
    if !metadata.is_file() {
        return Err(not_a_file(path, metadata.is_dir()));
    }
    handle::unblock(&file).map_err(|error| system_error(&error, path))?;
    Ok(Allowed {
        value: file,
        grant: located.grant,
    })
}

fn not_a_file(asked: &str, is_dir: bool) -> ServiceError {
    if is_dir {
        ServiceError::invalid(format!("`{asked}` is a folder, not a file"))
    } else {
        ServiceError::invalid(format!("`{asked}` is not a plain file"))
    }
}

/// Reads a whole file of at most `max_bytes`, once it is checked against `granted` (the
/// patterns of `fs.read`). A larger file is refused as too large.
pub fn read(
    places: &Places,
    granted: &[String],
    path: &str,
    max_bytes: u64,
) -> Result<Allowed<Vec<u8>>, ServiceError> {
    let Allowed { value: file, grant } = open_read(places, granted, path)?;
    let too_large =
        || ServiceError::too_large(format!("`{path}` is larger than {max_bytes} bytes"));
    let size = file.metadata().map_or(0, |metadata| metadata.len());
    if size > max_bytes {
        return Err(too_large());
    }
    let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| system_error(&error, path))?;
    if bytes.len() as u64 > max_bytes {
        return Err(too_large());
    }
    Ok(Allowed {
        value: bytes,
        grant,
    })
}

/// How a write treats the file that is there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteMode {
    /// What was there is replaced, or the file is made.
    Replace,
    /// The bytes go after what is there, or the file is made.
    Append,
    /// The file is made, and the write fails if it is there already.
    CreateNew,
}

/// Writes a file, once it is checked against `granted` (the patterns of `fs.write`).
///
/// A file being replaced is changed in place, not written beside it and renamed over it.
/// The checks are made on the handle the bytes go through: that it is the file checked
/// and that it has no other name. A rename would put the new file at whatever the name
/// leads to by then, past those checks. In place, the file also keeps what the system
/// keeps about it (its permissions, its owner, the programs that have it open), the
/// folder needs no permission to make files, and nothing is left behind if Lume stops
/// halfway. The cost is that someone reading the file during the write can see part of
/// it.
pub fn write(
    places: &Places,
    granted: &[String],
    path: &str,
    bytes: &[u8],
    mode: WriteMode,
) -> Result<Allowed<()>, ServiceError> {
    let located = locate(places, granted, path, Access::Write)?;
    let hard_link = || {
        ServiceError::denied(format!(
            "`{path}` has other names (hard links), which a write would change too"
        ))
    };
    if let Ok(metadata) = std::fs::symlink_metadata(&located.path) {
        if !metadata.is_file() {
            return Err(not_a_file(path, metadata.is_dir()));
        }
        if handle::links_at(&located.path).is_ok_and(|links| links > 1) {
            return Err(hard_link());
        }
    }
    let append = mode == WriteMode::Append;
    let mut file = match mode {
        WriteMode::CreateNew => {
            handle::open(&located.path, Open::Create { append }).map_err(|error| {
                match error.kind() {
                    io::ErrorKind::AlreadyExists => {
                        ServiceError::failed(format!("`{path}` already exists"))
                    }
                    _ => system_error(&error, path),
                }
            })?
        }
        WriteMode::Replace | WriteMode::Append => open_or_make(&located.path, append, path)?,
    };
    // A file made through a folder that changed under it is left as it is, empty: the
    // name it has now could lead anywhere, and removing what is there could remove
    // something else.
    verify(places, granted, &file, &located.path, path, Access::Write)?;
    let metadata = file
        .metadata()
        .map_err(|error| system_error(&error, path))?;
    if !metadata.is_file() {
        return Err(not_a_file(path, metadata.is_dir()));
    }
    if handle::links(&file).map_err(|error| system_error(&error, path))? > 1 {
        return Err(hard_link());
    }
    if mode == WriteMode::Replace {
        file.set_len(0)
            .map_err(|error| system_error(&error, path))?;
    }
    file.write_all(bytes)
        .and_then(|()| file.flush())
        .map_err(|error| system_error(&error, path))?;
    Ok(Allowed {
        value: (),
        grant: located.grant,
    })
}

/// Opens a file that is there to change it, or makes it.
fn open_or_make(path: &Path, append: bool, asked: &str) -> Result<File, ServiceError> {
    for _ in 0..3 {
        match handle::open(path, Open::Write { append }) {
            Ok(file) => return Ok(file),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(system_error(&error, asked)),
        }
        match handle::open(path, Open::Create { append }) {
            Ok(file) => return Ok(file),
            // Made by someone else in between: open it again.
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(system_error(&error, asked)),
        }
    }
    Err(ServiceError::failed(format!(
        "`{asked}` kept appearing and disappearing while it was opened"
    )))
}

/// What a path names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Dir,
    /// A link, a junction, a device or anything else.
    Other,
}

impl From<RawKind> for EntryKind {
    fn from(kind: RawKind) -> Self {
        match kind {
            RawKind::File => Self::File,
            RawKind::Dir => Self::Dir,
            RawKind::Other => Self::Other,
        }
    }
}

/// What the system says about a file or a folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Stat {
    pub kind: EntryKind,
    /// The file's size in bytes; 0 for a folder.
    pub size: u64,
    /// When it last changed, in milliseconds since 1970, where the system says.
    pub modified_ms: Option<u64>,
    pub readonly: bool,
}

/// What the system says about a file or a folder, once it is checked against `granted`
/// (the patterns of `fs.read`).
pub fn stat(
    places: &Places,
    granted: &[String],
    path: &str,
) -> Result<Allowed<Stat>, ServiceError> {
    let located = locate(places, granted, path, Access::Read)?;
    let file = open_checked(places, granted, &located, path, Access::Read, Open::Meta)?;
    let metadata = file
        .metadata()
        .map_err(|error| system_error(&error, path))?;
    let kind = if metadata.is_file() {
        EntryKind::File
    } else if metadata.is_dir() {
        EntryKind::Dir
    } else {
        EntryKind::Other
    };
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|since| u64::try_from(since.as_millis()).ok());
    Ok(Allowed {
        value: Stat {
            kind,
            size: if kind == EntryKind::File {
                metadata.len()
            } else {
                0
            },
            modified_ms,
            readonly: metadata.permissions().readonly(),
        },
        grant: located.grant,
    })
}

/// One entry of a folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
    /// The file's size in bytes; 0 for anything else.
    pub size: u64,
}

/// The entries of a folder, by name, once the folder itself is checked against
/// `granted` (the patterns of `fs.read`). An entry whose own path no grant covers, or
/// that lies in Lume's own folders, is left out. A link is listed as what it is, not as
/// what it leads to. More than `max_entries` is refused as too large.
pub fn list(
    places: &Places,
    granted: &[String],
    path: &str,
    max_entries: usize,
) -> Result<Allowed<Vec<Entry>>, ServiceError> {
    let located = locate(places, granted, path, Access::Read)?;
    let dir = open_checked(places, granted, &located, path, Access::Read, Open::Dir)?;
    let metadata = dir.metadata().map_err(|error| system_error(&error, path))?;
    if !metadata.is_dir() {
        return Err(ServiceError::invalid(format!("`{path}` is not a folder")));
    }
    let raw = handle::list(&dir).map_err(|error| system_error(&error, path))?;
    let mut entries = Vec::new();
    for entry in raw {
        let Some(name) = entry.name.to_str() else {
            continue;
        };
        let own = located.path.join(name);
        let covered =
            Parts::of(&own).is_some_and(|parts| located.policy.grant_for(&parts).is_some());
        if !covered {
            continue;
        }
        if entries.len() == max_entries {
            return Err(ServiceError::too_large(format!(
                "`{path}` holds more than {max_entries} entries"
            )));
        }
        entries.push(Entry {
            name: name.to_owned(),
            kind: entry.kind.into(),
            size: entry.size,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Allowed {
        value: entries,
        grant: located.grant,
    })
}

/// What happened to a path being watched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WatchKind {
    Created,
    Changed,
    Removed,
    Other,
}

/// A change to a path being watched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WatchEvent {
    /// The path that changed, absolute.
    pub path: String,
    pub kind: WatchKind,
}

/// A watch on a file or a folder. Dropping it stops watching.
pub struct Watcher {
    _inner: notify::RecommendedWatcher,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watcher").finish_non_exhaustive()
    }
}

/// Watches a file or a folder (with `recursive`, everything under it too), once it is
/// checked against `granted` (the patterns of `fs.read`). Each change is given to
/// `on_event`, on a thread of the watcher's own; a change to a path no grant covers, or
/// in Lume's own folders, is not.
pub fn watch(
    places: &Places,
    granted: &[String],
    path: &str,
    recursive: bool,
    on_event: Box<dyn Fn(WatchEvent) + Send + 'static>,
) -> Result<Allowed<Watcher>, ServiceError> {
    use notify::Watcher as _;

    let located = locate(places, granted, path, Access::Read)?;
    let policy = located.policy;
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let Ok(event) = result else {
            return;
        };
        for (changed, kind) in changes(&event) {
            if let Some(own) = own_path(changed)
                && Parts::of(&own).is_some_and(|parts| policy.grant_for(&parts).is_some())
            {
                on_event(WatchEvent {
                    path: own.display().to_string(),
                    kind,
                });
            }
        }
    })
    .map_err(|error| ServiceError::failed(format!("`{path}` cannot be watched: {error}")))?;
    let mode = if recursive {
        notify::RecursiveMode::Recursive
    } else {
        notify::RecursiveMode::NonRecursive
    };
    watcher
        .watch(&located.path, mode)
        .map_err(|error| match error.kind {
            notify::ErrorKind::PathNotFound => {
                ServiceError::not_found(format!("`{path}` does not exist"))
            }
            _ => ServiceError::failed(format!("`{path}` cannot be watched: {error}")),
        })?;
    Ok(Allowed {
        value: Watcher { _inner: watcher },
        grant: located.grant,
    })
}

/// The paths an event is about, each with what happened to it. A rename is the old name
/// removed and the new one made. Reading a file is no change and gives nothing.
fn changes(event: &notify::Event) -> Vec<(&Path, WatchKind)> {
    use notify::EventKind;
    use notify::event::{ModifyKind, RenameMode};

    let all = |kind| {
        event
            .paths
            .iter()
            .map(|path| (path.as_path(), kind))
            .collect()
    };
    match event.kind {
        EventKind::Access(_) => Vec::new(),
        EventKind::Create(_) => all(WatchKind::Created),
        EventKind::Remove(_) => all(WatchKind::Removed),
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => all(WatchKind::Removed),
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => all(WatchKind::Created),
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => event
            .paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let kind = if index == 0 {
                    WatchKind::Removed
                } else {
                    WatchKind::Created
                };
                (path.as_path(), kind)
            })
            .collect(),
        EventKind::Modify(_) => all(WatchKind::Changed),
        EventKind::Any | EventKind::Other => all(WatchKind::Other),
    }
}

/// The path of an entry itself, not of what a link there leads to: its folder made
/// canonical and its name. A path whose folder is gone too is taken as it is.
fn own_path(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?;
    match path.parent().map(paths::canonical) {
        Some(Ok(folder)) => Some(folder.join(name)),
        _ => Some(paths::strip_verbatim(path.to_path_buf())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_refuses_names_it_would_change() {
        for name in ["CON", "nul.txt", "COM1", "a.", "a ", "a:stream", "a?"] {
            assert_eq!(check_new_name(name, name).is_err(), cfg!(windows), "{name}");
        }
        for name in ["console.txt", "COM", "COMX", "a.b", "notes"] {
            assert!(check_new_name(name, name).is_ok(), "{name}");
        }
    }

    #[test]
    fn a_rename_is_a_removal_and_a_creation() {
        use notify::event::{EventKind, ModifyKind, RenameMode};
        let event = notify::Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
            .add_path(PathBuf::from("old"))
            .add_path(PathBuf::from("new"));
        let kinds: Vec<WatchKind> = changes(&event).into_iter().map(|(_, kind)| kind).collect();
        assert_eq!(kinds, [WatchKind::Removed, WatchKind::Created]);
        let read = notify::Event::new(EventKind::Access(notify::event::AccessKind::Any))
            .add_path(PathBuf::from("file"));
        assert!(changes(&read).is_empty());
    }
}
