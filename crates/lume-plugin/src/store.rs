//! The plugins folder: what is installed there, and installing, removing and changing
//! installs.
//!
//! ```text
//! <state>/plugins/
//!   lock.toml                         what is installed (see `lock`)
//!   packages/<id>/<version>/          each install's copy of its package
//!   data/<id>/                        each plugin's own data
//!   logs/<id>.log                     what each plugin's code wrote to its log
//!   audit.log                         what plugins were allowed and refused
//!   staging/                          packages being installed
//! ```
//!
//! Installing is done in two steps, so the user can be shown what the plugin may do
//! before anything is in place: [`Store::prepare`] copies the package into the staging
//! folder, reads and checks it and works out what is new; [`Store::commit`] puts it in
//! place and writes the lock file. A prepared install that is dropped leaves nothing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use crate::hash::{self, HashError};
use crate::id::{check_plugin_id, overlap};
use crate::lock::{Entry, LOCK_FILE, Lock, LockError, Source};
use crate::manifest::{MANIFEST_FILE, Manifest, Problem};
use crate::order::{Candidate, Resolution, resolve};
use crate::package::{self, PackageError, Staged};
use crate::permissions::{self, Item, Kind, Permissions, Request};

/// The most an SVG picture in a package may hold.
pub const ICON_MAX_BYTES: u64 = crate::assets::SVG_MAX_BYTES;

/// How long a change waits for another one to finish before giving up, and how old a
/// mark of a change in progress must be to be taken as left by a program that stopped.
const BUSY_WAIT: Duration = Duration::from_secs(5);
const BUSY_STALE: Duration = Duration::from_secs(30);

/// The file that marks a change to the lock file in progress.
const BUSY_FILE: &str = "lock.busy";

static STAGING: AtomicU32 = AtomicU32::new(1);

/// Why a change to the plugins could not be made.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error(transparent)]
    Package(#[from] PackageError),
    #[error(transparent)]
    Hash(#[from] HashError),
    #[error("{path}: {error}")]
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
    #[error("the manifest has problems:\n{}", list(.0))]
    Manifest(Vec<Problem>),
    #[error("the package has problems:\n{}", .0.iter().map(|p| format!("  {p}")).collect::<Vec<_>>().join("\n"))]
    Contents(Vec<String>),
    #[error("{0}")]
    BadId(String),
    #[error(
        "{local} is installed already, from {existing}; this package is another plugin of the \
         same id. Install it beside the other under a name of its own, with --as <id>"
    )]
    Taken { local: String, existing: String },
    #[error(
        "{local} cannot be installed beside {other}: one id is inside the other, so their \
         commands and settings could take each other's names. Choose another with --as <id>"
    )]
    Overlap { local: String, other: String },
    #[error("no plugin is installed as {0}")]
    NotInstalled(String),
    #[error("{0}")]
    Refused(String),
    #[error("another change to the plugins is under way; try again in a moment")]
    Busy,
}

fn list(problems: &[Problem]) -> String {
    problems
        .iter()
        .map(|problem| format!("  {problem}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> StoreError + '_ {
    move |error| StoreError::Io {
        path: path.to_owned(),
        error,
    }
}

/// What to install a package as.
#[derive(Clone, Debug, Default)]
pub struct InstallOptions {
    /// The id to install it under, in place of its manifest's.
    pub local: Option<String>,
    /// The name to show for it, in place of its manifest's.
    pub name: Option<String>,
}

/// What installing a package would do to what is installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// Nothing is installed under its id.
    New,
    /// The same plugin is installed at another version, or the same one.
    Replace { from: String, direction: Direction },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Newer,
    Same,
    Older,
}

/// A package copied and checked, waiting for the user's word.
#[derive(Debug)]
pub struct Prepared {
    pub manifest: Manifest,
    /// The id it will be installed under.
    pub local: String,
    pub name: Option<String>,
    pub change: Change,
    /// What the user must agree to: every permission for a new install, those it did
    /// not have for a replacement.
    pub new_permissions: Vec<Item>,
    /// The permissions it will have.
    pub grants: Permissions,
    /// Folders of the source that were not copied.
    pub left_out: Vec<String>,
    /// Things the user should know before agreeing.
    pub warnings: Vec<String>,
    pub source: Source,
    pub content_hash: String,
    manifest_hash: String,
    /// The bindings the install it replaces had.
    bindings: std::collections::BTreeMap<String, String>,
    enabled: bool,
    staged: Option<Staged>,
}

impl Drop for Prepared {
    fn drop(&mut self) {
        if let Some(staged) = self.staged.take() {
            clear_staging(&staged);
        }
    }
}

/// Removes a staged copy, and the staging folder with it when nothing else is being
/// installed.
fn clear_staging(staged: &Staged) {
    let _ = std::fs::remove_dir_all(&staged.folder);
    if let Some(staging) = staged.folder.parent() {
        // Fails, and is meant to, while another install is staged there.
        let _ = std::fs::remove_dir(staging);
    }
}

/// Whether an installed package is as it was installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Check {
    Intact,
    /// Its files are not the ones installed.
    Modified,
    /// Its folder is gone.
    Missing,
    /// It could not be looked at.
    Unreadable(String),
    /// Not looked at.
    Unchecked,
}

/// One install, as the plugins folder has it.
#[derive(Clone, Debug)]
pub struct Installed {
    pub entry: Entry,
    pub manifest: Result<Manifest, Vec<Problem>>,
    pub check: Check,
}

impl Installed {
    /// What it is called: the user's name for it, or its manifest's.
    pub fn name(&self) -> String {
        self.entry
            .name
            .clone()
            .or_else(|| self.manifest.as_ref().ok().map(|m| m.name.clone()))
            .unwrap_or_else(|| self.entry.local.clone())
    }

    /// Why it cannot be used whatever else is installed, if it cannot.
    pub fn unusable(&self) -> Option<String> {
        if !self.entry.enabled {
            return Some("it is turned off".to_owned());
        }
        match &self.check {
            Check::Modified => Some(
                "its files have changed since it was installed; install it again to use it"
                    .to_owned(),
            ),
            Check::Missing => Some("its folder is gone; install it again".to_owned()),
            Check::Unreadable(why) => Some(format!("its files cannot be read: {why}")),
            Check::Intact | Check::Unchecked => match &self.manifest {
                Ok(_) => None,
                Err(problems) => Some(format!("its manifest has problems: {}", problems[0])),
            },
        }
    }
}

