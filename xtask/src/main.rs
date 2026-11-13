//! `cargo xtask <command>`: build orchestration for Lume.
//!
//! Everything that needs more than one toolchain goes through here, so that a fresh clone
//! builds with `cargo xtask setup && cargo xtask build` on Windows, Linux and macOS.

mod fetch;
mod paths;
mod run;
mod tools;
mod vt;

use anyhow::{Result, bail};

const USAGE: &str = "\
cargo xtask <command>

Commands:
  setup        Fetch pinned Zig and libghostty-vt, install JS dependencies
  vt native    Build libghostty-vt for the host into target/vt/<triple>/
  vt wasm      Build libghostty-vt for wasm32 into packages/vt/dist/
  build        Build libghostty-vt if stale, then the Cargo and pnpm workspaces
  test         Run Cargo and pnpm tests
  lint         Run format, clippy, licence and JS lint checks
  dev          Run the app with hot reload
";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    match args.as_slice() {
        ["setup"] => setup(),
        ["vt", "native"] => vt::native(true),
        ["vt", "wasm"] => vt::wasm(true),
        ["build"] => build(),
        ["test"] => test(),
        ["lint"] => lint(),
        ["dev"] => dev(),
        [] | ["help"] | ["--help"] | ["-h"] => {
            print!("{USAGE}");
            Ok(())
        }
        other => bail!("unknown command: {}\n\n{USAGE}", other.join(" ")),
    }
}

fn setup() -> Result<()> {
    let tools = tools::Tools::load()?;
    fetch::ensure_zig(&tools)?;
    fetch::ensure_ghostty(&tools)?;
    run::pnpm(&["install", "--frozen-lockfile"]).or_else(|_| run::pnpm(&["install"]))?;
    println!("setup: done");
    Ok(())
}

fn build() -> Result<()> {
    vt::native(false)?;
    vt::wasm(false)?;
    run::pnpm(&["--recursive", "--if-present", "run", "build"])?;
    // `custom-protocol` makes lume-gui serve the UI built above instead of the dev server.
    run::cargo(&[
        "build",
        "--workspace",
        "--features",
        "lume-app/custom-protocol",
    ])?;
    Ok(())
}

fn test() -> Result<()> {
    vt::native(false)?;
    vt::wasm(false)?;
    run::cargo(&["test", "--workspace"])?;
    run::pnpm(&["--recursive", "--if-present", "run", "test"])?;
    Ok(())
}

fn lint() -> Result<()> {
    vt::native(false)?;
    run::cargo(&["fmt", "--all", "--check"])?;
    run::cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])?;
    if run::on_path("cargo-deny") {
        run::cargo(&["deny", "check"])?;
    } else {
        println!("lint: cargo-deny is not installed; skipping `cargo deny check` (CI runs it)");
    }
    run::pnpm(&["run", "lint"])?;
    Ok(())
}

fn dev() -> Result<()> {
    vt::native(false)?;
    vt::wasm(false)?;
    // The Tauri CLI finds `crates/lume-app/tauri.conf.json` by searching from the root.
    run::pnpm(&["exec", "tauri", "dev"])
}
