//! Download, verify and unpack pinned archives.
//!
//! Every archive is checked against the SHA-256 in `tools.toml` before anything is
//! unpacked. A mismatch is an error and leaves nothing behind.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::paths;
use crate::tools::Tools;

/// The pinned Zig executable, downloading it first if needed.
pub fn ensure_zig(tools: &Tools) -> Result<PathBuf> {
    let version = &tools.zig.version;
    let dir = paths::tools_dir().join(format!("zig-{version}"));
    let exe = dir.join(if cfg!(windows) { "zig.exe" } else { "zig" });
    if exe.is_file() {
        return Ok(exe);
    }

    let platform = paths::zig_platform()?;
    let archive = tools
        .zig
        .archives
        .get(platform)
        .with_context(|| format!("tools.toml has no [zig.archives.{platform}]"))?;

    println!("setup: fetching Zig {version} for {platform}");
    let file = download(&archive.url, &archive.sha256)?;

    // The archive holds one top-level directory. Unpack beside the target, then rename,
    // so an interrupted unpack never looks like a finished install.
    let staging = paths::tools_dir().join(format!("zig-{version}.partial"));
    reset_dir(&staging)?;
    unpack(&file, &staging)?;
    let inner = single_child_dir(&staging)?;
    if dir.exists() {
        fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
    }
    fs::rename(&inner, &dir)
        .with_context(|| format!("moving {} to {}", inner.display(), dir.display()))?;
    fs::remove_dir_all(&staging).ok();

    if !exe.is_file() {
        bail!("Zig archive did not contain {}", exe.display());
    }
    Ok(exe)
}

/// The unpacked libghostty-vt source and dependency directories.
pub struct Ghostty {
    pub source: PathBuf,
    pub deps: PathBuf,
}

/// The pinned libghostty-vt source and dependency tarballs, downloading them if needed.
///
/// Upstream asks packagers to build from these release tarballs, not from a Git checkout:
/// the tarball carries `LIB_VERSION`, which is what makes `zig build` build the library,
/// and the dependency tarball lets the build run with no network access.
pub fn ensure_ghostty(tools: &Tools) -> Result<Ghostty> {
    let g = &tools.ghostty_vt;
    let vendor = paths::vendor_dir();
    let source = vendor.join(g.source_name());
    let deps = vendor.join(g.deps_name());

    if !source.join("LIB_VERSION").is_file() {
        println!("setup: fetching libghostty-vt {} source", g.version);
        let file = download(&g.source_url(), &g.source_sha256)?;
        if source.exists() {
            fs::remove_dir_all(&source)?;
        }
        fs::create_dir_all(&vendor)?;
        unpack(&file, &vendor)?;
        if !source.join("LIB_VERSION").is_file() {
            bail!("source tarball did not unpack to {}", source.display());
        }
    }

    let deps_stamp = deps.join(".lume-complete");
    if !deps_stamp.is_file() {
        println!("setup: fetching libghostty-vt {} dependencies", g.version);
        let file = download(&g.deps_url(), &g.deps_sha256)?;
        if deps.exists() {
            fs::remove_dir_all(&deps)?;
        }
        fs::create_dir_all(&vendor)?;
        unpack(&file, &vendor)?;
        if !deps.is_dir() {
            bail!("dependency tarball did not unpack to {}", deps.display());
        }
        fs::write(&deps_stamp, &g.deps_sha256)?;
    }

    Ok(Ghostty { source, deps })
}

/// Download `url` into `.tools/downloads/` and verify it. Reuses a verified earlier download.
fn download(url: &str, expected_sha256: &str) -> Result<PathBuf> {
    let name = url.rsplit('/').next().context("URL has no file name")?;
    let dir = paths::tools_dir().join("downloads");
    fs::create_dir_all(&dir)?;
    let dest = dir.join(name);

    if dest.is_file() && sha256_of(&dest)? == expected_sha256 {
        return Ok(dest);
    }

    let partial = dir.join(format!("{name}.partial"));
    let response = ureq::get(url)
        .call()
        .with_context(|| format!("requesting {url}"))?;
    let mut reader = response.into_body().into_reader();
    let mut out =
        fs::File::create(&partial).with_context(|| format!("creating {}", partial.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = reader
            .read(&mut buf)
            .with_context(|| format!("downloading {url}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n])?;
    }
    out.flush()?;
    drop(out);

    let actual = hex(&hasher.finalize());
    if actual != expected_sha256 {
        fs::remove_file(&partial).ok();
        bail!(
            "checksum mismatch for {url}\n  expected {expected_sha256}\n  actual   {actual}\n\
             Nothing was unpacked. If the pin in tools.toml is out of date, update it there."
        );
    }
    fs::rename(&partial, &dest)?;
    Ok(dest)
}

fn sha256_of(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Unpack `.zip`, `.tar.gz` or `.tar.xz` into `into` with the system `tar`.
///
/// Windows 10 and later ship bsdtar as `tar.exe`, which reads all three; Linux and macOS
/// have a `tar` that reads the two tarball formats, which are the only ones used there.
fn unpack(archive: &Path, into: &Path) -> Result<()> {
    fs::create_dir_all(into)?;
    let status = Command::new(system_tar())
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(into)
        .status()
        .context("running `tar`")?;
    if !status.success() {
        bail!("tar failed to unpack {}", archive.display());
    }
    Ok(())
}

/// On Windows, name bsdtar by its full path: a `tar` from Git's MSYS tools may come first
/// on `PATH`, and it reads neither zip archives nor `C:\` paths.
pub fn system_tar() -> PathBuf {
    if cfg!(windows)
        && let Some(root) = std::env::var_os("SystemRoot")
    {
        let tar = PathBuf::from(root).join("System32").join("tar.exe");
        if tar.is_file() {
            return tar;
        }
    }
    PathBuf::from("tar")
}

fn reset_dir(dir: &Path) -> Result<()> {
    if dir.exists() {
        fs::remove_dir_all(dir).with_context(|| format!("removing {}", dir.display()))?;
    }
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))
}

fn single_child_dir(dir: &Path) -> Result<PathBuf> {
    let mut children = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir());
    let first = children
        .next()
        .with_context(|| format!("{} is empty after unpacking", dir.display()))?;
    if children.next().is_some() {
        bail!(
            "{} holds more than one directory after unpacking",
            dir.display()
        );
    }
    Ok(first)
}
