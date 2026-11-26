//! `cargo xtask notices`: the third-party notices shipped with the application.
//!
//! The file lists everything the shipped programs and page are made of that is not this
//! project's own, with its licence and the licence texts, from three sources:
//!
//! - **Rust crates:** what `cargo tree` lists for the shipped programs on the target,
//!   with the features the release build uses, along normal dependencies. Build scripts,
//!   procedural macros and tests run while building and are not shipped, so their
//!   dependencies are left out. `cargo metadata` gives each crate's details. Each crate's
//!   licence files come from its package; a `-sys` crate's subfolders are searched too,
//!   since they hold the C code it builds and its licence. For a crate that publishes no
//!   licence file, the standard text of its licence is taken from
//!   `resources/notices/licenses/`.
//! - **JavaScript packages:** the production dependencies of the page, followed through
//!   the workspace's own packages and resolved through `node_modules` as Node resolves
//!   them.
//! - **Other components:** libghostty-vt and the code upstream reports it links in, and
//!   the entries of `resources/notices/components.toml` (fonts, icon drawings).
//!
//! The command fails when something shipped is under a licence that is not allowed (the
//! list in `deny.toml`, and the SIL Open Font License for fonts), or comes with no licence
//! text at all.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{paths, run, tools};

/// The programs that are shipped, by their packages.
const SHIPPED_PACKAGES: [&str; 3] = ["lume-app", "lume-cli", "lume-host"];

/// The features the shipped application is built with.
const SHIPPED_FEATURES: &str = "lume-app/custom-protocol";

/// The page's package, whose production dependencies are shipped.
const PAGE: &str = "packages/ui";

/// The licence accepted for fonts only.
const FONT_LICENSE: &str = "OFL-1.1";

/// Names of the files that hold a package's licence, matched at their start and
/// without regard to case.
const LICENSE_FILE_PREFIXES: [&str; 6] = [
    "LICENSE",
    "LICENCE",
    "COPYING",
    "COPYRIGHT",
    "NOTICE",
    "UNLICENSE",
];

/// A licence text, and where it was read.
#[derive(Clone, Debug)]
struct Text {
    source: String,
    body: String,
}

/// One thing that is shipped and is not the project's own.
#[derive(Clone, Debug)]
struct Entry {
    name: String,
    version: String,
    license: String,
    url: Option<String>,
    texts: Vec<Text>,
    /// Said above the texts, when they are not the package's own files.
    remark: Option<String>,
}

/// What the notices are made of, before they are written.
struct Notices {
    triple: String,
    components: Vec<Entry>,
    crates: Vec<Entry>,
    packages: Vec<Entry>,
}

pub fn run(args: &[&str]) -> Result<()> {
    let mut triple = None;
    let mut out = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match *arg {
            "--target" => triple = Some(rest.next().context("--target needs a triple")?),
            "--out" => out = Some(PathBuf::from(rest.next().context("--out needs a file")?)),
            other => bail!(
                "usage: cargo xtask notices [--target <triple>] [--out <file>]; not {other:?}"
            ),
        }
    }
    let host = paths::host_triple()?;
    let triple = triple.map_or(host.clone(), |t| (*t).to_owned());
    let out = out.unwrap_or_else(|| {
        let name = if triple == host {
            "THIRD-PARTY-NOTICES.txt".to_owned()
        } else {
            format!("THIRD-PARTY-NOTICES-{triple}.txt")
        };
        paths::root().join("target").join("dist").join(name)
    });
    let written = write(&triple, &out)?;
    println!("notices: {}", written.display());
    Ok(())
}

