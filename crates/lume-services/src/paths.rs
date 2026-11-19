//! Paths as the services compare them, and the patterns grants are written in.
//!
//! A path is compared only once it is absolute and canonical: every link and junction on
//! the way resolved, and on Windows without the `\\?\` prefix the system adds. It is then
//! cut into its root (`/`, `C:` or `//server/share`) and its names. On Windows names
//! compare without regard to case, as the file system does.
//!
//! A pattern is a path whose names may hold `*` (any run of characters within one name)
//! and `?` (any one character), or be `**` (any number of names, none included). The
//! part before the first wildcard, the pattern's base, is made canonical too, so a
//! workspace reached through a link still matches itself.

use std::path::{Component, Path, PathBuf};

use lume_plugin::permissions::PATH_VARIABLES;

use crate::scope::{Places, ServiceError};

/// Whether names compare without regard to case on this system.
pub(crate) const IGNORE_CASE: bool = cfg!(windows);

/// A canonical absolute path in parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Parts {
    /// `/`, a drive such as `C:`, or `//server/share`.
    pub root: String,
    pub names: Vec<String>,
}

impl Parts {
    /// The parts of an absolute path. `None` for a path that is not absolute here, that
    /// names a device or a volume without a drive letter, or that holds `..` or a name
    /// that is not Unicode.
    pub fn of(path: &Path) -> Option<Self> {
        let mut components = path.components();
        let root = root_of(&mut components)?;
        let mut names = Vec::new();
        for component in components {
            match component {
                Component::Normal(name) => names.push(name.to_str()?.to_owned()),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
            }
        }
        Some(Self { root, names })
    }

    /// Whether two paths are the same, with names compared as `ignore_case` says.
    pub fn same(&self, other: &Self, ignore_case: bool) -> bool {
        self.names.len() == other.names.len() && self.starts_with(other, ignore_case)
    }

    /// Whether this path is `folder` or lies under it.
    pub fn starts_with(&self, folder: &Self, ignore_case: bool) -> bool {
        same_name(&self.root, &folder.root, true)
            && self.names.len() >= folder.names.len()
            && self
                .names
                .iter()
                .zip(&folder.names)
                .all(|(name, other)| same_name(name, other, ignore_case))
    }
}

#[cfg(windows)]
fn root_of(components: &mut std::path::Components<'_>) -> Option<String> {
    use std::path::Prefix;
    let Some(Component::Prefix(prefix)) = components.next() else {
        return None;
    };
    let root = match prefix.kind() {
        Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
            format!("{}:", char::from(drive).to_ascii_uppercase())
        }
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
            format!("//{}/{}", server.to_str()?, share.to_str()?)
        }
        Prefix::Verbatim(_) | Prefix::DeviceNS(_) => return None,
    };
    match components.next() {
        Some(Component::RootDir) => Some(root),
        _ => None,
    }
}

#[cfg(not(windows))]
fn root_of(components: &mut std::path::Components<'_>) -> Option<String> {
    match components.next() {
        Some(Component::RootDir) => Some("/".to_owned()),
        _ => None,
    }
}

/// Whether two names are the same, ignoring case when asked to.
pub(crate) fn same_name(a: &str, b: &str, ignore_case: bool) -> bool {
    if ignore_case {
        a == b || a.to_lowercase() == b.to_lowercase()
    } else {
        a == b
    }
}

/// A path without the `\\?\` prefix Windows gives canonical paths, so every path is
/// written one way. Elsewhere the path as it is.
pub(crate) fn strip_verbatim(path: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }
    let Some(text) = path.to_str() else {
        return path;
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        let bytes = rest.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\'
        {
            return PathBuf::from(rest);
        }
    }
    path
}

/// The canonical form of a path that exists: absolute, every link resolved.
pub(crate) fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path).map(strip_verbatim)
}

