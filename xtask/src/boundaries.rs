//! Which parts of the workspace may name which.
//!
//! The client of a Rex server is one part of the application: the rest shows sessions and
//! layouts whoever runs them. So only its own crate and the module of the application
//! that hands its connections on may name it.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::paths;

/// The crate that is kept apart, as code and as manifests write it.
const CRATE: &str = "lume_rex";
const PACKAGE: &str = "lume-rex";

/// Where the crate may be named, from the workspace's root.
fn may_name(file: &str) -> bool {
    file.starts_with("crates/lume-rex/")
        || file == "crates/lume-app/src/rex.rs"
        || file.starts_with("crates/lume-app/src/rex/")
        || file == "crates/lume-app/Cargo.toml"
}

/// Whether the text of a file names the crate: in code, or among what a manifest needs.
fn names(file: &str, text: &str) -> bool {
    if file.ends_with(".toml") {
        return text
            .lines()
            .any(|line| line.trim_start().starts_with(PACKAGE));
    }
    text.lines()
        .map(|line| line.trim_start())
        .filter(|line| !line.starts_with("//"))
        .any(|line| line.contains(CRATE))
}

fn walk(root: &Path, folder: &Path, found: &mut Vec<String>) -> Result<()> {
    let listed =
        std::fs::read_dir(folder).with_context(|| format!("{} is read", folder.display()))?;
    for entry in listed {
        let path = entry?.path();
        if path.is_dir() {
            walk(root, &path, found)?;
            continue;
        }
        let file = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        let read = file.ends_with(".rs") || file.ends_with("/Cargo.toml");
        if read && !may_name(&file) && names(&file, &std::fs::read_to_string(&path)?) {
            found.push(file);
        }
    }
    Ok(())
}

/// Fails if something names the Rex client that may not.
pub fn check() -> Result<()> {
    let root = paths::root();
    let mut found = Vec::new();
    walk(&root, &root.join("crates"), &mut found)?;
    if !found.is_empty() {
        found.sort();
        bail!(
            "only crates/lume-rex and crates/lume-app/src/rex* may name {CRATE}; these do too: {}",
            found.join(", ")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_client_and_what_hands_it_on_may_name_it() {
        for may in [
            "crates/lume-rex/src/lib.rs",
            "crates/lume-app/src/rex.rs",
            "crates/lume-app/src/rex/hosts.rs",
            "crates/lume-app/Cargo.toml",
        ] {
            assert!(may_name(may), "{may}");
        }
        for may_not in [
            "crates/lume-app/src/panes.rs",
            "crates/lume-app/src/rexical.rs",
            "crates/lume-cli/Cargo.toml",
            "crates/lume-session/src/lib.rs",
        ] {
            assert!(!may_name(may_not), "{may_not}");
        }
    }

    #[test]
    fn naming_is_in_code_or_in_a_manifest_and_not_in_words_about_it() {
        assert!(names("a.rs", "use lume_rex::Link;"));
        assert!(names("a.rs", "    let x = lume_rex::listens(y);"));
        assert!(!names(
            "a.rs",
            "// lume_rex does this\n/// and `lume_rex` that"
        ));
        assert!(names(
            "crates/a/Cargo.toml",
            "[dependencies]\nlume-rex.workspace = true"
        ));
        assert!(!names("crates/a/Cargo.toml", "# not lume-rex"));
    }

    #[test]
    fn the_workspace_holds_to_it() {
        check().unwrap();
    }
}
