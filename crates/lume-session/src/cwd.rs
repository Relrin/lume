//! From the directory a shell reports to a directory a new terminal may start in.
//!
//! A shell says where it is in a URL, `file://<machine>/<path>`, and a terminal opened
//! beside it should start there. But the report is only bytes in the terminal's
//! output: a program on another machine, or a file that was printed, can say the same
//! thing. So a report is used for one thing, picking where a new terminal on this
//! machine starts, and only when everything about it checks out:
//!
//! - the session that reported it runs on this machine (a shell, or a shell in a Linux
//!   distribution here), never over a connection to another;
//! - the machine it names is this one;
//! - the path is a directory that is there.
//!
//! What a report that passes can do is start a shell in a directory the user could
//! have gone to anyway. This is the only place a reported directory becomes a path.

use std::path::Path;

use lume_proto::Launcher;

use crate::shell::program_stem;
use crate::wsl;

/// The longest machine name the older Windows naming keeps. A longer name is cut to
/// this in the variable that holds the machine's name.
const NETBIOS_NAME_MAX_CHARS: usize = 15;

/// What of this machine a report is checked against.
pub struct Machine<'a> {
    /// The machine's name, as the system gives it.
    pub name: &'a str,
    pub windows: bool,
    pub is_dir: &'a dyn Fn(&Path) -> bool,
}

/// This machine's name, or nothing where the system would not say.
pub fn machine_name() -> Option<String> {
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME")
            .ok()
            .filter(|name| !name.is_empty())
    }
    #[cfg(unix)]
    {
        let mut buffer = [0u8; 256];
        // SAFETY: the buffer is valid for its length, and one byte is kept back so
        // that the name is terminated even when the system fills what it was given.
        let failed =
            unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len() - 1) } != 0;
        if failed {
            return None;
        }
        let end = buffer.iter().position(|&byte| byte == 0)?;
        String::from_utf8(buffer[..end].to_vec())
            .ok()
            .filter(|name| !name.is_empty())
    }
}

/// The machine and the path a `file:` URL names, the path with its escapes undone.
/// `None` for anything that is not such a URL.
pub fn parse_file_url(url: &str) -> Option<(String, String)> {
    let scheme = url.get(..7)?;
    if !scheme.eq_ignore_ascii_case("file://") {
        return None;
    }
    let rest = &url[7..];
    let slash = rest.find('/')?;
    let (host, path) = rest.split_at(slash);
    let path = String::from_utf8(percent_decode(path.as_bytes())?).ok()?;
    // A path with a control character in it is nobody's directory.
    if path.chars().any(char::is_control) {
        return None;
    }
    Some((host.to_owned(), path))
}

/// The `file:` URL of a directory of this machine, as a shell in it would report it. A
/// share of another machine has none, and neither has a path that does not begin at a
/// root or a drive.
pub fn file_url_of(path: &str) -> Option<String> {
    let mut chars = path.chars();
    let drive = chars.next()?.is_ascii_alphabetic()
        && chars.next() == Some(':')
        && matches!(chars.next(), Some('\\' | '/'));
    let path = if drive {
        format!("/{}", path.replace('\\', "/"))
    } else if path.starts_with('/') && !path.starts_with("//") {
        path.to_owned()
    } else {
        return None;
    };
    let mut url = String::from("file://");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/:._-~".contains(&byte) {
            url.push(char::from(byte));
        } else {
            url.push_str(&format!("%{byte:02X}"));
        }
    }
    Some(url)
}

/// The directory a `file:` URL names, as it is shown to the user: a path of a Windows
/// drive as Windows writes it, any other path as it is. Nothing is checked: this is
/// for showing, never for starting a program in.
pub fn shown_dir(url: &str) -> Option<String> {
    let (_, path) = parse_file_url(url)?;
    Some(windows_path(&path).unwrap_or(path))
}

/// The bytes a percent-encoded path stands for. `None` when a percent sign begins no
/// encoding: a shell that reports a path encodes every percent sign in it.
fn percent_decode(text: &[u8]) -> Option<Vec<u8>> {
    let hex = |byte: u8| char::from(byte).to_digit(16).map(|digit| digit as u8);
    let mut out = Vec::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        if text[at] == b'%' {
            let high = hex(*text.get(at + 1)?)?;
            let low = hex(*text.get(at + 2)?)?;
            out.push(high << 4 | low);
            at += 3;
        } else {
            out.push(text[at]);
            at += 1;
        }
    }
    Some(out)
}

