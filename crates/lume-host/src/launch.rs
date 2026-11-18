//! From what a session should run to the program that is started for it.
//!
//! A session runs a program on this machine, the shell of a Linux distribution, or a
//! shell on another machine. The last two are reached by starting the tool the system
//! has for it (`wsl.exe`, the OpenSSH client) in the session's terminal, so whatever
//! that tool asks the user, it asks in the terminal, as it would anywhere.
//!
//! Every program is started from a list of arguments, never through a shell, and the
//! words that name a distribution, a machine or a user are refused when the tool would
//! read them as options.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use lume_proto::{ErrorCode, Launcher, ProtoError, SpawnSpec};
use lume_pty::{CommandSpec, IDENTITY_ENV};

use crate::integration;

/// The program to start for a session, and what the session is listed as running.
pub(crate) struct Launch {
    pub command: CommandSpec,
    pub program: String,
}

/// The directory a distribution's shell starts in when none was asked for: the home
/// directory of the user it runs as, which `wsl.exe` spells like a shell does.
const WSL_HOME: &str = "~";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Os {
    Windows,
    Macos,
    Linux,
}

impl Os {
    fn this() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Linux
        }
    }
}

/// The variable that lists which of this machine's variables a Linux distribution is
/// given.
const WSLENV: &str = "WSLENV";

/// What of this machine decides where a tool is found.
struct Machine<'a> {
    os: Os,
    /// The Windows folder.
    system_root: Option<&'a Path>,
    /// The folders of `PATH`, in order.
    path: &'a [PathBuf],
    is_file: &'a dyn Fn(&Path) -> bool,
    /// A variable of this program's own environment, which a session starts with.
    var: &'a dyn Fn(&str) -> Option<String>,
}

/// The list of variables that cross into a distribution: the ones the user already
/// has cross, the ones that say what terminal this is, the session's own, and `more`,
/// whose entries may carry marks as the user's may. A distribution is given none of
/// this machine's variables unless they are listed, so without this a program in it
/// could not tell that the terminal draws 24-bit colour, and a profile's variables
/// would stop at `wsl.exe`.
fn crossing(own: Option<&str>, env: &[(String, String)], more: &[String]) -> String {
    let mut entries: Vec<&str> = own
        .map(|own| own.split(':').filter(|name| !name.is_empty()).collect())
        .unwrap_or_default();
    let wanted = IDENTITY_ENV
        .iter()
        .map(|(name, _)| *name)
        .chain(env.iter().map(|(name, _)| name.as_str()))
        .chain(more.iter().map(String::as_str));
    for entry in wanted {
        // An entry may carry marks after a slash, as in `HOME/p`. A variable is
        // listed once, and the user's way of listing it is the one that stays.
        let name = integration::listed_name(entry);
        let listed = entries
            .iter()
            .any(|listed| integration::listed_name(listed) == name);
        if name != WSLENV && !listed {
            entries.push(entry);
        }
    }
    entries.join(":")
}

/// The program a session's launcher asks for, with its arguments and environment.
///
/// `scripts` is the folder the shell scripts are in, when the session's shell is to be
/// asked to report its directory and its commands.
pub(crate) fn launch(spec: SpawnSpec, scripts: Option<&Path>) -> Result<Launch, ProtoError> {
    let system_root = std::env::var_os("SystemRoot").map(PathBuf::from);
    let path: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    launch_on(
        spec,
        scripts,
        &Machine {
            os: Os::this(),
            system_root: system_root.as_deref(),
            path: &path,
            is_file: &|candidate| candidate.is_file(),
            var: &|name| std::env::var(name).ok(),
        },
    )
}

fn invalid(message: impl Into<String>) -> ProtoError {
    ProtoError::new(ErrorCode::InvalidSpec, message)
}

