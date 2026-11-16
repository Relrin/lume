//! The machines named in the user's OpenSSH configuration.
//!
//! Only the names are read: the `Host` lines of `~/.ssh/config` and of the files it
//! includes. What the configuration says about a host (its address, its keys, how it is
//! reached) is the client's business and is not looked at, so a name offered here is
//! given to the client as it is and means there exactly what the user wrote.

use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::tuning::{
    SSH_CONFIG_FILE_MAX_BYTES, SSH_CONFIG_FILES_MAX, SSH_HOSTS_MAX, SSH_INCLUDE_DEPTH_MAX,
};

/// The hosts of a configuration, and the files they were read from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SshHosts {
    /// The literal host names, each once, in the order they are first written.
    pub hosts: Vec<String>,
    /// Every file that was read, the configuration itself first. A change to any of
    /// them may change the hosts.
    pub files: Vec<PathBuf>,
}

/// Where the user's own OpenSSH configuration is, whether or not there is one.
pub fn user_config() -> Option<PathBuf> {
    std::env::home_dir().map(|home| home.join(".ssh").join("config"))
}

/// The hosts named in `config` and in the files it includes.
///
/// A name is taken if it is written out: a pattern with `*` or `?` in it, and one
/// negated with `!`, name no single machine. A file that is not there or cannot be read
/// gives nothing.
pub fn hosts(config: &Path) -> SshHosts {
    let mut reading = Reading {
        base: config.parent().map(Path::to_owned).unwrap_or_default(),
        home: std::env::home_dir(),
        found: SshHosts::default(),
        known: HashSet::new(),
        read: HashSet::new(),
    };
    reading.file(config, 0);
    reading.found
}

struct Reading {
    /// The folder a relative `Include` is looked for in: the configuration's own.
    base: PathBuf,
    home: Option<PathBuf>,
    found: SshHosts,
    known: HashSet<String>,
    /// The files read so far, so that none is read twice however they include each
    /// other.
    read: HashSet<PathBuf>,
}

/// The text of a file, if it is one of a size a person writes.
fn read_text(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(SSH_CONFIG_FILE_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > SSH_CONFIG_FILE_MAX_BYTES {
        return None;
    }
    let text = String::from_utf8_lossy(&bytes);
    Some(text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned())
}

/// A line's keyword and what follows it. The two are separated by blanks, by an equals
/// sign, or both.
fn split_keyword(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let (keyword, rest) = line.split_at(end);
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=').unwrap_or(rest).trim_start();
    Some((keyword, rest))
}

/// The words of a line's arguments. Blanks separate them; quotes keep blanks inside a
/// word; a backslash makes a quote or a backslash an ordinary character and is itself
/// ordinary before anything else, so a Windows path stays as written. A `#` where a
/// word would begin starts a comment.
fn words(arguments: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = arguments.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quote) {
            ('\\', _) if matches!(chars.peek(), Some('\\' | '"' | '\'')) => {
                word.extend(chars.next());
                in_word = true;
            }
            ('"' | '\'', None) => {
                quote = Some(c);
                in_word = true;
            }
            (_, Some(open)) if c == open => quote = None,
            (_, Some(_)) => word.push(c),
            ('#', None) if !in_word => break,
            (_, None) if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (_, None) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        out.push(word);
    }
    out
}

/// Whether a `Host` pattern names one machine, and by a name that can be handed to the
/// client: not something it would read as an option.
fn is_literal_host(pattern: &str) -> bool {
    !pattern.is_empty()
        && !pattern.starts_with(['!', '-'])
        && !pattern.contains(['*', '?'])
        && !pattern.chars().any(char::is_control)
}

