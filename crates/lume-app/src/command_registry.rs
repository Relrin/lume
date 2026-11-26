//! The application's commands, registered from the one table in `command_table.rs`.
//!
//! The build step reads the same table to declare the commands to Tauri and to write the
//! capability files, so a command registered here is always declared and granted as the
//! table says, and the harness and the frame token take their lists from it too.

use tauri::Wry;
use tauri::ipc::Invoke;

use crate::capabilities;

/// The last part of a path, as text.
macro_rules! last {
    ($last:ident) => {
        stringify!($last)
    };
    ($first:ident :: $($rest:ident)::+) => {
        last!($($rest)::+)
    };
}

/// Reads the command table as each command's name with the pages that may call it, and
/// as the handler that runs them.
macro_rules! commands {
    ($($($path:ident)::+ => [$($page:ident),* $(,)?];)*) => {
        /// Every command and the pages that may call it, in the table's order.
        pub const TABLE: &[(&str, &[&str])] = &[$((last!($($path)::+), &[$(stringify!($page)),*])),*];

        /// Runs the application's commands.
        pub fn handler() -> impl Fn(Invoke<Wry>) -> bool + Send + Sync + 'static {
            tauri::generate_handler![$(crate::$($path)::+),*]
        }
    };
}

include!("command_table.rs");

/// The commands the pages named `page` may call.
pub fn granted(page: &str) -> Vec<&'static str> {
    capabilities::granted(TABLE, page)
}

/// Whether a command runs without the page's token.
pub fn is_open(command: &str) -> bool {
    granted("open").contains(&command)
}

/// The permissions a test suite's page is granted when the application runs one.
pub fn harness_permissions() -> Vec<String> {
    granted("harness")
        .into_iter()
        .map(capabilities::permission)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    /// Where the page's list of command names is kept, from the repository's root.
    const NAMES_PATH: &str = "packages/ui/src/transport/commands.generated.ts";

    fn crate_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    /// The page's list of command names, as TypeScript.
    fn names_typescript() -> String {
        let mut out = String::from(
            "// Generated from crates/lume-app/src/command_table.rs by a test of lume-app.\n\
             // Do not edit: add the command to the table and run\n\
             // `UPDATE_GOLDEN=1 cargo test -p lume-app command_registry`.\n\
             \n\
             /** Every command the core answers. */\n\
             export type CommandName =\n",
        );
        for (command, _) in TABLE {
            out.push_str(&format!("  | \"{command}\"\n"));
        }
        let end = out.trim_end().len();
        out.truncate(end);
        out.push_str(";\n");
        out
    }

    #[test]
    fn every_command_is_named_once_and_granted_to_some_page() {
        let mut seen = Vec::new();
        for (command, pages) in TABLE {
            assert!(!seen.contains(command), "{command} is listed twice");
            seen.push(*command);
            assert!(!pages.is_empty(), "{command} is granted to no page");
            for page in *pages {
                assert!(
                    ["open", "harness"].contains(page)
                        || capabilities::CAPABILITIES.iter().any(|c| c.name == *page),
                    "{command}: no page is called {page}"
                );
            }
        }
        assert!(is_open("frame_token"));
        assert!(!is_open("layout_apply"));
        assert!(harness_permissions().contains(&"allow-harness-done".to_owned()));
    }

    #[test]
    fn the_capability_files_are_what_the_table_grants() {
        for capability in &capabilities::CAPABILITIES {
            let path = crate_dir().join(capabilities::file_path(capability));
            let kept = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
                .replace("\r\n", "\n");
            assert!(
                kept == capabilities::file_text(capability, TABLE),
                "{} is not what the table grants; build the application to write it again",
                path.display()
            );
        }
    }

    #[test]
    fn the_pages_list_of_command_names_is_what_the_table_says() {
        let path = crate_dir().join("../..").join(NAMES_PATH);
        let text = names_typescript();
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            fs::write(&path, &text).unwrap();
        }
        let kept = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}; run with UPDATE_GOLDEN=1", path.display()))
            .replace("\r\n", "\n");
        assert!(
            kept == text,
            "{NAMES_PATH} is not what the table says; run `UPDATE_GOLDEN=1 cargo test -p lume-app command_registry`"
        );
    }
}
