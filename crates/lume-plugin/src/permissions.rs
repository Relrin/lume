//! What a plugin may do, and how that is put to the user.
//!
//! A plugin can do nothing by itself. Each thing it may do beyond its own data is a
//! permission it asks for in its manifest, often narrowed to a scope: the files it may
//! read, the programs it may run, the servers it may reach. The user sees the list in
//! plain words when installing and accepts all of it or none. What was accepted is kept
//! with the install, and every call the plugin makes is checked against it.
//!
//! A manifest writes permissions as a table:
//!
//! ```toml
//! [permissions]
//! "system.metrics" = true
//! "fs.read" = ["${workspace}/**"]
//! "process.run" = { scope = ["git"], prompt = "each" }
//!
//! [permissions.optional]
//! "net" = true
//! ```
//!
//! `prompt = "each"` has the user asked before every use, shown the exact text or
//! command. The kinds under `optional` are ones the plugin may ask for while it runs; the
//! user is asked then, and nothing is granted until they agree.

use std::collections::BTreeSet;
use std::fmt;

use crate::id::check_plugin_id;

/// The kinds of permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    FsRead,
    FsWrite,
    ProcessRun,
    /// Programs run where a session runs, on another machine.
    ProcessRunElsewhere,
    Net,
    PanesRead,
    PanesWrite,
    SessionsSpawn,
    WorkspaceRead,
    LayoutWrite,
    ClipboardRead,
    ClipboardWrite,
    SystemMetrics,
    ServicesUse,
}

/// What a kind's scope is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeKind {
    /// The kind has no scope: it is granted or not.
    None,
    /// Path patterns.
    Paths,
    /// Programs, by name.
    Programs,
    /// Servers: host names and ports.
    Hosts,
    /// Other plugins, by the ids their manifests give them.
    Plugins,
}

impl Kind {
    pub const ALL: [Self; 14] = [
        Self::FsRead,
        Self::FsWrite,
        Self::ProcessRun,
        Self::ProcessRunElsewhere,
        Self::Net,
        Self::PanesRead,
        Self::PanesWrite,
        Self::SessionsSpawn,
        Self::WorkspaceRead,
        Self::LayoutWrite,
        Self::ClipboardRead,
        Self::ClipboardWrite,
        Self::SystemMetrics,
        Self::ServicesUse,
    ];

    /// The kind's name as a manifest writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FsRead => "fs.read",
            Self::FsWrite => "fs.write",
            Self::ProcessRun => "process.run",
            Self::ProcessRunElsewhere => "process.run-elsewhere",
            Self::Net => "net",
            Self::PanesRead => "panes.read",
            Self::PanesWrite => "panes.write",
            Self::SessionsSpawn => "sessions.spawn",
            Self::WorkspaceRead => "workspace.read",
            Self::LayoutWrite => "layout.write",
            Self::ClipboardRead => "clipboard.read",
            Self::ClipboardWrite => "clipboard.write",
            Self::SystemMetrics => "system.metrics",
            Self::ServicesUse => "services.use",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }

    pub fn scope(self) -> ScopeKind {
        match self {
            Self::FsRead | Self::FsWrite => ScopeKind::Paths,
            Self::ProcessRun | Self::ProcessRunElsewhere => ScopeKind::Programs,
            Self::Net => ScopeKind::Hosts,
            Self::ServicesUse => ScopeKind::Plugins,
            _ => ScopeKind::None,
        }
    }

    /// Whether a grant of this kind can have the user asked before each use. Only where
    /// what is done can be shown exactly: the text typed, the command run.
    pub fn may_ask_each_time(self) -> bool {
        matches!(
            self,
            Self::PanesWrite | Self::ProcessRun | Self::ProcessRunElsewhere
        )
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether the user is asked before each use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Prompt {
    /// Asked once, when the permission is granted.
    #[default]
    Once,
    /// Asked before every use.
    Each,
}

/// One permission asked for, or granted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub kind: Kind,
    /// The scope, sorted and without repeats. Empty for a kind without one.
    pub scope: Vec<String>,
    pub prompt: Prompt,
}

