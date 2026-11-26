//! `cargo xtask dist`: what a user is given to run.
//!
//! The release build of the application, of the session host and of the console
//! command, with the page inside the application and without the test harness, put
//! together two ways:
//!
//! - a portable archive holding the three programs, which runs from wherever it is
//!   unpacked;
//! - with `--installers`, the platform's installers as well, made by the Tauri bundler:
//!   an installer on Windows, a disk image on macOS, an AppImage, a Debian and an RPM
//!   package on Linux.
//!
//! Each is named `lume-<version>-<os>-<arch>` with the kind's own ending, carries the
//! licence and the third-party notices, and is listed with its SHA-256 in `SHA256SUMS`
//! beside them.
//!
//! Nothing is signed. The bundler fetches the tools it needs for some installers the
//! first time it makes one, which is why they are asked for and not made by default.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use sha2::{Digest, Sha256};

use crate::{fetch, notices, paths, run, vt};

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
    paths::build_dir().join("release")
}

/// The libraries the programs load at run time and that must lie beside them. On
/// Windows the terminal engine is a library of its own; elsewhere it is linked in.
fn libraries() -> Vec<&'static str> {
    if cfg!(windows) {
        vec!["ghostty-vt.dll"]
    } else {
        Vec::new()
    }
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

/// The third-party notices, as they are named wherever they are shipped.
const NOTICES: &str = "THIRD-PARTY-NOTICES.txt";

/// The programs that are shipped, each beside the others: the application looks for
/// the session host there, and the console command for both.
const PROGRAMS: [&str; 3] = ["lume-gui", "lume-host", "lume"];

/// The programs the bundler ships beside the application, as it is given them.
const SIDECARS: [&str; 2] = ["lume", "lume-host"];

/// Packs the programs and the licence into one archive beside the others.
fn portable() -> Result<PathBuf> {
    let name = package_name();
    let stage = dist_dir().join("stage");
    let folder = stage.join(&name);
    // What an earlier run staged is not part of this one.
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&folder).with_context(|| format!("creating {}", folder.display()))?;
    for program in PROGRAMS.map(exe) {
        copy(&release_dir().join(&program), &folder.join(&program))?;
    }
    // The build leaves each library beside the programs that load it.
    for library in libraries() {
        copy(&release_dir().join(library), &folder.join(library))?;
    }
    copy(&paths::root().join("LICENSE"), &folder.join("LICENSE"))?;
    copy(&dist_dir().join(NOTICES), &folder.join(NOTICES))?;

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
        "linux" => "appimage,deb,rpm",
        other => bail!("no installer is made for {other}"),
    })
}

/// Makes the platform's installers and copies them beside the portable archive.
fn installers() -> Result<Vec<PathBuf>> {
    // The bundler ships the console command and the session host beside the
    // application. It looks for each file under the name it is given with the target's
    // triple added.
    let sidecar = dist_dir().join("sidecar");
    std::fs::create_dir_all(&sidecar)?;
    let triple = paths::host_triple()?;
    for program in SIDECARS {
        copy(
            &release_dir().join(exe(program)),
            &sidecar.join(exe(&format!("{program}-{triple}"))),
        )?;
    }

    // The libraries go into the installation folder itself, beside the programs, and
    // the notices with them.
    let mut resources: serde_json::Map<String, serde_json::Value> = libraries()
        .into_iter()
        .map(|library| {
            let from = release_dir().join(library).display().to_string();
            (from, serde_json::Value::String(library.to_owned()))
        })
        .collect();
    resources.insert(
        dist_dir().join(NOTICES).display().to_string(),
        serde_json::Value::String(NOTICES.to_owned()),
    );
    let resources_config = serde_json::json!({ "bundle": { "resources": resources } }).to_string();
    let mut args = vec![
        "exec",
        "tauri",
        "build",
        "--config",
        BUNDLE_CONFIG,
        "--config",
        resources_config.as_str(),
    ];
    args.extend(["--bundles", bundles()?]);
    run::pnpm(&args)?;

    let made = release_dir().join("bundle");
    let mut copied = Vec::new();
    for kind in std::fs::read_dir(&made).with_context(|| format!("reading {}", made.display()))? {
        let kind = kind?.path();
        if !kind.is_dir() {
            continue;
        }
        for file in std::fs::read_dir(&kind)? {
            let file = file?.path();
            let ending = file
                .extension()
                .and_then(|extension| shipped_ending(&extension.to_string_lossy()));
            if let Some(ending) = ending
                && file.is_file()
            {
                let to = dist_dir().join(format!("{}{ending}", package_name()));
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

/// The ending a shipped installer is named with, by the extension the bundler gave it;
/// `None` for what the bundler leaves that is not shipped.
fn shipped_ending(extension: &str) -> Option<&'static str> {
    Some(match extension {
        "exe" => "-setup.exe",
        "dmg" => ".dmg",
        "AppImage" => ".AppImage",
        "deb" => ".deb",
        "rpm" => ".rpm",
        _ => return None,
    })
}

/// Writes `SHA256SUMS` beside the files, in the form `sha256sum -c` reads.
fn checksums(files: &[PathBuf]) -> Result<PathBuf> {
    let mut lines = String::new();
    for file in files {
        let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
        let digest = Sha256::digest(&bytes);
        let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        let name = file.file_name().context("a file name")?.to_string_lossy();
        lines.push_str(&format!("{hex}  {name}\n"));
    }
    let path = dist_dir().join("SHA256SUMS");
    std::fs::write(&path, lines).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
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
        "-p",
        "lume-host",
        "--features",
        "lume-app/custom-protocol",
    ])?;

    std::fs::create_dir_all(dist_dir())?;
    notices::write(&paths::host_triple()?, &dist_dir().join(NOTICES))?;
    let mut made = vec![portable()?];
    if with_installers {
        made.extend(installers()?);
    }
    let sums = checksums(&made)?;
    for file in made.iter().chain([&sums]) {
        println!("dist: {}", file.display());
    }
    Ok(())
}
