//! Which shell to start when nothing says otherwise, and which shells there are to
//! choose from.

use std::path::PathBuf;

use lume_proto::Launcher;

/// The shell a new session runs by default.
///
/// On Windows: PowerShell 7 if it is installed, else Windows PowerShell, else the
/// command prompt. Elsewhere: the user's login shell from the account database, else
/// `$SHELL`, else `/bin/sh`.
pub fn default_shell() -> Launcher {
    platform::default_shell()
}

/// A shell that is installed on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownShell {
    /// A short name for it that never changes, to tell the shells apart by.
    pub id: &'static str,
    /// What a person calls it.
    pub name: &'static str,
    /// The file to run, as an absolute path.
    pub program: String,
}

/// The well-known shells installed on this machine, in a fixed order.
///
/// A shell is installed if its program is in one of the folders of `PATH`. The list of
/// shells looked for is short and fixed: this is what a user chooses a default from,
/// not everything that could be run.
pub fn known_shells() -> Vec<KnownShell> {
    let dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    shells_in(platform::SHELLS, &dirs)
}

/// Whether the file at `path` is a program that can be started.
#[cfg(unix)]
pub(crate) fn runnable(path: &std::path::Path) -> bool {
    platform::runnable(path)
}

/// The shells of `candidates` (an id, a name and a file name each) found in `dirs`,
/// each at the first folder that has it. Only absolute folders are looked in, so a file
/// planted in the working directory is never offered.
fn shells_in(
    candidates: &[(&'static str, &'static str, &str)],
    dirs: &[PathBuf],
) -> Vec<KnownShell> {
    candidates
        .iter()
        .filter_map(|(id, name, file)| {
            dirs.iter()
                .filter(|dir| dir.is_absolute())
                .map(|dir| dir.join(file))
                .find(|candidate| platform::runnable(candidate))
                .map(|program| KnownShell {
                    id,
                    name,
                    program: program.to_string_lossy().into_owned(),
                })
        })
        .collect()
}

#[cfg(windows)]
mod platform {
    use std::path::{Path, PathBuf};

    use lume_proto::Launcher;

    /// The shells looked for: each one's id, what it is called, and its program's
    /// file name.
    pub(super) const SHELLS: &[(&str, &str, &str)] = &[
        ("pwsh", "PowerShell", "pwsh.exe"),
        ("powershell", "Windows PowerShell", "powershell.exe"),
        ("cmd", "Command Prompt", "cmd.exe"),
        ("bash", "Bash", "bash.exe"),
        ("nu", "Nushell", "nu.exe"),
    ];

    pub(super) fn runnable(path: &Path) -> bool {
        path.is_file()
    }

    fn shell(program: PathBuf) -> Launcher {
        Launcher::Shell {
            program: program.to_string_lossy().into_owned(),
            args: Vec::new(),
            login: false,
        }
    }

    pub(super) fn default_shell() -> Launcher {
        // PowerShell 7 puts itself on `PATH` when installed. Only absolute directories
        // are looked in, so a file planted in the working directory is never chosen.
        let on_path = std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .filter(|dir| dir.is_absolute())
                .map(|dir| dir.join("pwsh.exe"))
                .find(|candidate| candidate.is_file())
        });
        if let Some(pwsh) = on_path {
            return shell(pwsh);
        }

        let windows_powershell = std::env::var_os("SystemRoot").map(|root| {
            PathBuf::from(root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
        });
        if let Some(powershell) = windows_powershell.filter(|path| path.is_file()) {
            return shell(powershell);
        }

        match std::env::var_os("ComSpec").map(PathBuf::from) {
            Some(command_prompt) if command_prompt.is_file() => shell(command_prompt),
            _ => shell(PathBuf::from("cmd.exe")),
        }
    }
}

#[cfg(unix)]
mod platform {
    use std::ffi::{CStr, OsString};
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use lume_proto::Launcher;

    const FALLBACK: &str = "/bin/sh";

    /// The shells looked for: each one's id, what it is called, and its program's
    /// file name.
    pub(super) const SHELLS: &[(&str, &str, &str)] = &[
        ("zsh", "zsh", "zsh"),
        ("bash", "bash", "bash"),
        ("fish", "fish", "fish"),
        ("nu", "Nushell", "nu"),
        ("pwsh", "PowerShell", "pwsh"),
        ("sh", "sh", "sh"),
    ];