/// Gathers the notices for `triple`, checks them and writes them to `out`.
pub fn write(triple: &str, out: &Path) -> Result<PathBuf> {
    let allowed = Allowed::load()?;
    let notices = Notices {
        triple: triple.to_owned(),
        components: components()?,
        crates: crates(triple)?,
        packages: packages()?,
    };
    let mut refused = Vec::new();
    let font_names: BTreeSet<String> = font_components()?;
    for entry in notices
        .components
        .iter()
        .chain(&notices.crates)
        .chain(&notices.packages)
    {
        let font = font_names.contains(&entry.name);
        if !allowed.permits(&entry.license, font)? {
            refused.push(format!(
                "{} {} is under {}, which is not allowed",
                entry.name, entry.version, entry.license
            ));
        }
        if entry.texts.is_empty() {
            refused.push(format!(
                "{} {} ({}) has no licence text",
                entry.name, entry.version, entry.license
            ));
        }
    }
    if !refused.is_empty() {
        bail!("the notices cannot be written:\n  {}", refused.join("\n  "));
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out, render(&notices)?).with_context(|| format!("writing {}", out.display()))?;
    println!(
        "notices: {} components, {} crates, {} JavaScript packages for {triple}",
        notices.components.len(),
        notices.crates.len(),
        notices.packages.len()
    );
    Ok(out.to_path_buf())
}

// ---------------------------------------------------------------------------------------
// Licences that are allowed

/// The licences `deny.toml` allows, which the shipped JavaScript and components are held
/// to as well.
struct Allowed {
    ids: BTreeSet<String>,
}

impl Allowed {
    fn load() -> Result<Self> {
        let path = paths::root().join("deny.toml");
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let table: toml::Table = toml::from_str(&text).context("deny.toml is not valid TOML")?;
        let ids = table
            .get("licenses")
            .and_then(|licenses| licenses.get("allow"))
            .and_then(|allow| allow.as_array())
            .context("deny.toml has no [licenses] allow list")?
            .iter()
            .filter_map(|id| id.as_str().map(str::to_owned))
            .collect();
        Ok(Self { ids })
    }

    fn permits(&self, expression: &str, font: bool) -> Result<bool> {
        let parsed = Expression::parse(expression)?;
        Ok(parsed.satisfied_by(&|id: &str| self.ids.contains(id) || (font && id == FONT_LICENSE)))
    }
}

/// An SPDX licence expression: `MIT OR Apache-2.0`, `(MIT OR Apache-2.0) AND Unicode-3.0`,
/// `Apache-2.0 WITH LLVM-exception`. The older `MIT/Apache-2.0` form means OR.
#[derive(Debug, PartialEq, Eq)]
enum Expression {
    Id(String),
    And(Vec<Expression>),
    Or(Vec<Expression>),
}

impl Expression {
    fn parse(text: &str) -> Result<Self> {
        let spaced = text
            .replace('/', " OR ")
            .replace('(', " ( ")
            .replace(')', " ) ");
        let tokens: Vec<&str> = spaced.split_whitespace().collect();
        if tokens.is_empty() {
            bail!("an empty licence expression");
        }
        let mut at = 0;
        let parsed = Self::or(&tokens, &mut at)?;
        if at != tokens.len() {
            bail!("the licence expression {text:?} does not parse");
        }
        Ok(parsed)
    }

    fn or(tokens: &[&str], at: &mut usize) -> Result<Self> {
        let mut parts = vec![Self::and(tokens, at)?];
        while tokens
            .get(*at)
            .is_some_and(|t| t.eq_ignore_ascii_case("OR"))
        {
            *at += 1;
            parts.push(Self::and(tokens, at)?);
        }
        Ok(if parts.len() == 1 {
            parts.remove(0)
        } else {
            Self::Or(parts)
        })
    }

    fn and(tokens: &[&str], at: &mut usize) -> Result<Self> {
        let mut parts = vec![Self::atom(tokens, at)?];
        while tokens
            .get(*at)
            .is_some_and(|t| t.eq_ignore_ascii_case("AND"))
        {
            *at += 1;
            parts.push(Self::atom(tokens, at)?);
        }
        Ok(if parts.len() == 1 {
            parts.remove(0)
        } else {
            Self::And(parts)
        })
    }

