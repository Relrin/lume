//! `cargo xtask <command>`: build orchestration for Lume.
//!
//! Everything that needs more than one toolchain goes through here, so that a fresh clone
//! builds with `cargo xtask setup && cargo xtask build` on Windows, Linux and macOS.

mod artwork;
mod bench;
mod boundaries;
mod compat;
mod dist;
mod fetch;
mod generate;
mod latency;
mod logic;
mod paths;
mod rex;
mod rex_e2e;
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
  logic        Build the core's layout logic for wasm32 into packages/ui/logic/, for
               the UI harness's pretend core
  build        Build libghostty-vt if stale, then the Cargo and pnpm workspaces
  test         Run Cargo and pnpm tests
  e2e          Build, then run the suites that need the application's window: the
               corpus and the renderer snapshots inside its webview, real shells
               driven through `lume ctl`, and a Rex server that is not to be trusted
  rex-e2e [<test>]
               Build, then drive the application against real Rex servers started for
               the tests (needs Rex on this machine; no build machine has it)
  smoke        Build, then start the application under WebDriver, type through its
               window and see the text arrive (needs tauri-driver; Windows and Linux)
  bench [--profile <name>]
               Build for release, measure, and hold the numbers to the limits in
               tests/bench/budgets.toml (profile `reference` unless another is named)
  bench --startup
               Build for release and time only the application's starts, printing
               each start's marks and their middle values; nothing is judged
  bench --views
               Build for release and time only what plugins' views cost: a stream
               read by a view, and the window's answers beside a view that is in
               the way; nothing is judged
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
  types        Write the page's copy of the core's wire types into
               packages/ui/src/wire/
  api          Write the plugin API's typings (packages/plugin-api), the runtime's
               prelude, the API reference (docs/plugins/api.md) and the window's
               list of places plugins add to
  api --pack   The same, then pack @lume/plugin-api into target/plugin-api/ and
               @lume/plugin-kit into target/plugin-kit/ for a plugin's own
               repository to install
  rex-api <dump> <revision>
               Keep the shapes of a Rex server's methods, from the output of
               `rex api call help.all`, in crates/lume-rex/api/<revision>.json
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
        ["logic"] => logic::wasm(),
        ["build"] => build(),
        ["test"] => test(),
        ["e2e"] => {
            build()?;
            suites::end_to_end()
        }
        ["rex-e2e", only @ ..] if only.len() <= 1 => {
            build()?;
            rex_e2e::run(only.first().copied())
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
        ["bench", "--views"] => {
            build_release_for_testing()?;
            bench::views_only()
        }
        ["bench", "--startup"] => {
            build_release_for_testing()?;
            bench::startup_only()
        }
        ["latency", rest @ ..] => latency::run(rest),
        ["schema"] => generate::write(GeneratedKind::Schema, "schema"),
        ["tokens"] => generate::write(GeneratedKind::Tokens, "tokens"),
        ["api"] => generate::write(GeneratedKind::Api, "api"),
        ["types"] => generate::write(GeneratedKind::Types, "types"),
        ["api", "--pack"] => {
            generate::write(GeneratedKind::Api, "api")?;
            generate::pack_api()
        }
        ["rex-api", dump, revision] => rex::capture(dump, revision),
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
    logic::wasm()?;
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
    logic::wasm()?;
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
    logic::wasm()?;
    // Every test binary runs even after one has failed: a run on a system nobody has
    // at hand should say everything that is wrong there, not only the first thing.
    run::cargo(&["test", "--workspace", "--no-fail-fast"])?;
    run::pnpm(&["--recursive", "--if-present", "run", "test"])?;
    Ok(())
}

fn lint() -> Result<()> {
    boundaries::check()?;
    artwork::check()?;
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
    logic::wasm()?;
    // The application starts the session host from beside itself, and the Tauri CLI
    // builds the application alone.
    run::cargo(&["build", "-p", "lume-host"])?;
    // The Tauri CLI finds `crates/lume-app/tauri.conf.json` by searching from the root.
    run::pnpm(&["exec", "tauri", "dev"])
}
