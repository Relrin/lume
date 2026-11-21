//! Keeping workspaces between runs.
//!
//! Each workspace is one JSON file, named after the workspace, in a directory of the
//! application's own. A file is written whole to a file beside it and then moved into
//! place, so a crash while writing leaves the last good file where it was.
//!
//! A file that cannot be read as a workspace is never fatal and never deleted: it is
//! moved aside under another name, and whoever asked is told, so that the application
//! starts with a fresh workspace and the user still has the file.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use lume_proto::SessionId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::Workspace;
use crate::tuning::{CORRUPT_KEPT, WORKSPACE_FILE_VERSION, WORKSPACE_NAME_MAX_CHARS};
use crate::validate::{is_normalised, validate};

/// What is in a workspace's file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFile {
    /// The version of this format. A reader migrates older files and sets newer ones
    /// aside.
    pub version: u32,
    /// The version of the application that wrote the file. Recorded only.
    pub app_version: String,
    /// When the file was written, in milliseconds since the Unix epoch.
    pub saved_unix_ms: u64,
    pub workspace: Workspace,
}

/// Brings a file's contents from one version of the format to the next.
type Migration = fn(Value) -> Result<Value, String>;

/// The migrations, each with the version it starts from. Versions 2 and 3 only added
/// fields that older files are read without.
const MIGRATIONS: &[(u32, Migration)] = &[(1, Ok), (2, Ok)];

/// What reading a workspace's file gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Loaded {
    /// The workspace, with the sessions its panes showed when it was written. Those
    /// may have ended since; whoever shows the workspace finds out which.
    Workspace(Box<Workspace>),
    /// There is no such file.
    Missing,
    /// The file could not be used. It is now at `to`.
    SetAside { to: PathBuf, reason: String },
}

/// Why something could not be done with a workspace's file.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("`{0}` cannot be a workspace's name")]
    Name(String),
    #[error("there is already a workspace called `{0}`")]
    Exists(String),
    #[error("there is no workspace called `{0}`")]
    Missing(String),
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the workspace could not be written as JSON: {0}")]
    Encode(#[from] serde_json::Error),
}

/// Names Windows keeps for devices. A file cannot be called by one, with or without
/// an extension.
const DEVICE_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Whether text can be a workspace's name. A name becomes a file name on every
/// platform, so it is plain: it begins with a letter or digit, goes on with letters,
/// digits, spaces, dots, underscores and hyphens, does not end with a dot or a space,
/// and is not a name Windows keeps for a device.
pub fn valid_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    let plain = first.is_ascii_alphanumeric()
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ' ' | '.' | '_' | '-')
        });
    let stem = name.split('.').next().unwrap_or(name).to_ascii_lowercase();
    plain
        && name.chars().count() <= WORKSPACE_NAME_MAX_CHARS
        && !name.ends_with(['.', ' '])
        && !DEVICE_NAMES.contains(&stem.as_str())
}

const EXTENSION: &str = ".json";

/// The part of a set-aside file's name that follows the workspace's file name.
const CORRUPT_MARK: &str = ".corrupt-";

