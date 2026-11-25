//! The Linux distributions of a Windows machine, and how their paths map to its own.
//!
//! The distributions are whatever `wsl.exe -l -v` lists. Nothing else about WSL is
//! asked or changed here.

/// One installed distribution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Distro {
    pub name: String,
    pub running: bool,
    /// The version of WSL it runs under: 1 or 2.
    pub version: u8,
    /// Whether it is the one `wsl.exe` starts when none is named.
    pub default: bool,
}

/// What listing the distributions came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Listing {
    Distros(Vec<Distro>),
    /// There are none to offer, and why, in a sentence for the user.
    None {
        reason: String,
    },
}

/// The text of a listing. `wsl.exe` writes UTF-16 with the low byte first and no mark
/// in front, unless the user has asked it for UTF-8 through its environment.
fn decode(bytes: &[u8]) -> String {
    let utf8 = !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok();
    let text = if utf8 {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        // A byte left over at the end is half a character and is dropped.
        let (pairs, _) = bytes.as_chunks::<2>();
        let units: Vec<u16> = pairs.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
        String::from_utf16_lossy(&units)
    };
    match text.strip_prefix('\u{feff}') {
        Some(rest) => rest.to_owned(),
        None => text,
    }
}

/// Whether `name` is one a distribution can have. A line of a message that happens to
/// end in a number is told from a row of the table by this.
fn is_distro_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// One row of the table: an optional `*`, the name, the state, the version.
fn parse_row(line: &str) -> Option<Distro> {
    let line = line.trim();
    let (default, rest) = match line.strip_prefix('*') {
        Some(rest) => (true, rest),
        None => (false, line),
    };
    let mut words = rest.split_whitespace();
    let name = words.next()?;
    // The state may be more than one word in some language; the version is the last.
    let version = words.next_back()?.parse::<u8>().ok()?;
    let state: Vec<&str> = words.collect();
    if state.is_empty() || !matches!(version, 1 | 2) || !is_distro_name(name) {
        return None;
    }
    Some(Distro {
        name: name.to_owned(),
        running: state.len() == 1 && state[0].eq_ignore_ascii_case("running"),
        version,
        default,
    })
}

/// The distributions in the output of `wsl.exe -l -v`.
///
/// The first line with anything on it is the table's header, in whatever language, and
/// is passed over. Every later line that is a row of the table gives a distribution;
/// anything else is skipped, so the messages `wsl.exe` prints when there is nothing to
/// list give an empty list.
pub fn parse_list(bytes: &[u8]) -> Vec<Distro> {
    decode(bytes)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .skip(1)
        .filter_map(parse_row)
        .collect()
}

/// `wsl.exe`: the one in the Windows folder and no other, so that a file of that name
/// somewhere on the path is never run in its place. `None` where WSL is not installed.
pub fn program() -> Option<std::path::PathBuf> {
    if cfg!(not(windows)) {
        return None;
    }
    std::env::var_os("SystemRoot")
        .map(|root| {
            std::path::PathBuf::from(root)
                .join("System32")
                .join("wsl.exe")
        })
        .filter(|program| program.is_file())
}

/// The distributions installed on this machine.
///
/// This starts `wsl.exe` and waits for it, so it is not for an async thread.
pub fn distros() -> Listing {
    platform::distros()
}

#[cfg(windows)]
mod platform {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Instant;

    use super::{Listing, parse_list};
    use crate::tuning::{WSL_LIST_MAX_BYTES, WSL_LIST_POLL, WSL_LIST_TIMEOUT, WSL_OUTPUT_GRACE};