    fn atom(tokens: &[&str], at: &mut usize) -> Result<Self> {
        let token = *tokens
            .get(*at)
            .context("a licence expression ends too soon")?;
        *at += 1;
        if token == "(" {
            let inner = Self::or(tokens, at)?;
            if tokens.get(*at) != Some(&")") {
                bail!("a licence expression is missing a `)`");
            }
            *at += 1;
            return Ok(inner);
        }
        if [")", "AND", "OR", "WITH"]
            .iter()
            .any(|k| token.eq_ignore_ascii_case(k))
        {
            bail!("a licence expression has {token:?} where a licence belongs");
        }
        if tokens
            .get(*at)
            .is_some_and(|t| t.eq_ignore_ascii_case("WITH"))
        {
            let exception = *tokens
                .get(*at + 1)
                .context("a licence expression ends after WITH")?;
            *at += 2;
            return Ok(Self::Id(format!("{token} WITH {exception}")));
        }
        Ok(Self::Id(token.to_owned()))
    }

    /// Whether the licences `allowed` accepts are enough: one side of each OR, every side
    /// of each AND. A licence with an exception is accepted when the licence alone is.
    fn satisfied_by(&self, allowed: &dyn Fn(&str) -> bool) -> bool {
        match self {
            Self::Id(id) => {
                allowed(id)
                    || id
                        .split_once(" WITH ")
                        .is_some_and(|(base, _)| allowed(base))
            }
            Self::And(parts) => parts.iter().all(|part| part.satisfied_by(allowed)),
            Self::Or(parts) => parts.iter().any(|part| part.satisfied_by(allowed)),
        }
    }

    /// Every licence the expression names, without exceptions.
    fn ids(&self, into: &mut Vec<String>) {
        match self {
            Self::Id(id) => {
                let base = id
                    .split_once(" WITH ")
                    .map_or(id.as_str(), |(base, _)| base);
                if !into.iter().any(|known| known == base) {
                    into.push(base.to_owned());
                }
            }
            Self::And(parts) | Self::Or(parts) => parts.iter().for_each(|part| part.ids(into)),
        }
    }
}

// ---------------------------------------------------------------------------------------
// Licence files

fn is_license_file(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    LICENSE_FILE_PREFIXES
        .iter()
        .any(|prefix| upper.starts_with(prefix))
}

/// Reads a licence file. Text that is not UTF-8 is read as well as it can be.
fn read_text(path: &Path, source: String) -> Result<Text> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let body = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
    Ok(Text {
        source,
        body: body.trim_end().to_owned(),
    })
}

/// The licence files directly in `folder`, and with `nested` those one folder down, in
/// name order. Each is named `<label>: <path inside the folder>`.
fn license_files(folder: &Path, label: &str, nested: bool) -> Result<Vec<Text>> {
    let mut found = Vec::new();
    let mut names: Vec<(PathBuf, String)> = Vec::new();
    let Ok(listed) = std::fs::read_dir(folder) else {
        return Ok(found);
    };
    for entry in listed {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_file() && is_license_file(&name) {
            names.push((path, name));
        } else if nested && path.is_dir() {
            for inner in std::fs::read_dir(&path)? {
                let inner = inner?;
                let inner_name = inner.file_name().to_string_lossy().into_owned();
                if inner.path().is_file() && is_license_file(&inner_name) {
                    names.push((inner.path(), format!("{name}/{inner_name}")));
                }
            }
        }
    }
    names.sort_by(|a, b| a.1.cmp(&b.1));
    for (path, name) in names {
        found.push(read_text(&path, format!("{label}: {name}"))?);
    }
    Ok(found)
}