/// A kind a plugin may ask for while it runs, perhaps only within some scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Optional {
    pub kind: Kind,
    /// The scope it may ask within; `None` for any.
    pub within: Option<Vec<String>>,
}

/// Everything a manifest asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Permissions {
    pub required: Vec<Request>,
    pub optional: Vec<Optional>,
}

/// The key of the table of kinds that may be asked for later.
const OPTIONAL_KEY: &str = "optional";

/// The variables a path pattern may begin with.
pub const PATH_VARIABLES: [&str; 3] = ["${workspace}", "${home}", "${plugin-data}"];

/// Reads the `[permissions]` table of a manifest, or the grants of an install.
pub fn parse_table(table: &toml::Table) -> Result<Permissions, Vec<String>> {
    let mut problems = Vec::new();
    let mut permissions = Permissions::default();
    for (key, value) in table {
        if key == OPTIONAL_KEY {
            match value.as_table() {
                Some(optional) => {
                    for (key, value) in optional {
                        match parse_optional(key, value) {
                            Ok(entry) => permissions.optional.push(entry),
                            Err(problem) => {
                                problems.push(format!("permissions.optional.\"{key}\": {problem}"));
                            }
                        }
                    }
                }
                None => problems.push(
                    "permissions.optional must be a table of the kinds that may be asked \
                     for later"
                        .to_owned(),
                ),
            }
            continue;
        }
        match parse_request(key, value) {
            Ok(request) => permissions.required.push(request),
            Err(problem) => problems.push(format!("permissions.\"{key}\": {problem}")),
        }
    }
    if problems.is_empty() {
        Ok(permissions)
    } else {
        Err(problems)
    }
}

fn kind_of(key: &str) -> Result<Kind, String> {
    Kind::parse(key).ok_or_else(|| {
        let known: Vec<&str> = Kind::ALL.iter().map(|kind| kind.as_str()).collect();
        format!(
            "`{key}` is not a permission; the permissions are {}",
            known.join(", ")
        )
    })
}

fn strings(value: &toml::Value) -> Result<Vec<String>, String> {
    let items = value
        .as_array()
        .ok_or("the scope must be a list of strings")?;
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or_else(|| "the scope must be a list of strings".to_owned())
        })
        .collect()
}

fn parse_request(key: &str, value: &toml::Value) -> Result<Request, String> {
    let kind = kind_of(key)?;
    let (scope, prompt) = match value {
        toml::Value::Boolean(true) => (Vec::new(), Prompt::Once),
        toml::Value::Boolean(false) => {
            return Err("leave out a permission that is not wanted".to_owned());
        }
        toml::Value::Array(_) => (strings(value)?, Prompt::Once),
        toml::Value::Table(table) => {
            let mut scope = Vec::new();
            let mut prompt = Prompt::Once;
            for (field, value) in table {
                match field.as_str() {
                    "scope" => scope = strings(value)?,
                    "prompt" => {
                        prompt = match value.as_str() {
                            Some("each") => Prompt::Each,
                            Some("once") => Prompt::Once,
                            _ => return Err("prompt is \"each\" or \"once\"".to_owned()),
                        };
                    }
                    other => {
                        return Err(format!(
                            "`{other}` is not part of a permission (scope, prompt)"
                        ));
                    }
                }
            }
            (scope, prompt)
        }
        _ => {
            return Err(
                "a permission is true, a list of what it covers, or a table with scope and \
                 prompt"
                    .to_owned(),
            );
        }
    };
    if prompt == Prompt::Each && !kind.may_ask_each_time() {
        return Err(format!(
            "only panes.write, process.run and process.run-elsewhere can ask before each use, \
             not {kind}"
        ));
    }
    let scope = check_scope(kind, scope)?;
    Ok(Request {
        kind,
        scope,
        prompt,
    })
}