/// Whether a report names this machine. A name is compared up to its first dot and
/// without regard to case, since a shell may give the machine's full name and the
/// system its short one; no name at all, and `localhost`, are this machine too.
fn is_this_machine(reported: &str, machine: &Machine<'_>) -> bool {
    if reported.is_empty() || reported.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let short = |name: &str| name.split('.').next().unwrap_or_default().to_lowercase();
    let (reported, own) = (short(reported), short(machine.name));
    if own.is_empty() {
        return false;
    }
    if reported == own {
        return true;
    }
    // Windows may hold only the beginning of a long name.
    machine.windows && own.chars().count() == NETBIOS_NAME_MAX_CHARS && reported.starts_with(&own)
}

/// A URL's path as a path of a Windows drive: `/C:/Users/me` is `C:\Users\me`.
/// A path on no drive has none.
fn windows_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix('/')?;
    let mut chars = rest.chars();
    let drive = chars.next().filter(char::is_ascii_alphabetic)?;
    if chars.next() != Some(':') {
        return None;
    }
    let below = chars.as_str();
    if !(below.is_empty() || below.starts_with('/')) {
        return None;
    }
    let mut native = format!("{}:", drive.to_ascii_uppercase());
    native.push_str(&below.replace('/', "\\"));
    if below.is_empty() {
        native.push('\\');
    }
    Some(native)
}

/// Whether what a session runs is known to be a connection to another machine, whose
/// directories are not this one's: one over SSH, or the SSH client run as a program.
pub fn is_elsewhere(launcher: &Launcher) -> bool {
    match launcher {
        Launcher::Ssh { .. } => true,
        Launcher::Shell { program, .. } => program_stem(program) == "ssh",
        _ => false,
    }
}

/// The directory a new terminal may start in, going by what a session that runs
/// `launcher` reported as its directory. `None` unless the report passes every check
/// this module's description lists.
///
/// For a shell on this machine the result is a path of this machine. For a shell in a
/// Linux distribution it is the path inside the distribution.
pub fn start_dir_on(launcher: &Launcher, url: &str, machine: &Machine<'_>) -> Option<String> {
    let (host, path) = parse_file_url(url)?;
    if is_elsewhere(launcher) || !is_this_machine(&host, machine) {
        return None;
    }
    match launcher {
        Launcher::Shell { .. } => {
            // A URL's path starts at the root, so either spelling is absolute.
            let native = if machine.windows {
                windows_path(&path)?
            } else {
                path
            };
            (machine.is_dir)(Path::new(&native)).then_some(native)
        }
        Launcher::Wsl { distro, .. } => {
            if !machine.windows {
                return None;
            }
            let unc = wsl::linux_to_unc(distro, &path)?;
            (machine.is_dir)(Path::new(&unc)).then_some(path)
        }
        // Another machine's directory is not one of this machine's, whatever it is
        // called.
        _ => None,
    }
}

