//! Every dependency is named at one exact version, so that a build today and a build of
//! the same commit next year are made of the same code, and every upgrade is a change
//! someone reviews.
//!
//! - Rust: each requirement in a manifest is `=x.y.z`. Workspace members and path
//!   dependencies are exempt.
//! - JavaScript: each dependency in a `package.json` is a bare version, or a link to a
//!   workspace package.
//! - Workflows: each action is named by the full SHA of a commit, with the version it was
//!   tagged as in a comment (`owner/repo@<sha> # v1.2.3`). Local actions are exempt.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::paths;

/// The manifests' tables that list dependencies.
const DEPENDENCY_TABLES: [&str; 4] = [
    "dependencies",
    "dev-dependencies",
    "build-dependencies",
    "workspace.dependencies",
];

/// What is wrong with one Rust requirement, if anything.
fn rust_requirement_problem(name: &str, value: &toml::Value) -> Option<String> {
    let version = match value {
        toml::Value::String(version) => Some(version.as_str()),
        toml::Value::Table(table) => {
            if table.contains_key("path")
                || table.get("workspace") == Some(&toml::Value::Boolean(true))
            {
                return None;
            }
            table.get("version").and_then(toml::Value::as_str)
        }
        _ => None,
    };
    match version {
        Some(version) if is_exact_rust(version) => None,
        Some(version) => Some(format!("{name} = {version:?} is not `=x.y.z`")),
        None => Some(format!("{name} names no version")),
    }
}

fn is_exact_rust(version: &str) -> bool {
    version
        .strip_prefix('=')
        .is_some_and(|rest| semver::Version::parse(rest.trim()).is_ok_and(|v| v.build.is_empty()))
}

/// The dependency tables of a manifest, with their names: the top-level ones, the
/// workspace's, and those of each `[target.<cfg>]`.
fn dependency_tables(manifest: &toml::Table) -> Vec<(String, &toml::Table)> {
    let mut found = Vec::new();
    add_dependency_tables(&mut found, "", manifest);
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for (cfg, table) in targets {
            if let Some(table) = table.as_table() {
                add_dependency_tables(&mut found, &format!("target.{cfg}."), table);
            }
        }
    }
    found
}

fn add_dependency_tables<'m>(
    found: &mut Vec<(String, &'m toml::Table)>,
    prefix: &str,
    table: &'m toml::Table,
) {
    for key in DEPENDENCY_TABLES {
        let mut at = Some(table);
        for part in key.split('.') {
            at = at.and_then(|t| t.get(part)).and_then(toml::Value::as_table);
        }
        if let Some(deps) = at {
            found.push((format!("{prefix}{key}"), deps));
        }
    }
}

fn rust_problems(file: &str, text: &str) -> Result<Vec<String>> {
    let manifest: toml::Table =
        toml::from_str(text).with_context(|| format!("{file} is not valid TOML"))?;
    let mut problems = Vec::new();
    for (table_name, deps) in dependency_tables(&manifest) {
        for (name, value) in deps {
            if let Some(problem) = rust_requirement_problem(name, value) {
                problems.push(format!("{file} [{table_name}]: {problem}"));
            }
        }
    }
    Ok(problems)
}

fn is_exact_js(version: &str) -> bool {
    version.starts_with("workspace:") || semver::Version::parse(version).is_ok()
}

fn js_problems(file: &str, text: &str) -> Result<Vec<String>> {
    let json: serde_json::Value =
        serde_json::from_str(text).with_context(|| format!("{file} is not valid JSON"))?;
    let mut problems = Vec::new();
    for table in [
        "dependencies",
        "devDependencies",
        "optionalDependencies",
        "peerDependencies",
    ] {
        let Some(deps) = json.get(table).and_then(serde_json::Value::as_object) else {
            continue;
        };
        // A peer dependency says what the package works with, not what is installed.
        if table == "peerDependencies" {
            continue;
        }
        for (name, version) in deps {
            let version = version.as_str().unwrap_or_default();
            if !is_exact_js(version) {
                problems.push(format!(
                    "{file} {table}: {name} = {version:?} is not an exact version"
                ));
            }
        }
    }
    Ok(problems)
}