/// The directory workspaces are kept in.
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// A store in `dir`. The directory is made when something is first saved.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn io(path: &Path) -> impl FnOnce(io::Error) -> StoreError + '_ {
        move |source| StoreError::Io {
            path: path.to_owned(),
            source,
        }
    }

    /// The file a workspace of this name is kept in.
    pub fn path(&self, name: &str) -> Result<PathBuf, StoreError> {
        if !valid_name(name) {
            return Err(StoreError::Name(name.to_owned()));
        }
        Ok(self.dir.join(format!("{name}{EXTENSION}")))
    }

    /// The names of the workspaces kept here, in alphabetical order.
    pub fn list(&self) -> Result<Vec<String>, StoreError> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(Self::io(&self.dir)(error)),
        };
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(Self::io(&self.dir))?;
            let file = entry.file_name();
            let Some(name) = file.to_str().and_then(|file| file.strip_suffix(EXTENSION)) else {
                continue;
            };
            if valid_name(name) {
                names.push(name.to_owned());
            }
        }
        names.sort_by_key(|name| name.to_ascii_lowercase());
        Ok(names)
    }

    /// The name a kept workspace has that differs from `name` at most in the case of
    /// its letters. Some file systems tell such names apart and some do not, so two
    /// workspaces may not differ in that alone.
    fn taken(&self, name: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .list()?
            .into_iter()
            .find(|kept| kept.eq_ignore_ascii_case(name)))
    }

    /// Reads a workspace. `now_unix_ms` goes into the name of a file that has to be
    /// set aside.
    pub fn load(&self, name: &str, now_unix_ms: u64) -> Result<Loaded, StoreError> {
        let path = self.path(name)?;
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Loaded::Missing),
            // Not text at all is as unreadable as text that is not a workspace.
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                return self.set_aside(&path, name, now_unix_ms, "it is not text".to_owned());
            }
            Err(error) => return Err(Self::io(&path)(error)),
        };
        match read(&text, name, MIGRATIONS, WORKSPACE_FILE_VERSION) {
            Ok(workspace) => Ok(Loaded::Workspace(Box::new(workspace))),
            Err(Unreadable::Newer(version)) => {
                let to = self.dir.join(format!("{name}.v{version}{EXTENSION}.bak"));
                fs::rename(&path, &to).map_err(Self::io(&path))?;
                Ok(Loaded::SetAside {
                    to,
                    reason: format!(
                        "it was written by a newer version of the application \
                         (format {version}; this one reads up to {WORKSPACE_FILE_VERSION})"
                    ),
                })
            }
            Err(Unreadable::Corrupt(reason)) => self.set_aside(&path, name, now_unix_ms, reason),
        }
    }

    /// The sessions the panes of a workspace's file show, without setting aside a file
    /// that cannot be read: one that cannot be read shows none.
    pub fn sessions_in(&self, name: &str) -> Result<Vec<SessionId>, StoreError> {
        let path = self.path(name)?;
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) if error.kind() == io::ErrorKind::InvalidData => return Ok(Vec::new()),
            Err(error) => return Err(Self::io(&path)(error)),
        };
        Ok(read(&text, name, MIGRATIONS, WORKSPACE_FILE_VERSION)
            .map(|workspace| {
                workspace
                    .sessions()
                    .into_iter()
                    .map(|(_, session)| session)
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Moves a file that cannot be used out of the way, and removes the oldest of the
    /// files set aside before it when there are more than are kept.
    fn set_aside(
        &self,
        path: &Path,
        name: &str,
        now_unix_ms: u64,
        reason: String,
    ) -> Result<Loaded, StoreError> {
        let prefix = format!("{name}{EXTENSION}{CORRUPT_MARK}");
        let to = self.dir.join(format!("{prefix}{now_unix_ms}"));
        fs::rename(path, &to).map_err(Self::io(path))?;

        let mut aside: Vec<(u64, PathBuf)> = fs::read_dir(&self.dir)
            .map_err(Self::io(&self.dir))?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let file = entry.file_name();
                let when = file.to_str()?.strip_prefix(&prefix)?.parse().ok()?;
                Some((when, entry.path()))
            })
            .collect();
        aside.sort_by_key(|(when, _)| std::cmp::Reverse(*when));
        for (_, old) in aside.into_iter().skip(CORRUPT_KEPT) {
            // Best effort: a file that will not go is one more kept, nothing worse.
            let _ = fs::remove_file(old);
        }
        Ok(Loaded::SetAside { to, reason })
    }

    /// Writes a workspace to its file, replacing what was there.
    pub fn save(
        &self,
        workspace: &Workspace,
        app_version: &str,
        now_unix_ms: u64,
    ) -> Result<(), StoreError> {
        let path = self.path(&workspace.name)?;
        let file = WorkspaceFile {
            version: WORKSPACE_FILE_VERSION,
            app_version: app_version.to_owned(),
            saved_unix_ms: now_unix_ms,
            workspace: workspace.clone(),
        };
        let mut text = serde_json::to_string_pretty(&file)?;
        text.push('\n');

        fs::create_dir_all(&self.dir).map_err(Self::io(&self.dir))?;
        // Written beside the file and moved over it: the file is either the old one or
        // the new one, never part of each.
        let beside = self.dir.join(format!("{}{EXTENSION}.tmp", workspace.name));
        let written = (|| {
            let mut file = fs::File::create(&beside)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&beside, &path)
        })();
        written.map_err(|error| {
            let _ = fs::remove_file(&beside);
            Self::io(&path)(error)
        })
    }

    /// Renames a kept workspace's file. The name inside the file follows the next time
    /// it is read.
    pub fn rename(&self, from: &str, to: &str) -> Result<(), StoreError> {
        let old = self.path(from)?;
        let new = self.path(to)?;
        if !old.is_file() {
            return Err(StoreError::Missing(from.to_owned()));
        }
        // Changing only the case of a workspace's own name is allowed.
        if let Some(kept) = self.taken(to)?
            && !kept.eq_ignore_ascii_case(from)
        {
            return Err(StoreError::Exists(kept));
        }
        fs::rename(&old, &new).map_err(Self::io(&old))
    }

    /// Removes a kept workspace's file.
    pub fn delete(&self, name: &str) -> Result<(), StoreError> {
        let path = self.path(name)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Err(StoreError::Missing(name.to_owned()))
            }
            Err(error) => Err(Self::io(&path)(error)),
        }
    }

    /// Whether a workspace of this name, or one differing only in the case of its
    /// letters, is kept here.
    pub fn contains(&self, name: &str) -> Result<bool, StoreError> {
        Ok(self.taken(name)?.is_some())
    }
}

