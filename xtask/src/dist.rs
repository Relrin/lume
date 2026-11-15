//! `cargo xtask dist`: what a user is given to run.
//!
//! The release build of the application and of the console command, with the page
//! inside the application and without the test harness, put together two ways:
//!
//! - a portable archive holding the two programs, which runs from wherever it is
//!   unpacked;
//! - with `--installers`, the platform's installers as well, made by the Tauri bundler:
//!   an installer on Windows, a disk image on macOS, an AppImage and a Debian package on
//!   Linux.
//!
//! Nothing is signed. The bundler fetches the tools it needs for some installers the
//! first time it makes one, which is why they are asked for and not made by default.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::{fetch, paths, run, vt};

/// The application's configuration for the bundler, laid over the one it is built with.
const BUNDLE_CONFIG: &str = "crates/lume-app/tauri.dist.conf.json";

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

fn release_dir() -> PathBuf {
    paths::root().join("target").join("release")
}

/// Where everything made here ends up.
fn dist_dir() -> PathBuf {
    paths::root().join("target").join("dist")
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    std::fs::copy(from, to)
        .with_context(|| format!("copying {} to {}", from.display(), to.display()))?;
    Ok(())
}

/// The name of what is made: the product, its version, and what it runs on.
fn package_name() -> String {
    format!(
        "lume-{}-{}-{}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

/// Packs the two programs and the licence into one archive beside the others.
fn portable() -> Result<PathBuf> {
    let name = package_name();
    let stage = dist_dir().join("stage");
    let folder = stage.join(&name);
    // What an earlier run staged is not part of this one.
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&folder).with_context(|| format!("creating {}", folder.display()))?;
    for program in [exe("lume-gui"), exe("lume")] {
        copy(&release_dir().join(&program), &folder.join(&program))?;
    }
    copy(&paths::root().join("LICENSE"), &folder.join("LICENSE"))?;

    // The system's `tar` writes both forms; the extension picks the one each platform's
    // users expect to open without installing anything.
    let archive = dist_dir().join(if cfg!(windows) {
        format!("{name}.zip")
    } else {
        format!("{name}.tar.gz")
    });
    let _ = std::fs::remove_file(&archive);
    let mut tar = Command::new(fetch::system_tar());
    tar.arg("-a").arg("-c").arg("-f").arg(&archive);
    if cfg!(windows) {
        // Left to itself the zip writer stores the files as they are.
        tar.args(["--options", "zip:compression=deflate"]);
    }
    tar.arg("-C").arg(&stage).arg(&name);
    run::check(tar, "tar")?;
    std::fs::remove_dir_all(&stage).ok();
    Ok(archive)
}

/// The installers this platform's users expect, as the bundler names them.
fn bundles() -> Result<&'static str> {
    Ok(match std::env::consts::OS {
        "windows" => "nsis",
        "macos" => "app,dmg",
        "linux" => "appimage,deb",
        other => bail!("no installer is made for {other}"),
    })
}

/// Makes the platform's installers and copies them beside the portable archive.
fn installers() -> Result<Vec<PathBuf>> {
    // The bundler ships the console command beside the application. It looks for the
    // file under the name it is given with the target's triple added.
    let sidecar = dist_dir().join("sidecar");
    std::fs::create_dir_all(&sidecar)?;
    let triple = paths::host_triple()?;
    copy(
        &release_dir().join(exe("lume")),
        &sidecar.join(exe(&format!("lume-{triple}"))),
    )?;

    run::pnpm(&[
        "exec",
        "tauri",
        "build",
        "--config",
        BUNDLE_CONFIG,
        "--bundles",
        bundles()?,
    ])?;

    let made = release_dir().join("bundle");
    let mut copied = Vec::new();
    for kind in std::fs::read_dir(&made).with_context(|| format!("reading {}", made.display()))? {
        let kind = kind?.path();
        if !kind.is_dir() {
            continue;
        }
        for file in std::fs::read_dir(&kind)? {
            let file = file?.path();
            let wanted = file.extension().is_some_and(|extension| {
                ["exe", "dmg", "AppImage", "deb"]
                    .iter()
                    .any(|known| extension == *known)
            });
            if wanted && file.is_file() {
                let to = dist_dir().join(file.file_name().context("a file name")?);
                copy(&file, &to)?;
                copied.push(to);
            }
        }
    }
    if copied.is_empty() {
        bail!("the bundler left no installer in {}", made.display());
    }
    Ok(copied)
}

/// Builds for release and packs the result.
pub fn run(with_installers: bool) -> Result<()> {
    vt::native(false)?;
    vt::wasm(false)?;
    // The page as it is shipped: no test harness, no source maps.
    run::pnpm(&["--recursive", "--if-present", "run", "build"])?;
    run::cargo(&[
        "build",
        "--release",
        "-p",
        "lume-app",
        "-p",
        "lume-cli",
        "--features",
        "lume-app/custom-protocol",
    ])?;

    std::fs::create_dir_all(dist_dir())?;
    let mut made = vec![portable()?];
    if with_installers {
        made.extend(installers()?);
    }
    for file in &made {
        println!("dist: {}", file.display());
    }
    Ok(())
}
