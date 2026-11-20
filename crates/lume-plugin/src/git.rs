//! Fetching a package from a git repository, with the system's `git`.
//!
//! One commit is fetched, without its history, into a folder of Lume's own, and checked
//! out there. Nothing of the repository is run: its hooks are pointed at an empty
//! folder, submodules are not followed, only the transports a user would expect are
//! allowed, and git may not stop to ask for a password. The commit that was fetched is
//! what the install is pinned to, beside the hash of the package's files.
//!
//! A source is written `<address>[#<ref>]`, where the address is anything `git fetch`
//! takes, or `owner/repo` for a repository on GitHub. The package may be in a folder of
//! the repository, named apart.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The most of what git printed that is shown when it fails.
const SHOWN_MAX: usize = 600;

/// A repository to take a package from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitSource {
    /// The address, as git is given it.
    pub url: String,
    /// The branch or tag to take; none is the repository's default branch.
    pub reference: Option<String>,
    /// The folder of the repository the package is in, written with `/`.
    pub subdir: Option<String>,
}

/// Why a package could not be fetched.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("{0}")]
    BadSource(String),
    #[error("git could not be run ({0}); installing from a repository needs git on the PATH")]
    NoGit(std::io::Error),
    #[error("git {what} failed: {output}")]
    Failed { what: &'static str, output: String },
    #[error("{path}: {error}")]
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
    #[error("there is no folder {0} in the repository")]
    NoSubdir(String),
}

/// A repository's commit, checked out.
#[derive(Debug)]
pub struct Fetched {
    /// The folder the package is in: the checkout, or the folder of it that was named.
    pub package: PathBuf,
    /// The commit that was checked out, in full.
    pub commit: String,
}

/// Whether what the user wrote names a repository rather than a folder or an archive
/// on this machine.
pub fn looks_like_git(text: &str) -> bool {
    let address = text.split_once('#').map_or(text, |(address, _)| address);
    if Path::new(address).exists() {
        return false;
    }
    address.contains("://")
        || address.starts_with("git@")
        || address.ends_with(".git")
        || is_owner_repo(address)
}

/// `owner/repo`, as GitHub names a repository.
fn is_owner_repo(text: &str) -> bool {
    let part = |part: &str| {
        !part.is_empty()
            && !part.starts_with(['.', '-'])
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    match text.split_once('/') {
        Some((owner, repo)) => part(owner) && part(repo) && !repo.contains('/'),
        None => false,
    }
}

impl GitSource {
    /// Reads `<address>[#<ref>]`, with the reference and the folder the user named
    /// apart, if any. A reference named apart must agree with one after `#`.
    pub fn parse(
        text: &str,
        reference: Option<&str>,
        subdir: Option<&str>,
    ) -> Result<Self, GitError> {
        let bad = |message: String| GitError::BadSource(message);
        let (address, after) = match text.split_once('#') {
            Some((address, after)) => (address, Some(after)),
            None => (text, None),
        };
        let reference = match (after, reference) {
            (Some(one), Some(other)) if one != other => {
                return Err(bad(format!(
                    "the source names {one} and --ref names {other}; name one of them"
                )));
            }
            (one, other) => one.or(other),
        };
        let address = address.trim();
        if address.is_empty() {
            return Err(bad("a repository's address is needed".to_owned()));
        }
        // An address that starts with `-` would be read by git as an option, and one
        // with `::` names a helper program to run.
        if address.starts_with('-') || address.chars().any(char::is_control) {
            return Err(bad(format!("{address} is not a repository's address")));
        }
        let scheme = address.split_once("://").map(|(scheme, _)| scheme);
        if address.contains("::")
            || scheme.is_some_and(|scheme| !matches!(scheme, "https" | "ssh" | "file"))
        {
            return Err(bad(format!(
                "{address} is not fetched: a repository is fetched over https or ssh"
            )));
        }
        let url = if scheme.is_none() && !address.starts_with("git@") && is_owner_repo(address) {
            format!("https://github.com/{address}.git")
        } else {
            address.to_owned()
        };
        if let Some(reference) = reference {
            let fine = !reference.is_empty()
                && !reference.starts_with('-')
                && !reference.contains("..")
                && reference.chars().all(|c| {
                    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '+' | '@')
                });
            if !fine {
                return Err(bad(format!(
                    "{reference} is not the name of a branch, a tag or a commit"
                )));
            }
        }
        let subdir = match subdir.map(|subdir| subdir.replace('\\', "/")) {
            None => None,
            Some(subdir) => {
                let parts: Vec<&str> = subdir.split('/').filter(|part| !part.is_empty()).collect();
                if parts.is_empty()
                    || parts.iter().any(|part| {
                        matches!(*part, "." | "..")
                            || part.contains(':')
                            || part.chars().any(char::is_control)
                    })
                {
                    return Err(bad(format!(
                        "{subdir} is not a folder inside the repository"
                    )));
                }
                Some(parts.join("/"))
            }
        };
        Ok(Self {
            url,
            reference: reference.map(str::to_owned),
            subdir,
        })
    }

    /// The source as the user is shown it.
    pub fn shown(&self) -> String {
        let mut text = self.url.clone();
        if let Some(reference) = &self.reference {
            text.push('#');
            text.push_str(reference);
        }
        if let Some(subdir) = &self.subdir {
            text.push_str(" (");
            text.push_str(subdir);
            text.push(')');
        }
        text
    }
}

