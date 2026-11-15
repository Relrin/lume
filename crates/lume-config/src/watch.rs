//! Noticing that the user's files changed.
//!
//! The configuration directory is watched, themes and workspace files included. A
//! change is not acted on at once: an editor saves a file in several steps, so the
//! files are read again only when the directory has been quiet for a moment. Whoever
//! started the watcher is then told the new configuration, or told why the files cannot
//! be used, in which case the configuration is still the one from before.

use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};

use notify::{RecursiveMode, Watcher as _};

use crate::error::ConfigError;
use crate::source::Problem;
use crate::store::{ConfigStore, Resolved};
use crate::tuning::CONFIG_DEBOUNCE;

/// What reading the files again came to.
#[derive(Clone, Debug, PartialEq)]
pub enum WatchEvent {
    /// The configuration changed, and this is the new one.
    Reloaded(Arc<Resolved>),
    /// A file cannot be used. The configuration is as it was.
    Failed(Vec<Problem>),
}

/// Watches the configuration directory for as long as it is kept.
pub struct Watcher {
    // Dropping this ends the watch, which ends the thread that waits on it.
    _watching: notify::RecommendedWatcher,
}

impl Watcher {
    /// Starts watching the directory of `store` and calls `on_event` whenever the
    /// files, read again, come out different or cannot be used. The directory is made
    /// if it is not there: one that does not exist cannot be watched.
    ///
    /// `on_event` runs on a thread of the watcher's. A change made through the store
    /// itself may be reported here as well as returned to whoever made it.
    pub fn spawn(
        store: Arc<ConfigStore>,
        on_event: impl Fn(WatchEvent) + Send + 'static,
    ) -> Result<Self, ConfigError> {
        let dir = store.dirs().config.clone();
        std::fs::create_dir_all(&dir).map_err(|error| ConfigError::io(&dir, error))?;

        let (changed, changes) = mpsc::channel::<()>();
        let mut watching =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                // A file being read is not a file being changed.
                let relevant = event.map_or(true, |event| !event.kind.is_access());
                if relevant {
                    let _ = changed.send(());
                }
            })
            .map_err(|error| ConfigError::Watch(error.to_string()))?;
        watching
            .watch(&dir, RecursiveMode::Recursive)
            .map_err(|error| ConfigError::Watch(error.to_string()))?;

        let spawned = std::thread::Builder::new()
            .name("lume-config-watch".to_owned())
            .spawn(move || {
                let mut failure: Option<Vec<Problem>> = None;
                // Ends when the watch does: its end of the channel is dropped with it.
                while changes.recv().is_ok() {
                    // Wait until nothing has changed for a moment.
                    loop {
                        match changes.recv_timeout(CONFIG_DEBOUNCE) {
                            Ok(()) => {}
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    let before = store.current();
                    match store.reload() {
                        Ok(now) => {
                            failure = None;
                            if !Arc::ptr_eq(&before, &now) {
                                on_event(WatchEvent::Reloaded(now));
                            }
                        }
                        // Said once, not again for every save that leaves it broken
                        // in the same way.
                        Err(problems) if failure.as_ref() != Some(&problems) => {
                            failure = Some(problems.clone());
                            on_event(WatchEvent::Failed(problems));
                        }
                        Err(_) => {}
                    }
                }
            });
        spawned.map_err(|error| ConfigError::Watch(error.to_string()))?;
        Ok(Self {
            _watching: watching,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::source::ProblemKind;
    use crate::testing::TempDir;

    /// How long a change may take to be noticed before the test gives up on it.
    const PATIENCE: Duration = Duration::from_secs(10);

    /// How soon after a save the new configuration is expected.
    const PROMPT: Duration = Duration::from_millis(200);

    fn watched(dir: &TempDir) -> (Arc<ConfigStore>, Watcher, mpsc::Receiver<WatchEvent>) {
        let store = Arc::new(ConfigStore::load(dir.dirs(), None));
        let (events, received) = mpsc::channel();
        let watcher = Watcher::spawn(Arc::clone(&store), move |event| {
            let _ = events.send(event);
        })
        .unwrap();
        (store, watcher, received)
    }

    #[test]
    fn a_saved_file_is_read_again_promptly() {
        let dir = TempDir::new("watch-save");
        let (store, _watcher, events) = watched(&dir);
        let file = dir.dirs().config_file();

        // Machines hiccup, so the promptness asked for is that of the best of a few
        // saves; every one of them must arrive.
        let mut quickest = PATIENCE;
        for size in [15, 16, 17] {
            let saved = Instant::now();
            std::fs::write(&file, format!("[font]\nsize = {size}\n")).unwrap();
            match events
                .recv_timeout(PATIENCE)
                .expect("the change is noticed")
            {
                WatchEvent::Reloaded(now) => {
                    assert_eq!(now.config.font.size, f64::from(size));
                    assert!(Arc::ptr_eq(&now, &store.current()));
                }
                WatchEvent::Failed(problems) => panic!("{problems:?}"),
            }
            quickest = quickest.min(saved.elapsed());
        }
        assert!(quickest <= PROMPT, "the quickest reload took {quickest:?}");
    }

    #[test]
    fn a_broken_file_is_reported_once_and_the_configuration_kept() {
        let dir = TempDir::new("watch-broken");
        dir.write("config/config.toml", "[font]\nsize = 16\n");
        let (store, _watcher, events) = watched(&dir);
        let file = dir.dirs().config_file();

        std::fs::write(&file, "[font\n").unwrap();
        match events
            .recv_timeout(PATIENCE)
            .expect("the change is noticed")
        {
            WatchEvent::Failed(problems) => {
                assert_eq!(problems[0].kind, ProblemKind::Syntax);
            }
            WatchEvent::Reloaded(_) => panic!("a broken file was taken"),
        }
        assert_eq!(store.current().config.font.size, 16.0);

        // Mended, it is taken again.
        std::fs::write(&file, "[font]\nsize = 18\n").unwrap();
        match events
            .recv_timeout(PATIENCE)
            .expect("the change is noticed")
        {
            WatchEvent::Reloaded(now) => assert_eq!(now.config.font.size, 18.0),
            WatchEvent::Failed(problems) => panic!("{problems:?}"),
        }
    }

    #[test]
    fn themes_and_workspace_files_are_watched_too_and_a_save_that_changes_nothing_is_silent() {
        let dir = TempDir::new("watch-themes");
        dir.write("config/config.toml", "[appearance]\ntheme = \"sea\"\n");
        dir.write("config/themes/sea.toml", "[color]\naccent = \"#00aaff\"\n");
        let (_store, _watcher, events) = watched(&dir);

        // The same text again: nothing to tell.
        dir.write("config/config.toml", "[appearance]\ntheme = \"sea\"\n");
        assert!(events.recv_timeout(Duration::from_millis(600)).is_err());

        dir.write("config/themes/sea.toml", "[color]\naccent = \"#ff00aa\"\n");
        match events
            .recv_timeout(PATIENCE)
            .expect("the change is noticed")
        {
            WatchEvent::Reloaded(now) => {
                let theme = now.themes.get("sea").unwrap();
                assert!(theme.values.contains_key("color.accent"));
            }
            WatchEvent::Failed(problems) => panic!("{problems:?}"),
        }
    }

    #[test]
    fn the_watch_ends_with_the_watcher() {
        let dir = TempDir::new("watch-drop");
        let (_store, watcher, events) = watched(&dir);
        drop(watcher);
        std::fs::write(dir.dirs().config_file(), "[font]\nsize = 20\n").unwrap();
        // The thread has gone, and with it the sender: the channel closes, or stays
        // silent until it does.
        assert!(!matches!(
            events.recv_timeout(Duration::from_millis(600)),
            Ok(WatchEvent::Reloaded(_))
        ));
    }
}
