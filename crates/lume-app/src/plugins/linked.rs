//! Plugins that are being worked on: folders used where they lie (`lume plugin link`).
//!
//! Each linked folder is watched, and a change to its files is followed a moment after
//! the last of them was written:
//!
//! - the manifest: what is installed is read again, as when the lock file changes, so
//!   what the plugin adds and what it is allowed follow the manifest; its code and its
//!   views start again;
//! - the plugin's code, or the map beside it: its code is stopped and started again;
//! - a file one of its views has loaded, or one beside or below one of its pages: the
//!   windows are told to load its views again, each from the state it kept;
//! - a picture, a theme or a shader its manifest names: what the plugins add is worked
//!   out again.
//!
//! Folders that are never part of a package (version control, the packages a build
//! installs) are not watched.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use lume_plugin::manifest::{Activation, MANIFEST_FILE, Manifest};
use lume_plugin::package::LEFT_OUT;
use lume_plugin_runtime::Level;
use notify::{RecursiveMode, Watcher};
use serde_json::json;
use tokio::sync::mpsc;

use super::Plugins;

/// How long after a file of a linked folder changes it is followed: a build writes
/// several files, one after another.
const SETTLE: Duration = Duration::from_millis(150);

/// What of a linked plugin changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Changed {
    pub manifest: bool,
    pub code: bool,
    pub pages: bool,
    pub named: bool,
}

impl Changed {
    fn any(self) -> bool {
        self.manifest || self.code || self.pages || self.named
    }
}

/// One linked install, as the watcher needs to know it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Folder {
    dir: PathBuf,
    /// The plugin's code, as a path in the folder written with `/`.
    main: Option<String>,
    /// The folders its pages are in, each ending in `/`; an empty one is the folder
    /// itself.
    pages: BTreeSet<String>,
    /// The other files its manifest names.
    named: BTreeSet<String>,
    /// The files its views were served since it was linked, wherever they are in the
    /// folder: a page may load what a build wrote elsewhere.
    served: BTreeSet<String>,
}

/// File names are compared as the system holds them: without regard to case where it
/// has none.
fn same_name(one: &str, other: &str) -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        one.eq_ignore_ascii_case(other)
    } else {
        one == other
    }
}

impl Folder {
    pub(super) fn new(dir: PathBuf, manifest: Option<&Manifest>) -> Self {
        let mut folder = Self {
            dir,
            main: None,
            pages: BTreeSet::new(),
            named: BTreeSet::new(),
            served: BTreeSet::new(),
        };
        let Some(manifest) = manifest else {
            return folder;
        };
        folder.main.clone_from(&manifest.main);
        let contributes = &manifest.contributes;
        let entries = contributes
            .panels
            .iter()
            .filter_map(|panel| panel.entry.as_deref())
            .chain(
                contributes
                    .pane_views
                    .iter()
                    .map(|view| view.entry.as_str()),
            );
        for entry in entries {
            let below = entry.rsplit_once('/').map_or("", |(folder, _)| folder);
            folder.pages.insert(if below.is_empty() {
                String::new()
            } else {
                format!("{below}/")
            });
        }
        let icons = manifest
            .icon
            .iter()
            .chain(
                contributes
                    .menus
                    .iter()
                    .filter_map(|item| item.icon.as_ref()),
            )
            .chain(
                contributes
                    .status
                    .iter()
                    .filter_map(|item| item.icon.as_ref()),
            )
            .chain(
                contributes
                    .panels
                    .iter()
                    .filter_map(|panel| panel.icon.as_ref()),
            )
            .chain(
                contributes
                    .pane_views
                    .iter()
                    .filter_map(|view| view.icon.as_ref()),
            );
        folder.named.extend(icons.cloned());
        folder
            .named
            .extend(contributes.images.iter().map(|image| image.file.clone()));
        folder
            .named
            .extend(contributes.themes.iter().map(|theme| theme.file.clone()));
        folder.named.extend(
            contributes
                .cursor_effects
                .iter()
                .map(|effect| effect.shader.clone()),
        );
        folder
    }

