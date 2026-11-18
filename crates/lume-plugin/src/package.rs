//! Reading a package from where the user has it: a folder, or a `.tar.gz` archive.
//!
//! The package is copied into a folder of Lume's own before anything else is done with
//! it, and only plain files and folders are copied. A link, a device, a path that climbs
//! out with `..`, a name Windows cannot hold or two names that differ only in case would
//! each mean the installed package is not what the user looked at, on some system, so
//! each stops the install. So does a package too large to be a plugin. Nothing in the
//! package is run.
//!
//! An archive may hold the package itself or one folder that holds it, as an archive of
//! a repository does.

use std::collections::BTreeSet;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};

use crate::manifest::MANIFEST_FILE;

/// The most files a package may have.
pub const FILES_MAX: usize = 4000;

/// The most a package may hold, unpacked.
pub const SIZE_MAX: u64 = 128 * 1024 * 1024;

/// Folders of a working copy that are never part of a package: version control, and
/// the packages a build installs. A plugin's code is bundled into its own files.
pub const LEFT_OUT: [&str; 4] = [".git", ".hg", ".svn", "node_modules"];

/// The names Windows keeps for devices, which no file may have, with any extension.
const DEVICE_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Why a package cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    #[error("{path}: {error}")]
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
    #[error("{0} is neither a folder nor a .tar.gz archive")]
    NotAPackage(PathBuf),
    #[error("{what} cannot be in a plugin's package: {path}")]
    Refused { path: String, what: &'static str },
    #[error("`{0}` cannot be a file's name on every system Lume runs on")]
    BadName(String),
    #[error("`{0}` and `{1}` differ only in case, which not every system can hold")]
    SameName(String, String),
    #[error(
        "the package has more than {FILES_MAX} files; a plugin's package holds its built files only"
    )]
    TooManyFiles,
    #[error("the package holds more than {} MiB; a plugin's package holds its built files only", SIZE_MAX / 1024 / 1024)]
    TooLarge,
    #[error("there is no {MANIFEST_FILE} in the package")]
    NoManifest,
    #[error("the archive cannot be read: {0}")]
    Archive(String),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> PackageError + '_ {
    move |error| PackageError::Io {
        path: path.to_owned(),
        error,
    }
}

/// Checks one part of a path in a package.
fn check_part(part: &str) -> Result<(), PackageError> {
    let stem = part.split('.').next().unwrap_or(part).to_ascii_lowercase();
    let bad = part.is_empty()
        || part.ends_with(['.', ' '])
        || part
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\'))
        || DEVICE_NAMES.contains(&stem.as_str());
    if bad {
        Err(PackageError::BadName(part.to_owned()))
    } else {
        Ok(())
    }
}

/// What has been copied so far, held to the limits.
struct Copying {
    into: PathBuf,
    files: usize,
    size: u64,
    /// Every path in lower case, to catch two that differ only in case.
    seen: BTreeSet<String>,
    left_out: BTreeSet<String>,
}

impl Copying {
    fn new(into: &Path) -> Result<Self, PackageError> {
        std::fs::create_dir_all(into).map_err(io(into))?;
        Ok(Self {
            into: into.to_owned(),
            files: 0,
            size: 0,
            seen: BTreeSet::new(),
            left_out: BTreeSet::new(),
        })
    }

    /// Checks a relative path written with `/`, and notes it.
    fn path(&mut self, relative: &str) -> Result<PathBuf, PackageError> {
        let mut path = PathBuf::new();
        for part in relative.split('/') {
            check_part(part)?;
            path.push(part);
        }
        let lower = relative.to_lowercase();
        if let Some(other) = self.seen.iter().find(|seen| **seen == lower) {
            return Err(PackageError::SameName(other.clone(), relative.to_owned()));
        }
        self.seen.insert(lower);
        Ok(path)
    }

    fn folder(&mut self, relative: &str) -> Result<(), PackageError> {
        let inside = self.path(relative)?;
        let path = self.into.join(inside);
        std::fs::create_dir_all(&path).map_err(io(&path))
    }

    /// Copies one file from `from`, which says it is `length` bytes long.
    fn file(
        &mut self,
        relative: &str,
        length: u64,
        from: &mut dyn Read,
    ) -> Result<(), PackageError> {
        self.files += 1;
        if self.files > FILES_MAX {
            return Err(PackageError::TooManyFiles);
        }
        self.size = self.size.saturating_add(length);
        if self.size > SIZE_MAX {
            return Err(PackageError::TooLarge);
        }
        let inside = self.path(relative)?;
        let path = self.into.join(inside);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        let mut file = std::fs::File::create_new(&path).map_err(io(&path))?;
        // No more than it said: an archive's header and its data must agree.
        let copied = std::io::copy(&mut from.take(length + 1), &mut file).map_err(io(&path))?;
        if copied != length {
            return Err(PackageError::Archive(format!(
                "{relative} is not as long as it says"
            )));
        }
        file.flush().map_err(io(&path))?;
        Ok(())
    }
}