    pub(super) fn runnable(path: &Path) -> bool {
        is_executable(path)
    }

    pub(super) fn default_shell() -> Launcher {
        let from_environment = || std::env::var_os("SHELL").map(PathBuf::from);
        let program = login_shell()
            .filter(|shell| is_executable(shell))
            .or_else(|| from_environment().filter(|shell| is_executable(shell)))
            .unwrap_or_else(|| PathBuf::from(FALLBACK));
        Launcher::Shell {
            program: program.to_string_lossy().into_owned(),
            args: Vec::new(),
            // A terminal on macOS starts a login shell: there is no login session whose
            // profile would otherwise have been read.
            login: cfg!(target_os = "macos"),
        }
    }

    fn is_executable(path: &Path) -> bool {
        std::fs::metadata(path)
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    }

    /// The shell recorded for the current user in the account database.
    fn login_shell() -> Option<PathBuf> {
        // The size the system suggests for this lookup, or a generous one if it has no
        // suggestion.
        // SAFETY: takes no pointers.
        let suggested = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
        let mut buffer = vec![0 as libc::c_char; usize::try_from(suggested).unwrap_or(16 * 1024)];
        // SAFETY: plain data for which all-zero is a valid value.
        let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: `entry` and `found` are valid out pointers and `buffer` is valid for
        // its length. The strings the entry points to live in `buffer`.
        let status = unsafe {
            libc::getpwuid_r(
                libc::geteuid(),
                &mut entry,
                buffer.as_mut_ptr(),
                buffer.len(),
                &mut found,
            )
        };
        if status != 0 || found.is_null() || entry.pw_shell.is_null() {
            return None;
        }
        // SAFETY: on success `pw_shell` is a NUL-terminated string inside `buffer`,
        // which is still alive.
        let shell = unsafe { CStr::from_ptr(entry.pw_shell) }
            .to_bytes()
            .to_vec();
        (!shell.is_empty()).then(|| PathBuf::from(OsString::from_vec(shell)))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn default_shell_exists_on_this_machine() {
        let Launcher::Shell {
            program,
            args,
            login,
        } = default_shell()
        else {
            panic!("the default is not a local shell");
        };
        let path = Path::new(&program);
        assert!(path.is_absolute(), "{program} is not an absolute path");
        assert!(path.is_file(), "{program} does not exist");
        assert!(args.is_empty());
        assert_eq!(login, cfg!(target_os = "macos"));
    }

    /// A folder of its own for a test, removed when the test is over.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let folder =
                std::env::temp_dir().join(format!("lume-shells-{}-{name}", std::process::id()));
            std::fs::create_dir_all(&folder).unwrap();
            Self(folder)
        }

        /// Puts a program that can be run into the folder.
        fn program(&self, file: &str) -> PathBuf {
            let path = self.0.join(file);
            std::fs::write(&path, b"").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            path
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_shells_found_are_listed_in_the_order_of_the_list_each_where_it_is_first() {
        let first = Folder::new("first");
        let second = Folder::new("second");
        let two = first.program("two");
        second.program("two");
        let one = second.program("one");
        let dirs = [first.0.clone(), second.0.clone()];

        let found = shells_in(
            &[
                ("1", "One", "one"),
                ("0", "Absent", "absent"),
                ("2", "Two", "two"),
            ],
            &dirs,
        );
        assert_eq!(
            found,
            [
                KnownShell {
                    id: "1",
                    name: "One",
                    program: one.to_string_lossy().into_owned(),
                },
                KnownShell {
                    id: "2",
                    name: "Two",
                    program: two.to_string_lossy().into_owned(),
                },
            ]
        );
    }

    #[test]
    fn a_folder_that_is_not_absolute_is_not_looked_in() {
        // Tests run in the crate's own folder, which holds this file.
        assert!(Path::new("Cargo.toml").is_file());
        let here = [PathBuf::from("."), PathBuf::new()];
        assert_eq!(shells_in(&[("m", "Manifest", "Cargo.toml")], &here), []);
        assert_eq!(shells_in(&[("m", "Manifest", "Cargo.toml")], &[]), []);
    }

    #[test]
    fn every_shell_offered_on_this_machine_is_a_program_that_exists() {
        for shell in known_shells() {
            let path = Path::new(&shell.program);
            assert!(path.is_absolute(), "{}", shell.program);
            assert!(path.is_file(), "{}", shell.program);
        }
    }
}
