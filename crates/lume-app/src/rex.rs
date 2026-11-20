//! The Rex server on this machine, as far as the application can tell without talking
//! to one: whether its program is installed, and which version it says it is.
//!
//! The program is looked for on the `PATH` and asked for its version the way any
//! program is, with `--version`. Only the version is kept. Where the program was found
//! is not: a path names the user, and what is read here is shown in a report the user
//! may pass on.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The name of the server's program.
pub const PROGRAM: &str = "rex-server";

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

/// The version in what a program printed when asked for it: the first word of the
/// first line that starts like a version, as in `rex-server 0.4.1 (abc123)`, or that
/// whole line when no word of it does. Nothing when there is no such line, or when it
/// is too long to be a version.
pub fn version_in(printed: &str) -> Option<String> {
    let line = printed
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
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
        .arg("--version")
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
/// found on the `PATH` or the one found does not say. Looking runs a program, so it is
/// done beside the threads the windows need.
pub async fn version() -> Option<String> {
    let path = std::env::var_os("PATH")?;
    tokio::task::spawn_blocking(move || {
        let file = find_in(&path)?;
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
        assert_eq!(version_in("rex-server 0.4.1\n").as_deref(), Some("0.4.1"));
        assert_eq!(
            version_in("\r\n  rex-server v1.2.0-beta.3 (abc123)\nbuilt today\n").as_deref(),
            Some("1.2.0-beta.3")
        );
        assert_eq!(version_in("2.0.0").as_deref(), Some("2.0.0"));
        // A line with no number in it is shown as it is, without what a terminal would
        // take for an instruction.
        assert_eq!(
            version_in("nightly\u{1b}[31m build").as_deref(),
            Some("nightly[31m build")
        );
    }

    #[test]
    fn what_is_no_version_is_not_taken_for_one() {
        assert_eq!(version_in(""), None);
        assert_eq!(version_in("\n \n"), None);
        assert_eq!(version_in(&"x".repeat(VERSION_MAX_CHARS + 1)), None);
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