    /// Start the process without a console window of its own.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    fn none(reason: &str) -> Listing {
        Listing::None {
            reason: reason.to_owned(),
        }
    }

    pub(super) fn distros() -> Listing {
        let Some(program) = super::program() else {
            return none("WSL is not installed");
        };

        let spawned = Command::new(program)
            .args(["-l", "-v"])
            // The listing is read as UTF-16 unless this is set; without it there is
            // one form to expect.
            .env_remove("WSL_UTF8")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        let Ok(mut child) = spawned else {
            return none("WSL could not be asked for its distributions");
        };

        // The output is read on a thread of its own, so that neither a full pipe nor a
        // pipe something else keeps open can hold this one up.
        let (sender, output) = mpsc::channel();
        if let Some(stdout) = child.stdout.take() {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                let _ = stdout.take(WSL_LIST_MAX_BYTES).read_to_end(&mut bytes);
                let _ = sender.send(bytes);
            });
        }

        let deadline = Instant::now() + WSL_LIST_TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => std::thread::sleep(WSL_LIST_POLL),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return none("WSL did not answer in time");
                }
                Err(_) => return none("WSL could not be asked for its distributions"),
            }
        };
        let bytes = output.recv_timeout(WSL_OUTPUT_GRACE).unwrap_or_default();
        let found = parse_list(&bytes);
        if !status.success() || found.is_empty() {
            return none("WSL has no distributions installed");
        }
        Listing::Distros(found)
    }
}

#[cfg(not(windows))]
mod platform {
    use super::Listing;

    pub(super) fn distros() -> Listing {
        Listing::None {
            reason: "WSL is a feature of Windows".to_owned(),
        }
    }
}

/// The two names Windows knows a distribution's files under, without the slashes.
const UNC_HOSTS: [&str; 2] = ["wsl.localhost", "wsl$"];

/// The Windows path of a directory inside a distribution:
/// `/home/me` in `Ubuntu` is `\\wsl.localhost\Ubuntu\home\me`.
///
/// Only an absolute Linux path has one. A name with a backslash in it has none that
/// can be written this way.
pub fn linux_to_unc(distro: &str, path: &str) -> Option<String> {
    if !is_distro_name(distro) || !path.starts_with('/') || path.contains('\\') {
        return None;
    }
    let mut unc = format!(r"\\{}\{distro}", UNC_HOSTS[0]);
    let inside = path.trim_matches('/');
    unc.push('\\');
    for (index, part) in inside
        .split('/')
        .filter(|part| !part.is_empty())
        .enumerate()
    {
        if index > 0 {
            unc.push('\\');
        }
        unc.push_str(part);
    }
    Some(unc)
}

/// The distribution and the Linux path a Windows path names, if it is one inside a
/// distribution: `\\wsl.localhost\Ubuntu\home\me` and `\\wsl$\Ubuntu\home\me` are
/// `/home/me` in `Ubuntu`. Either kind of slash is taken.
pub fn unc_to_linux(path: &str) -> Option<(String, String)> {
    let path = path.replace('/', "\\");
    let rest = path.strip_prefix(r"\\")?;
    let mut parts = rest.split('\\');
    let host = parts.next()?;
    if !UNC_HOSTS
        .iter()
        .any(|known| host.eq_ignore_ascii_case(known))
    {
        return None;
    }
    let distro = parts.next().filter(|distro| is_distro_name(distro))?;
    let mut linux = String::new();
    for part in parts.filter(|part| !part.is_empty()) {
        linux.push('/');
        linux.push_str(part);
    }
    if linux.is_empty() {
        linux.push('/');
    }
    Some((distro.to_owned(), linux))
}

