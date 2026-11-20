//! The command a terminal window's page calls to list the folders on this machine, for
//! choosing one to open as a workspace.
//!
//! A path may begin with `~`, for the user's home folder. Only folders are listed, never
//! files, each with whether it is hidden and, for a git repository, the branch it is
//! on. Nothing is written, and nothing below the folder asked for is read but the few
//! bytes that name a repository's branch.

use std::fs;
use std::path::{MAIN_SEPARATOR, Path, PathBuf};

use serde::Serialize;

use crate::error::CommandError;
use crate::tuning::FOLDERS_LISTED_MAX;

/// A folder inside the one asked for.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct FolderEntry {
    name: String,
    /// Whether the folder is hidden: its name begins with a dot, or the file system
    /// marks it so.
    hidden: bool,
    /// The branch a git repository in the folder is on, or the start of its commit
    /// when it is on none.
    branch: Option<String>,
}

/// The folders inside one folder.
#[derive(Debug, Serialize)]
pub struct FolderList {
    /// The user's home folder, which `~` stands for.
    home: Option<String>,
    /// What separates the folders of a path here.
    separator: String,
    /// The folder that was listed, with `~` written out.
    path: String,
    /// Whether there is no such folder.
    missing: bool,
    /// The folders in it, by name, at most so many.
    entries: Vec<FolderEntry>,
    /// Whether there were more than are listed.
    more: bool,
    /// The branch a git repository in the folder itself is on.
    branch: Option<String>,
}

/// `~` and what follows it as a path in the home folder; anything else as it is.
fn expand(path: &str, home: Option<&Path>) -> PathBuf {
    let rest = path
        .strip_prefix('~')
        .filter(|rest| rest.is_empty() || rest.starts_with(['/', '\\']));
    match (rest, home) {
        (Some(rest), Some(home)) => home.join(rest.trim_start_matches(['/', '\\'])),
        _ => PathBuf::from(path),
    }
}

/// The branch the git repository in `folder` is on, if `folder` holds one.
fn branch_of(folder: &Path) -> Option<String> {
    let dot_git = folder.join(".git");
    let head = if dot_git.is_dir() {
        dot_git.join("HEAD")
    } else {
        // A worktree's `.git` is a file that names where the repository keeps it.
        let named = fs::read_to_string(&dot_git).ok()?;
        let target = named.trim().strip_prefix("gitdir:")?.trim();
        folder.join(target).join("HEAD")
    };
    let head = fs::read_to_string(head).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref:") {
        Some(reference) => {
            let reference = reference.trim();
            Some(
                reference
                    .strip_prefix("refs/heads/")
                    .unwrap_or(reference)
                    .to_owned(),
            )
        }
        None => Some(head.chars().take(7).collect()),
    }
}

#[cfg(windows)]
fn marked_hidden(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    /// The attribute that hides a file from Explorer.
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
}

#[cfg(not(windows))]
fn marked_hidden(_metadata: &fs::Metadata) -> bool {
    false
}

/// The folders inside `folder`, by name, at most `most` of them, and whether there
/// were more.
fn list(folder: &Path, most: usize) -> std::io::Result<(Vec<FolderEntry>, bool)> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(folder)? {
        let Ok(entry) = entry else { continue };
        // A link to a folder is followed, as Explorer and a shell do.
        let Ok(metadata) = fs::metadata(entry.path()) else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let hidden = name.starts_with('.') || marked_hidden(&metadata);
        entries.push((name, hidden, entry.path()));
    }
    entries.sort_by(|(one, ..), (other, ..)| {
        one.to_lowercase()
            .cmp(&other.to_lowercase())
            .then_with(|| one.cmp(other))
    });
    let more = entries.len() > most;
    entries.truncate(most);
    let listed = entries
        .into_iter()
        .map(|(name, hidden, path)| FolderEntry {
            branch: branch_of(&path),
            name,
            hidden,
        })
        .collect();
    Ok((listed, more))
}

fn folders(path: &str, home: Option<&Path>) -> FolderList {
    let folder = expand(path.trim(), home);
    let (entries, more, missing) = match list(&folder, FOLDERS_LISTED_MAX) {
        Ok((entries, more)) => (entries, more, false),
        Err(_) => (Vec::new(), false, true),
    };
    FolderList {
        home: home.map(|home| home.to_string_lossy().into_owned()),
        separator: MAIN_SEPARATOR.to_string(),
        path: folder.to_string_lossy().into_owned(),
        branch: if missing { None } else { branch_of(&folder) },
        missing,
        entries,
        more,
    }
}

/// The folders inside the folder at `path` on this machine. `~` is the home folder.
#[tauri::command]
pub async fn folders_list(path: String) -> Result<FolderList, CommandError> {
    tokio::task::spawn_blocking(move || folders(&path, std::env::home_dir().as_deref()))
        .await
        .map_err(|error| CommandError::internal(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn a_tilde_is_the_home_folder_and_only_at_the_start() {
        let home = Path::new("/home/kai");
        assert_eq!(expand("~", Some(home)), PathBuf::from("/home/kai"));
        assert_eq!(expand("~/src", Some(home)), home.join("src"));
        assert_eq!(expand("~\\src", Some(home)), home.join("src"));
        assert_eq!(expand("~kai", Some(home)), PathBuf::from("~kai"));
        assert_eq!(expand("/tmp", Some(home)), PathBuf::from("/tmp"));
        assert_eq!(expand("~", None), PathBuf::from("~"));
    }

    #[test]
    fn folders_are_listed_by_name_with_what_is_hidden_and_the_branch_of_a_repository() {
        let dir = TempDir::new("folders");
        let root = dir.path();
        for folder in ["web", "Api", ".config", "repo/.git", "tree"] {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        fs::write(root.join("notes.txt"), "not a folder").unwrap();
        fs::write(root.join("repo/.git/HEAD"), "ref: refs/heads/main\n").unwrap();
        // A worktree names where its repository keeps what it is on.
        fs::create_dir_all(root.join("repo/.git/worktrees/tree")).unwrap();
        fs::write(
            root.join("repo/.git/worktrees/tree/HEAD"),
            "0123456789abcdef\n",
        )
        .unwrap();
        fs::write(
            root.join("tree/.git"),
            "gitdir: ../repo/.git/worktrees/tree\n",
        )
        .unwrap();

        let listed = folders(&root.to_string_lossy(), None);
        assert!(!listed.missing);
        assert_eq!(
            listed.entries,
            vec![
                FolderEntry {
                    name: ".config".into(),
                    hidden: true,
                    branch: None
                },
                FolderEntry {
                    name: "Api".into(),
                    hidden: false,
                    branch: None
                },
                FolderEntry {
                    name: "repo".into(),
                    hidden: false,
                    branch: Some("main".into())
                },
                FolderEntry {
                    name: "tree".into(),
                    hidden: false,
                    branch: Some("0123456".into())
                },
                FolderEntry {
                    name: "web".into(),
                    hidden: false,
                    branch: None
                },
            ]
        );
        let (cut, more) = list(root, 2).unwrap();
        assert_eq!((cut.len(), more), (2, true));
    }

    #[test]
    fn a_folder_that_is_not_there_is_said_to_be_missing() {
        let dir = TempDir::new("folders-missing");
        let listed = folders(&dir.path().join("nowhere").to_string_lossy(), None);
        assert!(listed.missing);
        assert!(listed.entries.is_empty());
    }
}