/// A package copied into a folder of Lume's own.
#[derive(Debug)]
pub struct Staged {
    /// The folder the package is in: the one with its manifest.
    pub root: PathBuf,
    /// The folder everything was copied into, to be removed when the package is not
    /// kept. The root is inside it, or is it.
    pub folder: PathBuf,
    /// The folders that were left out, by their paths in the package.
    pub left_out: Vec<String>,
}

/// Copies the package at `source` into `into`, which must not exist yet.
pub fn stage(source: &Path, into: &Path) -> Result<Staged, PackageError> {
    let metadata = std::fs::metadata(source).map_err(io(source))?;
    let mut copying = Copying::new(into)?;
    if metadata.is_dir() {
        copy_folder(source, &mut copying)?;
    } else if is_archive(source) {
        unpack(source, &mut copying)?;
    } else {
        return Err(PackageError::NotAPackage(source.to_owned()));
    }
    let root = package_root(into)?;
    Ok(Staged {
        root,
        folder: into.to_owned(),
        left_out: copying.left_out.into_iter().collect(),
    })
}

/// Whether a file is named as a `.tar.gz` archive.
pub fn is_archive(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    name.ends_with(".tar.gz") || name.ends_with(".tgz")
}

/// The folder with the manifest: the one copied into, or the only folder in it when
/// that holds nothing else.
fn package_root(folder: &Path) -> Result<PathBuf, PackageError> {
    if folder.join(MANIFEST_FILE).is_file() {
        return Ok(folder.to_owned());
    }
    let entries: Vec<_> = std::fs::read_dir(folder)
        .map_err(io(folder))?
        .collect::<Result<_, _>>()
        .map_err(io(folder))?;
    if let [only] = entries.as_slice() {
        let inner = only.path();
        if inner.is_dir() && inner.join(MANIFEST_FILE).is_file() {
            return Ok(inner);
        }
    }
    Err(PackageError::NoManifest)
}

fn copy_folder(source: &Path, copying: &mut Copying) -> Result<(), PackageError> {
    let mut folders = vec![String::new()];
    while let Some(relative) = folders.pop() {
        let folder = if relative.is_empty() {
            source.to_owned()
        } else {
            source.join(&relative)
        };
        let mut entries: Vec<_> = std::fs::read_dir(&folder)
            .map_err(io(&folder))?
            .collect::<Result<_, _>>()
            .map_err(io(&folder))?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = if relative.is_empty() {
                name.clone()
            } else {
                format!("{relative}/{name}")
            };
            let kind = entry.file_type().map_err(io(&entry.path()))?;
            if kind.is_dir() {
                if LEFT_OUT.contains(&name.as_str()) {
                    copying.left_out.insert(path);
                    continue;
                }
                copying.folder(&path)?;
                folders.push(path);
            } else if kind.is_file() {
                let file_path = entry.path();
                let mut file = std::fs::File::open(&file_path).map_err(io(&file_path))?;
                let length = file.metadata().map_err(io(&file_path))?.len();
                copying.file(&path, length, &mut file)?;
            } else {
                return Err(PackageError::Refused {
                    path,
                    what: "a link or a device",
                });
            }
        }
    }
    Ok(())
}