/// The standard texts of the licences an expression names, for a package that publishes
/// none, with `holders` as the copyright holders where the text names them.
fn standard_texts(expression: &str, holders: &str, label: &str) -> Result<Vec<Text>> {
    let mut ids = Vec::new();
    Expression::parse(expression)?.ids(&mut ids);
    let folder = paths::root()
        .join("resources")
        .join("notices")
        .join("licenses");
    let mut texts = Vec::new();
    for id in ids {
        let path = folder.join(format!("{id}.txt"));
        if path.is_file() {
            let mut text = read_text(&path, format!("the standard text of {id}, for {label}"))?;
            text.body = text.body.replace("{holders}", holders);
            texts.push(text);
        }
    }
    Ok(texts)
}

const STANDARD_TEXT_REMARK: &str = "No licence file is published with this package; the standard text of its licence \
     follows, with the package's authors as the copyright holders.";

// ---------------------------------------------------------------------------------------
// Rust crates

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_members: Vec<String>,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    version: String,
    license: Option<String>,
    license_file: Option<String>,
    repository: Option<String>,
    homepage: Option<String>,
    manifest_path: PathBuf,
    #[serde(default)]
    authors: Vec<String>,
}

impl Package {
    /// The authors without their addresses, as the copyright holders.
    fn holders(&self) -> String {
        if self.authors.is_empty() {
            return format!("the authors of {}", self.name);
        }
        self.authors
            .iter()
            .map(|author| author.split('<').next().unwrap_or(author).trim())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Runs cargo with `args` and returns what it printed.
fn cargo_output(args: &[&str]) -> Result<Vec<u8>> {
    let mut cmd = run::cargo_command();
    cmd.args(args);
    let output = cmd
        .output()
        .with_context(|| format!("running `cargo {}`", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "`cargo {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(output.stdout)
}

/// The crates, as name and version, that the shipped programs are built from for
/// `triple`. `cargo tree` resolves features for the selected packages together, as the
/// release build does; `cargo metadata` would unify them with the whole workspace's and
/// list crates only the build tool uses.
fn shipped_set(triple: &str) -> Result<BTreeSet<(String, String)>> {
    let mut args = vec![
        "tree",
        "--locked",
        "--edges",
        "normal,no-proc-macro",
        "--target",
        triple,
        "--features",
        SHIPPED_FEATURES,
        "--prefix",
        "none",
        "--format",
        "{p}",
    ];
    for package in SHIPPED_PACKAGES {
        args.extend(["--package", package]);
    }
    let text = String::from_utf8(cargo_output(&args)?).context("`cargo tree` is not UTF-8")?;
    let mut set = BTreeSet::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        if let (Some(name), Some(version)) = (words.next(), words.next()) {
            let version = version.strip_prefix('v').unwrap_or(version);
            set.insert((name.to_owned(), version.to_owned()));
        }
    }
    if set.is_empty() {
        bail!("`cargo tree` listed no crates for {triple}");
    }
    Ok(set)
}

fn crates(triple: &str) -> Result<Vec<Entry>> {
    let shipped = shipped_set(triple)?;
    let metadata: Metadata = serde_json::from_slice(&cargo_output(&[
        "metadata",
        "--format-version",
        "1",
        "--locked",
        "--filter-platform",
        triple,
    ])?)
    .context("reading `cargo metadata`")?;
    let members: BTreeSet<&str> = metadata
        .workspace_members
        .iter()
        .map(String::as_str)
        .collect();
    let mut packages: Vec<&Package> = metadata
        .packages
        .iter()
        .filter(|p| !members.contains(p.id.as_str()))
        .filter(|p| shipped.contains(&(p.name.clone(), p.version.clone())))
        .collect();
    packages.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    packages.dedup_by(|a, b| a.name == b.name && a.version == b.version);
    packages.into_iter().map(crate_entry).collect()
}

fn crate_entry(package: &Package) -> Result<Entry> {
    let license = package.license.clone().with_context(|| {
        format!(
            "the crate {} {} names no licence",
            package.name, package.version
        )
    })?;
    let folder = package
        .manifest_path
        .parent()
        .context("a manifest's folder")?;
    let label = format!("{} {}", package.name, package.version);
    let mut texts = license_files(folder, &label, package.name.ends_with("-sys"))?;
    if let Some(file) = &package.license_file {
        let path = folder.join(file);
        let already = texts
            .iter()
            .any(|t| t.source.ends_with(&format!(": {file}")));
        if path.is_file() && !already {
            texts.push(read_text(&path, format!("{label}: {file}"))?);
        }
    }
    let mut remark = None;
    // A `-sys` crate may carry only the licence of the C code it builds.
    let only_nested = !texts.is_empty() && texts.iter().all(|t| t.source.contains('/'));
    if texts.is_empty() || only_nested {
        let standard = standard_texts(&license, &package.holders(), &label)?;
        if !standard.is_empty() {
            remark = Some(STANDARD_TEXT_REMARK.to_owned());
            texts.extend(standard);
        }
    }
    Ok(Entry {
        name: package.name.clone(),
        version: package.version.clone(),
        license,
        url: package
            .repository
            .clone()
            .or_else(|| package.homepage.clone()),
        texts,
        remark,
    })
}

// ---------------------------------------------------------------------------------------
// JavaScript packages

#[derive(Deserialize)]
struct PackageJson {
    name: Option<String>,
    version: Option<String>,
    license: Option<serde_json::Value>,
    licenses: Option<Vec<serde_json::Value>>,
    repository: Option<serde_json::Value>,
    homepage: Option<String>,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
    #[serde(default, rename = "optionalDependencies")]
    optional_dependencies: BTreeMap<String, String>,
    #[serde(default)]
    author: Option<serde_json::Value>,
}

fn read_package_json(folder: &Path) -> Result<PackageJson> {
    let path = folder.join("package.json");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("reading {}", path.display()))
}

/// Where Node finds `name` when it is imported from code in `from`: in the nearest
/// `node_modules` folder that has it, going up from `from`, but not above `top`.
fn resolve_package(from: &Path, name: &str, top: &Path) -> Option<PathBuf> {
    for folder in from.ancestors() {
        let candidate = if folder.file_name().is_some_and(|n| n == "node_modules") {
            folder.join(name)
        } else {
            folder.join("node_modules").join(name)
        };
        if candidate.join("package.json").is_file() {
            return std::fs::canonicalize(candidate).ok();
        }
        if folder == top {
            break;
        }
    }
    None
}

/// Whether a resolved package is a dependency rather than one of the workspace's own.
fn is_installed(folder: &Path) -> bool {
    folder
        .components()
        .any(|part| part.as_os_str() == "node_modules")
}

fn js_license(json: &PackageJson) -> Option<String> {
    let of = |value: &serde_json::Value| -> Option<String> {
        match value {
            serde_json::Value::String(text) => Some(text.clone()),
            serde_json::Value::Object(map) => map.get("type")?.as_str().map(str::to_owned),
            _ => None,
        }
    };
    if let Some(license) = json.license.as_ref().and_then(of) {
        return Some(license);
    }
    let all: Vec<String> = json.licenses.as_ref()?.iter().filter_map(of).collect();
    (!all.is_empty()).then(|| all.join(" OR "))
}

fn js_url(json: &PackageJson) -> Option<String> {
    let repository = match &json.repository {
        Some(serde_json::Value::String(text)) => Some(text.clone()),
        Some(serde_json::Value::Object(map)) => {
            map.get("url").and_then(|u| u.as_str()).map(str::to_owned)
        }
        _ => None,
    };
    repository
        .map(|url| {
            url.trim_start_matches("git+")
                .trim_end_matches(".git")
                .to_owned()
        })
        .or_else(|| json.homepage.clone())
}

fn js_holders(json: &PackageJson, name: &str) -> String {
    match &json.author {
        Some(serde_json::Value::String(text)) => {
            text.split('<').next().unwrap_or(text).trim().to_owned()
        }
        Some(serde_json::Value::Object(map)) => map
            .get("name")
            .and_then(|n| n.as_str())
            .map_or_else(|| format!("the authors of {name}"), str::to_owned),
        _ => format!("the authors of {name}"),
    }
}

fn packages() -> Result<Vec<Entry>> {
    let root = std::fs::canonicalize(paths::root())?;
    let page = root.join(PAGE);
    let mut entries: BTreeMap<(String, String), Entry> = BTreeMap::new();
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let mut stack = vec![std::fs::canonicalize(&page)?];
    while let Some(folder) = stack.pop() {
        if !seen.insert(folder.clone()) {
            continue;
        }
        let json = read_package_json(&folder)?;
        let name = json.name.clone().unwrap_or_default();
        for dependency in json.dependencies.keys() {
            match resolve_package(&folder, dependency, &root) {
                Some(found) => stack.push(found),
                None => bail!(
                    "{dependency}, a dependency of {name}, is not installed; run `cargo xtask setup`"
                ),
            }
        }
        // An optional dependency that is not installed is not shipped either.
        for dependency in json.optional_dependencies.keys() {
            if let Some(found) = resolve_package(&folder, dependency, &root) {
                stack.push(found);
            }
        }
        if !is_installed(&folder) {
            continue;
        }
        let version = json.version.clone().unwrap_or_default();
        let license = js_license(&json)
            .with_context(|| format!("the package {name} {version} names no licence"))?;
        let label = format!("{name} {version}");
        let mut texts = license_files(&folder, &label, false)?;
        let mut remark = None;
        if texts.is_empty() {
            texts = standard_texts(&license, &js_holders(&json, &name), &label)?;
            if !texts.is_empty() {
                remark = Some(STANDARD_TEXT_REMARK.to_owned());
            }
        }
        entries.insert(
            (name.clone(), version.clone()),
            Entry {
                url: js_url(&json),
                name,
                version,
                license,
                texts,
                remark,
            },
        );
    }
    Ok(entries.into_values().collect())
}

// ---------------------------------------------------------------------------------------
// Other components

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentsFile {
    component: Vec<Component>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Component {
    name: String,
    #[serde(default)]
    version: Option<String>,
    license: String,
    url: String,
    files: Vec<String>,
    #[serde(default)]
    font: bool,
}

fn components_file() -> Result<ComponentsFile> {
    let path = paths::root()
        .join("resources")
        .join("notices")
        .join("components.toml");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("reading {}", path.display()))
}

fn font_components() -> Result<BTreeSet<String>> {
    Ok(components_file()?
        .component
        .into_iter()
        .filter(|c| c.font)
        .map(|c| c.name)
        .collect())
}

/// Upstream's report of the third-party code in libghostty-vt.
#[derive(Deserialize)]
struct GhosttyReport {
    third_party: Vec<GhosttyPart>,
}

#[derive(Deserialize)]
struct GhosttyPart {
    name: String,
    kind: String,
    version: Option<String>,
    url: Option<String>,
    hash: Option<String>,
    path: Option<String>,
    license: String,
    #[serde(rename = "use")]
    used: String,
}

fn ghostty() -> Result<Vec<Entry>> {
    let tools = tools::Tools::load()?;
    let version = &tools.ghostty_vt.version;
    let source = paths::vendor_dir().join(format!("libghostty-vt-{version}"));
    let deps = paths::vendor_dir().join(format!("libghostty-vt-{version}-deps"));
    if !source.is_dir() {
        bail!(
            "the libghostty-vt source is not unpacked in {}; run `cargo xtask setup`",
            source.display()
        );
    }
    let mut entries = vec![Entry {
        name: "libghostty-vt".to_owned(),
        version: version.clone(),
        license: "MIT".to_owned(),
        url: Some("https://github.com/ghostty-org/ghostty".to_owned()),
        texts: license_files(&source, &format!("libghostty-vt {version}"), false)?,
        remark: None,
    }];
    let report_path = source.join("deps").join("libghostty-vt-third-party.json");
    let text = std::fs::read_to_string(&report_path)
        .with_context(|| format!("reading {}", report_path.display()))?;
    let report: GhosttyReport = serde_json::from_str(&text)
        .with_context(|| format!("reading {}", report_path.display()))?;
    for part in report
        .third_party
        .into_iter()
        .filter(|p| p.used == "linked")
    {
        let folder = match (part.kind.as_str(), &part.hash, &part.path) {
            ("zig-package", Some(hash), _) => deps.join(hash),
            ("vendored", _, Some(path)) => source.join(path),
            (kind, _, _) => bail!(
                "libghostty-vt's report has a part {} of kind {kind}",
                part.name
            ),
        };
        let part_version = part.version.clone().unwrap_or_default();
        let label = format!("{} {part_version}", part.name);
        let mut texts = license_files(&folder, &label, false)?;
        // Vendored code may keep its licence in the folder above the sources.
        if texts.is_empty()
            && let Some(parent) = folder.parent()
        {
            texts = license_files(parent, &label, false)?;
        }
        entries.push(Entry {
            name: format!("{} (in libghostty-vt)", part.name),
            version: part_version,
            license: part.license,
            url: part.url,
            texts,
            remark: None,
        });
    }
    Ok(entries)
}

fn components() -> Result<Vec<Entry>> {
    let mut entries = ghostty()?;
    for component in components_file()?.component {
        let mut texts = Vec::new();
        for file in &component.files {
            let path = paths::root().join(file);
            texts.push(read_text(&path, format!("{}: {file}", component.name))?);
        }
        entries.push(Entry {
            name: component.name,
            version: component.version.unwrap_or_default(),
            license: component.license,
            url: Some(component.url),
            texts,
            remark: None,
        });
    }
    Ok(entries)
}

// ---------------------------------------------------------------------------------------
// The file

/// A text's key for telling identical texts apart: its words, without the spacing.
fn text_key(body: &str) -> [u8; 32] {
    let words: Vec<&str> = body.split_whitespace().collect();
    Sha256::digest(words.join(" ").as_bytes()).into()
}

/// Numbers each distinct licence text in the order it is first referred to.
struct Numbering<'n> {
    numbers: HashMap<[u8; 32], usize>,
    texts: Vec<&'n Text>,
}

impl<'n> Numbering<'n> {
    fn number(&mut self, text: &'n Text) -> usize {
        *self.numbers.entry(text_key(&text.body)).or_insert_with(|| {
            self.texts.push(text);
            self.texts.len()
        })
    }
}

fn render_section<'n>(
    out: &mut String,
    title: &str,
    entries: &'n [Entry],
    numbering: &mut Numbering<'n>,
) {
    out.push_str(&format!("\n\n{title}\n{}\n", "=".repeat(title.len())));
    for entry in entries {
        out.push('\n');
        if entry.version.is_empty() {
            out.push_str(&format!("{}\n", entry.name));
        } else {
            out.push_str(&format!("{} {}\n", entry.name, entry.version));
        }
        out.push_str(&format!("  Licence: {}\n", entry.license));
        if let Some(url) = &entry.url {
            out.push_str(&format!("  Source: {url}\n"));
        }
        if let Some(remark) = &entry.remark {
            out.push_str(&format!("  {remark}\n"));
        }
        let refs: Vec<String> = entry
            .texts
            .iter()
            .map(|text| format!("[{}]", numbering.number(text)))
            .collect();
        out.push_str(&format!("  Texts: {}\n", refs.join(" ")));
    }
}

