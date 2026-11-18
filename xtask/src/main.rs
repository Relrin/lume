//! `cargo xtask <command>`: build orchestration for Lume.
//!
//! Everything that needs more than one toolchain goes through here, so that a fresh clone
//! builds with `cargo xtask setup && cargo xtask build` on Windows, Linux and macOS.

mod bench;
mod compat;
mod dist;
mod fetch;
mod generate;
mod latency;
mod paths;
mod run;
mod smoke;
mod suites;
mod tools;
mod vt;

use anyhow::{Result, bail};
use lume_config::generate::GeneratedKind;

/// The variable that makes the UI's build include the test harness page.
const UI_FOR_TESTING: &str = "LUME_HARNESS";

const USAGE: &str = "\
cargo xtask <command>

Commands:
  setup        Fetch pinned Zig and libghostty-vt, install JS dependencies
  vt native    Build libghostty-vt for the host into target/vt/<triple>/
  vt wasm      Build libghostty-vt for wasm32 into packages/vt/dist/
  build        Build libghostty-vt if stale, then the Cargo and pnpm workspaces
  test         Run Cargo and pnpm tests
  e2e          Build, then run the suites that need the application's window: the
               corpus and the renderer snapshots inside its webview, and real shells
               driven through `lume ctl`
  smoke        Build, then start the application under WebDriver, type through its
               window and see the text arrive (needs tauri-driver; Windows and Linux)
  bench [--profile <name>]
               Build for release, measure, and hold the numbers to the limits in
               tests/bench/budgets.toml (profile `reference` unless another is named)
  latency [--label <name>] [--samples <n>] [--wait <seconds>]
               Time keys from the desktop to the screen in whichever terminal has the
               focus after the wait (Windows, and Linux under X11): run `cat` or
               `cmd.exe /d` there with the cursor's blink off. The report is kept in
               target/bench/latency/<name>.json
  latency --table
               Set the kept latency reports side by side
  dist [--installers]
               Build for release and pack the application, the session host and the
               console command into a portable archive in target/dist; with
               --installers, make the platform's installers too (the bundler may fetch
               tools to do so)
  compat --host-ref <ref> | --latest-release
               Build the session host as it was at a tag, branch or commit (or the
               newest release whose host runs on its own), and run this tree's backend
               against it
  schema       Write the JSON Schemas of the settings, of themes and of plugin
               manifests into schemas/
  tokens       Write the design tokens' TypeScript module and stylesheet into
               packages/tokens/src/
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
        ["e2e"] => {
            build()?;
            suites::end_to_end()
        }
        ["smoke"] => {
            build()?;
            smoke::run()
        }
        ["dist"] => dist::run(false),
        ["compat", "--host-ref", host_ref] => compat::run(host_ref),
        ["compat", "--latest-release"] => compat::run_latest_release(),
        ["dist", "--installers"] => dist::run(true),
        ["bench"] => bench("reference"),
        ["bench", "--profile", profile] => bench(profile),
        ["latency", rest @ ..] => latency::run(rest),
        ["schema"] => generate::write(GeneratedKind::Schema, "schema"),
        ["tokens"] => generate::write(GeneratedKind::Tokens, "tokens"),
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
    // A build for development and testing: the UI also contains the page that runs the
    // test suites inside the webview. A build that is shipped is made without it.
    run::pnpm_env(
        &["--recursive", "--if-present", "run", "build"],
        &[(UI_FOR_TESTING, "1")],
    )?;
    // `custom-protocol` makes lume-gui serve the UI built above instead of the dev server.
    run::cargo(&[
        "build",
        "--workspace",
        "--features",
        "lume-app/custom-protocol",
    ])?;
    Ok(())
}

/// The release build of the application and the console command, with the test page
/// inside the application: what the benchmarks measure is the product's own code.
fn build_release_for_testing() -> Result<()> {
    vt::native(false)?;
    vt::wasm(false)?;
    run::pnpm_env(
        &["--recursive", "--if-present", "run", "build"],
        &[(UI_FOR_TESTING, "1")],
    )?;
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
    ])
}

fn bench(profile: &str) -> Result<()> {
    build_release_for_testing()?;
    bench::run(profile)
}

fn test() -> Result<()> {
    vt::native(false)?;
    vt::wasm(false)?;
    // Every test binary runs even after one has failed: a run on a system nobody has
    // at hand should say everything that is wrong there, not only the first thing.
    run::cargo(&["test", "--workspace", "--no-fail-fast"])?;
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
    // The application starts the session host from beside itself, and the Tauri CLI
    // builds the application alone.
    run::cargo(&["build", "-p", "lume-host"])?;
    // The Tauri CLI finds `crates/lume-app/tauri.conf.json` by searching from the root.
    run::pnpm(&["exec", "tauri", "dev"])
}
