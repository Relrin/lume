//! The manifests the documentation for plugin authors shows are manifests: every whole
//! one in it is read as an installed plugin's would be, so an example cannot teach a
//! key or a value the application refuses.

use std::path::PathBuf;

use lume_plugin::Manifest;

/// The folder the pages for plugin authors are in.
fn pages() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/plugins")
}

/// The TOML examples of a page, with the line each begins at.
fn examples(text: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut open: Option<(usize, String)> = None;
    for (number, line) in text.lines().enumerate() {
        match &mut open {
            None if line.trim_start().starts_with("```toml") => {
                open = Some((number + 2, String::new()));
            }
            None => {}
            Some(_) if line.trim_start().starts_with("```") => found.extend(open.take()),
            Some((_, body)) => {
                body.push_str(line);
                body.push('\n');
            }
        }
    }
    found
}

#[test]
fn every_whole_manifest_in_the_authors_pages_is_one_the_application_takes() {
    let mut whole = 0;
    let mut refused = Vec::new();
    let mut pages: Vec<PathBuf> = std::fs::read_dir(pages())
        .expect("the pages are there")
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.extension().is_some_and(|ending| ending == "md"))
        .collect();
    pages.sort();
    for page in pages {
        let text = std::fs::read_to_string(&page).expect("a page is text");
        for (line, example) in examples(&text) {
            // A piece of a manifest shows one table; a whole one begins as they all do.
            if !example.lines().any(|line| line.trim() == "schema = 1") {
                continue;
            }
            whole += 1;
            if let Err(problems) = Manifest::parse(&example) {
                let name = page.file_name().unwrap_or_default().to_string_lossy();
                for problem in problems {
                    refused.push(format!("{name}:{line}: {problem}"));
                }
            }
        }
    }
    assert!(refused.is_empty(), "{}", refused.join("\n"));
    // The pages do show whole manifests; a count of none would mean none was looked at.
    assert!(whole >= 5, "only {whole} whole manifests were found");
}
