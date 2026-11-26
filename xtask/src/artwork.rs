//! What the application may not be drawn with, and where it says whose it is not.
//!
//! Lume is a client of the Rex server that its makers did not make. They allow one that
//! says so and uses none of their logos and icons, so no picture here is theirs. What is
//! in a picture cannot be told by looking at its bytes: this goes by what it is called,
//! in the pictures that are kept and in the lists of the marks that are drawn.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::paths;

/// Where pictures are kept, from the workspace's root.
const FOLDERS: [&str; 5] = [
    "crates",
    "docs/screenshots",
    "packages",
    "resources",
    "tests",
];

/// Folders that hold what was built or installed.
const PASSED_OVER: [&str; 3] = ["dist", "node_modules", "target"];

/// How the name of a file that is a picture ends.
const PICTURES: [&str; 9] = [
    "avif", "gif", "icns", "ico", "jpeg", "jpg", "png", "svg", "webp",
];

/// The files that say which marks are drawn, or hold them, from the workspace's root.
const MARKS: [&str; 3] = [
    "packages/ui/scripts/icons.ts",
    "packages/ui/src/icons/glyphs.generated.ts",
    "packages/ui/src/icons/tiles.ts",
];

/// Where it is said that Lume is not theirs, from the workspace's root, and the words
/// each says it in.
const SAID: [(&str, &str); 2] = [
    (
        "README.md",
        "Lume is an unofficial, third-party client of the Rex server",
    ),
    (
        "packages/ui/src/settings/About.tsx",
        "Lume is a third-party client of the Rex server",
    ),
];

/// Whether a name is one of theirs: a word of it is, or begins with it. `forex` is not,
/// and nor is a word that merely has the letters in it.
fn theirs(name: &str) -> bool {
    name.to_lowercase()
        .split(|letter: char| !letter.is_ascii_alphanumeric())
        .any(|word| word.starts_with("rex") || word.contains("superlogical"))
}

/// Whether a file is a picture of theirs, by its own name and the names of the folders
/// it is in below `folder`.
fn picture(file: &str) -> bool {
    let Some((name, ending)) = file.rsplit_once('.') else {
        return false;
    };
    PICTURES.contains(&ending.to_lowercase().as_str()) && theirs(name)
}

/// The lines of a list of marks that name one of theirs. Words about the lists are not
/// looked at.
fn marks(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| {
            !["//", "/*", "*"]
                .iter()
                .any(|about| line.starts_with(about))
        })
        .filter(|line| theirs(line))
        .collect()
}

fn walk(root: &Path, folder: &Path, found: &mut Vec<String>) -> Result<()> {
    let listed =
        std::fs::read_dir(folder).with_context(|| format!("{} is read", folder.display()))?;
    for entry in listed {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        // A folder that is a link is somewhere else's.
        if entry.file_type()?.is_dir() {
            if !PASSED_OVER.iter().any(|passed| name == *passed) {
                walk(root, &path, found)?;
            }
            continue;
        }
        let file = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        // The client's own crate and module are named for the server they speak to.
        let below = file
            .strip_prefix("crates/lume-rex/")
            .or_else(|| file.strip_prefix("crates/lume-app/src/rex/"))
            .unwrap_or(&file);
        if picture(below) {
            found.push(file);
        }
    }
    Ok(())
}

/// Fails if a picture or a mark is called by a name of theirs, or if it is no longer
/// said that Lume is not theirs.
pub fn check() -> Result<()> {
    let root = paths::root();
    for (file, said) in SAID {
        let text =
            std::fs::read_to_string(root.join(file)).with_context(|| format!("{file} is read"))?;
        if !text.contains(said) {
            bail!("{file} has to say: {said}");
        }
    }
    let mut found = Vec::new();
    for folder in FOLDERS {
        let folder = root.join(folder);
        if folder.is_dir() {
            walk(&root, &folder, &mut found)?;
        }
    }
    for file in MARKS {
        let text =
            std::fs::read_to_string(root.join(file)).with_context(|| format!("{file} is read"))?;
        found.extend(marks(&text).iter().map(|line| format!("{file}: {line}")));
    }
    if !found.is_empty() {
        found.sort();
        bail!(
            "none of the logos and icons of the Rex server's makers may be used; these are \
             called by their names: {}",
            found.join(", ")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_theirs_by_its_name_or_its_folders() {
        for theirs in [
            "packages/ui/src/rex.svg",
            "packages/ui/src/Rex-logo.PNG",
            "packages/ui/src/icons/rexserver.ico",
            "packages/ui/src/superlogical/mark.svg",
            "packages/ui/src/icon_superlogical.webp",
        ] {
            assert!(picture(theirs), "{theirs}");
        }
        for other in [
            "packages/ui/src/connect.svg",
            "packages/ui/src/forex.svg",
            "packages/ui/src/rex/model.ts",
            "packages/ui/test/rex.json",
            "tests/snapshots/rex",
        ] {
            assert!(!picture(other), "{other}");
        }
    }

    #[test]
    fn a_mark_is_theirs_where_a_list_names_it_and_not_in_words_about_the_list() {
        assert_eq!(marks("  \"redis\",\n  \"rex\",\n"), ["\"rex\","]);
        assert_eq!(marks("  rex: \"tmux\","), ["rex: \"tmux\","]);
        assert_eq!(marks("superlogical: { title: \"x\" }").len(), 1);
        assert!(marks("// not rex\n * nor superlogical\n  \"redis\",").is_empty());
    }

    #[test]
    fn the_workspace_holds_to_it() {
        check().unwrap();
    }
}