/// Why a file's text is not a workspace this version can use.
#[derive(Debug, PartialEq, Eq)]
enum Unreadable {
    /// The file is of a later version of the format.
    Newer(u64),
    Corrupt(String),
}

/// Reads a file's text as the workspace called `name`.
fn read(
    text: &str,
    name: &str,
    migrations: &[(u32, Migration)],
    current: u32,
) -> Result<Workspace, Unreadable> {
    let corrupt = |reason: String| Unreadable::Corrupt(reason);
    let value: Value =
        serde_json::from_str(text).map_err(|error| corrupt(format!("it is not JSON: {error}")))?;
    let version = value
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| corrupt("it does not say which version of the format it is".to_owned()))?;
    if version > u64::from(current) {
        return Err(Unreadable::Newer(version));
    }
    let value = migrate(value, version as u32, migrations, current).map_err(corrupt)?;
    let file: WorkspaceFile = serde_json::from_value(value)
        .map_err(|error| corrupt(format!("it does not describe a workspace: {error}")))?;

    let mut workspace = file.workspace;
    // The file is named after the workspace, and a file that was renamed or copied by
    // hand is the workspace its new name says.
    workspace.name = name.to_owned();
    if let Err(violations) = validate(&workspace) {
        let first = violations
            .first()
            .map(ToString::to_string)
            .unwrap_or_default();
        return Err(corrupt(format!("its layout is not sound: {first}")));
    }
    if !is_normalised(&workspace) {
        return Err(corrupt("its layout is not in its proper form".to_owned()));
    }
    Ok(workspace)
}

/// Brings a file's contents from the version it was written in to the current one,
/// one version at a time.
fn migrate(
    mut value: Value,
    from: u32,
    migrations: &[(u32, Migration)],
    current: u32,
) -> Result<Value, String> {
    for version in from..current {
        let (_, step) = migrations
            .iter()
            .find(|(starts, _)| *starts == version)
            .ok_or_else(|| format!("nothing brings format {version} up to date"))?;
        value = step(value)?;
        if let Some(object) = value.as_object_mut() {
            object.insert("version".to_owned(), Value::from(version + 1));
        }
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_plain_enough_to_be_a_file_name_anywhere() {
        for name in ["default", "Work 2", "api.v2", "a", "notes_old-1", "console"] {
            assert!(valid_name(name), "{name:?} was refused");
        }
        let long = "x".repeat(WORKSPACE_NAME_MAX_CHARS + 1);
        for name in [
            "",
            " lead",
            ".hidden",
            "trail ",
            "trail.",
            "a/b",
            "a\\b",
            "a:b",
            "caf\u{e9}",
            "con",
            "NUL",
            "com1.txt",
            "Lpt9",
            "..",
            &long,
        ] {
            assert!(!valid_name(name), "{name:?} was taken");
        }
        assert!(valid_name(&"x".repeat(WORKSPACE_NAME_MAX_CHARS)));
    }

    fn rename_field(mut value: Value) -> Result<Value, String> {
        let object = value.as_object_mut().ok_or("the file is not an object")?;
        let workspace = object.remove("space").ok_or("the file has no `space`")?;
        object.insert("workspace".to_owned(), workspace);
        Ok(value)
    }

    fn fail(_: Value) -> Result<Value, String> {
        Err("this step cannot be done".to_owned())
    }

    #[test]
    fn an_older_file_is_brought_up_to_date_a_version_at_a_time() {
        let old = serde_json::json!({ "version": 1, "space": { "kept": true } });
        let steps: &[(u32, Migration)] = &[(1, rename_field), (2, Ok)];
        let new = migrate(old, 1, steps, 3).unwrap();
        assert_eq!(
            new,
            serde_json::json!({ "version": 3, "workspace": { "kept": true } })
        );
    }

    #[test]
    fn a_file_no_step_starts_from_cannot_be_brought_up_to_date() {
        let old = serde_json::json!({ "version": 1 });
        assert!(migrate(old.clone(), 1, &[], 2).is_err());
        assert!(migrate(old.clone(), 1, &[(1, fail)], 2).is_err());
        // A file of the current version needs no steps.
        assert_eq!(migrate(old.clone(), 1, &[], 1), Ok(old));
    }

    #[test]
    fn a_file_from_a_later_version_is_told_apart_from_a_broken_one() {
        let later = r#"{ "version": 9, "anything": [] }"#;
        assert_eq!(read(later, "w", &[], 1), Err(Unreadable::Newer(9)));
        for broken in [
            "",
            "{",
            "[]",
            r#"{ "version": "1" }"#,
            r#"{ "version": 1 }"#,
        ] {
            assert!(
                matches!(read(broken, "w", &[], 1), Err(Unreadable::Corrupt(_))),
                "{broken:?} was read"
            );
        }
    }
}
