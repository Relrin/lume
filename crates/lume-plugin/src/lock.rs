//! The lock file: what is installed, where from, what it was found to be, and what it
//! may do.
//!
//! It is the record the application trusts. An install is used only as the lock file
//! describes it: from its folder, with its hash, with the permissions the user granted.
//! It is written in one step, so it is always either as it was or as it is meant to be.
//!
//! ```toml
//! schema = 1
//! order = ["acme.metrics", "acme.git"]
//!
//! [[plugin]]
//! local = "acme.metrics"
//! manifest-id = "acme.metrics"
//! version = "1.2.0"
//! dir = "packages/acme.metrics/1.2.0"
//! source = { kind = "folder", path = "D:\\src\\metrics" }
//! content-hash = "sha256:…"
//! manifest-hash = "sha256:…"
//! enabled = true
//!
//! [plugin.grants]
//! "system.metrics" = true
//!
//! [plugin.bindings]
//! "acme.core" = "acme.core-fork"
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The version of the lock file's format this build reads and writes.
pub const LOCK_SCHEMA: u32 = 1;

/// The lock file's name in the plugins folder.
pub const LOCK_FILE: &str = "lock.toml";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Lock {
    pub schema: u32,
    /// The order the user wants plugins in where nothing else decides it, by the ids
    /// they are installed under.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
    #[serde(default, rename = "plugin", skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<Entry>,
}

/// One install.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Entry {
    /// The id it is installed under: its namespace everywhere.
    pub local: String,
    /// The id its manifest gives it.
    pub manifest_id: String,
    /// What the user calls it, where they have named it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub version: String,
    /// Its folder, inside the plugins folder, written with `/`.
    pub dir: String,
    pub source: Source,
    pub content_hash: String,
    pub manifest_hash: String,
    pub enabled: bool,
    /// What it was allowed, as a manifest writes permissions.
    #[serde(default, skip_serializing_if = "toml::Table::is_empty")]
    pub grants: toml::Table,
    /// For each plugin it depends on, by the id its manifest names, the install it uses
    /// where that is not the one installed under the same id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bindings: BTreeMap<String, String>,
    /// For each plugin whose services it may use, by the id its manifest names, the
    /// install the user agreed to its using. Bound to another install, it is asked
    /// again.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub services: BTreeMap<String, String>,
}

/// Where a package was installed from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Source {
    /// `folder`, `archive`, `git` or `link`.
    pub kind: String,
    /// Where it came from, as the user is shown it: the folder or archive, or the
    /// repository's address.
    pub path: String,
    /// For `git`: the repository's address as git was given it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// For `git`: the branch or tag that is followed; none is the repository's own
    /// default branch.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// For `git`: the commit the installed package was taken from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// For `git`: the folder of the repository the package is in, where that is not
    /// its top.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subdir: Option<String>,
}

/// The kind of a source that is a repository fetched with git.
pub const SOURCE_GIT: &str = "git";

/// The kind of a source that is a folder used where it lies, for a plugin being
/// worked on.
pub const SOURCE_LINK: &str = "link";

impl Source {
    /// A folder or an archive on this machine.
    pub fn local(kind: &str, path: String) -> Self {
        Self {
            kind: kind.to_owned(),
            path,
            url: None,
            reference: None,
            commit: None,
            subdir: None,
        }
    }

    pub fn is_link(&self) -> bool {
        self.kind == SOURCE_LINK
    }

    pub fn is_git(&self) -> bool {
        self.kind == SOURCE_GIT
    }
}

impl Entry {
    /// Whether the install is a folder used where it lies.
    pub fn is_link(&self) -> bool {
        self.source.is_link()
    }
}

/// Why a lock file cannot be used.
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("{path}: {error}")]
    Io { path: String, error: std::io::Error },
    #[error("{path} cannot be read: {message}")]
    Unreadable { path: String, message: String },
    #[error("{path} is of schema {found}, which this Lume does not read")]
    Schema { path: String, found: u32 },
}

impl Lock {
    pub fn new() -> Self {
        Self {
            schema: LOCK_SCHEMA,
            ..Self::default()
        }
    }