fn parse_optional(key: &str, value: &toml::Value) -> Result<Optional, String> {
    let kind = kind_of(key)?;
    let within = match value {
        toml::Value::Boolean(true) => None,
        toml::Value::Array(_) => {
            if kind.scope() == ScopeKind::None {
                return Err(format!("{kind} has no scope to limit"));
            }
            Some(check_scope(kind, strings(value)?)?)
        }
        _ => return Err("an optional permission is true, or the list it may ask within".into()),
    };
    Ok(Optional { kind, within })
}

/// A permission asked for while a plugin runs: `kind` over `scope`, checked as a
/// manifest's would be.
pub fn request(kind: Kind, scope: Vec<String>) -> Result<Request, String> {
    Ok(Request {
        kind,
        scope: check_scope(kind, scope)?,
        prompt: Prompt::Once,
    })
}

/// Checks a scope for its kind, and returns it sorted and without repeats.
fn check_scope(kind: Kind, scope: Vec<String>) -> Result<Vec<String>, String> {
    let checker: fn(&str) -> Result<String, String> = match kind.scope() {
        ScopeKind::None => {
            return if scope.is_empty() {
                Ok(scope)
            } else {
                Err(format!("{kind} has no scope; write `\"{kind}\" = true`"))
            };
        }
        ScopeKind::Paths => check_path_pattern,
        ScopeKind::Programs => check_program,
        ScopeKind::Hosts => check_host,
        ScopeKind::Plugins => |id: &str| check_plugin_id(id).map(|()| id.to_owned()),
    };
    if scope.is_empty() {
        return Err(format!("{kind} needs a scope: what it covers"));
    }
    let mut checked = BTreeSet::new();
    for item in &scope {
        checked.insert(checker(item)?);
    }
    Ok(checked.into_iter().collect())
}

/// Checks a path pattern and writes it with forward slashes. A pattern begins with one
/// of [`PATH_VARIABLES`] or is absolute; `*`, `**` and `?` match as in a shell; no part
/// may be `..`.
pub fn check_path_pattern(pattern: &str) -> Result<String, String> {
    let written = pattern.replace('\\', "/");
    let variable = PATH_VARIABLES
        .iter()
        .find(|variable| written.starts_with(*variable));
    let rest = match variable {
        Some(variable) => &written[variable.len()..],
        None => {
            if written.contains("${") {
                return Err(format!(
                    "`{pattern}`: the variables a path may begin with are {}",
                    PATH_VARIABLES.join(", ")
                ));
            }
            let bytes = written.as_bytes();
            let drive = bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && bytes[2] == b'/';
            if !(written.starts_with('/') || drive) {
                return Err(format!(
                    "`{pattern}` is not a path pattern: it must be absolute or begin with {}",
                    PATH_VARIABLES.join(", ")
                ));
            }
            written.as_str()
        }
    };
    if !(rest.is_empty() || rest.starts_with('/') || variable.is_none()) {
        return Err(format!(
            "`{pattern}`: a variable is followed by `/` or nothing"
        ));
    }
    if rest.contains("${") {
        return Err(format!("`{pattern}`: a variable may only begin a path"));
    }
    if written.split('/').any(|part| part == "..") {
        return Err(format!(
            "`{pattern}`: a path pattern cannot go up with `..`"
        ));
    }
    if written.chars().any(char::is_control) {
        return Err(format!("`{pattern}` holds a control character"));
    }
    Ok(written)
}

/// Checks a program's name: a name alone, which is looked up as a shell would look it up,
/// never a path.
pub fn check_program(name: &str) -> Result<String, String> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-');
    if name.is_empty()
        || !name.starts_with(|c: char| c.is_ascii_alphanumeric())
        || !name.chars().all(allowed)
    {
        return Err(format!(
            "`{name}` is not a program's name: the name alone, as in `git`, without a folder"
        ));
    }
    Ok(name.to_owned())
}