fn is_sha(text: &str) -> bool {
    text.len() == 40
        && text
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn workflow_problems(file: &str, text: &str) -> Vec<String> {
    let mut problems = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let trimmed = line.trim_start().trim_start_matches("- ");
        let Some(rest) = trimmed.strip_prefix("uses:") else {
            continue;
        };
        let (action, comment) = match rest.split_once('#') {
            Some((action, comment)) => (action.trim(), Some(comment.trim())),
            None => (rest.trim(), None),
        };
        let action = action.trim_matches(['"', '\'']);
        if action.starts_with("./") {
            continue;
        }
        let at = number + 1;
        match action.rsplit_once('@') {
            Some((_, reference)) if is_sha(reference) => {
                if !comment.is_some_and(|c| c.starts_with('v')) {
                    problems.push(format!(
                        "{file}:{at}: {action} has no `# v<version>` comment"
                    ));
                }
            }
            _ => problems.push(format!(
                "{file}:{at}: {action} is not pinned to a commit SHA"
            )),
        }
    }
    problems
}

fn files_in(folder: &Path, wanted: impl Fn(&Path) -> bool) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let Ok(listed) = std::fs::read_dir(folder) else {
        return Ok(found);
    };
    for entry in listed {
        let path = entry?.path();
        if wanted(&path) {
            found.push(path);
        }
    }
    found.sort();
    Ok(found)
}

/// Fails if a dependency or an action is not pinned.
pub fn check() -> Result<()> {
    let root = paths::root();
    let relative = |path: &Path| {
        path.strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    };
    let read = |path: &Path| {
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
    };

    let mut problems = Vec::new();
    let mut manifests = vec![
        root.join("Cargo.toml"),
        root.join("xtask").join("Cargo.toml"),
    ];
    for folder in files_in(&root.join("crates"), |p| p.is_dir())? {
        let manifest = folder.join("Cargo.toml");
        if manifest.is_file() {
            manifests.push(manifest);
        }
    }
    for manifest in &manifests {
        problems.extend(rust_problems(&relative(manifest), &read(manifest)?)?);
    }

    let mut package_files = vec![root.join("package.json")];
    for folder in files_in(&root.join("packages"), |p| p.is_dir())? {
        let file = folder.join("package.json");
        if file.is_file() {
            package_files.push(file);
        }
    }
    for file in &package_files {
        problems.extend(js_problems(&relative(file), &read(file)?)?);
    }

    let workflows = files_in(&root.join(".github").join("workflows"), |p| {
        p.extension().is_some_and(|e| e == "yml" || e == "yaml")
    })?;
    for workflow in &workflows {
        problems.extend(workflow_problems(&relative(workflow), &read(workflow)?));
    }

    if !problems.is_empty() {
        bail!(
            "dependencies that are not pinned:\n  {}",
            problems.join("\n  ")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rust_requirement_must_be_exact() {
        let manifest = r#"
[workspace.dependencies]
lume-proto = { path = "crates/lume-proto" }
anyhow = "=1.0.104"
serde = { version = "=1.0.229", features = ["derive"] }
loose = "1"
caret = { version = "^0.4" }
metadata = "=1.2.1+spec-1.1.0"

[dependencies]
anyhow.workspace = true

[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.61" }
"#;
        let problems = rust_problems("Cargo.toml", manifest).unwrap();
        assert_eq!(problems.len(), 4, "{problems:#?}");
        for name in ["loose", "caret", "metadata", "windows-sys"] {
            assert!(
                problems.iter().any(|p| p.contains(name)),
                "{name}: {problems:#?}"
            );
        }
    }

    #[test]
    fn a_js_dependency_must_be_exact_or_a_workspace_link() {
        let file = r#"{
  "dependencies": { "react": "19.3.1", "@lume/vt": "workspace:*", "loose": "^1.0.0" },
  "devDependencies": { "tilde": "~2.0.0", "pre": "1.0.0-beta.1" },
  "peerDependencies": { "react": ">=18" }
}"#;
        let problems = js_problems("package.json", file).unwrap();
        assert_eq!(problems.len(), 2, "{problems:#?}");
        assert!(problems[0].contains("loose"));
        assert!(problems[1].contains("tilde"));
    }

    #[test]
    fn an_action_must_be_pinned_to_a_commit_with_its_version_noted() {
        let workflow = "\
steps:
  - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262 # v4.4.0
  - uses: actions/checkout@v4
  - uses: actions/cache@0057852bfaa89a56745cba8c7296529d2fc39830
  - uses: ./.github/actions/local
    uses: ./.github/workflows/ci.yml
";
        let problems = workflow_problems("ci.yml", workflow);
        assert_eq!(problems.len(), 2, "{problems:#?}");
        assert!(problems[0].contains(":3:") && problems[0].contains("not pinned"));
        assert!(problems[1].contains(":4:") && problems[1].contains("comment"));
    }
}
