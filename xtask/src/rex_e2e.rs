//! `cargo xtask rex-e2e [<test>]`: the application against a real Rex server.
//!
//! Rex is not something a build machine has, so this is not part of `cargo xtask e2e`
//! and runs nowhere but on a machine that has Rex installed. The tests start servers of
//! their own beside whatever runs already; this finds the program and runs them.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::run;

/// The variable that names the program, to this and to the tests.
const PROGRAM: &str = "LUME_REX_BIN";

/// The program: the one the variable names, the one on the `PATH`, or the one where its
/// installer puts it on Windows.
fn program() -> Result<PathBuf> {
    if let Some(named) = std::env::var_os(PROGRAM) {
        let named = PathBuf::from(named);
        if !named.is_file() {
            bail!("{PROGRAM} names {}, which is not there", named.display());
        }
        return Ok(named);
    }
    let name = if cfg!(windows) { "rex.exe" } else { "rex" };
    let mut folders: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    if let Some(programs) = std::env::var_os("ProgramFiles") {
        folders.push(PathBuf::from(programs).join("Rex Server"));
    }
    folders
        .into_iter()
        .map(|folder| folder.join(name))
        .find(|file| file.is_file())
        .with_context(|| {
            format!("Rex is not installed here; name its program with {PROGRAM} if it is")
        })
}

/// Runs the tests whose names hold `only`, or all of them.
pub fn run(only: Option<&str>) -> Result<()> {
    let program = program()?;
    println!("rex-e2e: {}", program.display());
    let mut tests = run::cargo_command();
    tests
        .args(["test", "-p", "lume-cli", "--test", "rex_e2e", "--"])
        .args(["--ignored", "--test-threads=1"])
        .args(only)
        .env(PROGRAM, program);
    run::check(tests, "the tests against a Rex server")
}