/// Checks a server: `host`, `host:port` or `host:*`, where the host is a name, `*.name`
/// for every name under one, an IPv4 address, `[an IPv6 address]`, or `*` for any. A host
/// without a port means ports 80 and 443.
pub fn check_host(written: &str) -> Result<String, String> {
    let lower = written.to_ascii_lowercase();
    let (host, port) = split_host(&lower)
        .ok_or_else(|| format!("`{written}` is not a server: a host name and maybe a port"))?;
    if let Some(port) = port
        && port != "*"
        && !port.parse::<u16>().is_ok_and(|port| port > 0)
    {
        return Err(format!("`{written}`: {port} is not a port"));
    }
    let name = host.strip_prefix("*.").unwrap_or(host);
    let label = |label: &str| {
        !label.is_empty()
            && label.len() <= 63
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    };
    let valid = host == "*"
        || (host.starts_with('[')
            && host.ends_with(']')
            && host[1..host.len() - 1]
                .parse::<std::net::Ipv6Addr>()
                .is_ok())
        || (!name.is_empty() && name.split('.').all(label));
    if !valid {
        return Err(format!("`{written}`: {host} is not a host name"));
    }
    Ok(lower)
}

/// The host and the port of a server as written.
pub fn split_host(written: &str) -> Option<(&str, Option<&str>)> {
    if written.starts_with('[') {
        let end = written.find(']')?;
        let (host, rest) = written.split_at(end + 1);
        return match rest {
            "" => Some((host, None)),
            _ => Some((host, Some(rest.strip_prefix(':')?))),
        };
    }
    match written.split_once(':') {
        Some((host, port)) => Some((host, Some(port))),
        None => Some((written, None)),
    }
}

/// Writes permissions back as a table, as a lock file keeps grants.
pub fn to_table(permissions: &Permissions) -> toml::Table {
    let mut table = toml::Table::new();
    for request in &permissions.required {
        let scope = || {
            toml::Value::Array(
                request
                    .scope
                    .iter()
                    .cloned()
                    .map(toml::Value::String)
                    .collect(),
            )
        };
        let value = match (request.scope.is_empty(), request.prompt) {
            (true, Prompt::Once) => toml::Value::Boolean(true),
            (false, Prompt::Once) => scope(),
            (empty, Prompt::Each) => {
                let mut inner = toml::Table::new();
                if !empty {
                    inner.insert("scope".to_owned(), scope());
                }
                inner.insert("prompt".to_owned(), toml::Value::String("each".to_owned()));
                toml::Value::Table(inner)
            }
        };
        table.insert(request.kind.as_str().to_owned(), value);
    }
    if !permissions.optional.is_empty() {
        let mut optional = toml::Table::new();
        for entry in &permissions.optional {
            let value = match &entry.within {
                None => toml::Value::Boolean(true),
                Some(within) => {
                    toml::Value::Array(within.iter().cloned().map(toml::Value::String).collect())
                }
            };
            optional.insert(entry.kind.as_str().to_owned(), value);
        }
        table.insert(OPTIONAL_KEY.to_owned(), toml::Value::Table(optional));
    }
    table
}

/// One thing granted: a kind, one part of its scope, and whether each use is asked
/// about. Comparing these says whether new permissions are wider than old ones.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Item {
    pub kind: Kind,
    pub scope: Option<String>,
    pub prompt: Prompt,
}

/// The items of some requests.
pub fn items(requests: &[Request]) -> BTreeSet<Item> {
    let mut items = BTreeSet::new();
    for request in requests {
        if request.scope.is_empty() {
            items.insert(Item {
                kind: request.kind,
                scope: None,
                prompt: request.prompt,
            });
        }
        for scope in &request.scope {
            items.insert(Item {
                kind: request.kind,
                scope: Some(scope.clone()),
                prompt: request.prompt,
            });
        }
    }
    items
}

