//! Generates bindings from the libghostty-vt headers and links the library.
//!
//! This script never invokes Zig. It uses the install tree that
//! `cargo xtask vt native` produced under `target/vt/<triple>/`, so `cargo check` does not
//! rebuild C code. Set `LUME_VT_DIR` to point at a different install tree.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=LUME_VT_DIR");

    let target = env::var("TARGET").expect("TARGET is set by Cargo");
    let vt_dir = match env::var_os("LUME_VT_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => workspace_root().join("target").join("vt").join(&target),
    };

    let header = vt_dir.join("include").join("ghostty").join("vt.h");
    if !header.is_file() {
        panic!(
            "\n\nlibghostty-vt is not built for {target}.\n\
             Expected {}.\n\
             Run `cargo xtask setup` once, then `cargo xtask vt native`.\n\n",
            header.display()
        );
    }
    // The stamp changes whenever xtask rebuilds the library from a different pin.
    println!("cargo:rerun-if-changed={}", vt_dir.join(".stamp").display());

    link(&vt_dir, &target);
    generate(&vt_dir, &header);
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives at <root>/crates/lume-vt-sys")
        .to_path_buf()
}

fn link(vt_dir: &Path, target: &str) {
    let lib_dir = vt_dir.join("lib");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());

    if target.contains("windows") {
        // Upstream builds for the GNU ABI on Windows. An MSVC program links it through the
        // import library and loads `ghostty-vt.dll` at run time (PACKAGING-LIBGHOSTTY.md,
        // "Platform Support").
        println!("cargo:rustc-link-lib=dylib=ghostty-vt");
        // `cargo run` and `cargo test` put link-search directories that lie inside the
        // target directory on the dynamic library path, which is how tests find the DLL
        // wherever Cargo places their executables.
        let bin_dir = vt_dir.join("bin");
        println!("cargo:rustc-link-search=native={}", bin_dir.display());
        copy_dll_to_profile_dir(&bin_dir.join("ghostty-vt.dll"));
    } else {
        // Elsewhere, link the static library so there is nothing to ship beside the binary.
        println!("cargo:rustc-link-lib=static=ghostty-vt");
    }
}

/// Windows also looks for a DLL in the directory of the executable. Final binaries such as
/// `lume-gui.exe` land in `target/<profile>/`, so the DLL is copied there too; that lets
/// them start when launched directly instead of through Cargo. Best effort: a failure is
/// a warning, because Cargo-run binaries do not depend on it.
fn copy_dll_to_profile_dir(dll: &Path) {
    if !dll.is_file() {
        panic!("{} is missing; run `cargo xtask vt native`", dll.display());
    }
    println!("cargo:rerun-if-changed={}", dll.display());

    // OUT_DIR is somewhere under target/<profile>/build/. The layout below `build` has
    // changed between Cargo versions, so find `build` by name instead of counting levels.
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let profile_dir = out_dir
        .ancestors()
        .find(|dir| dir.file_name().is_some_and(|name| name == "build"))
        .and_then(Path::parent);
    let Some(profile_dir) = profile_dir else {
        return;
    };
    let dest = profile_dir.join("ghostty-vt.dll");
    if let Err(err) = fs::copy(dll, &dest) {
        println!(
            "cargo:warning=could not copy ghostty-vt.dll to {}: {err}",
            dest.display()
        );
    }
}

fn generate(vt_dir: &Path, header: &Path) {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));

    let bindings = bindgen::Builder::default()
        .header(header.to_string_lossy())
        .clang_arg(format!("-I{}", vt_dir.join("include").display()))
        .allowlist_function("ghostty_.*")
        .allowlist_type("Ghostty.*")
        .allowlist_var("GHOSTTY_.*")
        .prepend_enum_name(false)
        .rust_edition(bindgen::RustEdition::Edition2024)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("generating libghostty-vt bindings");

    bindings
        .write_to_file(out.join("bindings.rs"))
        .expect("writing bindings.rs");
}