/// [`start_dir_on`] for the machine this runs on. It looks at the file system, which
/// for a distribution's directory may take a moment.
pub fn start_dir_from_report(launcher: &Launcher, url: &str) -> Option<String> {
    let name = machine_name()?;
    start_dir_on(
        launcher,
        url,
        &Machine {
            name: &name,
            windows: cfg!(windows),
            is_dir: &|dir| dir.is_dir(),
        },
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn shell() -> Launcher {
        Launcher::Shell {
            program: "sh".to_owned(),
            args: Vec::new(),
            login: false,
        }
    }

    fn wsl(distro: &str) -> Launcher {
        Launcher::Wsl {
            distro: distro.to_owned(),
            user: None,
        }
    }

    fn ssh() -> Launcher {
        Launcher::Ssh {
            host: "box".to_owned(),
            user: None,
            port: None,
            extra_args: Vec::new(),
            client: None,
        }
    }

    /// What `launcher`'s report `url` comes to on a machine called `name` that has
    /// exactly the directories `dirs`.
    fn on(
        windows: bool,
        name: &str,
        dirs: &[&str],
        launcher: &Launcher,
        url: &str,
    ) -> Option<String> {
        let dirs: Vec<PathBuf> = dirs.iter().map(PathBuf::from).collect();
        start_dir_on(
            launcher,
            url,
            &Machine {
                name,
                windows,
                is_dir: &|dir| dirs.iter().any(|known| known == dir),
            },
        )
    }

    /// A path that is absolute on the machine the tests run on, in the two spellings.
    fn local(name: &str) -> (bool, String, String) {
        if cfg!(windows) {
            (true, format!(r"C:\{name}"), format!("/C:/{name}"))
        } else {
            (false, format!("/{name}"), format!("/{name}"))
        }
    }

    #[test]
    fn a_shells_directory_is_used_when_it_is_this_machines_and_is_there() {
        let (windows, dir, in_url) = local("work");
        let found = |url: String| on(windows, "Box", &[&dir], &shell(), &url);
        assert_eq!(found(format!("file://box{in_url}")), Some(dir.clone()));
        assert_eq!(
            found(format!("file://BOX.example.org{in_url}")),
            Some(dir.clone())
        );
        assert_eq!(found(format!("file://{in_url}")), Some(dir.clone()));
        assert_eq!(
            found(format!("FILE://localhost{in_url}")),
            Some(dir.clone())
        );
    }

    #[test]
    fn another_machines_directory_is_never_used() {
        let (windows, dir, in_url) = local("work");
        for host in ["other", "box2", "bo", "box-old.example.org", "evil.box"] {
            assert_eq!(
                on(
                    windows,
                    "box",
                    &[&dir],
                    &shell(),
                    &format!("file://{host}{in_url}")
                ),
                None,
                "{host}"
            );
        }
        // A machine that does not know its own name takes no named machine for itself.
        assert_eq!(
            on(
                windows,
                "",
                &[&dir],
                &shell(),
                &format!("file://box{in_url}")
            ),
            None
        );
    }

    #[test]
    fn a_directory_that_is_not_there_is_not_used() {
        let (windows, dir, in_url) = local("work");
        assert_eq!(
            on(
                windows,
                "box",
                &[],
                &shell(),
                &format!("file://box{in_url}")
            ),
            None
        );
        assert_eq!(
            on(
                windows,
                "box",
                &[&dir],
                &shell(),
                &format!("file://box{in_url}/gone")
            ),
            None
        );
    }

    #[test]
    fn a_session_on_another_machine_gives_no_directory_whatever_it_reports() {
        let (windows, dir, in_url) = local("work");
        // The report is as good as a report gets: this machine's name, a directory
        // that is there.
        assert_eq!(
            on(
                windows,
                "box",
                &[&dir],
                &ssh(),
                &format!("file://box{in_url}")
            ),
            None
        );
        assert_eq!(
            on(windows, "box", &[&dir], &ssh(), &format!("file://{in_url}")),
            None
        );
        // The client run by its name is a connection all the same.
        for program in ["ssh", "ssh.exe", r"C:\Windows\System32\OpenSSH\SSH.EXE"] {
            let client = Launcher::Shell {
                program: program.to_owned(),
                args: vec!["box".to_owned()],
                login: false,
            };
            assert!(is_elsewhere(&client));
            let url = format!("file://{in_url}");
            assert_eq!(
                on(windows, "box", &[&dir], &client, &url),
                None,
                "{program}"
            );
        }
        assert!(!is_elsewhere(&shell()) && !is_elsewhere(&wsl("Ubuntu")));
    }

    #[test]
    fn what_is_not_a_file_url_gives_no_directory() {
        let (windows, dir, in_url) = local("work");
        let refused = [
            String::new(),
            dir.clone(),
            in_url.clone(),
            format!("http://box{in_url}"),
            format!("file:{in_url}"),
            "file://box".to_owned(),
            // An escape that is none, and bytes that are no text.
            format!("file://box{in_url}%"),
            format!("file://box{in_url}%zz"),
            format!("file://box{in_url}%ff"),
            // A control character hidden in an escape.
            format!("file://box{in_url}%0a"),
        ];
        for url in refused {
            assert_eq!(on(windows, "box", &[&dir], &shell(), &url), None, "{url:?}");
        }
    }

    #[test]
    fn escapes_in_the_path_are_undone() {
        assert_eq!(
            parse_file_url("file://box/home/me/a%20b/%C3%A9t%C3%A9%25"),
            Some(("box".to_owned(), "/home/me/a b/été%".to_owned()))
        );
    }

    #[test]
    fn a_directory_of_this_machine_has_the_url_a_shell_in_it_would_report() {
        for (path, url) in [
            (r"C:\Users\me", "file:///C:/Users/me"),
            (r"D:\", "file:///D:/"),
            ("c:/work/a b", "file:///c:/work/a%20b"),
            ("/home/me/été%", "file:///home/me/%C3%A9t%C3%A9%25"),
            (r"/home/a\b", "file:///home/a%5Cb"),
            ("/", "file:///"),
        ] {
            assert_eq!(file_url_of(path).as_deref(), Some(url), "{path}");
            let (host, named) = parse_file_url(url).unwrap();
            assert_eq!(host, "");
            let back = windows_path(&named).unwrap_or(named);
            let same = |text: &str| text.replace('\\', "/").to_lowercase();
            assert_eq!(same(&back), same(path));
        }
        for path in [
            "",
            r"\\server\share",
            "//server/share",
            r"\\?\C:\x",
            r"C:relative",
            "C:",
            "relative",
            r"\rooted",
            "é:/x",
        ] {
            assert_eq!(file_url_of(path), None, "{path}");
        }
    }

    #[test]
    fn on_windows_the_path_is_a_drives() {
        let dirs = [r"C:\Users\me", r"D:\"];
        let found = |url: &str| on(true, "BOX", &dirs, &shell(), url);
        assert_eq!(
            found("file://box/C:/Users/me"),
            Some(r"C:\Users\me".to_owned())
        );
        assert_eq!(
            found("file://box/c:/Users/me"),
            Some(r"C:\Users\me".to_owned())
        );
        assert_eq!(found("file://box/D:"), Some(r"D:\".to_owned()));
        assert_eq!(found("file://box/D:/"), Some(r"D:\".to_owned()));
        // A path on no drive, a share, a path relative to a drive's directory.
        for url in [
            "file://box/home/me",
            "file://box//server/share",
            "file://box/C:Users",
            "file://box/CC:/Users/me",
        ] {
            assert_eq!(found(url), None, "{url}");
        }
    }

    #[test]
    fn a_long_windows_name_is_known_by_its_beginning() {
        let dirs = [r"C:\w"];
        let url = "file://a-very-long-machine-name/C:/w";
        assert_eq!(
            on(true, "A-VERY-LONG-MAC", &dirs, &shell(), url),
            Some(r"C:\w".to_owned())
        );
        // Only a name cut at the limit is taken for a beginning.
        assert_eq!(on(true, "A-VERY", &dirs, &shell(), url), None);
        assert_eq!(
            on(
                false,
                "a-very-long-mac",
                &["/w"],
                &shell(),
                "file://a-very-long-machine-name/w"
            ),
            None
        );
    }

    #[test]
    fn a_distributions_directory_is_checked_through_its_share_and_kept_as_it_is() {
        let dirs = [
            r"\\wsl.localhost\Ubuntu\home\me\src",
            r"\\wsl.localhost\Ubuntu\",
        ];
        let found = |launcher: &Launcher, url: &str| on(true, "BOX", &dirs, launcher, url);
        // The name inside a distribution is this machine's.
        assert_eq!(
            found(&wsl("Ubuntu"), "file://Box/home/me/src"),
            Some("/home/me/src".to_owned())
        );
        assert_eq!(found(&wsl("Ubuntu"), "file://Box/"), Some("/".to_owned()));
        assert_eq!(found(&wsl("Ubuntu"), "file://Box/home/gone"), None);
        assert_eq!(found(&wsl("Ubuntu"), "file://other/home/me/src"), None);
        // Another distribution has directories of its own.
        assert_eq!(found(&wsl("Debian"), "file://Box/home/me/src"), None);
        // A name the share could not be spelt with.
        assert_eq!(found(&wsl("Ubuntu"), "file://Box/home/a%5Cb"), None);
        // There are no distributions off Windows.
        assert_eq!(
            on(
                false,
                "box",
                &dirs,
                &wsl("Ubuntu"),
                "file://box/home/me/src"
            ),
            None
        );
    }

    #[test]
    fn directories_said_by_output_that_is_no_shells_start_nothing_they_should_not() {
        // The directories the corpus's recording of hostile reports names.
        let said = [
            "file://elsewhere/etc",
            "file://localhost/",
            "file://localhost/tmp/%0a%1b%5d2;taken",
            "javascript:alert(1)",
        ];
        // Each of them is there, as far as this machine can tell.
        let dirs = ["/", "/etc", "/tmp"];
        for url in said {
            // Printed over a connection to another machine: never a directory.
            assert_eq!(on(false, "box", &dirs, &ssh(), url), None, "{url}");
        }
        // Printed in a shell on this machine, the most a report can do is name a
        // directory of this machine that is there: one the user could have gone to.
        let local: Vec<Option<String>> = said
            .iter()
            .map(|url| on(false, "box", &dirs, &shell(), url))
            .collect();
        assert_eq!(local, [None, Some("/".to_owned()), None, None]);
    }

    #[test]
    fn this_machine_has_a_name() {
        assert!(machine_name().is_some_and(|name| !name.is_empty()));
    }
}
