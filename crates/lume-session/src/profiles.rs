//! The profiles the application makes itself, for what it finds on this machine.
//!
//! A user can open a terminal in any shell that is installed, in any Linux
//! distribution and on any machine their OpenSSH configuration names, without writing
//! anything down first. Each of those gets a profile here, with an id that stays the
//! same from one start to the next (`auto.pwsh`, `auto.wsl.Ubuntu`, `auto.ssh.work`),
//! so that it can be made the default, or changed by an entry of the user's with the
//! same id. None of these is ever written to a file.

use std::collections::BTreeMap;
use std::path::PathBuf;

use lume_config::profiles::{GENERATED_PREFIX, check_profile_id};
use lume_config::{CloseOnExit, Profile, ProfileOrigin, ShellIntegration};
use lume_proto::Launcher;

use crate::ssh_config::{self, SshHosts};
use crate::tuning::{SSH_CONTROL_NAME_BYTES, SSH_CONTROL_PATH_MAX_BYTES};
use crate::wsl::{self, Distro, Listing};

/// What to make of what is found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DetectOptions {
    /// Let the terminals opened on one machine share a connection, which the client
    /// keeps for this long after the last of them has closed (as OpenSSH writes times:
    /// `10m`, `600`, `yes`). `None` leaves every terminal with a connection of its own.
    pub ssh_sharing: Option<String>,
}

/// What was found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Detected {
    pub profiles: Vec<Profile>,
    /// Why something that might have been here is not, each a sentence for the user.
    pub notes: Vec<String>,
    /// The files that, when they change, may change what is found.
    pub watch: Vec<PathBuf>,
}

/// A shell on this machine to make a profile for.
#[derive(Clone, Debug, PartialEq, Eq)]
struct LocalShell {
    /// What follows the prefix in the profile's id.
    id: String,
    name: String,
    program: String,
    args: Vec<String>,
    login: bool,
}

/// What is said of a host that comes from the user's OpenSSH configuration.
const SSH_DETAIL: &str = "from ~/.ssh/config";

/// Looks for the shells, the distributions and the hosts of this machine.
///
/// This reads files and starts `wsl.exe`, so it is not for an async thread.
pub fn detect(options: &DetectOptions) -> Detected {
    let mut notes = Vec::new();
    let shells = platform::local_shells();
    let distros = if cfg!(windows) {
        wsl::distros()
    } else {
        // Nothing is missing where there could be none.
        Listing::Distros(Vec::new())
    };
    let hosts = ssh_config::user_config()
        .map(|config| ssh_config::hosts(&config))
        .unwrap_or_default();
    let sharing = match options.ssh_sharing.as_deref().map(sharing) {
        Some(Ok(arguments)) => arguments,
        Some(Err(why)) => {
            notes.push(why);
            None
        }
        None => None,
    };
    let mut detected = assemble(shells, distros, hosts, sharing.as_deref());
    notes.append(&mut detected.notes);
    detected.notes = notes;
    detected
}

fn generated(id: String, name: String, launcher: Launcher) -> Profile {
    Profile {
        id,
        name,
        icon: None,
        launcher,
        cwd: None,
        env: BTreeMap::new(),
        theme: None,
        font_size: None,
        shell_integration: ShellIntegration::Auto,
        close_on_exit: None,
        hidden: false,
        origin: ProfileOrigin::Generated,
        detail: None,
    }
}

/// Whether a distribution is another program's machinery rather than a place to work.
fn is_tool_distro(name: &str) -> bool {
    matches!(name, "docker-desktop" | "docker-desktop-data") || name.starts_with("rancher-desktop")
}

fn distro_detail(distro: &Distro) -> String {
    let state = if distro.running { "running" } else { "stopped" };
    format!("WSL {} · {state}", distro.version)
}