/// Where in the user's order to move a plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Position {
    First,
    Last,
    Before(String),
    After(String),
}

/// The plugins folder.
#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

/// A change to the lock file in progress; others wait for it.
struct Busy(PathBuf);

impl Drop for Busy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The plugins folder of the user's state directory.
    pub fn of(dirs: &lume_config::Dirs) -> Self {
        Self::new(dirs.plugins_dir())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn lock_file(&self) -> PathBuf {
        self.root.join(LOCK_FILE)
    }

    /// The folder of a plugin's own data: `${plugin-data}`.
    pub fn data_dir(&self, local: &str) -> PathBuf {
        self.root.join("data").join(local)
    }

    /// The file a plugin's code logs to.
    pub fn log_file(&self, local: &str) -> PathBuf {
        self.root.join("logs").join(format!("{local}.log"))
    }

    /// The record of what plugins were allowed and refused.
    pub fn audit_file(&self) -> PathBuf {
        self.root.join("audit.log")
    }

    /// The folder an install's package is in.
    pub fn package_dir(&self, entry: &Entry) -> PathBuf {
        self.root.join(&entry.dir)
    }

    pub fn lock(&self) -> Result<Lock, StoreError> {
        Ok(Lock::read(&self.lock_file())?)
    }

    /// Waits for any other change to finish and marks one in progress.
    fn busy(&self) -> Result<Busy, StoreError> {
        std::fs::create_dir_all(&self.root).map_err(io(&self.root))?;
        let path = self.root.join(BUSY_FILE);
        let started = std::time::Instant::now();
        loop {
            match std::fs::File::create_new(&path) {
                Ok(_) => return Ok(Busy(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&path)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|time| SystemTime::now().duration_since(time).ok())
                        .is_some_and(|age| age > BUSY_STALE);
                    if stale {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if started.elapsed() > BUSY_WAIT {
                        return Err(StoreError::Busy);
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(error) => return Err(StoreError::Io { path, error }),
            }
        }
    }

    /// Changes the lock file, with no other change in between.
    fn change<T>(
        &self,
        change: impl FnOnce(&mut Lock) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let _busy = self.busy()?;
        let mut lock = self.lock()?;
        let result = change(&mut lock)?;
        lock.write(&self.lock_file())?;
        Ok(result)
    }

    /// Reads the manifest in a package folder.
    fn read_manifest(folder: &Path) -> Result<(Manifest, String), StoreError> {
        let path = folder.join(MANIFEST_FILE);
        let text = std::fs::read_to_string(&path).map_err(io(&path))?;
        let manifest = Manifest::parse(&text).map_err(StoreError::Manifest)?;
        Ok((manifest, hash::of_bytes(text.as_bytes())))
    }

    /// Copies the package at `source`, checks it, and works out what installing it would
    /// change. Nothing is installed until [`Store::commit`].
    pub fn prepare(&self, source: &Path, options: InstallOptions) -> Result<Prepared, StoreError> {
        let source = std::path::absolute(source).map_err(io(source))?;
        let staging = self.root.join("staging").join(format!(
            "{}-{}",
            std::process::id(),
            STAGING.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&staging);
        let staged = match package::stage(&source, &staging) {
            Ok(staged) => staged,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(error.into());
            }
        };
        let root = staged.root.clone();
        // From here a failure drops the prepared install, which removes the copy.
        let mut prepared = Prepared {
            manifest: placeholder(),
            local: String::new(),
            name: options.name.clone(),
            change: Change::New,
            new_permissions: Vec::new(),
            grants: Permissions::default(),
            left_out: staged.left_out.clone(),
            warnings: Vec::new(),
            source: Source {
                kind: if package::is_archive(&source) {
                    "archive"
                } else {
                    "folder"
                }
                .to_owned(),
                path: source.display().to_string(),
            },
            content_hash: String::new(),
            manifest_hash: String::new(),
            bindings: std::collections::BTreeMap::new(),
            enabled: true,
            staged: Some(staged),
        };
        let (manifest, manifest_hash) = Self::read_manifest(&root)?;
        check_contents(&root, &manifest)?;
        prepared.content_hash = hash::of_package(&root)?;
        prepared.manifest_hash = manifest_hash;

        let local = options.local.clone().unwrap_or_else(|| manifest.id.clone());
        check_plugin_id(&local).map_err(StoreError::BadId)?;
        if let Some(name) = &options.name
            && (name.trim().is_empty()
                || name.chars().count() > 64
                || name.chars().any(char::is_control))
        {
            return Err(StoreError::Refused(
                "a plugin's name is one line of 64 characters at most".to_owned(),
            ));
        }
        let lock = self.lock()?;
        if let Some(other) = lock
            .plugins
            .iter()
            .find(|entry| entry.local != local && overlap(&entry.local, &local))
        {
            return Err(StoreError::Overlap {
                local,
                other: other.local.clone(),
            });
        }

        let wanted = manifest.requested.required.clone();
        match lock.get(&local) {
            None => {
                prepared.new_permissions = permissions::items(&wanted).into_iter().collect();
                prepared.grants = Permissions {
                    required: wanted,
                    optional: Vec::new(),
                };
            }
            Some(existing) => {
                let installed = Self::read_manifest(&self.package_dir(existing)).ok();
                let same_plugin = existing.manifest_id == manifest.id
                    && match &installed {
                        Some((old, _)) => match (&old.repository, &manifest.repository) {
                            (Some(old), Some(new)) => same_repository(old, new),
                            (None, None) => old.name == manifest.name,
                            _ => false,
                        },
                        None => true,
                    };
                if !same_plugin {
                    let existing_from = installed
                        .as_ref()
                        .and_then(|(old, _)| old.repository.clone())
                        .unwrap_or_else(|| existing.source.path.clone());
                    return Err(StoreError::Taken {
                        local,
                        existing: existing_from,
                    });
                }
                if installed.is_none() {
                    prepared.warnings.push(format!(
                        "the installed {local} cannot be read, so whether this is the same \
                         plugin is taken from its id alone"
                    ));
                }
                let granted = permissions::parse_table(&existing.grants)
                    .unwrap_or_default()
                    .required;
                prepared.new_permissions = permissions::new_items(&granted, &wanted);
                // What the user granted while it ran stays, where the new version may
                // still ask for it.
                let kept: Vec<Request> = granted
                    .into_iter()
                    .filter(|old| {
                        !wanted.iter().any(|new| new.kind == old.kind)
                            && manifest
                                .requested
                                .optional
                                .iter()
                                .any(|o| o.kind == old.kind)
                    })
                    .collect();
                let mut required = wanted;
                required.extend(kept);
                prepared.grants = Permissions {
                    required,
                    optional: Vec::new(),
                };
                let old_version = semver::Version::parse(&existing.version).ok();
                let direction = match old_version.map(|old| manifest.version().cmp(&old)) {
                    Some(std::cmp::Ordering::Less) => Direction::Older,
                    Some(std::cmp::Ordering::Equal) => Direction::Same,
                    _ => Direction::Newer,
                };
                if direction == Direction::Older {
                    prepared.warnings.push(format!(
                        "{} is older than the installed {}",
                        manifest.version, existing.version
                    ));
                }
                prepared.change = Change::Replace {
                    from: existing.version.clone(),
                    direction,
                };
                prepared.bindings = existing.bindings.clone();
                prepared.enabled = existing.enabled;
                if prepared.name.is_none() {
                    prepared.name = existing.name.clone();
                }
            }
        }
        for request in &prepared.grants.required {
            if let Some(warning) = permissions::describe(request).warning
                && !prepared.warnings.contains(&warning)
            {
                prepared.warnings.push(warning);
            }
        }
        prepared.local = local;
        prepared.manifest = manifest;
        Ok(prepared)
    }

    /// Puts a prepared install in place and records it.
    pub fn commit(&self, mut prepared: Prepared) -> Result<Installed, StoreError> {
        let staged = prepared.staged.take().ok_or_else(|| {
            StoreError::Refused("the install was put in place already".to_owned())
        })?;
        let result = self.put_in_place(&prepared, &staged);
        clear_staging(&staged);
        result
    }

    fn put_in_place(&self, prepared: &Prepared, staged: &Staged) -> Result<Installed, StoreError> {
        let _busy = self.busy()?;
        let mut lock = self.lock()?;
        let version = prepared.manifest.version.clone();
        let relative = format!("packages/{}/{version}", prepared.local);
        let target = self.root.join(&relative);
        let parent = target.parent().unwrap_or(&self.root).to_owned();
        std::fs::create_dir_all(&parent).map_err(io(&parent))?;
        let set_aside = parent.join(format!("{version}.old"));
        let _ = std::fs::remove_dir_all(&set_aside);
        let had_same = target.exists();
        if had_same {
            std::fs::rename(&target, &set_aside).map_err(io(&target))?;
        }
        if let Err(error) = std::fs::rename(&staged.root, &target) {
            if had_same {
                let _ = std::fs::rename(&set_aside, &target);
            }
            return Err(StoreError::Io {
                path: target,
                error,
            });
        }
        let previous = lock.get(&prepared.local).cloned();
        let entry = Entry {
            local: prepared.local.clone(),
            manifest_id: prepared.manifest.id.clone(),
            name: prepared.name.clone(),
            version,
            dir: relative,
            source: prepared.source.clone(),
            content_hash: prepared.content_hash.clone(),
            manifest_hash: prepared.manifest_hash.clone(),
            enabled: prepared.enabled,
            grants: permissions::to_table(&prepared.grants),
            bindings: prepared.bindings.clone(),
            services: prepared
                .grants
                .required
                .iter()
                .filter(|request| request.kind == Kind::ServicesUse)
                .flat_map(|request| request.scope.iter())
                .map(|wanted| {
                    let bound = prepared.bindings.get(wanted).unwrap_or(wanted);
                    (wanted.clone(), bound.clone())
                })
                .collect(),
        };
        lock.put(entry.clone());
        if let Err(error) = lock.write(&self.lock_file()) {
            let _ = std::fs::remove_dir_all(&target);
            if had_same {
                let _ = std::fs::rename(&set_aside, &target);
            }
            return Err(error.into());
        }
        let _ = std::fs::remove_dir_all(&set_aside);
        if let Some(previous) = previous
            && previous.dir != entry.dir
        {
            let _ = std::fs::remove_dir_all(self.root.join(&previous.dir));
        }
        Ok(Installed {
            entry,
            manifest: Ok(prepared.manifest.clone()),
            check: Check::Intact,
        })
    }

    /// Removes an install. Its data and log go too when `purge` says so; otherwise
    /// they stay for an install under the same id.
    pub fn remove(&self, local: &str, purge: bool) -> Result<Entry, StoreError> {
        let entry = self.change(|lock| {
            lock.take(local)
                .ok_or_else(|| StoreError::NotInstalled(local.to_owned()))
        })?;
        let folder = self.root.join("packages").join(local);
        let _ = std::fs::remove_dir_all(&folder);
        if purge {
            let _ = std::fs::remove_dir_all(self.data_dir(local));
            let _ = std::fs::remove_file(self.log_file(local));
        }
        Ok(entry)
    }

    /// Turns an install on or off.
    pub fn set_enabled(&self, local: &str, enabled: bool) -> Result<(), StoreError> {
        self.change(|lock| {
            let entry = lock
                .get_mut(local)
                .ok_or_else(|| StoreError::NotInstalled(local.to_owned()))?;
            entry.enabled = enabled;
            Ok(())
        })
    }

    /// Adds a permission the user granted while the plugin ran to its grants. Its scope
    /// joins what the install held of that kind; being asked before each use, once
    /// granted so, stays.
    pub fn grant(&self, local: &str, request: &Request) -> Result<(), StoreError> {
        self.change(|lock| {
            let entry = lock
                .get_mut(local)
                .ok_or_else(|| StoreError::NotInstalled(local.to_owned()))?;
            let mut granted = permissions::parse_table(&entry.grants)
                .map_err(|problems| StoreError::Refused(problems.join("; ")))?;
            match granted
                .required
                .iter_mut()
                .find(|held| held.kind == request.kind)
            {
                Some(held) => {
                    let mut scope: std::collections::BTreeSet<String> =
                        held.scope.drain(..).collect();
                    scope.extend(request.scope.iter().cloned());
                    held.scope = scope.into_iter().collect();
                    held.prompt = held.prompt.max(request.prompt);
                }
                None => granted.required.push(request.clone()),
            }
            entry.grants = permissions::to_table(&granted);
            Ok(())
        })
    }

    /// Records that the user agreed to an install's using the services of `bound` for
    /// the plugin its manifest calls `wanted`.
    pub fn pin_service(&self, local: &str, wanted: &str, bound: &str) -> Result<(), StoreError> {
        self.change(|lock| {
            let entry = lock
                .get_mut(local)
                .ok_or_else(|| StoreError::NotInstalled(local.to_owned()))?;
            entry.services.insert(wanted.to_owned(), bound.to_owned());
            Ok(())
        })
    }

    /// Gives an install a name of the user's, or takes it away.
    pub fn rename(&self, local: &str, name: Option<String>) -> Result<(), StoreError> {
        if let Some(name) = &name
            && (name.trim().is_empty()
                || name.chars().count() > 64
                || name.chars().any(char::is_control))
        {
            return Err(StoreError::Refused(
                "a plugin's name is one line of 64 characters at most".to_owned(),
            ));
        }
        self.change(|lock| {
            let entry = lock
                .get_mut(local)
                .ok_or_else(|| StoreError::NotInstalled(local.to_owned()))?;
            entry.name = name;
            Ok(())
        })
    }

    /// Binds what a plugin depends on to another install, or back to the one of the same
    /// id when `target` is `None`.
    pub fn bind(&self, local: &str, wanted: &str, target: Option<&str>) -> Result<(), StoreError> {
        let dependencies = {
            let lock = self.lock()?;
            let entry = lock
                .get(local)
                .ok_or_else(|| StoreError::NotInstalled(local.to_owned()))?;
            let (manifest, _) = Self::read_manifest(&self.package_dir(entry))?;
            manifest.dependencies
        };
        if !dependencies.contains_key(wanted) {
            return Err(StoreError::Refused(format!(
                "{local} does not depend on {wanted}; it depends on {}",
                if dependencies.is_empty() {
                    "nothing".to_owned()
                } else {
                    dependencies.keys().cloned().collect::<Vec<_>>().join(", ")
                }
            )));
        }
        self.change(|lock| {
            if let Some(target) = target {
                if lock.get(target).is_none() {
                    return Err(StoreError::NotInstalled(target.to_owned()));
                }
                if target == local {
                    return Err(StoreError::Refused("a plugin cannot use itself".to_owned()));
                }
            }
            let entry = lock
                .get_mut(local)
                .ok_or_else(|| StoreError::NotInstalled(local.to_owned()))?;
            match target {
                Some(target) if target != wanted => {
                    entry.bindings.insert(wanted.to_owned(), target.to_owned());
                }
                _ => {
                    entry.bindings.remove(wanted);
                }
            }
            Ok(())
        })
    }

    /// Moves an install in the user's order.
    pub fn move_to(&self, local: &str, position: Position) -> Result<(), StoreError> {
        self.change(|lock| {
            if lock.get(local).is_none() {
                return Err(StoreError::NotInstalled(local.to_owned()));
            }
            let mut order = lock.full_order();
            order.retain(|other| other != local);
            let at = match &position {
                Position::First => 0,
                Position::Last => order.len(),
                Position::Before(other) | Position::After(other) => {
                    let found = order
                        .iter()
                        .position(|candidate| candidate == other)
                        .ok_or_else(|| StoreError::NotInstalled(other.clone()))?;
                    if matches!(position, Position::After(_)) {
                        found + 1
                    } else {
                        found
                    }
                }
            };
            order.insert(at, local.to_owned());
            lock.order = order;
            Ok(())
        })
    }

    /// Whether an install's package is as it was installed.
    pub fn verify(&self, entry: &Entry) -> Check {
        let folder = self.package_dir(entry);
        if !folder.is_dir() {
            return Check::Missing;
        }
        match hash::of_package(&folder) {
            Ok(found) if found == entry.content_hash => Check::Intact,
            Ok(_) | Err(HashError::NotAFile(_)) => Check::Modified,
            Err(error) => Check::Unreadable(error.to_string()),
        }
    }

    /// Whether an install's package is as it was installed, hashing it only when this
    /// run has not hashed it since its files were last touched. A package that cannot
    /// be looked at from outside is hashed, which says why.
    pub fn verify_seen(&self, entry: &Entry, seen: &Seen) -> Check {
        let folder = self.package_dir(entry);
        let look = hash::fingerprint(&folder).ok();
        if let Some(look) = &look
            && let Some(check) = seen.get(entry, look)
        {
            return check;
        }
        let check = self.verify(entry);
        // Looked at again after the hashing: a file written while it was hashed leaves
        // nothing to go by.
        match (look, hash::fingerprint(&folder).ok()) {
            (Some(before), Some(after)) if before == after => {
                seen.put(entry, before, check.clone());
            }
            _ => seen.forget(&entry.local),
        }
        check
    }

    /// The installs of a lock file, with their manifests and what `check` says of each.
    fn installed_of(&self, entries: Vec<Entry>, checks: Vec<Check>) -> Vec<Installed> {
        entries
            .into_iter()
            .zip(checks)
            .map(|(entry, check)| {
                let manifest = Self::read_manifest(&self.package_dir(&entry))
                    .map(|(manifest, _)| manifest)
                    .map_err(|error| match error {
                        StoreError::Manifest(problems) => problems,
                        other => vec![Problem {
                            at: None,
                            message: other.to_string(),
                        }],
                    });
                Installed {
                    entry,
                    manifest,
                    check,
                }
            })
            .collect()
    }

    /// Everything installed, with its manifest. `verify` checks each package's hash.
    pub fn installed(&self, verify: bool) -> Result<Vec<Installed>, StoreError> {
        let lock = self.lock()?;
        let checks = lock
            .plugins
            .iter()
            .map(|entry| {
                if verify {
                    self.verify(entry)
                } else {
                    Check::Unchecked
                }
            })
            .collect();
        Ok(self.installed_of(lock.plugins, checks))
    }

    /// Everything installed, and the order it loads in.
    pub fn resolve(&self, verify: bool) -> Result<(Vec<Installed>, Resolution), StoreError> {
        let lock = self.lock()?;
        let order = lock.full_order();
        let checks = lock
            .plugins
            .iter()
            .map(|entry| {
                if verify {
                    self.verify(entry)
                } else {
                    Check::Unchecked
                }
            })
            .collect();
        Ok(Self::resolved(
            self.installed_of(lock.plugins, checks),
            &order,
        ))
    }

    /// Everything installed and the order it loads in, as a program that stays up reads
    /// it again and again: a package this run has hashed is hashed again only when its
    /// files were touched since, the packages that do need hashing are hashed several at
    /// a time, and one that is turned off is not looked at until it is turned on.
    pub fn resolve_seen(&self, seen: &Seen) -> Result<(Vec<Installed>, Resolution), StoreError> {
        let lock = self.lock()?;
        let order = lock.full_order();
        seen.keep(lock.plugins.iter().map(|entry| entry.local.as_str()));
        let checks: Vec<std::sync::OnceLock<Check>> =
            lock.plugins.iter().map(|_| Default::default()).collect();
        let next = AtomicUsize::new(0);
        let workers = lock.plugins.len().clamp(1, HASHING_AT_ONCE);
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(entry) = lock.plugins.get(index) else {
                            break;
                        };
                        let check = if entry.enabled {
                            self.verify_seen(entry, seen)
                        } else {
                            Check::Unchecked
                        };
                        let _ = checks[index].set(check);
                    }
                });
            }
        });
        let checks = checks
            .into_iter()
            .map(|check| check.into_inner().unwrap_or(Check::Unchecked))
            .collect();
        Ok(Self::resolved(
            self.installed_of(lock.plugins, checks),
            &order,
        ))
    }

    fn resolved(installed: Vec<Installed>, order: &[String]) -> (Vec<Installed>, Resolution) {
        let unusable: Vec<Option<String>> = installed.iter().map(Installed::unusable).collect();
        let candidates: Vec<Candidate<'_>> = installed
            .iter()
            .zip(&unusable)
            .map(|(one, unusable)| Candidate {
                entry: &one.entry,
                manifest: one.manifest.as_ref().ok(),
                unusable: unusable.clone(),
            })
            .collect();
        let resolution = resolve(&candidates, order);
        (installed, resolution)
    }
}