/// A git command that runs nothing of the repository's and never stops to ask.
fn git(folder: &Path, hooks: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(folder)
        .arg("-c")
        .arg(format!("core.hooksPath={}", hooks.display()))
        .args(["-c", "protocol.allow=never"])
        .args(["-c", "protocol.https.allow=always"])
        .args(["-c", "protocol.ssh.allow=always"])
        .args(["-c", "protocol.file.allow=always"])
        .args(["-c", "submodule.recurse=false"])
        .args(["-c", "core.symlinks=false"])
        .args(["-c", "core.autocrlf=false"])
        .args(["-c", "advice.detachedHead=false"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // An ssh that cannot ask for a password or about a host's key fails instead.
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // No console window, when the application itself has none.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn run(mut command: Command, what: &'static str) -> Result<String, GitError> {
    let output = command.output().map_err(GitError::NoGit)?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
    }
    let mut shown = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if shown.is_empty() {
        shown = format!("it ended with {}", output.status);
    }
    if shown.len() > SHOWN_MAX {
        let mut end = SHOWN_MAX;
        while !shown.is_char_boundary(end) {
            end -= 1;
        }
        shown.truncate(end);
        shown.push('…');
    }
    Err(GitError::Failed {
        what,
        output: shown,
    })
}

/// Fetches the source's commit into `into`, which must not exist yet, and checks it
/// out there.
pub fn fetch(source: &GitSource, into: &Path) -> Result<Fetched, GitError> {
    let io = |path: &Path| {
        let path = path.to_owned();
        move |error| GitError::Io { path, error }
    };
    let checkout = into.join("repository");
    let hooks = into.join("no-hooks");
    std::fs::create_dir_all(&checkout).map_err(io(&checkout))?;
    std::fs::create_dir_all(&hooks).map_err(io(&hooks))?;

    let mut init = git(&checkout, &hooks);
    init.args(["init", "--quiet"]);
    run(init, "init")?;

    let mut fetch = git(&checkout, &hooks);
    fetch.args([
        "fetch",
        "--quiet",
        "--depth",
        "1",
        "--no-tags",
        "--no-recurse-submodules",
        "--",
    ]);
    fetch.arg(&source.url);
    fetch.arg(source.reference.as_deref().unwrap_or("HEAD"));
    run(fetch, "fetch")?;

    let mut checked_out = git(&checkout, &hooks);
    checked_out.args(["checkout", "--quiet", "--detach", "FETCH_HEAD"]);
    run(checked_out, "checkout")?;

    let mut parsed = git(&checkout, &hooks);
    parsed.args(["rev-parse", "--verify", "HEAD"]);
    let commit = run(parsed, "rev-parse")?;
    if commit.len() < 40 || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(GitError::Failed {
            what: "rev-parse",
            output: format!("{commit} is not a commit"),
        });
    }

    let package = match &source.subdir {
        None => checkout,
        Some(subdir) => {
            let folder = subdir
                .split('/')
                .fold(checkout, |folder, part| folder.join(part));
            // The entry itself: a link out of the repository is not a folder of it.
            let is_folder = std::fs::symlink_metadata(&folder)
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false);
            if !is_folder {
                return Err(GitError::NoSubdir(subdir.clone()));
            }
            folder
        }
    };
    Ok(Fetched { package, commit })
}

