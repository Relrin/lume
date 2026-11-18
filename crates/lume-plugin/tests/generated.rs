//! The manifest's JSON Schema in the repository is what the code generates.
//!
//! Editors complete `plugin.toml` from it, so it is kept as a file. This test fails when
//! the file is not what the code would write now. Run `cargo xtask schema` to write it
//! again, or run this test with `UPDATE_GOLDEN=1`.

use std::fs;
use std::path::PathBuf;

use lume_plugin::manifest::generated_files;

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the crate is two directories below the repository's root")
        .to_path_buf()
}

#[test]
fn the_generated_files_are_what_the_code_generates() {
    let root = repository();
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let hint = "run `cargo xtask schema`, or this test with UPDATE_GOLDEN=1";
    for file in generated_files() {
        let path = root.join(file.path);
        if update {
            fs::create_dir_all(path.parent().expect("a generated file is in a directory")).unwrap();
            fs::write(&path, &file.text).unwrap();
        }
        let kept = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}; {hint}", path.display()));
        assert!(
            kept.replace("\r\n", "\n") == file.text,
            "{} is not what the code generates now; {hint}",
            file.path
        );
    }
}