fn render(notices: &Notices) -> Result<String> {
    let version = crate::release::workspace_version()?;
    let mut out = String::new();
    out.push_str(&format!("Third-party software in Lume {version}\n"));
    out.push_str(&format!("Built for {}\n\n", notices.triple));
    out.push_str(
        "Lume is released under the Apache License 2.0. It is built with the software \
         listed below, each under its own licence. The licence texts follow the lists; \
         a number in brackets refers to one of them.\n",
    );
    let mut numbering = Numbering {
        numbers: HashMap::new(),
        texts: Vec::new(),
    };
    render_section(&mut out, "Components", &notices.components, &mut numbering);
    render_section(&mut out, "Rust crates", &notices.crates, &mut numbering);
    render_section(
        &mut out,
        "JavaScript packages",
        &notices.packages,
        &mut numbering,
    );

    out.push_str("\n\nLicence texts\n=============\n");
    for (index, text) in numbering.texts.iter().enumerate() {
        out.push_str(&format!(
            "\n[{}] {}\n{}\n\n{}\n",
            index + 1,
            text.source,
            "-".repeat(78),
            text.body
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(ids: &[&str]) -> impl Fn(&str) -> bool {
        let ids: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
        move |id: &str| ids.iter().any(|known| known == id)
    }

    #[test]
    fn an_or_needs_one_side_and_an_and_needs_both() {
        let permissive = allowed(&["MIT", "Apache-2.0", "Unicode-3.0"]);
        for good in [
            "MIT",
            "MIT OR Apache-2.0",
            "Unlicense OR MIT",
            "MIT/Apache-2.0",
            "Apache-2.0 / MIT",
            "(MIT OR Apache-2.0) AND Unicode-3.0",
            "Apache-2.0 WITH LLVM-exception",
        ] {
            assert!(
                Expression::parse(good).unwrap().satisfied_by(&permissive),
                "{good}"
            );
        }
        for bad in [
            "GPL-3.0",
            "MIT AND GPL-2.0",
            "LGPL-2.1 OR GPL-3.0",
            "(MIT OR Apache-2.0) AND MPL-2.0",
        ] {
            assert!(
                !Expression::parse(bad).unwrap().satisfied_by(&permissive),
                "{bad}"
            );
        }
    }

    #[test]
    fn and_binds_tighter_than_or() {
        assert_eq!(
            Expression::parse("MIT OR Apache-2.0 AND ISC").unwrap(),
            Expression::Or(vec![
                Expression::Id("MIT".into()),
                Expression::And(vec![
                    Expression::Id("Apache-2.0".into()),
                    Expression::Id("ISC".into())
                ])
            ])
        );
    }

    #[test]
    fn an_expression_that_does_not_parse_is_an_error() {
        for bad in ["", "MIT OR", "(MIT", "MIT)", "AND MIT", "MIT WITH"] {
            assert!(Expression::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_ids_of_an_expression_are_listed_once_without_exceptions() {
        let mut ids = Vec::new();
        Expression::parse("(MIT OR Apache-2.0 WITH LLVM-exception) AND MIT")
            .unwrap()
            .ids(&mut ids);
        assert_eq!(ids, ["MIT", "Apache-2.0"]);
    }

    #[test]
    fn licence_files_are_recognised_by_their_names() {
        for name in [
            "LICENSE",
            "LICENSE-MIT",
            "license.md",
            "LICENCE",
            "COPYING",
            "NOTICE",
            "UNLICENSE",
        ] {
            assert!(is_license_file(name), "{name}");
        }
        for name in ["README.md", "Cargo.toml", "src"] {
            assert!(!is_license_file(name), "{name}");
        }
    }

    #[test]
    fn texts_that_differ_only_in_spacing_are_one_text() {
        assert_eq!(
            text_key("MIT  License\n\nPermission"),
            text_key("MIT License Permission")
        );
        assert_ne!(text_key("MIT License"), text_key("ISC License"));
    }
}
