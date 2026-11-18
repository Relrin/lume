//! The hash that says whether an installed package is still what the user accepted.
//!
//! The hash covers every file of the package and nothing else: each file's path inside
//! the package, its length and the SHA-256 of its bytes, in the order of the paths. A
//! file added, removed, renamed or changed gives another hash. So does anything that is
//! not a plain file: an installed package never has links or devices in it, so one that
//! has gained one has been tampered with.

use std::fmt::Write as _;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// What a hash is written with.
pub const PREFIX: &str = "sha256:";

/// The hash of some bytes, written in hexadecimal.
pub fn hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// The hash of a file's text, as a lock file keeps it.
pub fn of_bytes(bytes: &[u8]) -> String {
    format!("{PREFIX}{}", hex(bytes))
}

/// Why a package's hash could not be worked out.
#[derive(Debug, thiserror::Error)]
pub enum HashError {
    #[error("{path}: {error}")]
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
    #[error("{0} is neither a file nor a folder")]
    NotAFile(PathBuf),
}

/// Every file under `root`, as paths inside it written with `/`, in byte order.
pub fn files(root: &Path) -> Result<Vec<String>, HashError> {
    let mut found = Vec::new();
    let mut folders = vec![PathBuf::new()];
    while let Some(relative) = folders.pop() {
        let folder = root.join(&relative);
        let entries = std::fs::read_dir(&folder).map_err(|error| HashError::Io {
            path: folder.clone(),
            error,
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| HashError::Io {
                path: folder.clone(),
                error,
            })?;
            let path = relative.join(entry.file_name());
            // The entry itself, never what a link points at.
            let kind = entry.file_type().map_err(|error| HashError::Io {
                path: root.join(&path),
                error,
            })?;
            if kind.is_dir() {
                folders.push(path);
            } else if kind.is_file() {
                found.push(slashed(&path));
            } else {
                return Err(HashError::NotAFile(root.join(path)));
            }
        }
    }
    found.sort();
    Ok(found)
}

/// A relative path written with `/`.
pub fn slashed(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// The hash of the package in `root`.
pub fn of_package(root: &Path) -> Result<String, HashError> {
    let mut whole = Sha256::new();
    for relative in files(root)? {
        let path = root.join(&relative);
        let io = |error| HashError::Io {
            path: path.clone(),
            error,
        };
        let mut file = std::fs::File::open(&path).map_err(io)?;
        let mut one = Sha256::new();
        let mut length: u64 = 0;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(io)?;
            if read == 0 {
                break;
            }
            one.update(&buffer[..read]);
            length += read as u64;
        }
        let mut line = String::new();
        let _ = write!(line, "{relative}\0{length}\0");
        for byte in one.finalize() {
            let _ = write!(line, "{byte:02x}");
        }
        line.push('\n');
        whole.update(line.as_bytes());
    }
    let mut text = String::from(PREFIX);
    for byte in whole.finalize() {
        let _ = write!(text, "{byte:02x}");
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testing::TempDir;

    #[test]
    fn the_hash_changes_with_any_file_and_only_then() {
        let dir = TempDir::new("hash");
        dir.write("plugin.toml", "id = 1");
        dir.write("dist/host.js", "run()");
        let first = of_package(dir.path()).unwrap();
        assert!(first.starts_with(PREFIX));
        assert_eq!(of_package(dir.path()).unwrap(), first);

        dir.write("dist/host.js", "run();");
        let changed = of_package(dir.path()).unwrap();
        assert_ne!(changed, first);

        dir.write("dist/host.js", "run()");
        assert_eq!(of_package(dir.path()).unwrap(), first);

        dir.write("dist/extra.js", "");
        assert_ne!(of_package(dir.path()).unwrap(), first);
        std::fs::remove_file(dir.path().join("dist/extra.js")).unwrap();

        // The same bytes under another name are another package.
        std::fs::rename(dir.path().join("dist"), dir.path().join("out")).unwrap();
        assert_ne!(of_package(dir.path()).unwrap(), first);
    }

    #[test]
    fn files_are_listed_with_slashes_in_order() {
        let dir = TempDir::new("files");
        dir.write("b.txt", "");
        dir.write("a/z.txt", "");
        dir.write("a/b/c.txt", "");
        assert_eq!(
            files(dir.path()).unwrap(),
            ["a/b/c.txt", "a/z.txt", "b.txt"]
        );
    }
}