/// Whether `name` matches `pattern`, in which `*` stands for any run of characters and
/// `?` for any one. Names differ by case only where the file system's do.
fn matches(pattern: &str, name: &str) -> bool {
    let same = |a: char, b: char| {
        if cfg!(windows) {
            a.eq_ignore_ascii_case(&b)
        } else {
            a == b
        }
    };
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    // Where the last `*` was, and how much of the name it had taken, to come back to
    // when what follows it does not fit.
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, n));
                p += 1;
            }
            Some(&c) if c == '?' || same(c, name[n]) => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some((at, taken)) => {
                    star = Some((at, taken + 1));
                    p = at + 1;
                    n = taken + 1;
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

impl Reading {
    fn full(&self) -> bool {
        self.found.hosts.len() >= SSH_HOSTS_MAX
    }

    fn file(&mut self, path: &Path, depth: usize) {
        if depth > SSH_INCLUDE_DEPTH_MAX || self.found.files.len() >= SSH_CONFIG_FILES_MAX {
            return;
        }
        // The same file under two spellings is one file.
        let identity = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        if self.read.contains(&identity) {
            return;
        }
        let Some(text) = read_text(path) else {
            return;
        };
        self.read.insert(identity);
        self.found.files.push(path.to_owned());

        for line in text.lines() {
            let Some((keyword, arguments)) = split_keyword(line) else {
                continue;
            };
            if keyword.eq_ignore_ascii_case("host") {
                for pattern in words(arguments) {
                    if is_literal_host(&pattern)
                        && !self.full()
                        && self.known.insert(pattern.clone())
                    {
                        self.found.hosts.push(pattern);
                    }
                }
            } else if keyword.eq_ignore_ascii_case("include") {
                for included in words(arguments) {
                    for file in self.included(&included) {
                        self.file(&file, depth + 1);
                    }
                }
            }
        }
    }

    /// The files an `Include` argument names, in the order of their names.
    fn included(&self, argument: &str) -> Vec<PathBuf> {
        let path = if let Some(rest) = argument
            .strip_prefix("~/")
            .or_else(|| argument.strip_prefix("~\\"))
        {
            match &self.home {
                Some(home) => home.join(rest),
                None => return Vec::new(),
            }
        } else if argument.starts_with('~') {
            // Another user's home is not something to go looking for.
            return Vec::new();
        } else {
            let path = PathBuf::from(argument);
            if path.is_absolute() {
                path
            } else {
                self.base.join(path)
            }
        };

        let wild = |text: &str| text.contains(['*', '?']);
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return Vec::new();
        };
        if !wild(name) {
            return vec![path];
        }
        let Some(folder) = path.parent() else {
            return Vec::new();
        };
        if wild(&folder.to_string_lossy()) {
            // Only a file's name may stand for several; a folder's is taken as written
            // and so names nothing.
            return Vec::new();
        }
        let Ok(entries) = std::fs::read_dir(folder) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            // As in a shell, a name that begins with a dot is matched only by a
            // pattern that begins with one.
            .filter(|found| !found.starts_with('.') || name.starts_with('.'))
            .filter(|found| matches(name, found))
            .collect();
        names.sort();
        names.into_iter().map(|found| folder.join(found)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own for a test, removed when the test is over.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let folder =
                std::env::temp_dir().join(format!("lume-ssh-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&folder);
            std::fs::create_dir_all(&folder).unwrap();
            Self(folder)
        }

        fn write(&self, file: &str, text: &str) -> PathBuf {
            let path = self.0.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
            path
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn names(found: &SshHosts) -> Vec<&str> {
        found.hosts.iter().map(String::as_str).collect()
    }

    #[test]
    fn hosts_written_out_are_taken_and_patterns_are_not() {
        let folder = Folder::new("literal");
        let config = folder.write(
            "config",
            "# my machines\n\
             Host work\n\
             \x20   HostName 10.0.0.4\n\
             \x20   User me\n\
             \n\
             Host *.internal !bastion.internal\n\
             \x20   ProxyJump bastion\n\
             Host bastion db?  home\n\
             Host *\n\
             \x20   ServerAliveInterval 30\n",
        );
        let found = hosts(&config);
        assert_eq!(names(&found), ["work", "bastion", "home"]);
        assert_eq!(found.files, [config]);
    }

    #[test]
    fn the_keyword_may_be_written_any_way_the_client_takes() {
        let folder = Folder::new("forms");
        let config = folder.write(
            "config",
            "host one\n\
             HOST=two\n\
             Host = three four\n\
             \tHost\tfive\n\
             Host \"six\" 'seven'\n\
             Host eight # the last\n\
             Hostname nine\n\
             HostKeyAlias ten\n",
        );
        assert_eq!(
            names(&hosts(&config)),
            [
                "one", "two", "three", "four", "five", "six", "seven", "eight"
            ]
        );
    }

    #[test]
    fn a_name_is_listed_once_where_it_is_first_written() {
        let folder = Folder::new("twice");
        let config = folder.write("config", "Host b a\nHost a\nHost c b\n");
        assert_eq!(names(&hosts(&config)), ["b", "a", "c"]);
    }

    #[test]
    fn a_host_after_a_match_block_still_counts() {
        let folder = Folder::new("match");
        let config = folder.write(
            "config",
            "Match host *.example.com user admin\n\
             \x20   IdentityFile ~/.ssh/admin\n\
             Match all\n\
             Host after\n",
        );
        assert_eq!(names(&hosts(&config)), ["after"]);
    }

    #[test]
    fn a_name_the_client_would_read_as_an_option_is_dropped() {
        let folder = Folder::new("option");
        let config = folder.write(
            "config",
            "Host -oProxyCommand=calc good\nHost \"bad\u{7}name\" fine\n",
        );
        assert_eq!(names(&hosts(&config)), ["good", "fine"]);
    }

    #[test]
    fn included_files_are_read_where_they_are_included() {
        let folder = Folder::new("include");
        folder.write("conf.d/10-work.conf", "Host work-a work-b\n");
        folder.write("conf.d/20-home.conf", "Host home\n");
        folder.write("conf.d/.hidden.conf", "Host hidden\n");
        folder.write("conf.d/notes.txt", "Host notes\n");
        let extra = folder.write("elsewhere/extra", "Host extra\nInclude nested\n");
        // A path that is not absolute is looked for beside the configuration itself,
        // wherever the file that includes it is.
        folder.write("nested", "Host nested\n");
        let config = folder.write(
            "config",
            &format!(
                "Host first\n\
                 Include conf.d/*.conf \"{}\"\n\
                 Include missing conf.d/??-nothing\n\
                 Host last\n",
                extra.display()
            ),
        );
        let found = hosts(&config);
        assert_eq!(
            names(&found),
            [
                "first", "work-a", "work-b", "home", "extra", "nested", "last"
            ]
        );
        assert_eq!(
            found.files,
            [
                config,
                folder.0.join("conf.d").join("10-work.conf"),
                folder.0.join("conf.d").join("20-home.conf"),
                extra,
                folder.0.join("nested"),
            ]
        );
    }

    #[test]
    fn files_that_include_each_other_are_each_read_once() {
        let folder = Folder::new("cycle");
        folder.write("other", "Host other\nInclude config\n");
        let config = folder.write("config", "Host top\nInclude other\nInclude other\n");
        let found = hosts(&config);
        assert_eq!(names(&found), ["top", "other"]);
        assert_eq!(found.files.len(), 2);
    }

    #[test]
    fn including_stops_at_a_depth() {
        let folder = Folder::new("depth");
        let last = SSH_INCLUDE_DEPTH_MAX + 3;
        for level in 1..=last {
            folder.write(
                &format!("level{level}"),
                &format!("Host host{level}\nInclude level{}\n", level + 1),
            );
        }
        let config = folder.write("config", "Host host0\nInclude level1\n");
        let found = hosts(&config);
        // The configuration is at depth nought, so the deepest file read is the one
        // as many includes down as the limit.
        assert_eq!(found.hosts.len(), SSH_INCLUDE_DEPTH_MAX + 1);
        assert_eq!(
            found.hosts.last().unwrap(),
            &format!("host{SSH_INCLUDE_DEPTH_MAX}")
        );
    }

    #[test]
    fn a_file_that_is_not_there_gives_nothing() {
        let folder = Folder::new("missing");
        assert_eq!(hosts(&folder.0.join("config")), SshHosts::default());
        // Nor does a folder of that name.
        std::fs::create_dir_all(folder.0.join("dir")).unwrap();
        assert_eq!(hosts(&folder.0.join("dir")), SshHosts::default());
    }

    #[test]
    fn a_file_too_large_to_be_a_persons_is_not_read() {
        let folder = Folder::new("large");
        let line = "Host big\n";
        let lines = usize::try_from(SSH_CONFIG_FILE_MAX_BYTES).unwrap() / line.len() + 1;
        let config = folder.write("config", &line.repeat(lines));
        assert_eq!(hosts(&config), SshHosts::default());
    }

    #[test]
    fn words_are_split_as_the_client_splits_them() {
        let cases: [(&str, &[&str]); 8] = [
            ("a b  c", &["a", "b", "c"]),
            ("\"a b\" c", &["a b", "c"]),
            ("'a b'c", &["a bc"]),
            (r#""a \"b\"""#, &["a \"b\""]),
            (r"C:\Users\me\conf.d\*", &[r"C:\Users\me\conf.d\*"]),
            (r"a\\b", &[r"a\b"]),
            ("a # b", &["a"]),
            ("a#b \"\"", &["a#b", ""]),
        ];
        for (text, expected) in cases {
            assert_eq!(words(text), expected, "{text}");
        }
    }

    #[test]
    fn a_pattern_matches_names_as_a_shell_would() {
        let cases = [
            ("*.conf", "work.conf", true),
            ("*.conf", "work.conf.bak", false),
            ("*", "anything", true),
            ("??-*", "10-work", true),
            ("??-*", "1-work", false),
            ("a*b*c", "a-b-b-c", true),
            ("a*b*c", "a-b-b", false),
            ("exact", "exact", true),
            ("exact", "exact1", false),
            ("", "", true),
            ("*", "", true),
            ("?", "", false),
        ];
        for (pattern, name, expected) in cases {
            assert_eq!(matches(pattern, name), expected, "{pattern} against {name}");
        }
    }

    #[test]
    fn the_users_configuration_is_in_their_home() {
        let config = user_config().unwrap();
        assert!(config.is_absolute());
        assert!(config.ends_with(Path::new(".ssh").join("config")));
    }
}