    /// What the change of the file at `path` is to the plugin, if the file is in its
    /// folder and is one that matters.
    pub(super) fn changed(&self, path: &Path) -> Option<Changed> {
        let relative = path.strip_prefix(&self.dir).ok()?;
        let parts: Vec<String> = relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect();
        if parts.is_empty() || parts.iter().any(|part| LEFT_OUT.contains(&part.as_str())) {
            return None;
        }
        let relative = parts.join("/");
        let same = same_name;
        let mut changed = Changed::default();
        if same(&relative, MANIFEST_FILE) {
            changed.manifest = true;
        } else if self
            .main
            .as_deref()
            .is_some_and(|main| same(&relative, main) || same(&relative, &format!("{main}.map")))
        {
            changed.code = true;
        } else {
            changed.named = self.named.iter().any(|named| same(&relative, named));
            changed.pages = self.served.iter().any(|served| same(&relative, served))
                || self.pages.iter().any(|folder| {
                    relative
                        .get(..folder.len())
                        .is_some_and(|start| same(start, folder))
                });
        }
        changed.any().then_some(changed)
    }
}

/// The linked folders that are watched.
#[derive(Default)]
pub(super) struct Linked {
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
    folders: Mutex<BTreeMap<String, Folder>>,
    /// Where the watcher's events go; made when the first folder is watched.
    changes: Mutex<Option<mpsc::UnboundedSender<PathBuf>>>,
    /// When each plugin was last loaded again, and how long after the change that
    /// was: what `lume ctl plugins` shows of a linked plugin.
    reloaded: Mutex<BTreeMap<String, Reloaded>>,
}

/// How a linked plugin was last loaded again.
#[derive(Clone, Copy, Debug)]
pub(super) struct Reloaded {
    pub times: u64,
    /// From the first change that was seen until its code had started again and the
    /// windows were told, in milliseconds.
    pub took_ms: u64,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Watches a linked folder: the folder itself, and everything below each folder in it
/// that may be part of a package.
fn watch(watcher: &mut notify::RecommendedWatcher, dir: &Path) {
    if let Err(error) = watcher.watch(dir, RecursiveMode::NonRecursive) {
        tracing::warn!(folder = %dir.display(), %error, "a linked plugin's folder is not watched");
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let left_out = LEFT_OUT.iter().any(|left| name == *left);
        if !left_out && entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            let _ = watcher.watch(&entry.path(), RecursiveMode::Recursive);
        }
    }
}

impl Plugins {
    /// Whether any plugin that can be used is a folder used where it lies.
    pub fn any_linked(&self) -> bool {
        self.inner()
            .loaded
            .values()
            .any(|loaded| loaded.installed.entry.is_link() && loaded.is_ready())
    }

    /// Notes that a view of a linked plugin was served the file at `path` of its
    /// folder, so that a change to the file loads the plugin's views again.
    pub fn note_served(&self, local: &str, path: &str) {
        if let Some(folder) = lock(&self.linked.folders).get_mut(local)
            && !folder.served.contains(path)
        {
            folder.served.insert(path.to_owned());
        }
    }

    /// How a linked plugin was last loaded again, if it was.
    pub(super) fn reloaded(&self, local: &str) -> Option<Reloaded> {
        lock(&self.linked.reloaded).get(local).copied()
    }