/// Refuses a word that names something to a tool if the tool would not take it as a
/// name: nothing at all, an option, or something with control characters in it.
fn check_word(what: &str, word: &str) -> Result<(), ProtoError> {
    if word.is_empty() {
        return Err(invalid(format!("the {what} is empty")));
    }
    if word.starts_with('-') {
        return Err(invalid(format!("the {what} cannot start with `-`")));
    }
    if word.chars().any(char::is_control) {
        return Err(invalid(format!(
            "the {what} cannot hold control characters"
        )));
    }
    Ok(())
}

/// Where `wsl.exe` is: in the Windows folder and nowhere else, so a file of that name
/// somewhere on `PATH` is never run in its place.
fn wsl_program(machine: &Machine<'_>) -> Result<PathBuf, ProtoError> {
    if machine.os != Os::Windows {
        return Err(ProtoError::new(
            ErrorCode::Unsupported,
            "a Linux distribution is started through WSL, which only Windows has",
        ));
    }
    machine
        .system_root
        .map(|root| root.join("System32").join("wsl.exe"))
        .filter(|program| (machine.is_file)(program))
        .ok_or_else(|| {
            ProtoError::new(
                ErrorCode::SpawnFailed,
                "wsl.exe was not found: WSL is not installed on this machine \
                 (`wsl --install` in an administrator's terminal installs it)",
            )
        })
}

/// The first of `PATH`'s folders that has `file`. Only absolute folders are looked in,
/// so a file planted in the working directory is never chosen.
fn on_path(machine: &Machine<'_>, file: &str) -> Option<PathBuf> {
    machine
        .path
        .iter()
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(file))
        .find(|candidate| (machine.is_file)(candidate))
}

/// The OpenSSH client to run: the one asked for, else the system's own, else the first
/// on `PATH`.
fn ssh_client(explicit: Option<&str>, machine: &Machine<'_>) -> Option<PathBuf> {
    if let Some(client) = explicit.filter(|client| !client.is_empty()) {
        return Some(PathBuf::from(client));
    }
    if machine.os == Os::Windows {
        let inbox = machine
            .system_root
            .map(|root| root.join("System32").join("OpenSSH").join("ssh.exe"))
            .filter(|client| (machine.is_file)(client));
        return inbox.or_else(|| on_path(machine, "ssh.exe"));
    }
    on_path(machine, "ssh")
}

/// What to tell someone whose machine has no OpenSSH client.
fn no_ssh_client(os: Os) -> ProtoError {
    let how = match os {
        Os::Windows => {
            "add it in Settings, System, Optional features, \"OpenSSH Client\", \
             or set the profile's `client` to an ssh.exe"
        }
        Os::Macos => {
            "it is part of macOS as /usr/bin/ssh, so check PATH, \
             or set the profile's `client`"
        }
        Os::Linux => {
            "install your distribution's `openssh-client` (or `openssh`) package, \
             or set the profile's `client`"
        }
    };
    ProtoError::new(
        ErrorCode::SpawnFailed,
        format!("the OpenSSH client (ssh) was not found: {how}"),
    )
}

