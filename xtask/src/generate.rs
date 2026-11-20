//! `cargo xtask schema` and `cargo xtask tokens`: write the files that are generated
//! from the configuration's definitions.
//!
//! The settings' JSON Schema, the theme schema, the plugin manifest's schema and the
//! page's token module are kept in the repository, because editors and the page's build
//! read them as files. What goes in them is decided by the configuration and plugin
//! crates; this only puts it on disk. A test in each crate fails when a file here is not
//! what the crate would generate.

use std::fs;

use anyhow::{Context, Result, bail};
use lume_config::generate::{GeneratedKind, generated_files};

use crate::paths;

/// Writes the generated files of one kind. A file that already holds what would be
/// written is left alone, so a second run changes nothing.
pub fn write(kind: GeneratedKind, command: &str) -> Result<()> {
    let root = paths::root();
    for file in generated_files()
        .into_iter()
        .chain(lume_plugin::manifest::generated_files())
        .chain(lume_plugin::places::generated_files())
        .chain(lume_plugin_api::generate::generated_files())
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

/// Packs the packages a plugin's own repository builds against, each as a tarball:
/// `@lume/plugin-api` into `target/plugin-api/`, with the typings generated now, and
/// `@lume/plugin-kit` into `target/plugin-kit/`, built as it is packed.
pub fn pack_api() -> Result<()> {
    pack("plugin-api")?;
    pack("plugin-kit")
}

/// Packs `packages/<name>` into `target/<name>/`.
fn pack(name: &str) -> Result<()> {
    let root = paths::root();
    let out = root.join("target").join(name);
    fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    let package = root.join("packages").join(name);
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.arg("/C").arg("npm");
        c
    } else {
        std::process::Command::new("npm")
    };
    cmd.args(["pack", "--pack-destination"])
        .arg(&out)
        .current_dir(&package);
    let status = cmd
        .status()
        .context("starting npm, which comes with Node")?;
    if !status.success() {
        bail!("npm pack failed in {}", package.display());
    }
    println!("api: packed @lume/{name} into {}", out.display());
    Ok(())
}