#[cfg(test)]
pub mod testing {
    //! Repositories for the tests, made with the system's git in a temporary folder.

    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// Runs git in `folder` as someone with a name, whatever this machine is set to.
    pub fn git(folder: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(folder)
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.org"])
            .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
            .args(["-c", "init.defaultBranch=main"])
            .args(args)
            .output()
            .expect("git is installed");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    /// Makes `folder` a repository with everything in it committed. Returns the commit.
    pub fn commit_all(folder: &Path, message: &str) -> String {
        if !folder.join(".git").exists() {
            git(folder, &["init", "--quiet"]);
        }
        git(folder, &["add", "-A"]);
        git(folder, &["commit", "--quiet", "-m", message]);
        git(folder, &["rev-parse", "HEAD"])
    }

    /// The address of a repository on this machine, as a `file` address.
    pub fn address(folder: &Path) -> String {
        let path: PathBuf = std::path::absolute(folder).expect("the path can be made whole");
        let slashed = path.display().to_string().replace('\\', "/");
        if slashed.starts_with('/') {
            format!("file://{slashed}")
        } else {
            format!("file:///{slashed}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{address, commit_all, git as run_git};
    use super::*;
    use crate::store::testing::TempDir;

    #[test]
    fn a_source_is_read_with_its_reference_and_its_folder() {
        let source = GitSource::parse("acme/tools#v1.2", None, Some("plugins\\review/")).unwrap();
        assert_eq!(source.url, "https://github.com/acme/tools.git");
        assert_eq!(source.reference.as_deref(), Some("v1.2"));
        assert_eq!(source.subdir.as_deref(), Some("plugins/review"));
        assert_eq!(
            source.shown(),
            "https://github.com/acme/tools.git#v1.2 (plugins/review)"
        );

        let source = GitSource::parse("git@example.org:acme/tools.git", Some("main"), None);
        let source = source.unwrap();
        assert_eq!(source.url, "git@example.org:acme/tools.git");
        assert_eq!(source.reference.as_deref(), Some("main"));

        assert!(GitSource::parse("acme/tools#a", Some("a"), None).is_ok());
        for (text, reference, subdir) in [
            ("acme/tools#a", Some("b"), None),
            ("", None, None),
            ("--upload-pack=evil", None, None),
            ("ext::sh -c evil", None, None),
            ("http://example.org/x.git", None, None),
            ("git://example.org/x.git", None, None),
            ("acme/tools", Some("--evil"), None),
            ("acme/tools", Some("a..b"), None),
            ("acme/tools", None, Some("../outside")),
            ("acme/tools", None, Some("C:/outside")),
            ("acme/tools", None, Some("/")),
        ] {
            assert!(
                GitSource::parse(text, reference, subdir).is_err(),
                "{text} {reference:?} {subdir:?}"
            );
        }
    }

    #[test]
    fn what_names_a_repository_is_told_from_what_is_on_this_machine() {
        let dir = TempDir::new("looks");
        dir.write("acme/tools/plugin.toml", "");
        let here = dir.path().join("acme").join("tools");
        assert!(!looks_like_git(&here.display().to_string()));
        for git in [
            "https://example.org/x",
            "git@example.org:x/y.git",
            "acme/tools",
            "acme/tools#v1",
            "ssh://example.org/x.git",
        ] {
            assert!(looks_like_git(git), "{git}");
        }
        for not in ["./plugin", "plugin.tar.gz", "D:\\src\\plugin", "a/b/c"] {
            assert!(!looks_like_git(not), "{not}");
        }
    }

    #[test]
    fn one_commit_is_fetched_and_named_from_a_branch_a_tag_or_a_folder() {
        let dir = TempDir::new("git-fetch");
        dir.write("origin/plugins/one/plugin.toml", "one");
        dir.write("origin/README.md", "first");
        let origin = dir.path().join("origin");
        let first = commit_all(&origin, "first");
        run_git(&origin, &["tag", "v1"]);
        dir.write("origin/README.md", "second");
        let second = commit_all(&origin, "second");

        let whole = GitSource::parse(&address(&origin), None, None).unwrap();
        let fetched = fetch(&whole, &dir.path().join("a")).unwrap();
        assert_eq!(fetched.commit, second);
        assert_eq!(
            std::fs::read_to_string(fetched.package.join("README.md")).unwrap(),
            "second"
        );

        let tagged = GitSource::parse(&address(&origin), Some("v1"), Some("plugins/one")).unwrap();
        let fetched = fetch(&tagged, &dir.path().join("b")).unwrap();
        assert_eq!(fetched.commit, first);
        assert!(fetched.package.ends_with("plugins/one"));
        assert!(fetched.package.join("plugin.toml").is_file());
        // Only that commit was fetched.
        let count = run_git(
            &dir.path().join("b").join("repository"),
            &["rev-list", "--count", "HEAD"],
        );
        assert_eq!(count, "1");

        let missing = GitSource::parse(&address(&origin), None, Some("plugins/two")).unwrap();
        assert!(matches!(
            fetch(&missing, &dir.path().join("c")),
            Err(GitError::NoSubdir(_))
        ));
        let nowhere = GitSource::parse(&address(&origin), Some("no-such-branch"), None).unwrap();
        assert!(matches!(
            fetch(&nowhere, &dir.path().join("d")),
            Err(GitError::Failed { what: "fetch", .. })
        ));
    }
}