    /// Reads the lock file at `path`; none there is nothing installed.
    pub fn read(path: &Path) -> Result<Self, LockError> {
        let shown = path.display().to_string();
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(error) => return Err(LockError::Io { path: shown, error }),
        };
        let lock: Self = toml::from_str(&text).map_err(|error| LockError::Unreadable {
            path: shown.clone(),
            message: error.message().trim().to_owned(),
        })?;
        if lock.schema != LOCK_SCHEMA {
            return Err(LockError::Schema {
                path: shown,
                found: lock.schema,
            });
        }
        Ok(lock)
    }

    /// The lock file's text.
    pub fn text(&self) -> String {
        let body = toml::to_string_pretty(self).unwrap_or_default();
        format!("# What is installed. Written by Lume; change it with `lume plugin`.\n{body}")
    }

    /// Writes the lock file in one step.
    pub fn write(&self, path: &Path) -> Result<(), LockError> {
        lume_config::write::write_atomic(path, &self.text()).map_err(|error| LockError::Io {
            path: path.display().to_string(),
            error,
        })
    }

    pub fn get(&self, local: &str) -> Option<&Entry> {
        self.plugins.iter().find(|entry| entry.local == local)
    }

    pub fn get_mut(&mut self, local: &str) -> Option<&mut Entry> {
        self.plugins.iter_mut().find(|entry| entry.local == local)
    }

    /// Puts an install in, in place of one under the same id, and keeps the order list
    /// holding every install once.
    pub fn put(&mut self, entry: Entry) {
        match self.get_mut(&entry.local) {
            Some(old) => *old = entry,
            None => {
                if !self.order.contains(&entry.local) {
                    self.order.push(entry.local.clone());
                }
                self.plugins.push(entry);
            }
        }
        self.plugins.sort_by(|a, b| a.local.cmp(&b.local));
    }

    /// Takes an install out, and its place in the order.
    pub fn take(&mut self, local: &str) -> Option<Entry> {
        let index = self.plugins.iter().position(|entry| entry.local == local)?;
        self.order.retain(|other| other != local);
        Some(self.plugins.remove(index))
    }

    /// The order list as it is to be used: every install once, those the list leaves out
    /// after the rest, by id.
    pub fn full_order(&self) -> Vec<String> {
        let mut order: Vec<String> = Vec::new();
        for local in &self.order {
            if self.get(local).is_some() && !order.contains(local) {
                order.push(local.clone());
            }
        }
        for entry in &self.plugins {
            if !order.contains(&entry.local) {
                order.push(entry.local.clone());
            }
        }
        order
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testing::TempDir;

    fn entry(local: &str) -> Entry {
        Entry {
            local: local.to_owned(),
            manifest_id: local.to_owned(),
            name: None,
            version: "1.0.0".to_owned(),
            dir: format!("packages/{local}/1.0.0"),
            source: Source::local("folder", r"D:\src\x".to_owned()),
            content_hash: "sha256:00".to_owned(),
            manifest_hash: "sha256:11".to_owned(),
            enabled: true,
            grants: "\"system.metrics\" = true".parse().unwrap(),
            services: BTreeMap::new(),
            bindings: BTreeMap::from([("acme.core".to_owned(), "acme.fork".to_owned())]),
        }
    }

    #[test]
    fn the_lock_file_is_read_back_as_it_was_written() {
        let dir = TempDir::new("lock");
        let path = dir.path().join(LOCK_FILE);
        assert_eq!(Lock::read(&path).unwrap(), Lock::new());

        let mut lock = Lock::new();
        lock.put(entry("b.plugin"));
        lock.put(entry("a.plugin"));
        lock.write(&path).unwrap();
        let read = Lock::read(&path).unwrap();
        assert_eq!(read, lock);
        assert_eq!(read.order, ["b.plugin", "a.plugin"]);
        assert_eq!(read.plugins[0].local, "a.plugin");
    }

    #[test]
    fn the_order_holds_every_install_once() {
        let mut lock = Lock::new();
        lock.put(entry("a"));
        lock.put(entry("b"));
        lock.order = vec!["b".to_owned(), "gone".to_owned(), "b".to_owned()];
        lock.put(entry("c"));
        assert_eq!(lock.full_order(), ["b", "c", "a"]);
        lock.take("b");
        assert_eq!(lock.full_order(), ["c", "a"]);
    }

    #[test]
    fn a_lock_file_of_another_schema_is_refused() {
        let dir = TempDir::new("lock-schema");
        let path = dir.write(LOCK_FILE, "schema = 2\n");
        assert!(matches!(
            Lock::read(&path),
            Err(LockError::Schema { found: 2, .. })
        ));
    }
}
