//! The generated files in the repository are what the code generates.
//!
//! The JSON Schemas of the settings and of themes, and the token package's generated
//! module and stylesheet, are kept in the repository so that editors and the page's
//! build can read them. This test fails when one of them is not what the code would
//! write now. Run `cargo xtask schema` and `cargo xtask tokens` to write them again, or
//! run this test with `UPDATE_GOLDEN=1`.

use std::fs;
use std::path::PathBuf;

use lume_config::generate::generated_files;

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
    let hint =
        "run `cargo xtask schema` and `cargo xtask tokens`, or this test with UPDATE_GOLDEN=1";
    for file in generated_files() {
        let path = root.join(file.path);
        if update {
            fs::create_dir_all(path.parent().expect("a generated file is in a directory")).unwrap();
            fs::write(&path, &file.text).unwrap();
        }
        let kept = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}; {hint}", path.display()));
        // A checkout may have turned the line ends into the platform's.
        assert!(
            kept.replace("\r\n", "\n") == file.text,
            "{} is not what the code generates now; {hint}",
            file.path
        );
    }
}