/// The canonical form of a path that may not exist yet: the longest part of it that
/// exists made canonical, and the rest added as written.
pub(crate) fn canonical_lenient(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut current = path;
    loop {
        if let Ok(mut found) = canonical(current) {
            for name in rest.iter().rev() {
                found.push(name);
            }
            return found;
        }
        match (current.parent(), current.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                current = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// The path a plugin asked for, with its variable expanded. It must be absolute or begin
/// with one of the path variables, and cannot go up with `..`.
pub(crate) fn expand(places: &Places, asked: &str) -> Result<PathBuf, ServiceError> {
    if asked.chars().any(char::is_control) {
        return Err(ServiceError::invalid(format!(
            "`{asked}` holds a control character"
        )));
    }
    if asked
        .split(|c| c == '/' || (cfg!(windows) && c == '\\'))
        .any(|name| name == "..")
    {
        return Err(ServiceError::invalid(format!(
            "`{asked}`: a path cannot go up with `..`"
        )));
    }
    if asked.starts_with("${") {
        let variable = PATH_VARIABLES.iter().find(|variable| {
            asked
                .strip_prefix(**variable)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '\\']))
        });
        let Some(variable) = variable else {
            return Err(ServiceError::invalid(format!(
                "`{asked}`: the variables a path may begin with are {}",
                PATH_VARIABLES.join(", ")
            )));
        };
        let folder = folder_of(places, variable).ok_or_else(|| {
            ServiceError::denied(format!(
                "`{asked}`: no workspace folder is set, so `${{workspace}}` reaches nothing"
            ))
        })?;
        let rest = asked[variable.len()..].trim_start_matches(['/', '\\']);
        return Ok(join(folder, rest));
    }
    let native = native(asked);
    if cfg!(windows) && (native.starts_with(r"\\?\") || native.starts_with(r"\\.\")) {
        return Err(ServiceError::invalid(format!(
            "`{asked}` is a device path, not a file's"
        )));
    }
    let path = PathBuf::from(native);
    if !path.is_absolute() {
        return Err(ServiceError::invalid(format!(
            "`{asked}` is not a path a plugin can ask for: it must be absolute or begin with {}",
            PATH_VARIABLES.join(", ")
        )));
    }
    Ok(path)
}

/// The folder a path variable stands for.
fn folder_of(places: &Places, variable: &str) -> Option<PathBuf> {
    match variable {
        "${workspace}" => places.workspace.clone(),
        "${home}" => Some(places.home.clone()),
        "${plugin-data}" => Some(places.plugin_data.clone()),
        _ => None,
    }
}

/// A path written with forward slashes, with this system's separators.
fn native(written: &str) -> String {
    if cfg!(windows) {
        written.replace('/', "\\")
    } else {
        written.to_owned()
    }
}

/// A folder and a relative path under it.
fn join(folder: PathBuf, rest: &str) -> PathBuf {
    if rest.is_empty() {
        folder
    } else {
        folder.join(native(rest))
    }
}

/// One name of a pattern.
#[derive(Clone, Debug)]
enum Part {
    /// A name as it is.
    Name(String),
    /// A name with `*` or `?` in it.
    Glob(Vec<char>),
    /// `**`: any number of names.
    Any,
}

/// A grant's path pattern, ready to match canonical paths.
#[derive(Clone, Debug)]
pub(crate) struct Pattern {
    root: String,
    parts: Vec<Part>,
}

impl Pattern {
    /// The pattern of a grant, with its variable expanded and its base made canonical.
    /// `None` for one that can match nothing here: it begins with `${workspace}` and no
    /// workspace is set, or it is not an absolute path on this system.
    pub fn compile(places: &Places, grant: &str) -> Option<Self> {
        let written = grant.replace('\\', "/");
        let (folder, rest) = match PATH_VARIABLES
            .iter()
            .find(|variable| written.starts_with(**variable))
        {
            Some(variable) => (folder_of(places, variable)?, &written[variable.len()..]),
            None => split_absolute(&written)?,
        };
        let names: Vec<&str> = rest
            .split('/')
            .filter(|name| !name.is_empty() && *name != ".")
            .collect();
        if names.contains(&"..") {
            return None;
        }
        let wild = |name: &str| name.contains(['*', '?']);
        let literal = names.iter().take_while(|name| !wild(name)).count();
        let mut base = folder;
        for name in &names[..literal] {
            base.push(name);
        }
        let base = Parts::of(&canonical_lenient(&base))?;
        let mut parts: Vec<Part> = base.names.into_iter().map(Part::Name).collect();
        for name in &names[literal..] {
            parts.push(match *name {
                "**" => Part::Any,
                name if wild(name) => Part::Glob(name.chars().collect()),
                name => Part::Name(name.to_owned()),
            });
        }
        Some(Self {
            root: base.root,
            parts,
        })
    }

    /// Whether a canonical path matches the pattern.
    pub fn matches(&self, path: &Parts) -> bool {
        same_name(&self.root, &path.root, true) && match_names(&self.parts, &path.names)
    }
}

/// The root folder and the rest of an absolute pattern written with forward slashes.
fn split_absolute(written: &str) -> Option<(PathBuf, &str)> {
    if cfg!(windows) {
        let bytes = written.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'/'
        {
            return Some((PathBuf::from(format!("{}\\", &written[..2])), &written[3..]));
        }
        let unc = written.strip_prefix("//")?;
        let mut pieces = unc.splitn(3, '/');
        let server = pieces.next().filter(|server| !server.is_empty())?;
        let share = pieces.next().filter(|share| !share.is_empty())?;
        if format!("{server}{share}").contains(['*', '?']) {
            return None;
        }
        Some((
            PathBuf::from(format!(r"\\{server}\{share}\")),
            pieces.next().unwrap_or(""),
        ))
    } else {
        written
            .strip_prefix('/')
            .map(|rest| (PathBuf::from("/"), rest))
    }
}

/// Whether the names of a path match the parts of a pattern. `**` matches any number of
/// names, so each step keeps every number of names matched so far.
fn match_names(parts: &[Part], names: &[String]) -> bool {
    let mut matched = vec![false; names.len() + 1];
    matched[0] = true;
    for part in parts {
        let mut next = vec![false; names.len() + 1];
        match part {
            Part::Any => {
                let mut any = false;
                for (count, slot) in next.iter_mut().enumerate() {
                    any |= matched[count];
                    *slot = any;
                }
            }
            Part::Name(want) => {
                for count in 1..=names.len() {
                    next[count] =
                        matched[count - 1] && same_name(want, &names[count - 1], IGNORE_CASE);
                }
            }
            Part::Glob(glob) => {
                for count in 1..=names.len() {
                    next[count] = matched[count - 1] && glob_matches(glob, &names[count - 1]);
                }
            }
        }
        matched = next;
        if !matched.contains(&true) {
            return false;
        }
    }
    matched[names.len()]
}

/// Whether one name matches a glob of `*` and `?`.
fn glob_matches(glob: &[char], name: &str) -> bool {
    let fold = |c: char| {
        if IGNORE_CASE {
            c.to_lowercase().next().unwrap_or(c)
        } else {
            c
        }
    };
    let name: Vec<char> = name.chars().map(fold).collect();
    let glob: Vec<char> = glob.iter().copied().map(fold).collect();
    let (mut at, mut in_name) = (0, 0);
    // Where the last `*` was, and how much of the name it had taken.
    let mut star: Option<(usize, usize)> = None;
    while in_name < name.len() {
        match glob.get(at) {
            Some('*') => {
                star = Some((at, in_name));
                at += 1;
            }
            Some(&c) if c == '?' || c == name[in_name] => {
                at += 1;
                in_name += 1;
            }
            _ => match star {
                Some((star_at, taken)) => {
                    at = star_at + 1;
                    in_name = taken + 1;
                    star = Some((star_at, taken + 1));
                }
                None => return false,
            },
        }
    }
    glob[at..].iter().all(|&c| c == '*')
}

/// Lume's own folders, which no grant reaches, and the plugin's data folder among them,
/// which is reached.
pub(crate) struct Fence {
    closed: Vec<Parts>,
    open: Option<Parts>,
}

impl Fence {
    pub fn new(places: &Places) -> Self {
        let parts = |path: &Path| Parts::of(&canonical_lenient(path));
        Self {
            closed: places
                .forbidden
                .iter()
                .filter_map(|folder| parts(folder))
                .collect(),
            open: parts(&places.plugin_data),
        }
    }

    /// Whether a path lies in Lume's own folders, outside the plugin's data folder. The
    /// folders are compared without regard to case everywhere: a file system may ignore
    /// case even where the system does not, and refusing too much here costs nothing.
    pub fn shuts_out(&self, path: &Parts) -> bool {
        self.within_closed(path)
            && !self
                .open
                .as_ref()
                .is_some_and(|open| path.starts_with(open, IGNORE_CASE))
    }

    /// Whether a path lies in Lume's own folders, the plugin's data folder included.
    pub fn within_closed(&self, path: &Parts) -> bool {
        self.closed
            .iter()
            .any(|folder| path.starts_with(folder, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(names: &[&str]) -> Parts {
        Parts {
            root: if cfg!(windows) { "C:" } else { "/" }.to_owned(),
            names: names.iter().map(|name| (*name).to_owned()).collect(),
        }
    }

    fn pattern(parts: Vec<Part>) -> Pattern {
        Pattern {
            root: if cfg!(windows) { "C:" } else { "/" }.to_owned(),
            parts,
        }
    }

    fn glob(text: &str) -> Part {
        Part::Glob(text.chars().collect())
    }

    fn name(text: &str) -> Part {
        Part::Name(text.to_owned())
    }

    #[test]
    fn a_star_matches_within_one_name_and_a_question_mark_one_character() {
        let chars = |text: &str| text.chars().collect::<Vec<_>>();
        assert!(glob_matches(&chars("*.md"), "notes.md"));
        assert!(glob_matches(&chars("*.md"), ".md"));
        assert!(!glob_matches(&chars("*.md"), "notes.mdx"));
        assert!(glob_matches(&chars("a*b*c"), "aXXbYYc"));
        assert!(!glob_matches(&chars("a*b*c"), "aXXbYY"));
        assert!(glob_matches(&chars("?.txt"), "a.txt"));
        assert!(!glob_matches(&chars("?.txt"), "ab.txt"));
        assert!(glob_matches(&chars("*"), ""));
        assert!(glob_matches(&chars("**"), "anything"));
    }

    #[test]
    fn two_stars_match_any_number_of_names_including_none() {
        let all = pattern(vec![name("ws"), Part::Any]);
        assert!(all.matches(&parts(&["ws"])));
        assert!(all.matches(&parts(&["ws", "a"])));
        assert!(all.matches(&parts(&["ws", "a", "b", "c"])));
        assert!(!all.matches(&parts(&["other", "a"])));
        assert!(!all.matches(&parts(&[])));

        let sources = pattern(vec![name("ws"), Part::Any, glob("*.rs")]);
        assert!(sources.matches(&parts(&["ws", "main.rs"])));
        assert!(sources.matches(&parts(&["ws", "src", "deep", "lib.rs"])));
        assert!(!sources.matches(&parts(&["ws", "src", "lib.rs", "x"])));
        assert!(!sources.matches(&parts(&["ws"])));

        let middle = pattern(vec![name("ws"), Part::Any, name("notes"), Part::Any]);
        assert!(middle.matches(&parts(&["ws", "a", "notes", "b"])));
        assert!(middle.matches(&parts(&["ws", "notes"])));
        assert!(!middle.matches(&parts(&["ws", "a", "b"])));
    }

    #[test]
    fn one_star_does_not_reach_into_a_folder_below() {
        let top = pattern(vec![name("ws"), glob("*")]);
        assert!(top.matches(&parts(&["ws", "a"])));
        assert!(!top.matches(&parts(&["ws", "a", "b"])));
        assert!(!top.matches(&parts(&["ws"])));
    }

    #[test]
    fn names_compare_without_case_on_windows_only() {
        let notes = pattern(vec![name("Users"), name("me"), glob("*.MD")]);
        assert_eq!(
            notes.matches(&parts(&["users", "ME", "a.md"])),
            cfg!(windows)
        );
        assert!(notes.matches(&parts(&["Users", "me", "a.MD"])));
    }

    #[test]
    fn a_folder_holds_what_is_under_it_and_nothing_beside_it() {
        let folder = parts(&["home", "me", ".config", "lume"]);
        assert!(parts(&["home", "me", ".config", "lume"]).starts_with(&folder, false));
        assert!(parts(&["home", "me", ".config", "lume", "x"]).starts_with(&folder, false));
        assert!(!parts(&["home", "me", ".config", "lume2"]).starts_with(&folder, false));
        assert!(!parts(&["home", "me", ".config"]).starts_with(&folder, false));
        assert!(parts(&["home", "me", ".config", "LUME", "x"]).starts_with(&folder, true));
    }

    #[test]
    fn a_path_is_cut_into_its_root_and_names() {
        let text = if cfg!(windows) {
            r"C:\Users\me\notes.md"
        } else {
            "/Users/me/notes.md"
        };
        let cut = Parts::of(Path::new(text)).unwrap();
        assert_eq!(cut.names, ["Users", "me", "notes.md"]);
        assert!(Parts::of(Path::new("relative/path")).is_none());
        let up = if cfg!(windows) {
            r"C:\Users\..\x"
        } else {
            "/Users/../x"
        };
        assert!(Parts::of(Path::new(up)).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_lose_the_verbatim_prefix_and_devices_have_no_parts() {
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\C:\Users\me")),
            PathBuf::from(r"C:\Users\me")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\UNC\server\share\x")),
            PathBuf::from(r"\\server\share\x")
        );
        let unc = Parts::of(Path::new(r"\\server\share\x")).unwrap();
        assert_eq!(unc.root, "//server/share");
        assert_eq!(unc.names, ["x"]);
        assert!(Parts::of(Path::new(r"\\.\pipe\x")).is_none());
        assert!(Parts::of(Path::new(r"\\?\Volume{0000}\x")).is_none());
        assert!(Parts::of(Path::new(r"\x")).is_none());
    }

    fn places() -> Places {
        let root = if cfg!(windows) {
            r"C:\nowhere-lume"
        } else {
            "/nowhere-lume"
        };
        let at = |rest: &str| PathBuf::from(root).join(rest);
        Places {
            workspace: Some(at("ws")),
            home: at("home"),
            plugin_data: at("state/plugin-data/a.x"),
            forbidden: vec![at("state")],
            own_programs: Vec::new(),
        }
    }

    #[test]
    fn a_pattern_expands_its_variable_against_the_places() {
        let places = places();
        let pattern = Pattern::compile(&places, "${workspace}/src/**/*.rs").unwrap();
        let path = Parts::of(&places.workspace.clone().unwrap().join("src").join("a.rs")).unwrap();
        assert!(pattern.matches(&path));
        let home = Pattern::compile(&places, "${home}/**").unwrap();
        assert!(home.matches(&Parts::of(&places.home).unwrap()));
        assert!(!home.matches(&path));

        let mut unset = places.clone();
        unset.workspace = None;
        assert!(Pattern::compile(&unset, "${workspace}/**").is_none());
    }

    #[test]
    fn an_absolute_pattern_is_read_as_this_system_writes_paths() {
        let places = places();
        if cfg!(windows) {
            let pattern = Pattern::compile(&places, "C:/nowhere-lume/ws/*.txt").unwrap();
            let path = Parts::of(Path::new(r"c:\NOWHERE-LUME\ws\a.TXT")).unwrap();
            assert!(pattern.matches(&path));
            assert!(Pattern::compile(&places, "/nowhere-lume/**").is_none());
        } else {
            let pattern = Pattern::compile(&places, "/nowhere-lume/ws/*.txt").unwrap();
            assert!(pattern.matches(&Parts::of(Path::new("/nowhere-lume/ws/a.txt")).unwrap()));
            assert!(Pattern::compile(&places, "C:/nowhere-lume/**").is_none());
        }
    }

    #[test]
    fn an_asked_path_is_absolute_or_begins_with_a_variable() {
        let places = places();
        let ws = places.workspace.clone().unwrap();
        assert_eq!(expand(&places, "${workspace}").unwrap(), ws);
        assert_eq!(
            expand(&places, "${workspace}/a/b.txt").unwrap(),
            ws.join("a").join("b.txt")
        );
        for wrong in [
            "relative/path",
            "${workspace}x",
            "${secret}/x",
            "${workspace}/../x",
            "${workspace}/a\u{7}b",
        ] {
            assert_eq!(
                expand(&places, wrong).unwrap_err().kind,
                crate::ErrorKind::Invalid,
                "{wrong}"
            );
        }
        let mut unset = places.clone();
        unset.workspace = None;
        assert_eq!(
            expand(&unset, "${workspace}/a").unwrap_err().kind,
            crate::ErrorKind::Denied
        );
    }

    #[test]
    fn the_fence_shuts_out_lumes_folders_but_not_the_plugins_own() {
        let places = places();
        let fence = Fence::new(&places);
        let state = places.forbidden[0].clone();
        assert!(fence.shuts_out(&Parts::of(&state.join("config.toml")).unwrap()));
        assert!(!fence.shuts_out(&Parts::of(&places.plugin_data.join("cache.json")).unwrap()));
        assert!(!fence.shuts_out(&Parts::of(&places.plugin_data).unwrap()));
        assert!(fence.shuts_out(&Parts::of(&state.join("plugin-data").join("b.y")).unwrap()));
        assert!(fence.within_closed(&Parts::of(&places.plugin_data).unwrap()));
        assert!(!fence.shuts_out(&Parts::of(&places.home.join("x")).unwrap()));
    }
}
