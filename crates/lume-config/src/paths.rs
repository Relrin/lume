//! Where the user's files are.
//!
//! Two directories: one for what the user writes (the configuration, workspace files and
//! themes), and one for what the application writes for itself (the layout of each
//! workspace, window positions). They are kept apart so that the first can be put under
//! version control or copied to another machine without the second.
//!
//! A build for development uses directories of its own, so that trying something out
//! never touches the files of the installed application.

use std::path::{Path, PathBuf};

use crate::tuning::WORKSPACE_NAME_MAX;

/// The variable that names the configuration directory outright.
pub const CONFIG_DIR_VARIABLE: &str = "LUME_CONFIG_DIR";

/// The variable that names the state directory outright.
pub const STATE_DIR_VARIABLE: &str = "LUME_STATE_DIR";

/// What is added to the directories' names by a build for development.
const DEVELOPMENT_SUFFIX: &str = "-dev";

/// The variable that names the application's log file, in place of the one in the log
/// directory.
pub const LOG_FILE_VARIABLE: &str = "LUME_LOG_FILE";

/// The application's log file's name in the log directory.
const LOG_FILE_NAME: &str = "lume.log";

/// The session host's log file's name in the log directory.
const HOST_LOG_FILE_NAME: &str = "host.log";

/// The user's directory for the application's logs, from the variables that place it.
pub fn log_dir_with(variable: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    if cfg!(windows) {
        Some(variable("LOCALAPPDATA")?.join("Lume").join("logs"))
    } else if cfg!(target_os = "macos") {
        Some(variable("HOME")?.join("Library/Logs/Lume"))
    } else {
        let state = variable("XDG_STATE_HOME")
            .filter(|dir| dir.is_absolute())
            .or_else(|| Some(variable("HOME")?.join(".local/state")))?;
        Some(state.join("lume"))
    }
}

fn environment(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The file the application logs to: the one `LUME_LOG_FILE` names, or the default in
/// the user's log directory.
pub fn log_file() -> Option<PathBuf> {
    environment(LOG_FILE_VARIABLE).or_else(|| Some(log_dir_with(environment)?.join(LOG_FILE_NAME)))
}

/// The file the session host that runs on its own logs to: beside the application's.
/// When `LUME_LOG_FILE` names the application's, the host's is named after it.
pub fn host_log_file() -> Option<PathBuf> {
    match environment(LOG_FILE_VARIABLE) {
        Some(file) => Some(file.with_extension(HOST_LOG_FILE_NAME)),
        None => Some(log_dir_with(environment)?.join(HOST_LOG_FILE_NAME)),
    }
}

/// An operating system, where its conventions differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    Windows,
    Macos,
    Linux,
}

impl Platform {
    pub const ALL: [Self; 3] = [Self::Windows, Self::Macos, Self::Linux];

    /// The platform this program was built for. Every Unix that is not a Mac follows
    /// the Linux conventions.
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Linux
        }
    }

    /// The platform's name as configuration files and the command line spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Macos => "macos",
            Self::Linux => "linux",
        }
    }

    /// Reads a platform's name.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|platform| platform.as_str() == name)
    }
}

/// The two directories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dirs {
    /// What the user writes: `config.toml`, `workspaces/`, `themes/`.
    pub config: PathBuf,
    /// What the application writes for itself.
    pub state: PathBuf,
}