/// How many packages are hashed at a time.
const HASHING_AT_ONCE: usize = 4;

/// What a run has found of the packages it hashed, kept in memory only: for each
/// install, the hash its lock entry names, how its files looked from outside then, and
/// what the hashing said. Nothing of it outlives the program, so the first look of
/// every run hashes.
#[derive(Debug, Default)]
pub struct Seen {
    by_local: std::sync::Mutex<std::collections::HashMap<String, SeenOne>>,
}

#[derive(Debug)]
struct SeenOne {
    content_hash: String,
    look: String,
    check: Check,
}

impl Seen {
    fn all(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, SeenOne>> {
        self.by_local
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn get(&self, entry: &Entry, look: &str) -> Option<Check> {
        self.all()
            .get(&entry.local)
            .filter(|one| one.content_hash == entry.content_hash && one.look == look)
            .map(|one| one.check.clone())
    }

    fn put(&self, entry: &Entry, look: String, check: Check) {
        self.all().insert(
            entry.local.clone(),
            SeenOne {
                content_hash: entry.content_hash.clone(),
                look,
                check,
            },
        );
    }

    /// Forgets what was found of an install, so it is hashed the next time.
    pub fn forget(&self, local: &str) {
        self.all().remove(local);
    }

    /// Forgets every install that is not among `locals`.
    fn keep<'a>(&self, locals: impl Iterator<Item = &'a str>) {
        let locals: BTreeSet<&str> = locals.collect();
        self.all()
            .retain(|local, _| locals.contains(local.as_str()));
    }

    /// How many installs it holds a finding of.
    pub fn len(&self) -> usize {
        self.all().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Whether two repository addresses are the same, ignoring case, a trailing `/` and a
/// trailing `.git`.
fn same_repository(one: &str, other: &str) -> bool {
    let plain = |address: &str| {
        let lower = address.trim().to_ascii_lowercase();
        let lower = lower.trim_end_matches('/');
        lower.strip_suffix(".git").unwrap_or(lower).to_owned()
    };
    plain(one) == plain(other)
}

/// A manifest to hold the place of the real one while a package is read.
fn placeholder() -> Manifest {
    Manifest {
        schema: 0,
        id: String::new(),
        name: String::new(),
        version: String::new(),
        description: None,
        icon: None,
        license: None,
        repository: None,
        api: 0,
        main: None,
        activation: Vec::new(),
        permissions: toml::Table::new(),
        limits: crate::manifest::Limits::default(),
        dependencies: std::collections::BTreeMap::new(),
        load: crate::manifest::Load::default(),
        contributes: crate::manifest::Contributes::default(),
        requested: Permissions::default(),
        events: Vec::new(),
    }
}

/// Checks that the files a manifest names are in the package, and are what they say.
fn check_contents(root: &Path, manifest: &Manifest) -> Result<(), StoreError> {
    let mut problems = Vec::new();
    let need = |key: &str, path: &str, problems: &mut Vec<String>| -> Option<PathBuf> {
        let file = root.join(path);
        if file.is_file() {
            Some(file)
        } else {
            problems.push(format!("{key}: there is no {path} in the package"));
            None
        }
    };
    if let Some(main) = &manifest.main {
        need("main", main, &mut problems);
    }
    let mut icons: Vec<(String, &str)> = Vec::new();
    if let Some(icon) = &manifest.icon {
        icons.push(("icon".to_owned(), icon));
    }
    let contributes = &manifest.contributes;
    for (index, item) in contributes.menus.iter().enumerate() {
        if let Some(icon) = &item.icon {
            icons.push((format!("contributes.menus[{index}].icon"), icon));
        }
    }
    for (index, item) in contributes.status.iter().enumerate() {
        if let Some(icon) = &item.icon {
            icons.push((format!("contributes.status[{index}].icon"), icon));
        }
    }
    for (index, panel) in contributes.panels.iter().enumerate() {
        if let Some(icon) = &panel.icon {
            icons.push((format!("contributes.panels[{index}].icon"), icon));
        }
    }
    let images: BTreeSet<&str> = contributes
        .images
        .iter()
        .map(|image| image.id.as_str())
        .collect();
    let mut pictures: Vec<(String, &str)> = icons
        .into_iter()
        .filter(|(_, icon)| {
            !images.contains(icon) && (icon.contains('/') || crate::assets::is_image_path(icon))
        })
        .collect();
    for (index, image) in contributes.images.iter().enumerate() {
        pictures.push((format!("contributes.images[{index}].file"), &image.file));
    }
    let mut total = 0u64;
    for (key, path) in pictures {
        match crate::assets::read_image(root, path) {
            Ok(image) => total += image.bytes.len() as u64,
            Err(problem) => problems.push(format!("{key}: {problem}")),
        }
    }
    if total > crate::assets::IMAGES_MAX_BYTES {
        problems.push(format!(
            "the pictures hold {total} bytes together; a plugin's may hold {}",
            crate::assets::IMAGES_MAX_BYTES
        ));
    }
    for (index, effect) in contributes.cursor_effects.iter().enumerate() {
        if let Err(problem) = crate::assets::read_shader(root, &effect.shader) {
            problems.push(format!(
                "contributes.cursor-effects[{index}].shader: {problem}"
            ));
        }
    }
    for (index, theme) in contributes.themes.iter().enumerate() {
        let key = format!("contributes.themes[{index}]");
        let Some(file) = need(&key, &theme.file, &mut problems) else {
            continue;
        };
        match std::fs::read_to_string(&file) {
            Ok(text) => match lume_config::parse_theme(&theme.id, &text, Some(&file)) {
                Ok((_, found)) => {
                    for problem in found {
                        problems.push(format!("{key}: {}", problem.message));
                    }
                }
                Err(error) => problems.push(format!("{key}: {error}")),
            },
            Err(error) => problems.push(format!("{key}: {error}")),
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(StoreError::Contents(problems))
    }
}

#[cfg(test)]
pub mod testing {
    //! What the tests share: folders of their own to write in.

    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT: AtomicU32 = AtomicU32::new(1);

    /// A folder that exists for one test and is removed when the test is over.
    pub struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        pub fn new(what: &str) -> Self {
            let number = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "lume-plugin-test-{}-{number}-{what}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("a temporary folder can be made");
            Self { path }
        }

        pub fn path(&self) -> &Path {
            &self.path
        }

        /// Writes a file under the folder, making the folders on the way.
        pub fn write(&self, relative: &str, text: &str) -> PathBuf {
            let path = self.path.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("the folders can be made");
            }
            std::fs::write(&path, text).expect("the file can be written");
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::TempDir;
    use super::*;
    use crate::order::State;

    /// Writes a package into `folder` with the manifest's `extra` lines.
    fn package(dir: &TempDir, folder: &str, id: &str, version: &str, extra: &str) -> PathBuf {
        dir.write(
            &format!("{folder}/plugin.toml"),
            &format!(
                "schema = 1\nid = \"{id}\"\nname = \"Plugin {id}\"\nversion = \"{version}\"\n\
                 api = \"1\"\nmain = \"host.js\"\nrepository = \"https://example.org/{id}\"\n\
                 activation = [\"onStartup\"]\n{extra}"
            ),
        );
        dir.write(&format!("{folder}/host.js"), "// code");
        dir.path().join(folder)
    }

    fn store(dir: &TempDir) -> Store {
        Store::new(dir.path().join("state").join("plugins"))
    }

    #[test]
    fn a_package_is_installed_with_its_permissions_and_its_hash() {
        let dir = TempDir::new("install");
        let source = package(
            &dir,
            "src",
            "acme.metrics",
            "1.0.0",
            "[permissions]\n\"system.metrics\" = true\n",
        );
        let store = store(&dir);
        let prepared = store.prepare(&source, InstallOptions::default()).unwrap();
        assert_eq!(prepared.local, "acme.metrics");
        assert_eq!(prepared.change, Change::New);
        assert_eq!(prepared.new_permissions.len(), 1);
        // Nothing is in place yet.
        assert!(store.lock().unwrap().plugins.is_empty());

        let installed = store.commit(prepared).unwrap();
        assert_eq!(installed.entry.dir, "packages/acme.metrics/1.0.0");
        assert!(
            store
                .package_dir(&installed.entry)
                .join("host.js")
                .is_file()
        );
        assert_eq!(store.verify(&installed.entry), Check::Intact);
        let lock = store.lock().unwrap();
        assert_eq!(lock.plugins.len(), 1);
        assert_eq!(lock.order, ["acme.metrics"]);
        assert!(lock.plugins[0].grants.contains_key("system.metrics"));
        // The staging copy is gone.
        let staging = store.root().join("staging");
        assert!(
            std::fs::read_dir(&staging)
                .map(|mut d| d.next().is_none())
                .unwrap_or(true)
        );
    }

    #[test]
    fn a_prepared_install_that_is_dropped_leaves_nothing() {
        let dir = TempDir::new("dropped");
        let source = package(&dir, "src", "acme.x", "1.0.0", "");
        let store = store(&dir);
        drop(store.prepare(&source, InstallOptions::default()).unwrap());
        assert!(store.lock().unwrap().plugins.is_empty());
        let staging = store.root().join("staging");
        assert!(
            std::fs::read_dir(&staging)
                .map(|mut d| d.next().is_none())
                .unwrap_or(true)
        );
    }

    #[test]
    fn a_changed_file_is_found() {
        let dir = TempDir::new("tampered");
        let source = package(&dir, "src", "acme.x", "1.0.0", "");
        let store = store(&dir);
        let installed = store
            .commit(store.prepare(&source, InstallOptions::default()).unwrap())
            .unwrap();
        std::fs::write(
            store.package_dir(&installed.entry).join("host.js"),
            "evil()",
        )
        .unwrap();
        assert_eq!(store.verify(&installed.entry), Check::Modified);
        let (installed, resolution) = store.resolve(true).unwrap();
        assert_eq!(installed[0].check, Check::Modified);
        assert!(
            matches!(&resolution.states["acme.x"], State::Inactive(why) if why.contains("changed"))
        );

        std::fs::remove_file(store.package_dir(&installed[0].entry).join("host.js")).unwrap();
        assert_eq!(store.verify(&installed[0].entry), Check::Modified);
        std::fs::remove_dir_all(store.package_dir(&installed[0].entry)).unwrap();
        assert_eq!(store.verify(&installed[0].entry), Check::Missing);
    }

    #[test]
    fn a_package_is_hashed_again_only_when_its_files_were_touched() {
        let dir = TempDir::new("seen");
        let store = store(&dir);
        for id in ["acme.a", "acme.b", "acme.off"] {
            let source = package(&dir, &format!("src-{id}"), id, "1.0.0", "");
            store
                .commit(store.prepare(&source, InstallOptions::default()).unwrap())
                .unwrap();
        }
        store.set_enabled("acme.off", false).unwrap();
        let seen = Seen::default();
        let (installed, _) = store.resolve_seen(&seen).unwrap();
        let check = |installed: &[Installed], local: &str| {
            installed
                .iter()
                .find(|one| one.entry.local == local)
                .unwrap()
                .check
                .clone()
        };
        assert_eq!(check(&installed, "acme.a"), Check::Intact);
        assert_eq!(check(&installed, "acme.b"), Check::Intact);
        // One that is turned off is not looked at.
        assert_eq!(check(&installed, "acme.off"), Check::Unchecked);
        assert_eq!(seen.len(), 2);

        // What was found is kept while the files look the same: a finding put in by
        // hand is what comes back, so nothing was hashed.
        let entry = installed
            .iter()
            .find(|one| one.entry.local == "acme.a")
            .unwrap()
            .entry
            .clone();
        let look = hash::fingerprint(&store.package_dir(&entry)).unwrap();
        seen.put(&entry, look, Check::Unreadable("kept".to_owned()));
        let (installed, _) = store.resolve_seen(&seen).unwrap();
        assert_eq!(
            check(&installed, "acme.a"),
            Check::Unreadable("kept".to_owned())
        );

        // A file written since is found, whatever was kept.
        std::fs::write(store.package_dir(&entry).join("host.js"), "evil()").unwrap();
        let (installed, resolution) = store.resolve_seen(&seen).unwrap();
        assert_eq!(check(&installed, "acme.a"), Check::Modified);
        assert!(!resolution.is_ready("acme.a"));
        assert_eq!(check(&installed, "acme.b"), Check::Intact);

        // Turned on, the third is hashed; removed, an install is forgotten.
        store.set_enabled("acme.off", true).unwrap();
        let (installed, _) = store.resolve_seen(&seen).unwrap();
        assert_eq!(check(&installed, "acme.off"), Check::Intact);
        assert_eq!(seen.len(), 3);
        store.remove("acme.b", false).unwrap();
        store.resolve_seen(&seen).unwrap();
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn the_same_plugin_replaces_itself_and_only_new_permissions_need_agreeing_to() {
        let dir = TempDir::new("update");
        let store = store(&dir);
        let first = package(
            &dir,
            "v1",
            "acme.x",
            "1.0.0",
            "[permissions]\n\"system.metrics\" = true\n",
        );
        store
            .commit(store.prepare(&first, InstallOptions::default()).unwrap())
            .unwrap();
        store.rename("acme.x", Some("My X".to_owned())).unwrap();
        store.set_enabled("acme.x", false).unwrap();

        let second = package(
            &dir,
            "v2",
            "acme.x",
            "1.1.0",
            "[permissions]\n\"system.metrics\" = true\n\"clipboard.write\" = true\n",
        );
        let prepared = store.prepare(&second, InstallOptions::default()).unwrap();
        assert_eq!(
            prepared.change,
            Change::Replace {
                from: "1.0.0".to_owned(),
                direction: Direction::Newer
            }
        );
        let new: Vec<_> = prepared
            .new_permissions
            .iter()
            .map(|item| item.kind)
            .collect();
        assert_eq!(new, [permissions::Kind::ClipboardWrite]);
        let installed = store.commit(prepared).unwrap();
        // The user's name and switch stay; the old version's folder goes.
        assert_eq!(installed.entry.name.as_deref(), Some("My X"));
        assert!(!installed.entry.enabled);
        assert!(!store.root().join("packages/acme.x/1.0.0").exists());
        assert!(store.root().join("packages/acme.x/1.1.0").exists());

        let older = store.prepare(&first, InstallOptions::default()).unwrap();
        assert!(matches!(
            older.change,
            Change::Replace {
                direction: Direction::Older,
                ..
            }
        ));
        assert!(
            older
                .warnings
                .iter()
                .any(|warning| warning.contains("older"))
        );
        assert!(older.new_permissions.is_empty());

        // The same version again replaces it in place.
        let again = store.prepare(&second, InstallOptions::default()).unwrap();
        assert!(matches!(
            again.change,
            Change::Replace {
                direction: Direction::Same,
                ..
            }
        ));
        store.commit(again).unwrap();
        assert!(store.root().join("packages/acme.x/1.1.0/host.js").is_file());
    }

    #[test]
    fn another_plugin_of_the_same_id_goes_beside_it_under_a_name_of_its_own() {
        let dir = TempDir::new("taken");
        let store = store(&dir);
        let original = package(&dir, "a", "acme.x", "1.0.0", "");
        store
            .commit(store.prepare(&original, InstallOptions::default()).unwrap())
            .unwrap();

        let fork = package(&dir, "b", "acme.x", "2.0.0", "");
        std::fs::write(
            fork.join("plugin.toml"),
            std::fs::read_to_string(fork.join("plugin.toml"))
                .unwrap()
                .replace("example.org", "fork.example"),
        )
        .unwrap();
        let error = store.prepare(&fork, InstallOptions::default()).unwrap_err();
        assert!(matches!(error, StoreError::Taken { .. }), "{error}");
        assert!(error.to_string().contains("--as"));

        let beside = store
            .prepare(
                &fork,
                InstallOptions {
                    local: Some("me.x".to_owned()),
                    name: Some("X (fork)".to_owned()),
                },
            )
            .unwrap();
        assert_eq!(beside.change, Change::New);
        let installed = store.commit(beside).unwrap();
        assert_eq!(installed.entry.manifest_id, "acme.x");
        assert_eq!(installed.entry.local, "me.x");
        assert_eq!(store.lock().unwrap().plugins.len(), 2);
    }

    #[test]
    fn ids_that_would_share_names_cannot_be_installed_side_by_side() {
        let dir = TempDir::new("overlap");
        let store = store(&dir);
        let one = package(&dir, "a", "acme", "1.0.0", "");
        store
            .commit(store.prepare(&one, InstallOptions::default()).unwrap())
            .unwrap();
        let other = package(&dir, "b", "acme.tools", "1.0.0", "");
        assert!(matches!(
            store.prepare(&other, InstallOptions::default()),
            Err(StoreError::Overlap { .. })
        ));
        for reserved in ["lume", "lume.tools", "Bad Name"] {
            assert!(matches!(
                store.prepare(
                    &other,
                    InstallOptions {
                        local: Some(reserved.to_owned()),
                        name: None
                    }
                ),
                Err(StoreError::BadId(_))
            ));
        }
    }

    #[test]
    fn a_package_without_the_files_its_manifest_names_is_refused() {
        let dir = TempDir::new("contents");
        let store = store(&dir);
        let source = package(
            &dir,
            "a",
            "acme.x",
            "1.0.0",
            "icon = \"icon.svg\"\n[[contributes.themes]]\nid = \"night\"\nfile = \"night.toml\"\n",
        );
        std::fs::remove_file(source.join("host.js")).unwrap();
        let error = store
            .prepare(&source, InstallOptions::default())
            .unwrap_err();
        let text = error.to_string();
        for says in ["no host.js", "no icon.svg", "no night.toml"] {
            assert!(text.contains(says), "{text}");
        }
        dir.write("a/host.js", "");
        dir.write(
            "a/icon.svg",
            &"x".repeat(usize::try_from(ICON_MAX_BYTES).unwrap() + 1),
        );
        dir.write("a/night.toml", "[colors]\nbogus = 1\n");
        let text = store
            .prepare(&source, InstallOptions::default())
            .unwrap_err()
            .to_string();
        assert!(text.contains("a picture may hold"), "{text}");
    }

    #[test]
    fn a_permission_granted_while_it_runs_joins_its_grants_and_a_service_pin_is_kept() {
        let dir = TempDir::new("grant");
        let store = store(&dir);
        let source = package(
            &dir,
            "a",
            "a.x",
            "1.0.0",
            "[permissions]\n\"fs.read\" = [\"${plugin-data}/**\"]\n\"services.use\" = [\"core.x\"]\n",
        );
        store
            .commit(store.prepare(&source, InstallOptions::default()).unwrap())
            .unwrap();
        // The services it may use are pinned to the install the user agreed to.
        assert_eq!(
            store.lock().unwrap().get("a.x").unwrap().services["core.x"],
            "core.x"
        );

        let wider = Request {
            kind: Kind::FsRead,
            scope: vec!["${workspace}/**".to_owned()],
            prompt: crate::permissions::Prompt::Once,
        };
        store.grant("a.x", &wider).unwrap();
        let clipboard = Request {
            kind: Kind::ClipboardRead,
            scope: Vec::new(),
            prompt: crate::permissions::Prompt::Once,
        };
        store.grant("a.x", &clipboard).unwrap();
        let lock = store.lock().unwrap();
        let granted = permissions::parse_table(&lock.get("a.x").unwrap().grants).unwrap();
        let fs = granted
            .required
            .iter()
            .find(|request| request.kind == Kind::FsRead)
            .unwrap();
        assert_eq!(fs.scope, ["${plugin-data}/**", "${workspace}/**"]);
        assert!(crate::broker::allows(&granted, Kind::ClipboardRead, None));
        assert!(store.grant("not.there", &clipboard).is_err());

        store.pin_service("a.x", "core.x", "me.core").unwrap();
        assert_eq!(
            store.lock().unwrap().get("a.x").unwrap().services["core.x"],
            "me.core"
        );
    }

    #[test]
    fn installs_are_removed_turned_off_renamed_bound_and_ordered() {
        let dir = TempDir::new("manage");
        let store = store(&dir);
        for (folder, id, extra) in [
            ("a", "a.x", "[dependencies]\n\"core.x\" = \"1\"\n"),
            ("b", "b.x", ""),
            ("c", "core.x", ""),
        ] {
            let source = package(&dir, folder, id, "1.0.0", extra);
            store
                .commit(store.prepare(&source, InstallOptions::default()).unwrap())
                .unwrap();
        }
        let fork = package(&dir, "d", "core.x", "1.5.0", "");
        std::fs::write(
            fork.join("plugin.toml"),
            std::fs::read_to_string(fork.join("plugin.toml"))
                .unwrap()
                .replace("example.org", "fork.example"),
        )
        .unwrap();
        store
            .commit(
                store
                    .prepare(
                        &fork,
                        InstallOptions {
                            local: Some("me.core".to_owned()),
                            name: None,
                        },
                    )
                    .unwrap(),
            )
            .unwrap();

        store.bind("a.x", "core.x", Some("me.core")).unwrap();
        assert!(store.bind("a.x", "other.x", Some("me.core")).is_err());
        assert!(store.bind("a.x", "core.x", Some("not.there")).is_err());
        let (_, resolution) = store.resolve(true).unwrap();
        assert_eq!(resolution.bound["a.x"]["core.x"], "me.core");

        store.move_to("b.x", Position::First).unwrap();
        store
            .move_to("a.x", Position::After("b.x".to_owned()))
            .unwrap();
        assert_eq!(store.lock().unwrap().order[..2], ["b.x", "a.x"]);

        store.set_enabled("me.core", false).unwrap();
        let (_, resolution) = store.resolve(true).unwrap();
        assert!(!resolution.is_ready("a.x"));
        store.bind("a.x", "core.x", None).unwrap();
        let (_, resolution) = store.resolve(true).unwrap();
        assert!(resolution.is_ready("a.x"));

        std::fs::create_dir_all(store.data_dir("b.x")).unwrap();
        store.remove("b.x", false).unwrap();
        assert!(store.data_dir("b.x").exists());
        assert!(!store.root().join("packages/b.x").exists());
        assert!(matches!(
            store.remove("b.x", false),
            Err(StoreError::NotInstalled(_))
        ));
        store.remove("core.x", true).unwrap();
        assert_eq!(store.lock().unwrap().full_order(), ["a.x", "me.core"]);
    }
}