/// What `wanted` asks for that `granted` does not already allow. A grant that asked
/// before each use does not cover one that would not ask.
pub fn new_items(granted: &[Request], wanted: &[Request]) -> Vec<Item> {
    let granted = items(granted);
    items(wanted)
        .into_iter()
        .filter(|item| {
            !granted.iter().any(|old| {
                old.kind == item.kind
                    && old.scope == item.scope
                    && (old.prompt == Prompt::Once || item.prompt == Prompt::Each)
            })
        })
        .collect()
}

/// Programs that can do anything the user can, given the right arguments.
const GENERAL_PROGRAMS: [&str; 22] = [
    "sh",
    "bash",
    "zsh",
    "fish",
    "dash",
    "ksh",
    "cmd",
    "cmd.exe",
    "powershell",
    "pwsh",
    "python",
    "python3",
    "node",
    "deno",
    "bun",
    "perl",
    "ruby",
    "lua",
    "git",
    "wsl",
    "ssh",
    "env",
];

/// Whether one of these programs can do anything the user can, given the right arguments.
pub fn can_do_anything(programs: &[String]) -> bool {
    programs
        .iter()
        .any(|program| GENERAL_PROGRAMS.contains(&program.to_ascii_lowercase().as_str()))
}

/// Whether a path pattern covers the whole home folder or a whole disk.
fn covers_everything(pattern: &str) -> bool {
    let pattern = pattern.trim_end_matches("/**").trim_end_matches('/');
    let bytes = pattern.as_bytes();
    let drive = bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    pattern.is_empty() || pattern == "${home}" || drive
}

/// A server's host and its ports, in words.
fn describe_host(host: &str) -> String {
    let (name, port) = split_host(host).unwrap_or((host, None));
    let name = match name {
        "*" => "any server".to_owned(),
        other if other.starts_with("*.") => format!("any server under {}", &other[2..]),
        "localhost" | "127.0.0.1" | "[::1]" => format!("{other} (this computer)", other = name),
        other => other.to_owned(),
    };
    match port {
        None => format!("{name} (ports 80 and 443)"),
        Some("*") => format!("{name} (any port)"),
        Some(port) => format!("{name} (port {port})"),
    }
}

/// What a permission lets a plugin do, in words for the person installing it, with a
/// warning where that is close to trusting it with everything.
pub fn describe(request: &Request) -> Description {
    let list = || request.scope.join(", ");
    let mut warning = None;
    let what = match request.kind {
        Kind::FsRead => format!("Read files in: {}", list()),
        Kind::FsWrite => {
            if request.scope.iter().any(|scope| covers_everything(scope)) {
                warning = Some(
                    "Changing any file in your home folder is close to trusting this plugin \
                     with your account."
                        .to_owned(),
                );
            }
            format!("Create, change and delete files in: {}", list())
        }
        Kind::ProcessRun => {
            if can_do_anything(&request.scope) {
                warning = Some(
                    "A program such as a shell or git can do anything you can, so running it \
                     is close to trusting this plugin with your account."
                        .to_owned(),
                );
            }
            format!("Run these programs, with any arguments: {}", list())
        }
        Kind::ProcessRunElsewhere => format!(
            "On other machines that your terminals are on (servers, SSH, WSL), each once you \
             have allowed it there: run these programs, with any arguments: {}",
            list()
        ),
        Kind::Net => {
            let hosts: Vec<String> = request
                .scope
                .iter()
                .map(|host| describe_host(host))
                .collect();
            format!("Connect to: {}", hosts.join(", "))
        }
        Kind::PanesRead => {
            warning = Some(
                "That includes anything typed or printed there, passwords and keys too.".to_owned(),
            );
            "Read what your terminals show, and their scrollback".to_owned()
        }
        Kind::PanesWrite => "Type into your terminals".to_owned(),
        Kind::SessionsSpawn => {
            "Start new terminals, from your profiles or with a program it may run".to_owned()
        }
        Kind::WorkspaceRead => {
            "See your workspace: its windows, tabs and panes, their folders, and what runs \
             in them"
                .to_owned()
        }
        Kind::LayoutWrite => "Open, close, move and split your tabs and panes".to_owned(),
        Kind::ClipboardRead => "Read what you copy".to_owned(),
        Kind::ClipboardWrite => "Put text on the clipboard".to_owned(),
        Kind::SystemMetrics => {
            "See how busy the processor, memory, graphics card and network are".to_owned()
        }
        Kind::ServicesUse => format!("Use what these plugins offer: {}", list()),
    };
    let what = match request.prompt {
        Prompt::Once => what,
        Prompt::Each => match request.kind {
            Kind::PanesWrite => format!("{what}, asking you first each time with the text"),
            _ => format!("{what}, asking you first each time with the command"),
        },
    };
    Description { what, warning }
}

