//! Locations inside the repository.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// The repository root: the parent of this crate's directory.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the repository root")
        .to_path_buf()
}

/// Downloaded and unpacked tools. Git-ignored.
pub fn tools_dir() -> PathBuf {
    root().join(".tools")
}

/// Unpacked libghostty-vt source and dependency tarballs. Git-ignored.
pub fn vendor_dir() -> PathBuf {
    root().join("vendor").join("ghostty")
}

/// Where libghostty-vt build output for `triple` is installed.
pub fn vt_out(triple: &str) -> PathBuf {
    root().join("target").join("vt").join(triple)
}

/// The variables that tell the application where its configuration and its saved
/// workspaces are, in place of the user's own folders.
const CONFIG_DIR_VARIABLE: &str = "LUME_CONFIG_DIR";
const STATE_DIR_VARIABLE: &str = "LUME_STATE_DIR";

/// Folders for the configuration and the saved workspaces of an application that is
/// started only to be tested or measured, so that it reads and writes nothing of the
/// user's. They are under `target`, and are emptied each time they are asked for.
pub struct PrivateDirs {
    config: PathBuf,
    state: PathBuf,
}

impl PrivateDirs {
    /// Fresh, empty folders. `name` tells one run's folders from another's.
    pub fn fresh(name: &str) -> Result<Self> {
        let base = root().join("target").join("private").join(name);
        if base.exists() {
            std::fs::remove_dir_all(&base)
                .with_context(|| format!("emptying {}", base.display()))?;
        }
        let dirs = Self {
            config: base.join("config"),
            state: base.join("state"),
        };
        for dir in [&dirs.config, &dirs.state] {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        Ok(dirs)
    }

    /// The settings file of an application that uses these folders.
    pub fn config_file(&self) -> PathBuf {
        self.config.join("config.toml")
    }

    /// Has a program that is about to be started, and whatever it starts, use these
    /// folders.
    pub fn apply(&self, command: &mut Command) {
        command
            .env(CONFIG_DIR_VARIABLE, &self.config)
            .env(STATE_DIR_VARIABLE, &self.state);
    }
}

/// The Rust host triple, for example `x86_64-pc-windows-msvc`.
pub fn host_triple() -> Result<String> {
    let out = Command::new("rustc")
        .arg("-vV")
        .output()
        .context("running `rustc -vV`")?;
    let text = String::from_utf8(out.stdout).context("rustc output is not UTF-8")?;
    for line in text.lines() {
        if let Some(triple) = line.strip_prefix("host: ") {
            return Ok(triple.trim().to_owned());
        }
    }
    bail!("`rustc -vV` did not report a host triple")
}

/// The key used in `tools.toml` for the host's Zig archive, for example `x86_64-windows`.
pub fn zig_platform() -> Result<&'static str> {
    Ok(match (std::env::consts::ARCH, std::env::consts::OS) {
        ("x86_64", "windows") => "x86_64-windows",
        ("x86_64", "linux") => "x86_64-linux",
        ("aarch64", "linux") => "aarch64-linux",
        ("aarch64", "macos") => "aarch64-macos",
        ("x86_64", "macos") => "x86_64-macos",
        (arch, os) => bail!("no pinned Zig archive for {arch}-{os}; add one to tools.toml"),
    })
}
