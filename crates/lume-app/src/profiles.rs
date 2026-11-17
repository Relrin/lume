//! The profiles in effect: what the application found on this machine, with the
//! user's own laid over, and the way to tell pages about them.
//!
//! Finding what is installed means looking through folders and asking another program
//! which Linux distributions there are, so it happens off to the side, once at start
//! and again when asked or when the user's OpenSSH configuration changes. Until the
//! first look is done only the profiles the user wrote out in full are known. A pane
//! that names any other profile waits for the look to finish, for a moment and no
//! longer; a pane that names none never waits.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

use lume_config::{Profile, merge_profiles};
use lume_session::{DetectOptions, Detected, detect};
use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};
use serde_json::{Map, Value, json};
use tokio::runtime::Handle;
use tokio::sync::watch;

use crate::configsvc::ConfigService;
use crate::tuning::{PROFILE_DETECT_WAIT, PROFILE_REDETECT_DELAY};

/// Takes what pages are to be told.
type Tell = Arc<dyn Fn(Value) + Send + Sync>;

/// How the user's OpenSSH configuration is to be used when listing its hosts.
fn options_of(config: &lume_config::Config) -> DetectOptions {
    DetectOptions {
        ssh_sharing: config
            .ssh
            .connection_sharing
            .then(|| config.ssh.control_persist.clone()),
    }
}

/// What a look at the machine found.
#[derive(Default)]
struct Found {
    profiles: Vec<Profile>,
    /// Why something a user might expect is not among them.
    notes: Vec<String>,
    /// The options the look was made with, so that a change of them is noticed.
    options: Option<DetectOptions>,
}

/// The profiles as they stand.
#[derive(Clone, Default)]
struct Merged {
    profiles: Arc<Vec<Profile>>,
    notes: Vec<String>,
}

pub struct ProfileService {
    config: Arc<ConfigService>,
    found: Mutex<Found>,
    merged: Mutex<Merged>,
    /// Whether a look at the machine was ever asked for. Where none was, nothing waits
    /// for one.
    started: AtomicBool,
    /// Whether the first look is done.
    looked: watch::Sender<bool>,
    /// Counts the looks asked for, so that of several in a row only the last is made.
    asked: AtomicU64,
    /// What pages were last told.
    told: Mutex<Value>,
    revision: AtomicU64,
    tell: OnceLock<Tell>,
    runtime: OnceLock<Handle>,
    watcher: Mutex<Option<RecommendedWatcher>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ProfileService {
    /// The profiles the configuration writes out in full. Nothing is looked for on the
    /// machine until [`start`](Self::start).
    pub fn new(config: Arc<ConfigService>) -> Arc<Self> {
        let service = Arc::new(Self {
            config,
            found: Mutex::new(Found::default()),
            merged: Mutex::new(Merged::default()),
            started: AtomicBool::new(false),
            looked: watch::Sender::new(false),
            asked: AtomicU64::new(0),
            told: Mutex::new(Value::Null),
            revision: AtomicU64::new(1),
            tell: OnceLock::new(),
            runtime: OnceLock::new(),
            watcher: Mutex::new(None),
        });
        service.merge();
        *lock(&service.told) = service.body();
        service
    }

    /// A service that has found `profiles` on the machine, for tests.
    #[cfg(test)]
    pub fn with_found(config: Arc<ConfigService>, profiles: Vec<Profile>) -> Arc<Self> {
        let service = Self::new(config);
        lock(&service.found).profiles = profiles;
        service.looked.send_replace(true);
        service.merge();
        *lock(&service.told) = service.body();
        service
    }

    /// Lays the user's entries over what was found.
    fn merge(&self) {
        let resolved = self.config.current();
        let found = lock(&self.found);
        let (profiles, problems) = merge_profiles(&found.profiles, &resolved.config.profiles);
        let mut notes = found.notes.clone();
        // An entry for a profile that was not found reads as incomplete. Before the
        // first look that is every such entry, so nothing is said until then.
        if *self.looked.borrow() {
            notes.extend(
                problems
                    .into_iter()
                    .map(|(id, why)| format!("profile `{id}`: {why}")),
            );
        }
        *lock(&self.merged) = Merged {
            profiles: Arc::new(profiles),
            notes,
        };
    }

    /// The profiles in effect, the user's own first.
    pub fn list(&self) -> Arc<Vec<Profile>> {
        Arc::clone(&lock(&self.merged).profiles)
    }

    /// The profile called `id`, if it is known now.
    pub fn find(&self, id: &str) -> Option<Profile> {
        self.list().iter().find(|profile| profile.id == id).cloned()
    }

    /// The profile called `id`. If it is not known and the machine is still being
    /// looked at, waits for that to finish, since the profile may be one of those the
    /// application makes.
    pub async fn resolve(&self, id: &str) -> Option<Profile> {
        if let Some(profile) = self.find(id) {
            return Some(profile);
        }
        if !self.started.load(Ordering::Relaxed) {
            return None;
        }
        let mut looked = self.looked.subscribe();
        let waited = tokio::time::timeout(PROFILE_DETECT_WAIT, looked.wait_for(|done| *done)).await;
        if waited.is_err() {
            tracing::warn!(profile = id, "the profiles were still being found");
        }
        self.find(id)
    }

    /// The id of the profile new terminals use: the workspace's choice, else the
    /// user's, else none.
    pub fn default_id(&self) -> Option<String> {
        let resolved = self.config.current();
        resolved
            .workspace
            .default_profile
            .clone()
            .or_else(|| Some(resolved.config.default_profile.clone()))
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty())
    }