/// The profiles for what was found: the shells first, then the distributions with the
/// default one in front, then the hosts.
fn assemble(
    shells: Vec<LocalShell>,
    distros: Listing,
    hosts: SshHosts,
    sharing: Option<&[String]>,
) -> Detected {
    let mut profiles = Vec::new();
    let mut notes = Vec::new();

    for shell in shells {
        let id = format!("{GENERATED_PREFIX}{}", shell.id);
        if check_profile_id(&id).is_err() {
            continue;
        }
        profiles.push(generated(
            id,
            shell.name,
            Launcher::Shell {
                program: shell.program,
                args: shell.args,
                login: shell.login,
            },
        ));
    }

    match distros {
        Listing::Distros(mut found) => {
            found.retain(|distro| !is_tool_distro(&distro.name));
            // Stable, so the others stay in the order they were listed.
            found.sort_by_key(|distro| !distro.default);
            for distro in found {
                let id = format!("{GENERATED_PREFIX}wsl.{}", distro.name);
                if check_profile_id(&id).is_err() {
                    continue;
                }
                let mut profile = generated(
                    id,
                    distro.name.clone(),
                    Launcher::Wsl {
                        distro: distro.name.clone(),
                        user: None,
                    },
                );
                profile.detail = Some(distro_detail(&distro));
                profiles.push(profile);
            }
        }
        Listing::None { reason } => notes.push(reason),
    }

    for host in hosts.hosts {
        let id = format!("{GENERATED_PREFIX}ssh.{host}");
        if check_profile_id(&id).is_err() {
            notes.push(format!(
                "the SSH host `{host}` is not listed: a profile's id cannot be made from its name"
            ));
            continue;
        }
        let mut profile = generated(
            id,
            host.clone(),
            Launcher::Ssh {
                host,
                user: None,
                port: None,
                extra_args: sharing.map(<[String]>::to_vec).unwrap_or_default(),
                client: None,
            },
        );
        profile.detail = Some(SSH_DETAIL.to_owned());
        // A connection that failed should stay to be read.
        profile.close_on_exit = Some(CloseOnExit::Clean);
        profiles.push(profile);
    }

    Detected {
        profiles,
        notes,
        watch: hosts.files,
    }
}

/// Why a time cannot be handed to the client as how long a connection is kept.
#[cfg_attr(not(any(unix, test)), expect(dead_code))]
fn check_persist(persist: &str) -> Result<(), String> {
    let plain = !persist.is_empty() && persist.chars().all(|c| c.is_ascii_alphanumeric());
    if plain {
        Ok(())
    } else {
        Err(format!(
            "connections to SSH hosts are not shared: `{persist}` is not a time OpenSSH \
             takes (such as `10m`)"
        ))
    }
}

/// The folder the sockets of shared connections are kept in: the user's own runtime
/// folder if there is one, else one of their own under `/tmp`. Whichever it is must
/// leave room for a connection's name in a socket's path.
#[cfg_attr(not(any(unix, test)), expect(dead_code))]
fn control_dir(runtime: Option<&str>, uid: u32) -> String {
    let fits = |dir: &str| dir.len() + 1 + SSH_CONTROL_NAME_BYTES <= SSH_CONTROL_PATH_MAX_BYTES;
    runtime
        .map(|runtime| format!("{}/lume", runtime.trim_end_matches('/')))
        .filter(|dir| dir.starts_with('/') && fits(dir))
        .unwrap_or_else(|| format!("/tmp/lume-{uid}"))
}

/// What the client is told so that the terminals on one machine share a connection.
#[cfg_attr(not(any(unix, test)), expect(dead_code))]
fn sharing_arguments(dir: &str, persist: &str) -> Vec<String> {
    vec![
        "-o".to_owned(),
        "ControlMaster=auto".to_owned(),
        "-o".to_owned(),
        // The client puts a name of its own making for each connection where `%C` is.
        format!("ControlPath={dir}/%C"),
        "-o".to_owned(),
        format!("ControlPersist={persist}"),
    ]
}

/// The arguments that make connections shared, or `None` where the client cannot share
/// them. An error says why sharing was asked for and is not on.
#[cfg(unix)]
fn sharing(persist: &str) -> Result<Option<Vec<String>>, String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    use std::path::Path;

    check_persist(persist)?;
    // SAFETY: takes no pointers.
    let uid = unsafe { libc::geteuid() };
    let runtime = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|runtime| Path::new(runtime).is_absolute() && Path::new(runtime).is_dir());
    let dir = control_dir(runtime.as_deref(), uid);

    // A socket in this folder lets whoever can reach it use the user's connections, so
    // the folder must be the user's alone: made so here, or found so.
    let refuse = |why: &str| {
        Err(format!(
            "connections to SSH hosts are not shared: {dir} {why}"
        ))
    };
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let Ok(meta) = std::fs::symlink_metadata(&dir) else {
                return refuse("cannot be looked at");
            };
            if !meta.file_type().is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
                return refuse("is not a folder that only you can open");
            }
        }
        Err(error) => return refuse(&format!("could not be made ({error})")),
    }
    Ok(Some(sharing_arguments(&dir, persist)))
}

