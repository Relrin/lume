//! `cargo xtask schema` and `cargo xtask tokens`: write the files that are generated
//! from the configuration's definitions.
//!
//! The settings' JSON Schema, the theme schema, the plugin manifest's schema and the
//! page's token module are kept in the repository, because editors and the page's build
//! read them as files. What goes in them is decided by the configuration and plugin
//! crates; this only puts it on disk. A test in each crate fails when a file here is not
//! what the crate would generate.

use std::fs;

use anyhow::{Context, Result};
use lume_config::generate::{GeneratedKind, generated_files};

use crate::paths;

/// Writes the generated files of one kind. A file that already holds what would be
/// written is left alone, so a second run changes nothing.
pub fn write(kind: GeneratedKind, command: &str) -> Result<()> {
    let root = paths::root();
    for file in generated_files()
        .into_iter()
        .chain(lume_plugin::manifest::generated_files())
        .filter(|file| file.kind == kind)
    {
        let path = root.join(file.path);
        // A checkout may have turned the line ends into the platform's; that is not a
        // difference worth writing the file for.
        let current = fs::read_to_string(&path)
            .map(|kept| kept.replace("\r\n", "\n"))
            .ok();
        if current.as_deref() == Some(file.text.as_str()) {
            println!("{command}: {} is up to date", file.path);
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        fs::write(&path, &file.text).with_context(|| format!("writing {}", path.display()))?;
        println!("{command}: wrote {}", file.path);
    }
    Ok(())
}