    /// Follows the linked folders that are installed now: those that came are watched,
    /// those that went are let go of. Called whenever what is installed was read.
    pub(super) fn follow_links(self: &Arc<Self>) {
        let wanted: BTreeMap<String, Folder> = {
            let inner = self.inner();
            inner
                .loaded
                .iter()
                .filter(|(_, loaded)| {
                    loaded.installed.entry.is_link() && loaded.installed.entry.enabled
                })
                .map(|(local, loaded)| {
                    (
                        local.clone(),
                        Folder::new(
                            self.store.package_dir(&loaded.installed.entry),
                            loaded.manifest(),
                        ),
                    )
                })
                .collect()
        };
        let mut wanted = wanted;
        let mut folders = lock(&self.linked.folders);
        let same_dirs = folders.len() == wanted.len()
            && folders
                .iter()
                .zip(&wanted)
                .all(|(one, other)| one.0 == other.0 && one.1.dir == other.1.dir);
        // What its views were served is still what they load.
        for (local, folder) in &mut wanted {
            if let Some(before) = folders
                .get_mut(local)
                .filter(|before| before.dir == folder.dir)
            {
                folder.served = std::mem::take(&mut before.served);
            }
        }
        *folders = wanted;
        if same_dirs {
            return;
        }
        let mut held = lock(&self.linked.watcher);
        // Dropped first: a folder that is not linked any more is not watched.
        *held = None;
        if folders.is_empty() {
            return;
        }
        let sender = lock(&self.linked.changes)
            .get_or_insert_with(|| {
                let (sender, received) = mpsc::unbounded_channel();
                let this = Arc::clone(self);
                self.state.runtime().spawn(this.follow_changes(received));
                sender
            })
            .clone();
        let made = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            if matches!(event.kind, notify::EventKind::Access(_)) {
                return;
            }
            for path in event.paths {
                let _ = sender.send(path);
            }
        });
        match made {
            Ok(mut watcher) => {
                for folder in folders.values() {
                    watch(&mut watcher, &folder.dir);
                }
                *held = Some(watcher);
            }
            Err(error) => {
                tracing::warn!(%error, "linked plugins are not watched for changes");
            }
        }
    }

    async fn follow_changes(self: Arc<Self>, mut received: mpsc::UnboundedReceiver<PathBuf>) {
        while let Some(first) = received.recv().await {
            let began = Instant::now();
            let mut paths = vec![first];
            // Until nothing more has come for a moment.
            loop {
                match tokio::time::timeout(SETTLE, received.recv()).await {
                    Ok(Some(path)) => paths.push(path),
                    Ok(None) => return,
                    Err(_) => break,
                }
            }
            let mut changed: BTreeMap<String, Changed> = BTreeMap::new();
            {
                let folders = lock(&self.linked.folders);
                for path in &paths {
                    for (local, folder) in folders.iter() {
                        if let Some(what) = folder.changed(path) {
                            let all = changed.entry(local.clone()).or_default();
                            all.manifest |= what.manifest;
                            all.code |= what.code;
                            all.pages |= what.pages;
                            all.named |= what.named;
                        }
                    }
                }
                // A folder made since, such as the one a first build writes into, is
                // watched from now on.
                if let Some(watcher) = lock(&self.linked.watcher).as_mut() {
                    for folder in folders.values() {
                        if paths.iter().any(|path| path.parent() == Some(&folder.dir)) {
                            watch(watcher, &folder.dir);
                        }
                    }
                }
            }
            if changed.is_empty() {
                continue;
            }
            self.follow_changed(changed, began).await;
        }
    }

    /// Loads again what changed of each linked plugin.
    async fn follow_changed(self: &Arc<Self>, changed: BTreeMap<String, Changed>, began: Instant) {
        // The manifests and the files they name first, in one reading for all.
        if changed.values().any(|what| what.manifest || what.named) {
            self.reload().await;
        }
        for (local, what) in &changed {
            // A changed manifest has stopped the code already; it starts again here.
            let restarted = (what.code || what.manifest) && self.restart_code(local).await;
            if what.pages || what.manifest {
                self.state
                    .broadcast(&json!({ "type": "plugin_view_reload", "plugin": local }));
            }
            let took_ms = u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX);
            let mut parts = Vec::new();
            if what.manifest {
                parts.push("its manifest");
            }
            if what.code {
                parts.push("its code");
            }
            if what.pages {
                parts.push("its pages");
            }
            if what.named && !what.manifest {
                parts.push("files its manifest names");
            }
            self.log(
                local,
                Level::Info,
                &format!(
                    "loaded again after a change to {} ({took_ms} ms{})",
                    parts.join(", "),
                    if restarted { ", code started" } else { "" }
                ),
            );
            let mut reloaded = lock(&self.linked.reloaded);
            let times = reloaded.get(local).map_or(0, |one| one.times) + 1;
            reloaded.insert(local.clone(), Reloaded { times, took_ms });
        }
    }

    /// Stops a linked plugin's code and starts it again where it would be running: it
    /// was, it starts with the application, or one of its views is open. Returns
    /// whether it started.
    async fn restart_code(self: &Arc<Self>, local: &str) -> bool {
        let (running, wanted) = {
            let inner = self.inner();
            let Some(loaded) = inner.loaded.get(local).filter(|loaded| loaded.is_ready()) else {
                return false;
            };
            let Some(manifest) = loaded.manifest() else {
                return false;
            };
            if manifest.main.is_none() {
                return false;
            }
            let running = loaded
                .running
                .as_ref()
                .map(|running| (Arc::clone(&running.plugin), running.generation));
            (running, manifest.events.contains(&Activation::Startup))
        };
        let was_running = running.is_some();
        if let Some((plugin, generation)) = running {
            plugin.stop();
            self.ended(local, generation, "its code stopped");
        }
        let has_views = !self.view_instances(local, None).is_empty();
        if !(was_running || wanted || has_views) {
            return false;
        }
        match self.activate(local).await {
            Ok(()) => true,
            Err(problem) => {
                self.log(
                    local,
                    Level::Error,
                    &format!("its changed code did not start: {problem}"),
                );
                self.tell_state(local);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(extra: &str) -> Manifest {
        Manifest::parse(&format!(
            "schema = 1\nid = \"me.dev\"\nname = \"Dev\"\nversion = \"0.1.0\"\napi = \"1\"\n\
             main = \"dist/host.js\"\nactivation = [\"onStartup\"]\n{extra}"
        ))
        .unwrap_or_else(|problems| panic!("{problems:#?}"))
    }

    #[test]
    fn a_changed_file_is_told_apart_by_what_it_is_to_the_plugin() {
        let dir = std::env::temp_dir().join("lume-linked-test").join("work");
        let manifest = manifest(
            "icon = \"art/icon.svg\"\n\
             [[contributes.panels]]\nid = \"side\"\ntitle = \"Side\"\nkind = \"frame\"\n\
             area = \"right\"\nentry = \"dist/ui/panel.html\"\n\
             [[contributes.themes]]\nid = \"night\"\nfile = \"themes/night.toml\"\n",
        );
        let folder = Folder::new(dir.clone(), Some(&manifest));
        let what = |relative: &str| {
            let path = relative
                .split('/')
                .fold(dir.clone(), |path, part| path.join(part));
            folder.changed(&path)
        };
        let only = |manifest, code, pages, named| {
            Some(Changed {
                manifest,
                code,
                pages,
                named,
            })
        };
        assert_eq!(what("plugin.toml"), only(true, false, false, false));
        assert_eq!(what("dist/host.js"), only(false, true, false, false));
        assert_eq!(what("dist/host.js.map"), only(false, true, false, false));
        assert_eq!(what("dist/ui/panel.html"), only(false, false, true, false));
        assert_eq!(what("dist/ui/assets/a.js"), only(false, false, true, false));
        assert_eq!(what("themes/night.toml"), only(false, false, false, true));
        assert_eq!(what("art/icon.svg"), only(false, false, false, true));
        // Sources, and what a build installs, are nothing to the running plugin.
        assert_eq!(what("src/view/App.tsx"), None);
        assert_eq!(what("dist/other.js"), None);
        // A file a view was served is one of its pages' wherever it lies.
        let mut serving = folder.clone();
        serving.served.insert("dist/other.js".to_owned());
        assert_eq!(
            serving.changed(&dir.join("dist").join("other.js")),
            only(false, false, true, false)
        );
        assert_eq!(what("node_modules/react/index.js"), None);
        assert_eq!(what("dist/ui/node_modules/x.js"), None);
        assert_eq!(what(".git/index"), None);
        assert_eq!(folder.changed(&dir), None);
        assert_eq!(folder.changed(Path::new("elsewhere/plugin.toml")), None);

        // A page at the top of the folder makes every file one of its own.
        let top = manifest_with_top_page();
        let folder = Folder::new(dir.clone(), Some(&top));
        assert_eq!(
            folder.changed(&dir.join("view.js")),
            only(false, false, true, false)
        );
        // Without a manifest that can be read, only the manifest is waited for.
        let folder = Folder::new(dir.clone(), None);
        assert_eq!(
            folder.changed(&dir.join("plugin.toml")),
            only(true, false, false, false)
        );
        assert_eq!(folder.changed(&dir.join("host.js")), None);
    }

    fn manifest_with_top_page() -> Manifest {
        manifest(
            "[[contributes.pane-views]]\nid = \"main\"\ntitle = \"Main\"\nentry = \"view.html\"\n",
        )
    }
}
