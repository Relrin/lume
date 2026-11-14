//! Which shell to start when nothing says otherwise.

use lume_proto::Launcher;

/// The shell a new session runs by default.
///
/// On Windows: PowerShell 7 if it is installed, else Windows PowerShell, else the
/// command prompt. Elsewhere: the user's login shell from the account database, else
/// `$SHELL`, else `/bin/sh`.
pub fn default_shell() -> Launcher {
    platform::default_shell()
}

#[cfg(windows)]
mod platform {
    use std::path::PathBuf;

    use lume_proto::Launcher;

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
}
