//! Tauri build step.
//!
//! Declaring the app's commands here makes Tauri generate a permission for each one, so a
//! command is callable from the webview only if a capability file grants it. Without this
//! every registered command would be allowed by default.
//!
//! The commands and the pages that may call each are listed once, in
//! `src/command_table.rs`. This step reads that table, declares its commands, and writes
//! the capability files from it before Tauri reads them.

use std::fs;
use std::path::Path;

#[path = "src/capabilities.rs"]
#[allow(dead_code)]
mod capabilities;

/// The last part of a path, as text.
macro_rules! last {
    ($last:ident) => {
        stringify!($last)
    };
    ($first:ident :: $($rest:ident)::+) => {
        last!($($rest)::+)
    };
}

/// Reads the command table as each command's name and the pages that may call it.
macro_rules! commands {
    ($($($path:ident)::+ => [$($page:ident),* $(,)?];)*) => {
        const TABLE: &[(&str, &[&str])] = &[$((last!($($path)::+), &[$(stringify!($page)),*])),*];
        const NAMES: &[&str] = &[$(last!($($path)::+)),*];
    };
}

include!("src/command_table.rs");

fn main() {
    println!("cargo:rerun-if-changed=src/command_table.rs");
    println!("cargo:rerun-if-changed=src/capabilities.rs");
    // Written only when they differ, so a build that changes nothing touches nothing.
    for capability in &capabilities::CAPABILITIES {
        let path = capabilities::file_path(capability);
        let text = capabilities::file_text(capability, TABLE);
        let kept = fs::read_to_string(&path)
            .map(|kept| kept.replace("\r\n", "\n"))
            .ok();
        if kept.as_deref() != Some(text.as_str()) {
            if let Some(parent) = Path::new(&path).parent() {
                fs::create_dir_all(parent).expect("making the capabilities directory");
            }
            fs::write(&path, text).expect("writing a capability file");
        }
    }
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(NAMES));
    tauri_build::try_build(attributes).expect("tauri build step");
}
