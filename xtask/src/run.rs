//! Running other tools from the repository root.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::paths;

/// Run `cargo <args>` in the repository root.
pub fn cargo(args: &[&str]) -> Result<()> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.args(args).current_dir(paths::root());
    check(cmd, &format!("cargo {}", args.join(" ")))
}

/// Run `pnpm <args>` in the repository root.
///
/// The pnpm version is pinned by `packageManager` in `package.json`. If pnpm is not on
/// `PATH`, `npx` (shipped with Node) fetches and runs exactly that version. Corepack is
/// not used: the copy bundled with Node 24 cannot launch pnpm 12.
pub fn pnpm(args: &[&str]) -> Result<()> {
    pnpm_env(args, &[])
}

/// Run `pnpm <args>` in the repository root with the given variables set as well.
pub fn pnpm_env(args: &[&str], env: &[(&str, &str)]) -> Result<()> {
    let pinned;
    let (program, prefix): (&str, Vec<&str>) = if on_path("pnpm") {
        ("pnpm", vec![])
    } else if on_path("npx") {
        pinned = format!("pnpm@{}", pinned_pnpm_version()?);
        ("npx", vec!["--yes", &pinned])
    } else {
        bail!("neither pnpm nor npx is on PATH; install Node from .node-version");
    };

    // On Windows both are `.cmd` shims, which must be started through the command processor.
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(program);
        c
    } else {
        Command::new(program)
    };
    cmd.args(prefix)
        .args(args)
        .envs(env.iter().copied())
        .current_dir(paths::root());
    check(cmd, &format!("pnpm {}", args.join(" ")))
}

/// The version in `"packageManager": "pnpm@X.Y.Z"` of the root `package.json`.
fn pinned_pnpm_version() -> Result<String> {
    let path = paths::root().join("package.json");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let marker = "\"packageManager\"";
    let rest = text
        .find(marker)
        .map(|at| &text[at + marker.len()..])
        .context("package.json has no packageManager field")?;
    let version: String = rest
        .split_once("pnpm@")
        .map(|(_, after)| after)
        .context("packageManager does not name pnpm")?
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if version.is_empty() {
        bail!("could not read the pnpm version from package.json");
    }
    Ok(version)
}

/// Whether an executable called `name` is on `PATH`.
pub fn on_path(name: &str) -> bool {
    find_on_path(name).is_some()
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
            .map(str::to_owned)
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Run `cmd`, failing with `what` in the message if it does not exit successfully.
pub fn check(mut cmd: Command, what: &str) -> Result<()> {
    let status = cmd.status().with_context(|| format!("starting `{what}`"))?;
    if !status.success() {
        bail!("`{what}` failed with {status}");
    }
    Ok(())
}
