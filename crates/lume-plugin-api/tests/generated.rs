//! The files made from the plugin API's definitions are what the code generates.
//!
//! The typings, the runtime's prelude and the reference are kept in the repository.
//! This test fails when one of them is not what the code would write now, which is when
//! the typings would no longer match what the runtime offers. Run `cargo xtask api` to
//! write them again, or run this test with `UPDATE_GOLDEN=1`.

use std::fs;
use std::path::PathBuf;

use lume_plugin_api::generate::generated_files;

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
    let hint = "run `cargo xtask api`, or this test with UPDATE_GOLDEN=1";
    for file in generated_files() {
        assert!(
            !file.text.contains("could not be generated"),
            "{}: {}",
            file.path,
            file.text
        );
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