/// The arguments that make connections shared, or `None` where the client cannot share
/// them. The OpenSSH client that comes with Windows cannot: asked to, it fails before
/// it connects.
#[cfg(not(unix))]
fn sharing(_persist: &str) -> Result<Option<Vec<String>>, String> {
    Ok(None)
}

#[cfg(windows)]
mod platform {
    use std::path::{Path, PathBuf};

    use super::LocalShell;
    use crate::shell::known_shells;

    /// The id of the shell that Git for Windows brings.
    const GIT_BASH_ID: &str = "git-bash";

    /// The id of the shell Git Bash is listed in front of.
    const AFTER_GIT_BASH: &str = "nu";

    /// The folder Git for Windows is installed in, going by where its `git.exe` is:
    /// in `cmd` or `bin` under it, or in the `bin` of one of the systems it carries.
    pub(super) fn git_root(git: &Path) -> Option<PathBuf> {
        let folder = git.parent()?;
        let above = folder.parent()?;
        let name = |path: &Path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_ascii_lowercase())
        };
        match name(folder)?.as_str() {
            "cmd" => Some(above.to_owned()),
            "bin" => match name(above).as_deref() {
                Some("mingw64" | "mingw32" | "clangarm64" | "usr") => {
                    above.parent().map(Path::to_owned)
                }
                _ => Some(above.to_owned()),
            },
            _ => None,
        }
    }

    /// Git for Windows's bash, if it is installed: under the folder its `git.exe` on
    /// the path belongs to, else where its installer puts it.
    ///
    /// A `bash.exe` found on the path is not taken for it: on Windows that is as often
    /// the program that starts the default Linux distribution.
    pub(super) fn git_bash(
        path: &[PathBuf],
        program_files: Option<&Path>,
        is_file: &dyn Fn(&Path) -> bool,
    ) -> Option<PathBuf> {
        let from_path = path
            .iter()
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join("git.exe"))
            .filter(|git| is_file(git))
            .filter_map(|git| git_root(&git));
        let installed = program_files.map(|programs| programs.join("Git"));
        from_path
            .chain(installed)
            // Only a folder that is laid out as Git for Windows is: another system's
            // bash beside another system's git is not Git Bash.
            .filter(|root| is_file(&root.join("cmd").join("git.exe")))
            .map(|root| root.join("bin").join("bash.exe"))
            .find(|bash| is_file(bash))
    }

    pub(super) fn local_shells() -> Vec<LocalShell> {
        let mut shells: Vec<LocalShell> = known_shells()
            .into_iter()
            .filter(|shell| shell.id != "bash")
            .map(|shell| LocalShell {
                id: shell.id.to_owned(),
                name: shell.name.to_owned(),
                program: shell.program,
                args: Vec::new(),
                login: false,
            })
            .collect();

        let path: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default();
        let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from);
        if let Some(bash) = git_bash(&path, program_files.as_deref(), &|file| file.is_file()) {
            let place = shells
                .iter()
                .position(|shell| shell.id == AFTER_GIT_BASH)
                .unwrap_or(shells.len());
            shells.insert(
                place,
                LocalShell {
                    id: GIT_BASH_ID.to_owned(),
                    name: "Git Bash".to_owned(),
                    program: bash.to_string_lossy().into_owned(),
                    // As its own shortcut starts it: reading the profile that sets
                    // its path up, and talking to a person.
                    args: vec!["--login".to_owned(), "-i".to_owned()],
                    login: false,
                },
            );
        }
        shells
    }
}

/// The shells the system's list of login shells names, as each one's file name and
/// path, each name once. Programs that are listed there and are no place to work are
/// left out.
#[cfg_attr(not(any(unix, test)), expect(dead_code))]
fn listed_shells(text: &str) -> Vec<(String, String)> {
    const NOT_SHELLS: [&str; 4] = ["nologin", "false", "true", "git-shell"];
    let mut found: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let path = line.split('#').next().unwrap_or_default().trim();
        let Some(name) = path.rsplit('/').next().filter(|name| !name.is_empty()) else {
            continue;
        };
        let known = found.iter().any(|(seen, _)| seen == name);
        if path.starts_with('/') && !known && !NOT_SHELLS.contains(&name) {
            found.push((name.to_owned(), path.to_owned()));
        }
    }
    found
}