    /// A view without its revision. It holds the look of every theme a profile names,
    /// since a page has only the look of the theme in use.
    fn body(&self) -> Value {
        let merged = lock(&self.merged).clone();
        let mut notes = merged.notes.clone();
        let mut looks = Map::new();
        for profile in merged.profiles.iter() {
            let Some(theme) = profile.theme.as_deref() else {
                continue;
            };
            if looks.contains_key(theme) {
                continue;
            }
            match self.config.look_with_theme(theme) {
                Some(look) => {
                    looks.insert(theme.to_owned(), look);
                }
                None => notes.push(format!(
                    "profile `{}`: there is no theme called `{theme}`, so its terminals use the window's",
                    profile.id
                )),
            }
        }
        json!({
            "profiles": *merged.profiles,
            "default": self.default_id(),
            "notes": notes,
            "looked": *self.looked.borrow(),
            "looks": looks,
        })
    }

    /// The profiles as a page or the command line is told them.
    pub fn view(&self) -> Value {
        let mut view = lock(&self.told).clone();
        if let Some(view) = view.as_object_mut() {
            view.insert(
                "revision".to_owned(),
                json!(self.revision.load(Ordering::Relaxed)),
            );
        }
        view
    }

    /// Tells the pages if the profiles came out different.
    fn announce(&self) {
        let notice = {
            let mut told = lock(&self.told);
            let body = self.body();
            if body == *told {
                return;
            }
            *told = body;
            let revision = self.revision.fetch_add(1, Ordering::Relaxed) + 1;
            let mut notice = told.clone();
            if let Some(notice) = notice.as_object_mut() {
                notice.insert("type".to_owned(), json!("profiles_changed"));
                notice.insert("revision".to_owned(), json!(revision));
            }
            notice
        };
        if let Some(tell) = self.tell.get() {
            tell(notice);
        }
    }

    /// Starts telling pages of changes through `tell`, and looks at the machine for
    /// the first time.
    pub fn start(self: &Arc<Self>, runtime: &Handle, tell: impl Fn(Value) + Send + Sync + 'static) {
        if self.tell.set(Arc::new(tell)).is_err() {
            return;
        }
        let _ = self.runtime.set(runtime.clone());
        self.started.store(true, Ordering::Relaxed);
        self.look_soon(false);
    }

    /// The configuration changed: the user's entries are laid over again, and the
    /// machine is looked at again if how to look changed.
    pub fn config_changed(self: &Arc<Self>) {
        let options = options_of(&self.config.current().config);
        let stale = lock(&self.found)
            .options
            .as_ref()
            .is_some_and(|used| *used != options);
        self.merge();
        self.announce();
        if stale {
            self.look_soon(false);
        }
    }

    /// Looks at the machine again and answers when that is done.
    pub async fn refresh(self: &Arc<Self>) {
        self.asked.fetch_add(1, Ordering::Relaxed);
        self.look().await;
    }

