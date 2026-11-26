//! Planning how a pane's session starts: what it runs, in which directory and with which
//! variables, from what the pane asks, its profile and the settings. Nothing here starts
//! anything.

use super::*;

/// The directory a session starts in. `asked` is where its pane was last started; a
/// directory that is no longer there is passed over.
pub(super) fn start_dir(
    asked: Option<&str>,
    root: Option<&str>,
    new_panes: NewPaneCwd,
) -> Option<String> {
    let there = |dir: &&str| Path::new(dir).is_dir();
    if let Some(dir) = asked.filter(there) {
        return Some(dir.to_owned());
    }
    // A pane that had a directory belongs to the workspace's work, whatever new panes
    // are set to start in.
    let in_root = asked.is_some() || new_panes != NewPaneCwd::Home;
    if let Some(dir) = root.filter(|_| in_root).filter(there) {
        return Some(dir.to_owned());
    }
    std::env::home_dir().map(|home| home.to_string_lossy().into_owned())
}

/// The program a session runs when its pane names none: the configured shell, or the
/// user's own.
pub(super) fn start_launcher(shell: &str, args: &[String]) -> Launcher {
    if !shell.trim().is_empty() {
        return Launcher::Shell {
            program: shell.to_owned(),
            args: args.to_vec(),
            login: false,
        };
    }
    match default_shell() {
        Launcher::Shell { program, login, .. } if !args.is_empty() => Launcher::Shell {
            program,
            args: args.to_vec(),
            login,
        },
        launcher => launcher,
    }
}

/// What a pane's session is started with, once its profile and the settings have
/// been consulted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Start {
    pub(super) launcher: Launcher,
    pub(super) cwd: Option<String>,
    pub(super) env: Vec<(String, String)>,
}

/// A directory as a profile writes it, with a leading `~` standing for the user's
/// home directory.
pub(super) fn expand_home(dir: &str) -> String {
    let rest = match dir.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => rest,
        _ => return dir.to_owned(),
    };
    match std::env::home_dir() {
        Some(home) => {
            let rest = rest.trim_start_matches(['/', '\\']);
            // Joining nothing would still add a separator.
            let dir = if rest.is_empty() {
                home
            } else {
                home.join(rest)
            };
            dir.to_string_lossy().into_owned()
        }
        None => dir.to_owned(),
    }
}

/// The directory given to a Linux distribution's shell. A Linux path is taken on
/// trust, since only the distribution can say whether it is there; a path of this
/// machine has to be a folder. Anything else leaves the shell in its home directory.
pub(super) fn distro_dir(asked: Option<&str>) -> Option<String> {
    let dir = asked?;
    let linux = dir.starts_with('/') || dir == "~" || dir.starts_with("~/");
    (linux || Path::new(dir).is_dir()).then(|| dir.to_owned())
}

/// What a pane runs. A pane that names its program runs it as named. Any other runs
/// its profile; a profile that names no program runs the user's shell with the
/// profile's arguments. With neither, it is the shell of the settings.
pub(super) fn planned_launcher(
    named: Option<&Launcher>,
    profile: Option<&Profile>,
    terminal: &TerminalConfig,
) -> Launcher {
    match (named, profile) {
        (Some(launcher), _) => launcher.clone(),
        (None, Some(profile)) => match &profile.launcher {
            Launcher::Shell {
                program,
                args,
                login,
            } if program.trim().is_empty() => match start_launcher("", args) {
                Launcher::Shell {
                    program,
                    args,
                    login: own,
                } => Launcher::Shell {
                    program,
                    args,
                    login: *login || own,
                },
                other => other,
            },
            launcher => launcher.clone(),
        },
        (None, None) => start_launcher(&terminal.shell, &terminal.shell_args),
    }
}

/// Whether two sessions are in the same place, so that a directory of one is a
/// directory of the other: both on this machine, or both in the same Linux
/// distribution as the same user.
pub(super) fn same_place(one: &Launcher, other: &Launcher) -> bool {
    match (one, other) {
        (Launcher::Shell { .. }, Launcher::Shell { .. }) => true,
        (
            Launcher::Wsl { distro, user },
            Launcher::Wsl {
                distro: other_distro,
                user: other_user,
            },
        ) => distro.eq_ignore_ascii_case(other_distro) && user == other_user,
        _ => false,
    }
}

/// Works out what a pane's session is started with: what it runs, where, and with
/// which variables.
pub(super) fn plan_start(
    request: &SpawnRequest,
    profile: Option<&Profile>,
    terminal: &TerminalConfig,
    root: Option<&str>,
    new_panes: NewPaneCwd,
) -> Start {
    let profile = profile.filter(|_| request.launcher.is_none());
    let launcher = planned_launcher(request.launcher.as_ref(), profile, terminal);

    let from_profile = profile.and_then(|profile| profile.cwd.as_deref());
    let cwd = match &launcher {
        Launcher::Wsl { .. } => distro_dir(request.cwd.as_deref().or(from_profile)),
        // The directory is the other machine's business.
        Launcher::Ssh { .. } => None,
        _ => {
            let asked = request
                .cwd
                .clone()
                .or_else(|| from_profile.map(expand_home));
            start_dir(asked.as_deref(), root, new_panes)
        }
    };

    // The pane's own variables go last, so they are the ones that hold.
    let mut env: Vec<(String, String)> = profile
        .map(|profile| {
            profile
                .env
                .iter()
                .filter(|(name, _)| !request.env.iter().any(|(set, _)| set == *name))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default();
    env.extend(request.env.iter().cloned());

    Start { launcher, cwd, env }
}