fn unpack(source: &Path, copying: &mut Copying) -> Result<(), PackageError> {
    let file = std::fs::File::open(source).map_err(io(source))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let entries = archive
        .entries()
        .map_err(|error| PackageError::Archive(error.to_string()))?;
    for entry in entries {
        let mut entry = entry.map_err(|error| PackageError::Archive(error.to_string()))?;
        // An archive's paths are written with `/` whatever system made it, and are read
        // that way on every system, so that a name means the same everywhere.
        let shown = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        let leaves = || PackageError::Refused {
            path: shown.clone(),
            what: "a path that leaves the package",
        };
        if shown.starts_with('/') {
            return Err(leaves());
        }
        let mut parts = Vec::new();
        for part in shown.split('/') {
            match part {
                "" | "." => {}
                ".." => return Err(leaves()),
                other => parts.push(other.to_owned()),
            }
        }
        if parts.is_empty() {
            continue;
        }
        if parts.iter().any(|part| LEFT_OUT.contains(&part.as_str())) {
            copying.left_out.insert(parts.join("/"));
            continue;
        }
        let relative = parts.join("/");
        match entry.header().entry_type() {
            tar::EntryType::Directory => copying.folder(&relative)?,
            tar::EntryType::Regular | tar::EntryType::Continuous => {
                let length = entry
                    .header()
                    .size()
                    .map_err(|error| PackageError::Archive(error.to_string()))?;
                copying.file(&relative, length, &mut entry)?;
            }
            tar::EntryType::Symlink | tar::EntryType::Link => {
                return Err(PackageError::Refused {
                    path: relative,
                    what: "a link",
                });
            }
            _ => {
                return Err(PackageError::Refused {
                    path: relative,
                    what: "something that is not a file or a folder",
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testing::TempDir;

    /// An archive with the entries given: (path, type, contents).
    fn archive(dir: &TempDir, name: &str, entries: &[(&str, tar::EntryType, &str)]) -> PathBuf {
        let path = dir.path().join(name);
        let file = std::fs::File::create(&path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        for (entry_path, kind, contents) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_mode(0o644);
            header.set_size(if *kind == tar::EntryType::Regular {
                contents.len() as u64
            } else {
                0
            });
            if matches!(kind, tar::EntryType::Symlink | tar::EntryType::Link) {
                header.set_link_name(contents).unwrap();
            }
            // Written by hand, so that paths a builder would refuse can be tested.
            let name = entry_path.as_bytes();
            let gnu = header.as_gnu_mut().unwrap();
            gnu.name[..name.len()].copy_from_slice(name);
            header.set_cksum();
            let data: &[u8] = if *kind == tar::EntryType::Regular {
                contents.as_bytes()
            } else {
                &[]
            };
            builder.append(&header, data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
        path
    }

    const REGULAR: tar::EntryType = tar::EntryType::Regular;

    /// The entries of an archive: path, type and contents.
    type Entries<'a> = Vec<(&'a str, tar::EntryType, &'a str)>;

    #[test]
    fn a_folder_is_copied_without_version_control_or_installed_packages() {
        let dir = TempDir::new("stage-folder");
        dir.write("src/plugin.toml", "x");
        dir.write("src/dist/host.js", "run()");
        dir.write("src/.git/HEAD", "ref");
        dir.write("src/node_modules/a/index.js", "");
        let staged = stage(&dir.path().join("src"), &dir.path().join("out")).unwrap();
        assert_eq!(staged.root, dir.path().join("out"));
        assert_eq!(
            crate::hash::files(&staged.root).unwrap(),
            ["dist/host.js", "plugin.toml"]
        );
        assert_eq!(staged.left_out, [".git", "node_modules"]);
    }

    #[test]
    fn an_archive_may_hold_the_package_in_one_folder() {
        let dir = TempDir::new("stage-archive");
        let path = archive(
            &dir,
            "p.tar.gz",
            &[
                ("metrics-1.0/", tar::EntryType::Directory, ""),
                ("metrics-1.0/plugin.toml", REGULAR, "x"),
                ("./metrics-1.0/host.js", REGULAR, "run()"),
            ],
        );
        let staged = stage(&path, &dir.path().join("out")).unwrap();
        assert_eq!(staged.root, dir.path().join("out").join("metrics-1.0"));
        assert_eq!(
            std::fs::read_to_string(staged.root.join("host.js")).unwrap(),
            "run()"
        );
    }

    #[test]
    fn what_could_escape_or_differ_between_systems_is_refused() {
        let dir = TempDir::new("stage-refused");
        let cases: Vec<(&str, Entries<'_>, &str)> = vec![
            (
                "up",
                vec![("../evil.js", REGULAR, "x")],
                "leaves the package",
            ),
            (
                "abs",
                vec![("/etc/evil", REGULAR, "x")],
                "leaves the package",
            ),
            (
                "link",
                vec![("a", tar::EntryType::Symlink, "/etc/passwd")],
                "a link",
            ),
            (
                "hard",
                vec![("a", tar::EntryType::Link, "plugin.toml")],
                "a link",
            ),
            ("fifo", vec![("a", tar::EntryType::Fifo, "")], "not a file"),
            ("device", vec![("aux.js", REGULAR, "x")], "every system"),
            ("colon", vec![("a:b", REGULAR, "x")], "every system"),
            ("dot", vec![("a.", REGULAR, "x")], "every system"),
            (
                "case",
                vec![("A.js", REGULAR, "x"), ("a.js", REGULAR, "x")],
                "only in case",
            ),
            ("none", vec![("host.js", REGULAR, "x")], "no plugin.toml"),
        ];
        for (name, entries, says) in cases {
            let path = archive(&dir, &format!("{name}.tar.gz"), &entries);
            let error = stage(&path, &dir.path().join(format!("out-{name}"))).unwrap_err();
            assert!(error.to_string().contains(says), "{name}: {error}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_a_folder_is_refused() {
        let dir = TempDir::new("stage-link");
        dir.write("src/plugin.toml", "x");
        std::os::unix::fs::symlink("/etc/passwd", dir.path().join("src/passwd")).unwrap();
        let error = stage(&dir.path().join("src"), &dir.path().join("out")).unwrap_err();
        assert!(error.to_string().contains("a link"), "{error}");
    }

    #[test]
    fn a_file_that_is_not_a_package_is_refused() {
        let dir = TempDir::new("stage-not");
        let path = dir.write("plugin.zip", "x");
        assert!(matches!(
            stage(&path, &dir.path().join("out")),
            Err(PackageError::NotAPackage(_))
        ));
    }
}
