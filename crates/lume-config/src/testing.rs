//! What the tests share: directories of their own to write in.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::paths::Dirs;

static NEXT: AtomicU32 = AtomicU32::new(1);

/// A directory that exists for one test and is removed when the test is over. Tests
/// never write in the user's own directories.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(what: &str) -> Self {
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "lume-config-test-{}-{number}-{what}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary directory can be made");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The configuration and state directories, inside this one.
    pub fn dirs(&self) -> Dirs {
        Dirs {
            config: self.path.join("config"),
            state: self.path.join("state"),
        }
    }

    /// Writes a file under the directory, making the directories on the way.
    pub fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("the directories can be made");
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