/// Where a distribution sees a directory of this machine's drives: `C:\Users\me` is
/// `/mnt/c/Users/me`. Either kind of slash is taken. A path that does not begin with a
/// drive and its root (a network path, a path relative to a drive's directory) has no
/// such place.
pub fn windows_to_mnt(path: &str) -> Option<String> {
    let mut chars = path.chars();
    let drive = chars.next().filter(char::is_ascii_alphabetic)?;
    if chars.next() != Some(':') {
        return None;
    }
    let rest = chars.as_str();
    if !(rest.is_empty() || rest.starts_with(['\\', '/'])) {
        return None;
    }
    let mut mnt = format!("/mnt/{}", drive.to_ascii_lowercase());
    for part in rest.split(['\\', '/']).filter(|part| !part.is_empty()) {
        mnt.push('/');
        mnt.push_str(part);
    }
    Some(mnt)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `wsl.exe -l -v` printed on a machine with two distributions.
    const RECORDED: &[u8] = include_bytes!("../tests/fixtures/wsl-list.bin");

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    fn distro(name: &str, running: bool, version: u8, default: bool) -> Distro {
        Distro {
            name: name.to_owned(),
            running,
            version,
            default,
        }
    }

    #[test]
    fn a_recorded_listing_gives_its_distributions() {
        assert_eq!(
            parse_list(RECORDED),
            [
                distro("Ubuntu", true, 2, true),
                distro("docker-desktop", true, 2, false),
            ]
        );
    }

    #[test]
    fn states_versions_and_the_default_mark_are_read() {
        let listing = utf16(
            "  NAME            STATE           VERSION\r\n\
             \x20 Debian          Stopped         1\r\n\
             * Ubuntu-24.04    Running         2\r\n\
             \x20 Alpine          Installing      2\r\n",
        );
        assert_eq!(
            parse_list(&listing),
            [
                distro("Debian", false, 1, false),
                distro("Ubuntu-24.04", true, 2, true),
                distro("Alpine", false, 2, false),
            ]
        );
    }

    #[test]
    fn a_mark_in_front_and_bare_line_ends_are_tolerated() {
        let mut listing = vec![0xff, 0xfe];
        listing.extend(utf16("  NAME  STATE  VERSION\n* Ubuntu  Running  2\n"));
        assert_eq!(parse_list(&listing), [distro("Ubuntu", true, 2, true)]);
    }

    #[test]
    fn a_listing_in_utf8_is_read_too() {
        let listing = b"  NAME  STATE  VERSION\r\n* Ubuntu  Stopped  2\r\n";
        assert_eq!(parse_list(listing), [distro("Ubuntu", false, 2, true)]);
    }

    #[test]
    fn the_header_is_passed_over_whatever_it_says() {
        let listing = utf16("  NOM  ÉTAT  VERSION\r\n* Ubuntu  Arrêté  2\r\n");
        assert_eq!(parse_list(&listing), [distro("Ubuntu", false, 2, true)]);
        // A state of two words still leaves the name first and the version last.
        let listing = utf16("  NAME  STATE  VERSION\r\n  Ubuntu  Wird ausgeführt  2\r\n");
        assert_eq!(parse_list(&listing), [distro("Ubuntu", false, 2, false)]);
    }

    #[test]
    fn a_message_instead_of_a_table_gives_nothing() {
        let none_installed = utf16(
            "Windows Subsystem for Linux has no installed distributions.\r\n\
             You can resolve this by installing a distribution with the instructions below:\r\n\
             \r\n\
             Use 'wsl.exe --list --online' to list available distributions\r\n\
             and 'wsl.exe --install <Distro>' to install.\r\n",
        );
        assert_eq!(parse_list(&none_installed), []);

        let not_installed = utf16(
            "The Windows Subsystem for Linux is not installed. You can install by running \
             'wsl.exe --install'.\r\n\
             For more information please visit https://aka.ms/wslinstall\r\n\
             Error code: Wsl/WSL_E_WSL_OPTIONAL_COMPONENT_REQUIRED\r\n",
        );
        assert_eq!(parse_list(&not_installed), []);

        assert_eq!(parse_list(b""), []);
        assert_eq!(parse_list(&[0x41]), []);
    }

    #[test]
    fn a_row_that_is_not_one_is_skipped() {
        let listing = utf16(
            "  NAME  STATE  VERSION\r\n\
             \x20 Ubuntu  Running\r\n\
             \x20 Ubuntu  Running  7\r\n\
             \x20 Ubuntu  2\r\n\
             \x20 --exec  Running  2\r\n\
             \x20 Fedora  Stopped  2\r\n",
        );
        assert_eq!(parse_list(&listing), [distro("Fedora", false, 2, false)]);
    }

    #[test]
    fn a_linux_path_has_a_windows_name() {
        let cases = [
            ("/home/me", Some(r"\\wsl.localhost\Ubuntu\home\me")),
            ("/", Some(r"\\wsl.localhost\Ubuntu\")),
            ("/srv/www/", Some(r"\\wsl.localhost\Ubuntu\srv\www")),
            ("//srv//www", Some(r"\\wsl.localhost\Ubuntu\srv\www")),
            ("/a b/c", Some(r"\\wsl.localhost\Ubuntu\a b\c")),
            ("home/me", None),
            ("~", None),
            ("", None),
            (r"/odd\name", None),
        ];
        for (linux, unc) in cases {
            assert_eq!(linux_to_unc("Ubuntu", linux).as_deref(), unc, "{linux}");
        }
        assert_eq!(linux_to_unc("", "/home"), None);
        assert_eq!(linux_to_unc(r"a\b", "/home"), None);
    }

    #[test]
    fn a_windows_name_inside_a_distribution_has_a_linux_path() {
        let cases = [
            (
                r"\\wsl.localhost\Ubuntu\home\me",
                Some(("Ubuntu", "/home/me")),
            ),
            (r"\\wsl$\Ubuntu\home\me", Some(("Ubuntu", "/home/me"))),
            (r"\\WSL.LOCALHOST\Ubuntu\home", Some(("Ubuntu", "/home"))),
            (
                "//wsl.localhost/Debian/srv/www/",
                Some(("Debian", "/srv/www")),
            ),
            (r"\\wsl.localhost\Ubuntu", Some(("Ubuntu", "/"))),
            (r"\\wsl.localhost\Ubuntu\", Some(("Ubuntu", "/"))),
            (r"\\server\share\home", None),
            (r"\\wsl.localhost", None),
            (r"\\wsl.localhost\", None),
            (r"C:\Users\me", None),
            ("/home/me", None),
            ("", None),
        ];
        for (unc, linux) in cases {
            let expected = linux.map(|(distro, path)| (distro.to_owned(), path.to_owned()));
            assert_eq!(unc_to_linux(unc), expected, "{unc}");
        }
    }

    #[test]
    fn the_two_ways_between_the_names_agree() {
        for path in ["/", "/home/me", "/srv/a b"] {
            let unc = linux_to_unc("Ubuntu", path).unwrap();
            assert_eq!(
                unc_to_linux(&unc),
                Some(("Ubuntu".to_owned(), path.to_owned()))
            );
        }
    }

    #[test]
    fn a_drives_directory_has_a_place_under_mnt() {
        let cases = [
            (r"C:\Users\me", Some("/mnt/c/Users/me")),
            ("D:/Code/lume", Some("/mnt/d/Code/lume")),
            (r"c:\", Some("/mnt/c")),
            ("C:", Some("/mnt/c")),
            (r"C:\Program Files\Git\", Some("/mnt/c/Program Files/Git")),
            (r"\\server\share", None),
            (r"\\wsl.localhost\Ubuntu\home", None),
            ("C:relative", None),
            ("/home/me", None),
            (r"relative\path", None),
            ("", None),
            (r"1:\x", None),
        ];
        for (windows, mnt) in cases {
            assert_eq!(windows_to_mnt(windows).as_deref(), mnt, "{windows}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn listing_this_machines_distributions_ends_one_way_or_the_other() {
        match distros() {
            Listing::Distros(found) => assert!(!found.is_empty()),
            Listing::None { reason } => assert!(!reason.is_empty()),
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn there_are_no_distributions_off_windows() {
        assert!(matches!(distros(), Listing::None { .. }));
    }
}
