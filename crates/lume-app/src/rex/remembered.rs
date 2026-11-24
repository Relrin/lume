//! The session last shown on each server, kept across runs of the application.

use std::path::PathBuf;

use lume_config::Dirs;
use lume_proto::BackendId;
use serde::{Deserialize, Serialize};

/// The file, in the folder of the application's state.
const FILE: &str = "rex.json";
/// How many servers are remembered. The ones shown longest ago are forgotten.
const SERVERS_MAX: usize = 64;

/// What was on show on one server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Shown {
    server: BackendId,
    session: String,
}

/// The session last shown on each server, the server shown last first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Remembered {
    shown: Vec<Shown>,
}

fn file(dirs: &Dirs) -> PathBuf {
    dirs.state.join(FILE)
}

impl Remembered {
    /// What was remembered before; nothing if that cannot be read.
    pub fn load(dirs: &Dirs) -> Self {
        let mut read: Self = std::fs::read_to_string(file(dirs))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        read.shown.truncate(SERVERS_MAX);
        read
    }

    /// Remembers this for the next run. Failing to is not worth a word to the user.
    pub fn keep(&self, dirs: &Dirs) {
        let written = serde_json::to_string_pretty(self)
            .map_err(std::io::Error::other)
            .and_then(|text| {
                std::fs::create_dir_all(&dirs.state)?;
                // Written beside the file and moved over it, so that it is never half there.
                let beside = dirs.state.join(format!("{FILE}.tmp"));
                std::fs::write(&beside, text + "\n")?;
                std::fs::rename(&beside, file(dirs))
            });
        if let Err(error) = written {
            tracing::debug!(%error, "the session on show could not be remembered");
        }
    }

    pub fn of(&self, server: &BackendId) -> Option<&str> {
        let shown = self.shown.iter().find(|shown| shown.server == *server)?;
        Some(&shown.session)
    }

    pub fn put(&mut self, server: &BackendId, session: &str) {
        self.shown.retain(|shown| shown.server != *server);
        self.shown.insert(
            0,
            Shown {
                server: server.clone(),
                session: session.to_owned(),
            },
        );
        self.shown.truncate(SERVERS_MAX);
    }

    /// Forgets a session that has ended.
    pub fn forget(&mut self, server: &BackendId, session: &str) {
        self.shown
            .retain(|shown| shown.server != *server || shown.session != session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn id(name: &str) -> BackendId {
        BackendId(format!("rex:{name}"))
    }

    #[test]
    fn the_session_last_shown_is_known_at_the_next_start() {
        let dir = TempDir::new("rex-remembered");
        let dirs = dir.dirs();
        let mut held = Remembered::load(&dirs);
        assert_eq!(held, Remembered::default());
        held.put(&id("a"), "session:1");
        held.put(&id("b"), "session:2");
        held.put(&id("a"), "session:3");
        held.keep(&dirs);
        let mut read = Remembered::load(&dirs);
        assert_eq!(read, held);
        assert_eq!(read.of(&id("a")), Some("session:3"));
        assert_eq!(read.of(&id("c")), None);
        // Only the session that ended is forgotten.
        read.forget(&id("b"), "session:9");
        assert_eq!(read.of(&id("b")), Some("session:2"));
        read.forget(&id("b"), "session:2");
        assert_eq!(read.of(&id("b")), None);

        std::fs::write(dirs.state.join(FILE), "{ not json").unwrap();
        assert_eq!(Remembered::load(&dirs), Remembered::default());
    }

    #[test]
    fn no_more_servers_are_remembered_than_may_be() {
        let mut held = Remembered::default();
        for number in 0..=SERVERS_MAX {
            held.put(&id(&number.to_string()), "session:1");
        }
        assert_eq!(held.shown.len(), SERVERS_MAX);
        assert_eq!(held.of(&id("0")), None);
        assert!(held.of(&id(&SERVERS_MAX.to_string())).is_some());
    }
}