#[cfg(unix)]
mod platform {
    use std::io::Read;
    use std::path::Path;

    use super::{LocalShell, listed_shells};
    use crate::shell::{known_shells, runnable};
    use crate::tuning::SHELLS_FILE_MAX_BYTES;

    /// Where the system lists the shells a user may have.
    const SHELLS_FILE: &str = "/etc/shells";

    fn shells_file() -> String {
        let mut text = String::new();
        if let Ok(file) = std::fs::File::open(SHELLS_FILE) {
            // What cannot be read as text is no list of shells.
            if file
                .take(SHELLS_FILE_MAX_BYTES)
                .read_to_string(&mut text)
                .is_err()
            {
                text.clear();
            }
        }
        text
    }

    pub(super) fn local_shells() -> Vec<LocalShell> {
        // A terminal on macOS starts a login shell: there is no login session whose
        // profile would otherwise have been read.
        let login = cfg!(target_os = "macos");
        let mut shells: Vec<LocalShell> = known_shells()
            .into_iter()
            .map(|shell| LocalShell {
                id: shell.id.to_owned(),
                name: shell.name.to_owned(),
                program: shell.program,
                args: Vec::new(),
                login,
            })
            .collect();
        for (name, path) in listed_shells(&shells_file()) {
            let known = shells.iter().any(|shell| shell.id == name);
            if !known && runnable(Path::new(&path)) {
                shells.push(LocalShell {
                    id: name.clone(),
                    name,
                    program: path,
                    args: Vec::new(),
                    login,
                });
            }
        }
        shells
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(id: &str, name: &str, program: &str) -> LocalShell {
        LocalShell {
            id: id.to_owned(),
            name: name.to_owned(),
            program: program.to_owned(),
            args: Vec::new(),
            login: false,
        }
    }

    fn distro(name: &str, running: bool, version: u8, default: bool) -> Distro {
        Distro {
            name: name.to_owned(),
            running,
            version,
            default,
        }
    }

    fn hosts(names: &[&str]) -> SshHosts {
        SshHosts {
            hosts: names.iter().map(|name| (*name).to_owned()).collect(),
            files: vec![PathBuf::from("config"), PathBuf::from("conf.d/work")],
        }
    }

    fn ids(detected: &Detected) -> Vec<&str> {
        detected
            .profiles
            .iter()
            .map(|profile| profile.id.as_str())
            .collect()
    }

    #[test]
    fn what_is_found_gets_profiles_with_ids_that_do_not_change() {
        let mut git_bash = shell("git-bash", "Git Bash", r"C:\Git\bin\bash.exe");
        git_bash.args = vec!["--login".to_owned(), "-i".to_owned()];
        let detected = assemble(
            vec![shell("pwsh", "PowerShell", r"C:\pwsh.exe"), git_bash],
            Listing::Distros(vec![
                distro("Debian", false, 1, false),
                distro("docker-desktop", true, 2, false),
                distro("Ubuntu", true, 2, true),
                distro("docker-desktop-data", false, 2, false),
                distro("rancher-desktop-data", false, 2, false),
                distro("Alpine", false, 2, false),
            ]),
            hosts(&["work", "db.internal"]),
            None,
        );
        assert_eq!(
            ids(&detected),
            [
                "auto.pwsh",
                "auto.git-bash",
                "auto.wsl.Ubuntu",
                "auto.wsl.Debian",
                "auto.wsl.Alpine",
                "auto.ssh.work",
                "auto.ssh.db.internal",
            ]
        );
        assert_eq!(detected.notes, Vec::<String>::new());
        assert_eq!(
            detected.watch,
            [PathBuf::from("config"), PathBuf::from("conf.d/work")]
        );

        for profile in &detected.profiles {
            assert_eq!(profile.origin, ProfileOrigin::Generated);
            assert_eq!(profile.shell_integration, ShellIntegration::Auto);
            assert!(!profile.hidden);
            assert_eq!(check_profile_id(&profile.id), Ok(()));
        }

        let pwsh = &detected.profiles[0];
        assert_eq!(pwsh.name, "PowerShell");
        assert_eq!(pwsh.detail, None);
        assert_eq!(pwsh.close_on_exit, None);
        assert_eq!(
            detected.profiles[1].launcher,
            Launcher::Shell {
                program: r"C:\Git\bin\bash.exe".to_owned(),
                args: vec!["--login".to_owned(), "-i".to_owned()],
                login: false,
            }
        );

        let ubuntu = &detected.profiles[2];
        assert_eq!(ubuntu.name, "Ubuntu");
        assert_eq!(ubuntu.detail.as_deref(), Some("WSL 2 · running"));
        assert_eq!(
            ubuntu.launcher,
            Launcher::Wsl {
                distro: "Ubuntu".to_owned(),
                user: None,
            }
        );
        assert_eq!(
            detected.profiles[3].detail.as_deref(),
            Some("WSL 1 · stopped")
        );

        let work = &detected.profiles[5];
        assert_eq!(work.name, "work");
        assert_eq!(work.detail.as_deref(), Some("from ~/.ssh/config"));
        assert_eq!(work.close_on_exit, Some(CloseOnExit::Clean));
        assert_eq!(
            work.launcher,
            Launcher::Ssh {
                host: "work".to_owned(),
                user: None,
                port: None,
                extra_args: Vec::new(),
                client: None,
            }
        );
    }

    #[test]
    fn why_there_are_no_distributions_is_noted() {
        let detected = assemble(
            Vec::new(),
            Listing::None {
                reason: "WSL is not installed".to_owned(),
            },
            SshHosts::default(),
            None,
        );
        assert_eq!(detected.profiles, []);
        assert_eq!(detected.notes, ["WSL is not installed"]);
        assert_eq!(detected.watch, Vec::<PathBuf>::new());
    }

    #[test]
    fn a_host_whose_name_cannot_be_in_an_id_is_left_out_and_said() {
        let detected = assemble(
            Vec::new(),
            Listing::Distros(Vec::new()),
            hosts(&["fine", "odd/name", "also,odd"]),
            None,
        );
        assert_eq!(ids(&detected), ["auto.ssh.fine"]);
        assert_eq!(detected.notes.len(), 2);
        assert!(detected.notes[0].contains("odd/name"));
    }

    #[test]
    fn shared_connections_are_asked_of_the_client_for_every_host() {
        let arguments = sharing_arguments("/tmp/lume-501", "10m");
        assert_eq!(
            arguments,
            [
                "-o",
                "ControlMaster=auto",
                "-o",
                "ControlPath=/tmp/lume-501/%C",
                "-o",
                "ControlPersist=10m",
            ]
        );
        let detected = assemble(
            vec![shell("sh", "sh", "/bin/sh")],
            Listing::Distros(Vec::new()),
            hosts(&["a", "b"]),
            Some(&arguments),
        );
        for profile in &detected.profiles[1..] {
            let Launcher::Ssh { extra_args, .. } = &profile.launcher else {
                panic!("{} is not an SSH profile", profile.id);
            };
            assert_eq!(extra_args, &arguments);
        }
        // Nothing but a host's profile has them.
        assert!(matches!(
            &detected.profiles[0].launcher,
            Launcher::Shell { args, .. } if args.is_empty()
        ));
    }

    #[test]
    fn the_folder_for_shared_connections_is_the_users_own_and_short() {
        assert_eq!(
            control_dir(Some("/run/user/1000"), 1000),
            "/run/user/1000/lume"
        );
        assert_eq!(
            control_dir(Some("/run/user/1000/"), 1000),
            "/run/user/1000/lume"
        );
        assert_eq!(control_dir(None, 501), "/tmp/lume-501");
        assert_eq!(control_dir(Some("relative"), 501), "/tmp/lume-501");
        // A runtime folder so deep that a socket's path would not fit is passed over.
        let deep = format!("/{}", "d".repeat(SSH_CONTROL_PATH_MAX_BYTES));
        assert_eq!(control_dir(Some(&deep), 501), "/tmp/lume-501");
        // The longest path either choice makes leaves the client the room it needs.
        let longest = format!("{}/{}", control_dir(None, u32::MAX), "c".repeat(40));
        assert!(longest.len() <= SSH_CONTROL_PATH_MAX_BYTES);
    }

    #[test]
    fn only_a_time_the_client_takes_is_passed_on() {
        for good in ["10m", "600", "yes", "no", "1h30m", "10M", "0"] {
            assert_eq!(check_persist(good), Ok(()), "{good}");
        }
        for bad in [
            "",
            "10 m",
            "10m -oProxyCommand=x",
            "-1",
            "ten\nminutes",
            "10m;",
        ] {
            assert!(check_persist(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_systems_list_of_shells_is_read_by_name() {
        let text = "# /etc/shells: valid login shells\n\
                    /bin/sh\n\
                    /bin/bash\n\
                    /usr/bin/bash\n\
                    \n\
                    /usr/bin/tcsh   # for the old hands\n\
                    /usr/sbin/nologin\n\
                    /usr/bin/git-shell\n\
                    relative/ksh\n\
                    /bin/\n\
                    /opt/homebrew/bin/elvish\n";
        assert_eq!(
            listed_shells(text),
            [
                ("sh".to_owned(), "/bin/sh".to_owned()),
                ("bash".to_owned(), "/bin/bash".to_owned()),
                ("tcsh".to_owned(), "/usr/bin/tcsh".to_owned()),
                ("elvish".to_owned(), "/opt/homebrew/bin/elvish".to_owned()),
            ]
        );
    }

    #[test]
    fn every_shell_found_on_this_machine_is_a_program_that_exists() {
        let shells = platform::local_shells();
        assert!(!shells.is_empty());
        for shell in &shells {
            let program = std::path::Path::new(&shell.program);
            assert!(program.is_absolute(), "{}", shell.program);
            assert!(program.is_file(), "{}", shell.program);
        }
        // Each has an id of its own.
        let mut ids: Vec<&str> = shells.iter().map(|shell| shell.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), shells.len());
    }

    #[cfg(windows)]
    #[test]
    fn git_bash_is_found_from_where_git_is() {
        use std::path::Path;

        use super::platform::{git_bash, git_root};

        let roots = [
            (
                r"C:\Program Files\Git\cmd\git.exe",
                Some(r"C:\Program Files\Git"),
            ),
            (
                r"C:\Program Files\Git\bin\git.exe",
                Some(r"C:\Program Files\Git"),
            ),
            (r"D:\Git\mingw64\bin\git.exe", Some(r"D:\Git")),
            (r"D:\Git\CMD\git.exe", Some(r"D:\Git")),
            (r"C:\scoop\shims\git.exe", None),
            (r"C:\git.exe", None),
        ];
        for (git, root) in roots {
            assert_eq!(git_root(Path::new(git)), root.map(PathBuf::from), "{git}");
        }

        let files = |present: &'static [&'static str]| {
            move |file: &Path| present.iter().any(|known| Path::new(known) == file)
        };
        let path = [
            PathBuf::from("relative"),
            PathBuf::from(r"C:\Windows\System32"),
            PathBuf::from(r"D:\Git\cmd"),
        ];
        let programs = Path::new(r"C:\Program Files");

        // Under the folder the git on the path belongs to.
        let present = files(&[
            r"C:\Windows\System32\bash.exe",
            r"D:\Git\cmd\git.exe",
            r"D:\Git\bin\bash.exe",
            r"C:\Program Files\Git\cmd\git.exe",
            r"C:\Program Files\Git\bin\bash.exe",
        ]);
        assert_eq!(
            git_bash(&path, Some(programs), &present),
            Some(PathBuf::from(r"D:\Git\bin\bash.exe"))
        );

        // With no git on the path, where the installer puts it.
        let present = files(&[
            r"C:\Windows\System32\bash.exe",
            r"C:\Program Files\Git\cmd\git.exe",
            r"C:\Program Files\Git\bin\bash.exe",
        ]);
        assert_eq!(
            git_bash(&path, Some(programs), &present),
            Some(PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"))
        );

        // The bash that starts a Linux distribution is never taken for it, nor is the
        // bash of some other system that has a git of its own.
        let present = files(&[
            r"C:\Windows\System32\bash.exe",
            r"C:\msys64\usr\bin\git.exe",
            r"C:\msys64\usr\bin\bash.exe",
            r"C:\msys64\bin\bash.exe",
        ]);
        let path = [
            PathBuf::from(r"C:\Windows\System32"),
            PathBuf::from(r"C:\msys64\usr\bin"),
        ];
        assert_eq!(git_bash(&path, Some(programs), &present), None);
        assert_eq!(git_bash(&path, None, &present), None);
    }
}
