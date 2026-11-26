//! `cargo xtask logic`: build the core's layout logic for WebAssembly, for the UI
//! harness's pretend core (`crates/lume-logic-wasm`).
//!
//! The module goes to `packages/ui/logic/lume-logic.wasm`, which is not kept in the
//! repository. The harness page and the UI's tests load it from there.

use std::fs;

use anyhow::{Context, Result};

use crate::{paths, run};

/// Where the module goes, from the repository's root.
pub const OUT: &str = "packages/ui/logic/lume-logic.wasm";

/// Builds the module and puts it where the UI loads it from. Cargo leaves an
/// up-to-date build alone, so this is quick when nothing changed.
pub fn wasm() -> Result<()> {
    run::cargo(&[
        "build",
        "--package",
        "lume-logic-wasm",
        "--target",
        "wasm32-unknown-unknown",
        "--release",
    ])?;
    let root = paths::root();
    let built = root
        .join("target")
        .join("wasm32-unknown-unknown")
        .join("release")
        .join("lume_logic_wasm.wasm");
    let out = root.join(OUT);
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let fresh = fs::read(&built).with_context(|| format!("reading {}", built.display()))?;
    if fs::read(&out).ok().as_deref() != Some(fresh.as_slice()) {
        fs::write(&out, &fresh).with_context(|| format!("writing {}", out.display()))?;
        println!("logic: wrote {OUT}");
    }
    Ok(())
}
