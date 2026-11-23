//! The Rex server on this machine, as far as the application can tell without talking
//! to one: whether its program is installed, and which version it says it is.
//!
//! The program is looked for on the `PATH` and where its installer puts it, and is
//! asked for its version. It is told to start nothing. Only the version is kept. Where the program was found
//! is not: a path names the user, and what is read here is shown in a report the user
//! may pass on.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The name of the server's program.
pub const PROGRAM: &str = "rex";

/// What asks the program for its version, without it starting a server.
const ASK: [&str; 2] = ["--autostart=false", "version"];

/// The folder of the programs' folder that the installer for Windows uses.
#[cfg(windows)]
const FOLDER: &str = "Rex Server";

/// How long the program may take to say its version before it is given up on. A
/// program that prints a line and ends does so at once.
const VERSION_WAIT: Duration = Duration::from_secs(2);

/// How often a program that has not ended yet is looked at again.
const VERSION_POLL: Duration = Duration::from_millis(20);

/// How much of what the program prints is read. A version is a few characters.
const OUTPUT_MAX_BYTES: u64 = 4096;

/// The longest a version is shown. One that is longer is no version.
const VERSION_MAX_CHARS: usize = 48;

/// The endings a program's file may have on this platform.
fn endings() -> &'static [&'static str] {
    if cfg!(windows) { &["exe"] } else { &[""] }
}

/// The program's file in one of the folders of a `PATH`, the first there is.
fn find_in(path: &OsString) -> Option<PathBuf> {
    std::env::split_paths(path)
        .filter(|folder| !folder.as_os_str().is_empty())
        .flat_map(|folder| {
            endings().iter().map(move |ending| {
                let file = folder.join(PROGRAM);
                if ending.is_empty() {
                    file
                } else {
                    file.with_extension(ending)
                }
            })
        })
        .find(|file| file.is_file())
}

/// The program's file, wherever it is installed.
fn find() -> Option<PathBuf> {
    std::env::var_os("PATH")
        .and_then(|path| find_in(&path))
        .or_else(where_its_installer_puts_it)
}

#[cfg(windows)]
fn where_its_installer_puts_it() -> Option<PathBuf> {
    let folder = PathBuf::from(std::env::var_os("ProgramFiles")?).join(FOLDER);
    find_in(&folder.into_os_string())
}

#[cfg(not(windows))]
fn where_its_installer_puts_it() -> Option<PathBuf> {
    // On the search path, which has been looked through.
    None
}

/// Whether a Rex server is installed on this machine. Looks at files, so it is not
/// called on a thread the windows need.
pub fn is_installed() -> bool {
    find().is_some()
}

/// The version in what the program printed when asked for it: the first word of its
/// first line that starts like a version, as in `rex 0.4.1`, or the rest of that line
/// when no word of it does. Nothing when the line does not begin with the program's
/// name, as another program of that name would print, or when what follows is too long
/// to be a version.
pub fn version_in(printed: &str) -> Option<String> {
    let line = printed
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?
        .strip_prefix(PROGRAM)?
        .strip_prefix(' ')?;
    let plain: String = line.chars().filter(|char| !char.is_control()).collect();
    let numbered = |word: &&str| {
        let digits = word.strip_prefix('v').unwrap_or(word);
        digits.starts_with(|first: char| first.is_ascii_digit())
    };
    let version = plain
        .split_whitespace()
        .find(numbered)
        .map(|word| word.strip_prefix('v').unwrap_or(word).to_owned())
        .unwrap_or(plain);
    (!version.is_empty() && version.chars().count() <= VERSION_MAX_CHARS).then_some(version)
}

/// Asks the program at `file` for its version and waits a while for the answer.
fn ask(file: &Path) -> Option<String> {
    let mut command = Command::new(file);
    command
        .args(ASK)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashes up for a program that is one.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().ok()?;
    let deadline = Instant::now() + VERSION_WAIT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(VERSION_POLL),
            _ => {
                // It did not answer: it is not left running on the application's account.
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut printed = Vec::new();
    child
        .stdout
        .take()?
        .take(OUTPUT_MAX_BYTES)
        .read_to_end(&mut printed)
        .ok()?;
    version_in(&String::from_utf8_lossy(&printed))
}

/// The version of the Rex server installed on this machine, or nothing when none is
/// found or the one found does not say. Looking runs a program, so it is done beside
/// the threads the windows need.
pub async fn version() -> Option<String> {
    tokio::task::spawn_blocking(move || {
        let file = find()?;
        let version = ask(&file);
        tracing::debug!(found = version.is_some(), "looked for the Rex server");
        version
    })
    .await
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_is_read_from_what_the_program_prints() {
        assert_eq!(version_in("rex 0.4.1\n").as_deref(), Some("0.4.1"));
        assert_eq!(
            version_in("\r\n  rex v1.2.0-beta.3\r\ncommit: abc123\nbuilt: today\n").as_deref(),
            Some("1.2.0-beta.3")
        );
        // A line with no number in it is shown as it is, without what a terminal would
        // take for an instruction.
        assert_eq!(
            version_in("rex nightly\u{1b}[31m build").as_deref(),
            Some("nightly[31m build")
        );
    }

    #[test]
    fn what_is_no_version_is_not_taken_for_one() {
        assert_eq!(version_in(""), None);
        // Another program that is called the same.
        assert_eq!(version_in("(R)?ex 1.14.3\n"), None);
        assert_eq!(version_in("rexec 2.0\n"), None);
        assert_eq!(version_in("\n \n"), None);
        assert_eq!(
            version_in(&format!("rex {}", "x".repeat(VERSION_MAX_CHARS + 1))),
            None
        );
    }

    #[test]
    fn the_program_is_found_in_a_folder_of_the_path_and_nowhere_else() {
        let dir = crate::testing::TempDir::new("rex");
        let folder = dir.dirs().config.clone();
        std::fs::create_dir_all(&folder).unwrap();
        let other = folder.join("other");
        std::fs::create_dir_all(&other).unwrap();
        let path = std::env::join_paths([other.clone(), folder.clone()]).unwrap();
        assert_eq!(find_in(&path), None);

        let name = if cfg!(windows) {
            format!("{PROGRAM}.exe")
        } else {
            PROGRAM.to_owned()
        };
        std::fs::write(folder.join(&name), b"").unwrap();
        assert_eq!(find_in(&path), Some(folder.join(&name)));
        // A folder of that name is not the program.
        std::fs::create_dir_all(other.join(&name)).unwrap();
        assert_eq!(find_in(&path), Some(folder.join(&name)));
    }
}