    /// Looks at the machine in the background. `settle` waits a moment first, for a
    /// file that is still being written, and gives way to a later request.
    fn look_soon(self: &Arc<Self>, settle: bool) {
        let Some(runtime) = self.runtime.get() else {
            return;
        };
        let turn = self.asked.fetch_add(1, Ordering::Relaxed) + 1;
        let service = Arc::clone(self);
        runtime.spawn(async move {
            if settle {
                tokio::time::sleep(PROFILE_REDETECT_DELAY).await;
                if service.asked.load(Ordering::Relaxed) != turn {
                    return;
                }
            }
            service.look().await;
        });
    }

    async fn look(self: &Arc<Self>) {
        let options = options_of(&self.config.current().config);
        let looking = options.clone();
        let detected = tokio::task::spawn_blocking(move || detect(&looking)).await;
        let Detected {
            profiles,
            notes,
            mut watch,
        } = match detected {
            Ok(detected) => detected,
            Err(error) => {
                tracing::error!(%error, "looking for profiles failed");
                Detected::default()
            }
        };
        // A configuration that is not there yet is watched for too, so that the hosts
        // of one the user goes on to write show up.
        if let Some(own) = lume_session::ssh_config::user_config()
            && !watch.contains(&own)
        {
            watch.push(own);
        }
        *lock(&self.found) = Found {
            profiles,
            notes,
            options: Some(options),
        };
        self.looked.send_replace(true);
        self.merge();
        self.announce();
        self.watch(&watch);
    }

