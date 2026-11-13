//! Replays every recording under `tests/corpus/` through the native build of libghostty-vt
//! and compares the screen dump with the recording's `expect.txt`.
//!
//! Run with `UPDATE_GOLDEN=1` to rewrite the goldens, then read the diff before keeping it.

use std::path::Path;

use lume_vt::corpus::{Recording, recordings};

#[test]
fn golden_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives at <root>/crates/lume-vt")
        .join("tests")
        .join("corpus");

    let dirs = recordings(&root).expect("list recordings");
    assert!(
        !dirs.is_empty(),
        "no recordings found under {}",
        root.display()
    );

    let mut failures = Vec::new();
    for dir in &dirs {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let result = Recording::load(dir).and_then(|r| r.check());
        if let Err(err) = result {
            failures.push(format!("{name}: {err}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} recordings failed:\n\n{}",
        failures.len(),
        dirs.len(),
        failures.join("\n\n")
    );
}