/// A permission in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Description {
    pub what: String,
    pub warning: Option<String>,
}

/// Some permission items in words, put back together by kind and by whether the user is
/// asked each time, as a manifest would have asked for them.
pub fn describe_items(items: &[Item]) -> Vec<Description> {
    let mut requests: Vec<Request> = Vec::new();
    for item in items {
        match requests
            .iter_mut()
            .find(|request| request.kind == item.kind && request.prompt == item.prompt)
        {
            Some(request) => request.scope.extend(item.scope.clone()),
            None => requests.push(Request {
                kind: item.kind,
                scope: item.scope.clone().into_iter().collect(),
                prompt: item.prompt,
            }),
        }
    }
    requests.iter().map(describe).collect()
}

/// What an optional permission would let a plugin ask for, in words.
pub fn describe_optional(optional: &Optional) -> String {
    let request = Request {
        kind: optional.kind,
        scope: optional.within.clone().unwrap_or_default(),
        prompt: Prompt::Once,
    };
    match &optional.within {
        Some(_) => describe(&request).what,
        None => match optional.kind.scope() {
            ScopeKind::None => describe(&request).what,
            ScopeKind::Paths if optional.kind == Kind::FsRead => "Read files".to_owned(),
            ScopeKind::Paths => "Create, change and delete files".to_owned(),
            ScopeKind::Programs if optional.kind == Kind::ProcessRunElsewhere => {
                "Run programs on other machines that your terminals are on".to_owned()
            }
            ScopeKind::Programs => "Run programs".to_owned(),
            ScopeKind::Hosts => "Connect to servers".to_owned(),
            ScopeKind::Plugins => "Use what other plugins offer".to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> toml::Table {
        text.parse().unwrap()
    }

    #[test]
    fn a_manifest_asks_for_kinds_with_their_scopes() {
        let permissions = parse_table(&table(
            r#"
"system.metrics" = true
"fs.read" = ["${workspace}/**", '${home}\notes\*.md', "${workspace}/**"]
"process.run" = { scope = ["git"], prompt = "each" }
"process.run-elsewhere" = ["git"]
"net" = ["API.github.com", "localhost:8080", "*.example.org:*", "[::1]:9000"]

[optional]
"clipboard.read" = true
"fs.write" = ["${plugin-data}/**"]
"#,
        ))
        .unwrap();
        assert_eq!(permissions.required.len(), 5);
        let fs = permissions
            .required
            .iter()
            .find(|request| request.kind == Kind::FsRead)
            .unwrap();
        assert_eq!(fs.scope, ["${home}/notes/*.md", "${workspace}/**"]);
        let net = permissions
            .required
            .iter()
            .find(|request| request.kind == Kind::Net)
            .unwrap();
        assert_eq!(
            net.scope,
            [
                "*.example.org:*",
                "[::1]:9000",
                "api.github.com",
                "localhost:8080"
            ]
        );
        let run = permissions
            .required
            .iter()
            .find(|request| request.kind == Kind::ProcessRun)
            .unwrap();
        assert_eq!(run.prompt, Prompt::Each);
        assert_eq!(permissions.optional.len(), 2);

        // Written back and read again, nothing changes.
        assert_eq!(parse_table(&to_table(&permissions)).unwrap(), permissions);
    }

    #[test]
    fn what_cannot_be_granted_is_refused_with_the_reason() {
        for (text, says) in [
            (r#""fs.reed" = true"#, "not a permission"),
            (r#""system.metrics" = false"#, "leave out"),
            (r#""system.metrics" = ["x"]"#, "has no scope"),
            (r#""fs.read" = true"#, "needs a scope"),
            (r#""fs.read" = ["relative/path"]"#, "must be absolute"),
            (r#""fs.read" = ["${workspace}/../x"]"#, "`..`"),
            (
                r#""fs.read" = ["${secret}/x"]"#,
                "variables a path may begin with",
            ),
            (r#""fs.read" = ["${workspace}x"]"#, "followed by `/`"),
            (r#""process.run" = ["/usr/bin/git"]"#, "without a folder"),
            (
                r#""process.run-elsewhere" = ["../git"]"#,
                "without a folder",
            ),
            (r#""process.run-elsewhere" = true"#, "needs a scope"),
            (r#""net" = ["exa mple.org"]"#, "not a host name"),
            (r#""net" = ["example.org:99999"]"#, "not a port"),
            (
                r#""clipboard.read" = { prompt = "each" }"#,
                "only panes.write, process.run",
            ),
            (r#""services.use" = ["Acme"]"#, "not a plugin id"),
            (
                r#""panes.write" = { prompt = "sometimes" }"#,
                "\"each\" or \"once\"",
            ),
        ] {
            let problems = parse_table(&table(text)).unwrap_err();
            assert!(
                problems.iter().any(|problem| problem.contains(says)),
                "{text}: {problems:?}"
            );
        }
    }

    #[test]
    fn only_what_was_not_granted_before_is_new() {
        let read = |text: &str| parse_table(&table(text)).unwrap().required;
        let granted = read(
            r#"
"fs.read" = ["${workspace}/**"]
"process.run" = { scope = ["git"], prompt = "each" }
"#,
        );
        assert!(new_items(&granted, &granted).is_empty());
        let wanted = read(
            r#"
"fs.read" = ["${workspace}/**", "${home}/.gitconfig"]
"process.run" = ["git"]
"system.metrics" = true
"#,
        );
        let new = new_items(&granted, &wanted);
        let scopes: Vec<(Kind, Option<&str>)> = new
            .iter()
            .map(|item| (item.kind, item.scope.as_deref()))
            .collect();
        // Running git without asking is wider than running it after asking.
        assert_eq!(
            scopes,
            [
                (Kind::FsRead, Some("${home}/.gitconfig")),
                (Kind::ProcessRun, Some("git")),
                (Kind::SystemMetrics, None),
            ]
        );
        // Asking each time where it did not before is narrower: nothing new.
        let each = read(r#""process.run" = { scope = ["git"], prompt = "each" }"#);
        assert!(new_items(&read(r#""process.run" = ["git"]"#), &each).is_empty());
    }

    #[test]
    fn permissions_are_put_in_plain_words_with_a_warning_where_they_are_wide() {
        let read = |text: &str| parse_table(&table(text)).unwrap().required.remove(0);
        let git = describe(&read(r#""process.run" = ["git"]"#));
        assert_eq!(git.what, "Run these programs, with any arguments: git");
        assert!(git.warning.is_some());
        assert!(
            describe(&read(r#""process.run" = ["rg"]"#))
                .warning
                .is_none()
        );
        assert!(describe(&read(r#""panes.read" = true"#)).warning.is_some());
        assert!(
            describe(&read(r#""fs.write" = ["${home}/**"]"#))
                .warning
                .is_some()
        );
        assert_eq!(
            describe(&read(r#""net" = ["localhost:8080", "*.acme.dev"]"#)).what,
            "Connect to: any server under acme.dev (ports 80 and 443), localhost (this \
             computer) (port 8080)"
        );
        assert_eq!(
            describe(&read(r#""panes.write" = { prompt = "each" }"#)).what,
            "Type into your terminals, asking you first each time with the text"
        );
    }
}
