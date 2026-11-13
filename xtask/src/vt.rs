//! Building libghostty-vt, natively and for WebAssembly, from one pinned version.
//!
//! The invocations follow upstream's `PACKAGING-LIBGHOSTTY.md` for the pinned release:
//! build from the release source tarball, with the dependency tarball as `--system`, and a
//! Zig global cache directory that is new and empty.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::fetch::{self, Ghostty};
use crate::tools::Tools;
use crate::{paths, run};

/// Build for the host into `target/vt/<host triple>/` (`include/`, `lib/`, and `bin/` on Windows).
pub fn native(force: bool) -> Result<()> {
    let tools = Tools::load()?;
    let triple = paths::host_triple()?;
    let out = paths::vt_out(&triple);
    let stamp = stamp_text(&tools, "native");
    if !force && is_fresh(&out, &stamp) {
        return Ok(());
    }

    let zig = fetch::ensure_zig(&tools)?;
    let ghostty = fetch::ensure_ghostty(&tools)?;
    println!(
        "vt: building libghostty-vt {} for {triple}",
        tools.ghostty_vt.version
    );
    zig_build(
        &zig,
        &ghostty,
        &out,
        &["-Doptimize=ReleaseFast", "-Dcpu=baseline"],
    )?;

    let header = out.join("include").join("ghostty").join("vt.h");
    if !header.is_file() {
        bail!("build finished but {} is missing", header.display());
    }
    fs::write(out.join(".stamp"), stamp)?;
    println!("vt: installed to {}", out.display());
    Ok(())
}

/// Build for wasm32 and place the module at `packages/vt/dist/ghostty-vt.wasm`.
pub fn wasm(force: bool) -> Result<()> {
    let tools = Tools::load()?;
    let out = paths::vt_out("wasm32-freestanding");
    let dest = paths::root()
        .join("packages")
        .join("vt")
        .join("dist")
        .join("ghostty-vt.wasm");
    let stamp = stamp_text(&tools, "wasm");
    if !force && dest.is_file() && is_fresh(&out, &stamp) {
        return Ok(());
    }

    let zig = fetch::ensure_zig(&tools)?;
    let ghostty = fetch::ensure_ghostty(&tools)?;
    println!(
        "vt: building libghostty-vt {} for wasm32",
        tools.ghostty_vt.version
    );
    zig_build(
        &zig,
        &ghostty,
        &out,
        &["-Dtarget=wasm32-freestanding", "-Doptimize=ReleaseSmall"],
    )?;

    let built = out.join("bin").join("ghostty-vt.wasm");
    if !built.is_file() {
        bail!("build finished but {} is missing", built.display());
    }
    fs::create_dir_all(dest.parent().expect("dest has a parent"))?;
    fs::copy(&built, &dest)
        .with_context(|| format!("copying {} to {}", built.display(), dest.display()))?;
    fs::write(out.join(".stamp"), stamp)?;
    println!("vt: wrote {}", dest.display());
    Ok(())
}

fn zig_build(zig: &Path, ghostty: &Ghostty, prefix: &Path, flags: &[&str]) -> Result<()> {
    // Upstream: the global cache must be a new, empty directory. With Zig 0.17 a cache that
    // already holds a package archive can crash the build instead of reporting an error.
    let cache = prefix.with_extension("zig-cache");
    if cache.exists() {
        fs::remove_dir_all(&cache).with_context(|| format!("removing {}", cache.display()))?;
    }
    fs::create_dir_all(&cache)?;
    if prefix.exists() {
        fs::remove_dir_all(prefix).with_context(|| format!("removing {}", prefix.display()))?;
    }

    let mut cmd = Command::new(zig);
    cmd.current_dir(&ghostty.source)
        .env("ZIG_GLOBAL_CACHE_DIR", &cache)
        .arg("build")
        .arg("--prefix")
        .arg(prefix)
        .arg("--system")
        .arg(&ghostty.deps)
        .args(flags);
    let result = run::check(cmd, "zig build (libghostty-vt)");
    fs::remove_dir_all(&cache).ok();
    result
}

/// Identifies what a build output was produced from. A change to either pin rebuilds.
fn stamp_text(tools: &Tools, kind: &str) -> String {
    format!(
        "{kind} ghostty-vt={} sha256={} zig={}\n",
        tools.ghostty_vt.version, tools.ghostty_vt.source_sha256, tools.zig.version
    )
}

fn is_fresh(out: &Path, stamp: &str) -> bool {
    fs::read_to_string(out.join(".stamp")).is_ok_and(|s| s == stamp)
}
