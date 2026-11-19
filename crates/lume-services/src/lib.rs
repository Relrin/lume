//! What Lume offers plugins.
//!
//! A plugin's code can do nothing by itself; everything it does beyond its own data is
//! a service of the application's, and each is reached only through the permission
//! that covers it. The services live here: files ([`fs`]), programs ([`process`]), the
//! network ([`net`]), the keychain ([`secrets`]), the streams a plugin holds open
//! ([`streams`]), what a program's processes use ([`procs`]) and how busy the machine is
//! ([`metrics`]).
//!
//! Each service is given what was granted and checks the call against it itself, down
//! to the file actually opened and the address actually connected to. What it gives back
//! carries the grant that allowed it, for the audit log.

pub mod fs;
mod handle;
pub mod metrics;
pub mod net;
mod paths;
pub mod process;
pub mod procs;
pub mod scope;
pub mod secrets;
pub mod streams;

pub use scope::{Allowed, ErrorKind, Places, ServiceError};

#[cfg(test)]
pub(crate) mod testing {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    /// A folder of the test's own, removed when it is dropped.
    pub struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        pub fn new(what: &str) -> Self {
            let number = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "lume-services-test-{}-{number}-{what}",
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
        pub fn write(&self, relative: &str, bytes: &[u8]) -> PathBuf {
            let path = self.path.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("the folders can be made");
            }
            std::fs::write(&path, bytes).expect("the file can be written");
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
