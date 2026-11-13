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