fn launch_on(
    spec: SpawnSpec,
    scripts: Option<&Path>,
    machine: &Machine<'_>,
) -> Result<Launch, ProtoError> {
    let asked = spec.env;
    let env: Vec<(OsString, OsString)> = asked
        .iter()
        .map(|(name, value)| (name.into(), value.into()))
        .collect();
    match spec.launcher {
        Launcher::Shell {
            program,
            args,
            login,
        } => {
            let mut command = CommandSpec::new(&program).args(&args);
            command.login = login;
            command.cwd = spec.cwd.map(Into::into);
            command.env = env;
            if let Some(scripts) = scripts {
                let windows = machine.os == Os::Windows;
                integration::hook_shell(&mut command, &program, scripts, windows, machine.var);
            }
            Ok(Launch { command, program })
        }
        Launcher::Wsl { distro, user } => {
            check_word("distribution's name", &distro)?;
            if let Some(user) = &user {
                check_word("user's name", user)?;
            }
            let program = wsl_program(machine)?;
            let mut command = CommandSpec::new(&program).arg("-d").arg(&distro);
            if let Some(user) = &user {
                command = command.arg("-u").arg(user);
            }
            // The directory is one inside the distribution, which `wsl.exe` goes to
            // itself. This machine's own program has no use for it.
            let dir = spec
                .cwd
                .filter(|dir| !dir.is_empty())
                .unwrap_or_else(|| WSL_HOME.to_owned());
            command = command.arg("--cd").arg(dir);
            // A session that lists the crossing variables itself is listing them in
            // place of the user's own list, as setting any variable does.
            let inherited = (machine.var)(WSLENV);
            let own = asked
                .iter()
                .rev()
                .find(|(name, _)| name == WSLENV)
                .map(|(_, list)| list.as_str())
                .or(inherited.as_deref());
            command.env = env;
            // The shell inside is asked to report through variables, which reach it
            // only if they are listed with the rest.
            let more = match scripts {
                Some(scripts) => integration::hook_distro(&mut command.env, scripts, own),
                None => Vec::new(),
            };
            let list = crossing(own, &asked, &more);
            command.env.retain(|(name, _)| name != WSLENV);
            command.env.push((WSLENV.into(), list.into()));
            Ok(Launch {
                command,
                program: program.to_string_lossy().into_owned(),
            })
        }
        Launcher::Ssh {
            host,
            user,
            port,
            extra_args,
            client,
        } => {
            check_word("host", &host)?;
            if let Some(user) = &user {
                check_word("user's name", user)?;
            }
            let program =
                ssh_client(client.as_deref(), machine).ok_or_else(|| no_ssh_client(machine.os))?;
            let mut command = CommandSpec::new(&program).args(&extra_args);
            if let Some(port) = port {
                command = command.arg("-p").arg(port.to_string());
            }
            // Whatever follows `--` is the destination and never an option.
            command = command.arg("--").arg(match &user {
                Some(user) => format!("{user}@{host}"),
                None => host,
            });
            command.cwd = spec.cwd.map(Into::into);
            command.env = env;
            Ok(Launch {
                command,
                program: program.to_string_lossy().into_owned(),
            })
        }
        _ => Err(ProtoError::new(
            ErrorCode::Unsupported,
            "this host cannot start that",
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use lume_proto::TermSize;

    use super::*;

    fn spec(launcher: Launcher) -> SpawnSpec {
        SpawnSpec {
            launcher,
            cwd: None,
            env: Vec::new(),
            size: TermSize::new(80, 24).unwrap(),
            shell_integration: false,
            scrollback_lines: None,
        }
    }

    fn ssh(host: &str) -> Launcher {
        Launcher::Ssh {
            host: host.to_owned(),
            user: None,
            port: None,
            extra_args: Vec::new(),
            client: None,
        }
    }

    fn wsl(distro: &str, user: Option<&str>) -> Launcher {
        Launcher::Wsl {
            distro: distro.to_owned(),
            user: user.map(str::to_owned),
        }
    }

    /// A machine that has exactly the files named.
    fn with_files<T>(
        os: Os,
        root: Option<&str>,
        path: &[&str],
        files: &[&str],
        run: impl FnOnce(&Machine<'_>) -> T,
    ) -> T {
        with_vars(os, root, path, files, &[], run)
    }

    /// A machine that has exactly the files named, in a program that was itself
    /// started with the variables named.
    fn with_vars<T>(
        os: Os,
        root: Option<&str>,
        path: &[&str],
        files: &[&str],
        vars: &[(&str, &str)],
        run: impl FnOnce(&Machine<'_>) -> T,
    ) -> T {
        let files: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
        let path: Vec<PathBuf> = path.iter().map(PathBuf::from).collect();
        let root = root.map(PathBuf::from);
        run(&Machine {
            os,
            system_root: root.as_deref(),
            path: &path,
            is_file: &|candidate| files.iter().any(|file| file == candidate),
            var: &|name| {
                vars.iter()
                    .find(|(set, _)| *set == name)
                    .map(|(_, value)| (*value).to_owned())
            },
        })
    }

    /// What a command sets a variable to.
    fn set_to(command: &CommandSpec, name: &str) -> Option<String> {
        command
            .env
            .iter()
            .find(|(set, _)| set == name)
            .map(|(_, value)| value.to_string_lossy().into_owned())
    }

    /// A folder that is absolute on the machine the tests run on.
    fn folder(name: &str) -> String {
        if cfg!(windows) {
            format!(r"C:\{name}")
        } else {
            format!("/{name}")
        }
    }

    fn file(folder: &str, parts: &[&str]) -> String {
        let mut path = PathBuf::from(folder);
        path.extend(parts);
        path.to_string_lossy().into_owned()
    }

    fn words(command: &CommandSpec) -> Vec<String> {
        command
            .args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_program_is_started_as_it_was_asked_for() {
        let mut asked = spec(Launcher::Shell {
            program: "sh".to_owned(),
            args: vec!["-c".to_owned(), "true".to_owned()],
            login: true,
        });
        asked.cwd = Some("/tmp".to_owned());
        asked.env = vec![("A".to_owned(), "b".to_owned())];
        let launch = with_files(Os::Linux, None, &[], &[], |machine| {
            launch_on(asked, None, machine)
        })
        .unwrap();
        assert_eq!(launch.program, "sh");
        assert_eq!(launch.command.program, OsString::from("sh"));
        assert_eq!(words(&launch.command), ["-c", "true"]);
        assert!(launch.command.login);
        assert_eq!(launch.command.cwd, Some(PathBuf::from("/tmp")));
        assert_eq!(
            launch.command.env,
            [(OsString::from("A"), OsString::from("b"))]
        );
    }

    #[test]
    fn a_distribution_is_started_through_the_systems_wsl() {
        let root = folder("Windows");
        let program = file(&root, &["System32", "wsl.exe"]);
        let mut asked = spec(wsl("Ubuntu", Some("root")));
        asked.cwd = Some("/srv/www".to_owned());
        asked.env = vec![("A".to_owned(), "b".to_owned())];
        let launch = with_files(Os::Windows, Some(&root), &[], &[&program], |machine| {
            launch_on(asked, None, machine)
        })
        .unwrap();
        assert_eq!(launch.program, program);
        assert_eq!(
            words(&launch.command),
            ["-d", "Ubuntu", "-u", "root", "--cd", "/srv/www"]
        );
        // The directory is the distribution's, not this machine's.
        assert_eq!(launch.command.cwd, None);
        // The session's variable is set, and listed with the ones that say what
        // terminal this is, so that all of them reach the distribution.
        let env: Vec<(String, String)> = launch
            .command
            .env
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect();
        assert_eq!(
            env,
            [
                ("A".to_owned(), "b".to_owned()),
                (
                    "WSLENV".to_owned(),
                    "COLORTERM:TERM_PROGRAM:TERM_PROGRAM_VERSION:A".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn the_variables_that_cross_into_a_distribution_keep_the_users_own_list() {
        let set = |name: &str| (name.to_owned(), "x".to_owned());
        assert_eq!(
            crossing(None, &[], &[]),
            "COLORTERM:TERM_PROGRAM:TERM_PROGRAM_VERSION"
        );
        // What the user lists stays first and keeps its flags; nothing is listed twice.
        assert_eq!(
            crossing(
                Some("GOPATH/p:COLORTERM/u::EDITOR"),
                &[set("EDITOR"), set("B")],
                &[]
            ),
            "GOPATH/p:COLORTERM/u:EDITOR:TERM_PROGRAM:TERM_PROGRAM_VERSION:B"
        );
        // The list itself is not one of the variables that cross.
        assert_eq!(
            crossing(None, &[set("WSLENV")], &[]),
            "COLORTERM:TERM_PROGRAM:TERM_PROGRAM_VERSION"
        );
    }

    #[test]
    fn a_shell_is_asked_to_report_only_when_there_are_scripts_for_it() {
        let scripts = PathBuf::from(folder("scripts"));
        let shell = |program: &str| {
            let mut asked = spec(Launcher::Shell {
                program: program.to_owned(),
                args: vec!["-i".to_owned()],
                login: false,
            });
            asked.env = vec![("A".to_owned(), "b".to_owned())];
            asked
        };
        let vars = [("ZDOTDIR", "/home/me/.zsh")];

        // Without scripts nothing is added, whatever the shell.
        let plain = with_vars(Os::Linux, None, &[], &[], &vars, |machine| {
            launch_on(shell("/bin/zsh"), None, machine)
        })
        .unwrap();
        assert_eq!(words(&plain.command), ["-i"]);
        assert_eq!(
            plain.command.env,
            [(OsString::from("A"), OsString::from("b"))]
        );

        // With them, the shell's own way in is used, and what this program was
        // started with is what the shell is told the user had.
        let hooked = with_vars(Os::Linux, None, &[], &[], &vars, |machine| {
            launch_on(shell("/bin/zsh"), Some(&scripts), machine)
        })
        .unwrap();
        assert_eq!(hooked.program, "/bin/zsh");
        assert_eq!(words(&hooked.command), ["-i"]);
        assert_eq!(set_to(&hooked.command, "A").as_deref(), Some("b"));
        assert_eq!(
            set_to(&hooked.command, "ZDOTDIR"),
            Some(scripts.join("zsh").to_string_lossy().into_owned())
        );
        assert_eq!(
            set_to(&hooked.command, "LUME_ZSH_ZDOTDIR").as_deref(),
            Some("/home/me/.zsh")
        );
        assert_eq!(
            set_to(&hooked.command, "LUME_SHELL_INTEGRATION").as_deref(),
            Some("1")
        );
        assert_eq!(
            set_to(&hooked.command, "LUME_SHELL_INTEGRATION_DIR"),
            Some(scripts.to_string_lossy().into_owned())
        );

        // A program that is no shell we know is started as it was asked for.
        let other = with_vars(Os::Linux, None, &[], &[], &vars, |machine| {
            launch_on(shell("/usr/bin/nu"), Some(&scripts), machine)
        })
        .unwrap();
        assert_eq!(other.command, plain_command("/usr/bin/nu"));
    }

    /// What `shell` in the test above comes to when nothing is added to it.
    fn plain_command(program: &str) -> CommandSpec {
        CommandSpec::new(program).arg("-i").env("A", "b")
    }

    #[test]
    fn a_distribution_is_sent_what_its_shell_needs_to_report() {
        let root = folder("Windows");
        let program = file(&root, &["System32", "wsl.exe"]);
        let scripts = PathBuf::from(folder("scripts"));
        let mut asked = spec(wsl("Ubuntu", None));
        asked.env = vec![("A".to_owned(), "b".to_owned())];
        let launch = with_files(Os::Windows, Some(&root), &[], &[&program], |machine| {
            launch_on(asked, Some(&scripts), machine)
        })
        .unwrap();
        // Its arguments are as they are without the scripts.
        assert_eq!(words(&launch.command), ["-d", "Ubuntu", "--cd", "~"]);
        // Each variable is listed once, and the two that are folders of this machine
        // are marked to be spelt as the distribution spells them.
        assert_eq!(
            set_to(&launch.command, "WSLENV").as_deref(),
            Some(
                "COLORTERM:TERM_PROGRAM:TERM_PROGRAM_VERSION:A:LUME_SHELL_INTEGRATION:\
                 LUME_SHELL_INTEGRATION_DIR/p:PROMPT_COMMAND:ZDOTDIR/p"
            )
        );
        assert_eq!(
            set_to(&launch.command, "LUME_SHELL_INTEGRATION_DIR"),
            Some(scripts.to_string_lossy().into_owned())
        );
        assert_eq!(
            set_to(&launch.command, "ZDOTDIR"),
            Some(scripts.join("zsh").to_string_lossy().into_owned())
        );
        assert!(
            set_to(&launch.command, "PROMPT_COMMAND")
                .is_some_and(|line| line.contains("bash/bootstrap.bash"))
        );
        assert_eq!(set_to(&launch.command, "A").as_deref(), Some("b"));
    }

    #[test]
    fn a_folder_the_user_already_sends_into_the_distribution_is_not_replaced() {
        let root = folder("Windows");
        let program = file(&root, &["System32", "wsl.exe"]);
        let scripts = PathBuf::from(folder("scripts"));
        let vars = [("WSLENV", "ZDOTDIR:GOPATH/p")];
        let launch = with_vars(
            Os::Windows,
            Some(&root),
            &[],
            &[&program],
            &vars,
            |machine| launch_on(spec(wsl("Ubuntu", None)), Some(&scripts), machine),
        )
        .unwrap();
        assert_eq!(
            set_to(&launch.command, "WSLENV").as_deref(),
            Some(
                "ZDOTDIR:GOPATH/p:COLORTERM:TERM_PROGRAM:TERM_PROGRAM_VERSION:\
                 LUME_SHELL_INTEGRATION:LUME_SHELL_INTEGRATION_DIR/p:PROMPT_COMMAND"
            )
        );
        assert_eq!(set_to(&launch.command, "ZDOTDIR"), None);
    }

    #[test]
    fn a_distribution_starts_at_home_when_no_directory_is_asked_for() {
        let root = folder("Windows");
        let program = file(&root, &["System32", "wsl.exe"]);
        let launch = with_files(Os::Windows, Some(&root), &[], &[&program], |machine| {
            launch_on(spec(wsl("Debian", None)), None, machine)
        })
        .unwrap();
        assert_eq!(words(&launch.command), ["-d", "Debian", "--cd", "~"]);
    }

    #[test]
    fn wsl_is_only_ever_the_one_in_the_windows_folder() {
        let root = folder("Windows");
        let elsewhere = folder("tools");
        let planted = file(&elsewhere, &["wsl.exe"]);
        let error = with_files(
            Os::Windows,
            Some(&root),
            &[&elsewhere],
            &[&planted],
            |machine| launch_on(spec(wsl("Ubuntu", None)), None, machine),
        )
        .err()
        .unwrap();
        assert_eq!(error.code, ErrorCode::SpawnFailed);
        assert!(error.message.contains("not installed"), "{}", error.message);
    }

    #[test]
    fn a_distribution_cannot_be_started_off_windows() {
        for os in [Os::Linux, Os::Macos] {
            let error = with_files(os, None, &[], &[], |machine| {
                launch_on(spec(wsl("Ubuntu", None)), None, machine)
            })
            .err()
            .unwrap();
            assert_eq!(error.code, ErrorCode::Unsupported);
        }
    }

    #[test]
    fn the_systems_own_client_is_preferred_on_windows() {
        let root = folder("Windows");
        let git = folder("Git");
        let inbox = file(&root, &["System32", "OpenSSH", "ssh.exe"]);
        let other = file(&git, &["ssh.exe"]);
        with_files(
            Os::Windows,
            Some(&root),
            &[&git],
            &[&inbox, &other],
            |machine| {
                assert_eq!(ssh_client(None, machine), Some(PathBuf::from(&inbox)));
                assert_eq!(ssh_client(Some(""), machine), Some(PathBuf::from(&inbox)));
            },
        );
        // Without the system's, the first on the path.
        with_files(Os::Windows, Some(&root), &[&git], &[&other], |machine| {
            assert_eq!(ssh_client(None, machine), Some(PathBuf::from(&other)));
        });
    }

    #[test]
    fn elsewhere_the_client_is_the_first_on_the_path() {
        let first = folder("first");
        let second = folder("second");
        let wanted = file(&second, &["ssh"]);
        with_files(Os::Linux, None, &[&first, &second], &[&wanted], |machine| {
            assert_eq!(ssh_client(None, machine), Some(PathBuf::from(&wanted)));
        });
    }

    #[test]
    fn a_folder_that_is_not_absolute_is_not_looked_in() {
        with_files(Os::Linux, None, &[".", ""], &["./ssh", "ssh"], |machine| {
            assert_eq!(ssh_client(None, machine), None);
        });
    }

    #[test]
    fn a_client_that_was_asked_for_is_used_as_given() {
        with_files(Os::Windows, None, &[], &[], |machine| {
            assert_eq!(
                ssh_client(Some("D:/tools/ssh.exe"), machine),
                Some(PathBuf::from("D:/tools/ssh.exe"))
            );
        });
    }

    #[test]
    fn the_destination_follows_every_option_and_a_double_hyphen() {
        let mut asked = spec(Launcher::Ssh {
            host: "work".to_owned(),
            user: Some("me".to_owned()),
            port: Some(2222),
            extra_args: vec![
                "-A".to_owned(),
                "-o".to_owned(),
                "Compression=yes".to_owned(),
            ],
            client: Some("my-ssh".to_owned()),
        });
        asked.cwd = Some("/tmp".to_owned());
        let launch = with_files(Os::Linux, None, &[], &[], |machine| {
            launch_on(asked, None, machine)
        })
        .unwrap();
        assert_eq!(launch.program, "my-ssh");
        assert_eq!(
            words(&launch.command),
            ["-A", "-o", "Compression=yes", "-p", "2222", "--", "me@work"]
        );
        assert_eq!(launch.command.cwd, Some(PathBuf::from("/tmp")));
        assert!(!launch.command.login);
    }

    #[test]
    fn a_bare_host_is_the_only_argument_after_the_double_hyphen() {
        let bin = folder("bin");
        let client = file(&bin, &["ssh"]);
        let launch = with_files(Os::Macos, None, &[&bin], &[&client], |machine| {
            launch_on(spec(ssh("work")), None, machine)
        })
        .unwrap();
        assert_eq!(launch.program, client);
        assert_eq!(words(&launch.command), ["--", "work"]);
        assert_eq!(launch.command.cwd, None);
    }

    #[test]
    fn a_machine_without_a_client_says_how_to_get_one() {
        for (os, hint) in [
            (Os::Windows, "Optional features"),
            (Os::Macos, "/usr/bin/ssh"),
            (Os::Linux, "openssh-client"),
        ] {
            let error = with_files(os, None, &[], &[], |machine| {
                launch_on(spec(ssh("work")), None, machine)
            })
            .err()
            .unwrap();
            assert_eq!(error.code, ErrorCode::SpawnFailed);
            assert!(error.message.contains(hint), "{}", error.message);
        }
    }

    #[test]
    fn a_word_a_tool_would_read_as_an_option_is_refused() {
        let refused = |launcher: Launcher| {
            let root = folder("Windows");
            let wsl = file(&root, &["System32", "wsl.exe"]);
            let ssh = file(&root, &["System32", "OpenSSH", "ssh.exe"]);
            let error = with_files(Os::Windows, Some(&root), &[], &[&wsl, &ssh], |machine| {
                launch_on(spec(launcher), None, machine)
            })
            .err()
            .expect("the launcher was accepted");
            assert_eq!(error.code, ErrorCode::InvalidSpec, "{}", error.message);
        };
        refused(ssh("-oProxyCommand=calc"));
        refused(ssh(""));
        refused(ssh("host\nname"));
        refused(Launcher::Ssh {
            host: "work".to_owned(),
            user: Some("-l".to_owned()),
            port: None,
            extra_args: Vec::new(),
            client: None,
        });
        refused(Launcher::Ssh {
            host: "work".to_owned(),
            user: Some(String::new()),
            port: None,
            extra_args: Vec::new(),
            client: None,
        });
        refused(wsl("--exec", None));
        refused(wsl("", None));
        refused(wsl("Ubuntu", Some("-u")));
        refused(wsl("Ubuntu", Some("")));
    }
}