impl Dirs {
    /// The directories for this process: from its environment, on its platform.
    pub fn resolve() -> Option<Self> {
        Self::resolve_with(Platform::current(), cfg!(debug_assertions), |name| {
            std::env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
    }

    /// The directories on `platform`, from the variables that place them. `development`
    /// gives the directories of a build for development.
    ///
    /// A directory named outright by its variable is used as it is, development build
    /// or not: whoever names it has decided.
    pub fn resolve_with(
        platform: Platform,
        development: bool,
        variable: impl Fn(&str) -> Option<PathBuf>,
    ) -> Option<Self> {
        let suffix = if development { DEVELOPMENT_SUFFIX } else { "" };
        // A relative directory would mean a different place in every shell.
        let absolute = |name: &str| variable(name).filter(|dir| is_absolute(platform, dir));
        let home = || absolute("HOME");

        let config = match variable(CONFIG_DIR_VARIABLE) {
            Some(named) => named,
            None => match platform {
                Platform::Windows => absolute("APPDATA")?.join(format!("lume{suffix}")),
                Platform::Macos | Platform::Linux => absolute("XDG_CONFIG_HOME")
                    .or_else(|| Some(home()?.join(".config")))?
                    .join(format!("lume{suffix}")),
            },
        };
        let state = match variable(STATE_DIR_VARIABLE) {
            Some(named) => named,
            None => match platform {
                Platform::Windows => absolute("LOCALAPPDATA")?
                    .join(format!("Lume{suffix}"))
                    .join("state"),
                Platform::Macos => home()?
                    .join("Library")
                    .join("Application Support")
                    .join(format!("Lume{suffix}")),
                Platform::Linux => absolute("XDG_STATE_HOME")
                    .or_else(|| Some(home()?.join(".local").join("state")))?
                    .join(format!("lume{suffix}")),
            },
        };
        Some(Self { config, state })
    }

    /// The user's configuration file.
    pub fn config_file(&self) -> PathBuf {
        self.config.join("config.toml")
    }

    /// Where the user's workspace files are.
    pub fn workspaces_dir(&self) -> PathBuf {
        self.config.join("workspaces")
    }

    /// The file the user may write for a workspace: its folder and its overrides.
    pub fn workspace_file(&self, name: &str) -> PathBuf {
        self.workspaces_dir().join(format!("{name}.toml"))
    }

    /// Where the user's themes are.
    pub fn themes_dir(&self) -> PathBuf {
        self.config.join("themes")
    }

    /// Where the application keeps each workspace's layout.
    pub fn state_workspaces_dir(&self) -> PathBuf {
        self.state.join("workspaces")
    }

    /// The file the application keeps a workspace's layout in.
    pub fn state_workspace_file(&self, name: &str) -> PathBuf {
        self.state_workspaces_dir().join(format!("{name}.json"))
    }
}

/// Whether a path is absolute by the rules of `platform`, which need not be the one
/// this program runs on.
fn is_absolute(platform: Platform, path: &Path) -> bool {
    let text = path.to_string_lossy();
    match platform {
        Platform::Windows => {
            let bytes = text.as_bytes();
            let drive = bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && matches!(bytes[2], b'\\' | b'/');
            drive || text.starts_with("\\\\") || text.starts_with("//")
        }
        Platform::Macos | Platform::Linux => text.starts_with('/'),
    }
}

/// The names Windows keeps for devices. A file cannot be called by one, with any
/// extension.
const DEVICE_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Checks that `name` can be a workspace's. A workspace's name is also the name of its
/// files, on every platform, so it is held to what all of them allow: letters, digits,
/// spaces, dots, dashes and underscores, beginning with a letter or digit, not ending in
/// a dot or a space, and not a name Windows keeps for a device.
pub fn check_workspace_name(name: &str) -> Result<(), String> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-');
    if name.is_empty() {
        return Err("a workspace needs a name".to_owned());
    }
    if name.chars().count() > WORKSPACE_NAME_MAX {
        return Err(format!(
            "a workspace's name may have {WORKSPACE_NAME_MAX} characters at most"
        ));
    }
    if !name.chars().all(allowed) {
        return Err(format!(
            "`{name}` cannot be a workspace's name: only letters, digits, spaces, dots, \
             dashes and underscores can be used"
        ));
    }
    if !name.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return Err(format!(
            "`{name}` cannot be a workspace's name: it must begin with a letter or a digit"
        ));
    }
    if name.ends_with(['.', ' ']) {
        return Err(format!(
            "`{name}` cannot be a workspace's name: it must not end in a dot or a space"
        ));
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_lowercase();
    if DEVICE_NAMES.contains(&stem.as_str()) {
        return Err(format!(
            "`{name}` cannot be a workspace's name: Windows keeps it for a device"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn the_log_directory_is_the_users_own() {
        let home = |name: &str| match name {
            "LOCALAPPDATA" => Some(PathBuf::from(r"C:\Users\u\AppData\Local")),
            "HOME" => Some(PathBuf::from("/home/u")),
            _ => None,
        };
        let dir = log_dir_with(home).unwrap();
        if cfg!(windows) {
            assert_eq!(dir, PathBuf::from(r"C:\Users\u\AppData\Local\Lume\logs"));
        } else if cfg!(target_os = "macos") {
            assert_eq!(dir, PathBuf::from("/home/u/Library/Logs/Lume"));
        } else {
            assert_eq!(dir, PathBuf::from("/home/u/.local/state/lume"));
        }
        assert_eq!(log_dir_with(|_| None), None);
    }

    fn resolve(platform: Platform, development: bool, set: &[(&str, &str)]) -> Option<Dirs> {
        let variables: HashMap<String, PathBuf> = set
            .iter()
            .map(|(name, value)| ((*name).to_owned(), PathBuf::from(value)))
            .collect();
        Dirs::resolve_with(platform, development, |name| variables.get(name).cloned())
    }

    #[test]
    fn windows_keeps_the_configuration_roaming_and_the_state_local() {
        let dirs = resolve(
            Platform::Windows,
            false,
            &[
                ("APPDATA", r"C:\Users\u\AppData\Roaming"),
                ("LOCALAPPDATA", r"C:\Users\u\AppData\Local"),
            ],
        )
        .unwrap();
        assert_eq!(
            dirs.config,
            Path::new(r"C:\Users\u\AppData\Roaming").join("lume")
        );
        assert_eq!(
            dirs.state,
            Path::new(r"C:\Users\u\AppData\Local")
                .join("Lume")
                .join("state")
        );
    }

    #[test]
    fn linux_follows_the_base_directory_variables_and_falls_back_to_the_home() {
        let named = resolve(
            Platform::Linux,
            false,
            &[
                ("HOME", "/home/u"),
                ("XDG_CONFIG_HOME", "/cfg"),
                ("XDG_STATE_HOME", "/state"),
            ],
        )
        .unwrap();
        assert_eq!(named.config, Path::new("/cfg").join("lume"));
        assert_eq!(named.state, Path::new("/state").join("lume"));

        let home = resolve(Platform::Linux, false, &[("HOME", "/home/u")]).unwrap();
        assert_eq!(home.config, Path::new("/home/u/.config").join("lume"));
        assert_eq!(
            home.state,
            Path::new("/home/u")
                .join(".local")
                .join("state")
                .join("lume")
        );

        // A relative directory is not a place, and is passed over.
        let relative = resolve(
            Platform::Linux,
            false,
            &[("HOME", "/home/u"), ("XDG_CONFIG_HOME", "cfg")],
        )
        .unwrap();
        assert_eq!(relative.config, Path::new("/home/u/.config").join("lume"));
    }

    #[test]
    fn a_mac_keeps_the_configuration_where_linux_does_and_the_state_in_the_library() {
        let dirs = resolve(Platform::Macos, false, &[("HOME", "/Users/u")]).unwrap();
        assert_eq!(dirs.config, Path::new("/Users/u/.config").join("lume"));
        assert_eq!(
            dirs.state,
            Path::new("/Users/u")
                .join("Library")
                .join("Application Support")
                .join("Lume")
        );
    }

    #[test]
    fn a_development_build_has_directories_of_its_own() {
        let windows = resolve(
            Platform::Windows,
            true,
            &[("APPDATA", r"C:\r"), ("LOCALAPPDATA", r"C:\l")],
        )
        .unwrap();
        assert_eq!(windows.config, Path::new(r"C:\r").join("lume-dev"));
        assert_eq!(
            windows.state,
            Path::new(r"C:\l").join("Lume-dev").join("state")
        );
        let linux = resolve(Platform::Linux, true, &[("HOME", "/home/u")]).unwrap();
        assert_eq!(linux.config, Path::new("/home/u/.config").join("lume-dev"));
    }

    #[test]
    fn a_directory_named_outright_is_used_as_it_is() {
        let dirs = resolve(
            Platform::Windows,
            true,
            &[
                (CONFIG_DIR_VARIABLE, r"D:\tmp\cfg"),
                (STATE_DIR_VARIABLE, r"D:\tmp\state"),
            ],
        )
        .unwrap();
        assert_eq!(dirs.config, Path::new(r"D:\tmp\cfg"));
        assert_eq!(dirs.state, Path::new(r"D:\tmp\state"));
    }

    #[test]
    fn with_nothing_to_go_by_there_are_no_directories() {
        assert_eq!(resolve(Platform::Windows, false, &[]), None);
        assert_eq!(resolve(Platform::Linux, false, &[]), None);
        assert_eq!(
            resolve(Platform::Macos, false, &[("XDG_CONFIG_HOME", "/c")]),
            None
        );
    }

    #[test]
    fn the_files_have_their_places() {
        let dirs = Dirs {
            config: PathBuf::from("cfg"),
            state: PathBuf::from("state"),
        };
        assert_eq!(dirs.config_file(), Path::new("cfg").join("config.toml"));
        assert_eq!(
            dirs.workspace_file("work"),
            Path::new("cfg").join("workspaces").join("work.toml")
        );
        assert_eq!(dirs.themes_dir(), Path::new("cfg").join("themes"));
        assert_eq!(
            dirs.state_workspace_file("work"),
            Path::new("state").join("workspaces").join("work.json")
        );
    }

    #[test]
    fn a_workspace_name_is_one_every_platform_can_use_for_a_file() {
        for good in ["default", "Work 2", "a.b", "x_y-z", "9lives"] {
            assert_eq!(check_workspace_name(good), Ok(()), "{good}");
        }
        for bad in [
            "", " lead", "trail ", "dot.", "a/b", "a\\b", "a:b", "con", "NUL.txt", "é", ".hidden",
        ] {
            assert!(check_workspace_name(bad).is_err(), "{bad:?}");
        }
        assert!(check_workspace_name(&"a".repeat(WORKSPACE_NAME_MAX)).is_ok());
        assert!(check_workspace_name(&"a".repeat(WORKSPACE_NAME_MAX + 1)).is_err());
    }
}