    /// Watches the files the hosts were read from, so that a host the user adds shows
    /// up without their asking. A file is watched through its folder: editors replace
    /// files rather than write into them.
    fn watch(self: &Arc<Self>, files: &[PathBuf]) {
        let mut watcher = lock(&self.watcher);
        *watcher = None;
        if files.is_empty() {
            return;
        }
        let names: Vec<PathBuf> = files.to_vec();
        let service: Weak<Self> = Arc::downgrade(self);
        let made = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else {
                return;
            };
            if !event.paths.iter().any(|path| names.contains(path)) {
                return;
            }
            if let Some(service) = service.upgrade() {
                service.look_soon(true);
            }
        });
        let mut made = match made {
            Ok(made) => made,
            Err(error) => {
                tracing::warn!(%error, "the OpenSSH configuration is not watched for changes");
                return;
            }
        };
        let mut folders: Vec<&Path> = files.iter().filter_map(|file| file.parent()).collect();
        folders.sort();
        folders.dedup();
        for folder in folders {
            if let Err(error) = made.watch(folder, RecursiveMode::NonRecursive) {
                tracing::debug!(%error, folder = %folder.display(), "a folder is not watched");
            }
        }
        *watcher = Some(made);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use lume_config::{ProfileOrigin, ShellIntegration};
    use lume_session::Launcher;

    use super::*;
    use crate::testing::TempDir;

    fn found(id: &str, launcher: Launcher) -> Profile {
        Profile {
            id: id.to_owned(),
            name: id.to_owned(),
            icon: None,
            launcher,
            cwd: None,
            env: BTreeMap::new(),
            theme: None,
            font_size: None,
            shell_integration: ShellIntegration::Auto,
            close_on_exit: None,
            hidden: false,
            origin: ProfileOrigin::Generated,
            detail: None,
        }
    }

    fn pwsh() -> Profile {
        found(
            "auto.pwsh",
            Launcher::Shell {
                program: "pwsh.exe".to_owned(),
                args: vec![],
                login: false,
            },
        )
    }

    #[tokio::test]
    async fn the_users_entries_are_laid_over_what_was_found() {
        let dir = TempDir::new("profiles-merge");
        std::fs::create_dir_all(dir.dirs().config).unwrap();
        std::fs::write(
            dir.dirs().config_file(),
            "default-profile = \"box\"\n\n[[profile]]\nid = \"box\"\nlauncher = \"ssh\"\n\
             host = \"work\"\n\n[[profile]]\nid = \"auto.pwsh\"\nname = \"Shell\"\n\n\
             [[profile]]\nid = \"auto.wsl.Gone\"\nuser = \"root\"\n",
        )
        .unwrap();
        let config = ConfigService::load(dir.dirs(), None);
        let service = ProfileService::with_found(config, vec![pwsh()]);

        let ids: Vec<String> = service.list().iter().map(|p| p.id.clone()).collect();
        assert_eq!(ids, ["box", "auto.pwsh"]);
        assert_eq!(service.find("auto.pwsh").unwrap().name, "Shell");
        assert_eq!(service.default_id().as_deref(), Some("box"));
        let view = service.view();
        assert_eq!(view["default"], "box");
        assert_eq!(view["profiles"][0]["launcher"]["type"], "ssh");
        // The entry for a distribution that is not there is said, once looked.
        assert!(view["notes"][0].as_str().unwrap().contains("auto.wsl.Gone"));
        assert_eq!(service.resolve("nowhere").await, None);
    }

    #[tokio::test]
    async fn the_look_of_each_theme_a_profile_names_is_told_with_the_profiles() {
        let dir = TempDir::new("profiles-looks");
        std::fs::create_dir_all(dir.dirs().config).unwrap();
        std::fs::write(
            dir.dirs().config_file(),
            "[appearance]\ntheme = \"lume-dark\"\n\n\
             [[profile]]\nid = \"bright\"\nlauncher = \"shell\"\ntheme = \"lume-light\"\n\n\
             [[profile]]\nid = \"also\"\nlauncher = \"shell\"\ntheme = \"lume-light\"\n\n\
             [[profile]]\nid = \"odd\"\nlauncher = \"shell\"\ntheme = \"no-such-theme\"\n",
        )
        .unwrap();
        let config = ConfigService::load(dir.dirs(), None);
        let service = ProfileService::with_found(Arc::clone(&config), vec![pwsh()]);
        let view = service.view();
        let looks = view["looks"].as_object().unwrap();
        // One look for each theme named, whatever number of profiles name it.
        assert_eq!(looks.keys().collect::<Vec<_>>(), ["lume-light"]);
        let light = &looks["lume-light"];
        assert_eq!(light["scheme"], "light");
        // The same tokens a page is given for its own look, under the other theme.
        let own = config.view();
        assert_eq!(
            light["tokens"].as_object().unwrap().len(),
            own["tokens"].as_object().unwrap().len()
        );
        assert_ne!(light["tokens"], own["tokens"]);
        // A theme there is not is said, and the profile keeps the window's look.
        let notes: Vec<&str> = view["notes"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(
            notes
                .iter()
                .any(|note| note.contains("`odd`") && note.contains("no-such-theme")),
            "{notes:?}"
        );
    }

    #[tokio::test]
    async fn before_any_look_only_the_users_own_profiles_are_there_and_nothing_waits() {
        let dir = TempDir::new("profiles-early");
        std::fs::create_dir_all(dir.dirs().config).unwrap();
        std::fs::write(
            dir.dirs().config_file(),
            "[[profile]]\nid = \"mine\"\nlauncher = \"shell\"\n\n\
             [[profile]]\nid = \"auto.pwsh\"\nhidden = true\n",
        )
        .unwrap();
        let config = ConfigService::load(dir.dirs(), None);
        let service = ProfileService::new(config);
        let ids: Vec<String> = service.list().iter().map(|p| p.id.clone()).collect();
        assert_eq!(ids, ["mine"]);
        assert_eq!(service.view()["notes"], json!([]));
        assert_eq!(service.default_id(), None);
        // Not started, so a profile that is not known is not waited for.
        assert_eq!(service.resolve("auto.pwsh").await, None);
    }

    #[tokio::test]
    async fn a_change_of_the_users_file_is_told_once() {
        let dir = TempDir::new("profiles-told");
        std::fs::create_dir_all(dir.dirs().config).unwrap();
        let config = ConfigService::load(dir.dirs(), None);
        let service = ProfileService::with_found(Arc::clone(&config), vec![pwsh()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        let _ = service.tell.set(Arc::new(move |notice| {
            let _ = sender.send(notice);
        }));

        service.config_changed();
        assert!(receiver.try_recv().is_err(), "nothing changed");

        std::fs::write(
            dir.dirs().config_file(),
            "[[profile]]\nid = \"auto.pwsh\"\nhidden = true\n",
        )
        .unwrap();
        config.reload().unwrap();
        service.config_changed();
        let notice = receiver.try_recv().unwrap();
        assert_eq!(notice["type"], "profiles_changed");
        assert_eq!(notice["profiles"][0]["hidden"], true);
        assert_eq!(notice["revision"], 2);
        service.config_changed();
        assert!(receiver.try_recv().is_err());
    }
}
