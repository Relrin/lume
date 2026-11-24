//! Profiles: named ways to start a terminal.
//!
//! A profile says what a terminal runs (a program on this machine, a Linux
//! distribution's shell, a shell on another machine) and a few things about the pane it
//! runs in. The user writes profiles as `[[profile]]` tables in `config.toml` or in a
//! workspace's file. The application also makes profiles itself for what it finds
//! installed; those are never written to a file.
//!
//! A table in a file is a [`ProfileEntry`]: it holds what the user wrote and nothing for
//! what they left out. Entries with the same id are laid over each other field by
//! field, a workspace's over the user's, and both over a profile the application made
//! with that id. That is how a user changes one thing about a generated profile, or
//! hides it, without restating the rest. What comes out is a [`Profile`], which is
//! complete.

use std::collections::BTreeMap;

use lume_proto::Launcher;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::schema::CloseOnExit;
use crate::tuning::{FONT_SIZE_MAX, FONT_SIZE_MIN, PROFILE_ID_MAX_CHARS, PROFILE_ID_STEM_CHARS};

/// The ids of the profiles the application makes begin with this.
pub const GENERATED_PREFIX: &str = "auto.";

/// What a profile's terminal runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LauncherKind {
    /// A program on this machine.
    Shell,
    /// The shell of a Linux distribution on this Windows machine.
    Wsl,
    /// A shell on another machine, through the OpenSSH client.
    Ssh,
}

impl LauncherKind {
    pub fn of(launcher: &Launcher) -> Self {
        match launcher {
            Launcher::Wsl { .. } => Self::Wsl,
            Launcher::Ssh { .. } => Self::Ssh,
            // A kind of launcher this build does not know is some program to run.
            _ => Self::Shell,
        }
    }
}

/// Whether a shell is told to report its directory and its commands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ShellIntegration {
    /// The shell is set up to report them, without its own files being changed.
    #[default]
    Auto,
    /// The shell is started as it is.
    Off,
}

/// One `[[profile]]` table: what it sets, and nothing for what it leaves out.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ProfileEntry {
    /// The profile's id: what `default-profile`, commands and other entries name it by.
    /// An id the application uses for a profile it made (`auto.pwsh`,
    /// `auto.wsl.<distribution>`, `auto.ssh.<host>`) changes that profile.
    pub id: String,
    /// What the profile is called in lists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The name of the icon shown for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// What the terminal runs. Needed unless the id is that of a generated profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher: Option<LauncherKind>,
    /// `shell`: the program to run. Left out, the user's own shell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    /// `shell`: the program's arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    /// `shell`: start the program as a login shell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<bool>,
    /// `wsl`: the distribution's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distro: Option<String>,
    /// `ssh`: the machine, as it would be given to `ssh`: a name from the user's
    /// OpenSSH configuration, a host name or an address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// `wsl` and `ssh`: the user to be there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// `ssh`: the port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1))]
    pub port: Option<u16>,
    /// `ssh`: arguments given to the client before the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_args: Option<Vec<String>>,
    /// `ssh`: the client program, instead of the one found on this machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    /// The directory the terminal starts in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Environment variables set for the program.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
    /// The theme of this profile's terminals, instead of the window's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// The text size of this profile's terminals, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = FONT_SIZE_MIN, max = FONT_SIZE_MAX))]
    pub font_size: Option<f64>,
    /// Whether the shell is set up to report its directory and commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_integration: Option<ShellIntegration>,
    /// What happens to the pane when the program ends, instead of
    /// `terminal.close-on-exit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_on_exit: Option<CloseOnExit>,
    /// Leave the profile out of lists. It can still be named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
}

/// Where a profile comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileOrigin {
    /// The application made it for something it found on this machine.
    Generated,
    /// The user wrote it.
    User,
}

/// A way to start a terminal, complete.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub launcher: Launcher,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    pub shell_integration: ShellIntegration,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_on_exit: Option<CloseOnExit>,
    pub hidden: bool,
    pub origin: ProfileOrigin,
    /// A line about it for lists: where it was found, what state it is in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Why an id cannot be a profile's.
pub fn check_profile_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("a profile's id cannot be empty".to_owned());
    }
    if id.chars().count() > PROFILE_ID_MAX_CHARS {
        return Err(format!(
            "a profile's id is at most {PROFILE_ID_MAX_CHARS} characters"
        ));
    }
    let allowed = |c: char| c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '@' | ':');
    match id.chars().find(|c| !allowed(*c)) {
        Some(bad) => Err(format!(
            "a profile's id is letters, digits and `. _ - @ :`, not `{bad}`"
        )),
        None => Ok(()),
    }
}

/// An id for a profile called `name` that `taken` says no other has.
pub fn free_profile_id(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut stem = String::new();
    for c in name.trim().chars().take(PROFILE_ID_STEM_CHARS) {
        if c.is_alphanumeric() || matches!(c, '.' | '_') {
            stem.extend(c.to_lowercase());
        } else if !stem.is_empty() && !stem.ends_with('-') {
            stem.push('-');
        }
    }
    let stem = stem.trim_matches(['-', '.']);
    // The ids of the profiles the application makes are not for others.
    let stem = match stem {
        "" => "profile".to_owned(),
        stem if is_generated_id(stem) => format!("my-{stem}"),
        stem => stem.to_owned(),
    };
    let mut id = stem.clone();
    let mut number = 2;
    while taken(&id) {
        id = format!("{stem}-{number}");
        number += 1;
    }
    id
}

/// Whether the application makes profiles with ids like this one.
pub fn is_generated_id(id: &str) -> bool {
    id.starts_with(GENERATED_PREFIX)
}

/// Why a word cannot be given to a program as a host, user or distribution: a word
/// that starts with a hyphen would be read as an option.
fn check_word(what: &str, word: &str) -> Result<(), String> {
    if word.is_empty() {
        return Err(format!("`{what}` cannot be empty"));
    }
    if word.starts_with('-') {
        return Err(format!("`{what}` cannot start with `-`"));
    }
    if word.chars().any(char::is_control) {
        return Err(format!("`{what}` cannot hold control characters"));
    }
    Ok(())
}

impl ProfileEntry {
    /// An entry that sets nothing but its id.
    pub fn named(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ..Self::default()
        }
    }

    /// Lays `upper` over this entry: what `upper` sets replaces what was here.
    pub fn overlay(&mut self, upper: &Self) {
        fn lay<T: Clone>(lower: &mut Option<T>, upper: &Option<T>) {
            if upper.is_some() {
                lower.clone_from(upper);
            }
        }
        lay(&mut self.name, &upper.name);
        lay(&mut self.icon, &upper.icon);
        lay(&mut self.launcher, &upper.launcher);
        lay(&mut self.program, &upper.program);
        lay(&mut self.args, &upper.args);
        lay(&mut self.login, &upper.login);
        lay(&mut self.distro, &upper.distro);
        lay(&mut self.host, &upper.host);
        lay(&mut self.user, &upper.user);
        lay(&mut self.port, &upper.port);
        lay(&mut self.extra_args, &upper.extra_args);
        lay(&mut self.client, &upper.client);
        lay(&mut self.cwd, &upper.cwd);
        lay(&mut self.env, &upper.env);
        lay(&mut self.theme, &upper.theme);
        lay(&mut self.font_size, &upper.font_size);
        lay(&mut self.shell_integration, &upper.shell_integration);
        lay(&mut self.close_on_exit, &upper.close_on_exit);
        lay(&mut self.hidden, &upper.hidden);
    }

    /// What is wrong with the entry whatever it is laid over, if anything: its id, and
    /// the words it would hand to a program.
    pub fn check_written(&self) -> Result<(), String> {
        check_profile_id(&self.id)?;
        self.check_words()
    }

    /// What is wrong with the entry as a profile that stands on its own, if anything.
    /// An entry for a generated profile's id may leave anything out: it stands on the
    /// profile the application made.
    pub fn check(&self) -> Result<(), String> {
        self.check_written()?;
        if is_generated_id(&self.id) && self.launcher.is_none() {
            return Ok(());
        }
        self.clone().into_launcher().map(drop)
    }

    fn check_words(&self) -> Result<(), String> {
        for (what, word) in [
            ("distro", &self.distro),
            ("host", &self.host),
            ("user", &self.user),
        ] {
            if let Some(word) = word {
                check_word(what, word)?;
            }
        }
        Ok(())
    }

    /// The launcher the entry describes, with the fields that belong to it.
    fn into_launcher(self) -> Result<Launcher, String> {
        self.check_words()?;
        let kind = self
            .launcher
            .ok_or_else(|| "`launcher` is missing: `shell`, `wsl` or `ssh`".to_owned())?;
        let not_for = |set: bool, key: &str, kinds: &str| {
            if set {
                Err(format!("`{key}` is for a {kinds} profile"))
            } else {
                Ok(())
            }
        };
        match kind {
            LauncherKind::Shell => {
                not_for(self.distro.is_some(), "distro", "`wsl`")?;
                not_for(self.host.is_some(), "host", "`ssh`")?;
                not_for(self.user.is_some(), "user", "`wsl` or `ssh`")?;
                not_for(self.port.is_some(), "port", "`ssh`")?;
                not_for(self.extra_args.is_some(), "extra-args", "`ssh`")?;
                not_for(self.client.is_some(), "client", "`ssh`")?;
                Ok(Launcher::Shell {
                    program: self.program.unwrap_or_default(),
                    args: self.args.unwrap_or_default(),
                    login: self.login.unwrap_or(false),
                })
            }
            LauncherKind::Wsl => {
                not_for(self.program.is_some(), "program", "`shell`")?;
                not_for(self.args.is_some(), "args", "`shell`")?;
                not_for(self.login.is_some(), "login", "`shell`")?;
                not_for(self.host.is_some(), "host", "`ssh`")?;
                not_for(self.port.is_some(), "port", "`ssh`")?;
                not_for(self.extra_args.is_some(), "extra-args", "`ssh`")?;
                not_for(self.client.is_some(), "client", "`ssh`")?;
                let distro = self
                    .distro
                    .ok_or_else(|| "a `wsl` profile needs `distro`".to_owned())?;
                Ok(Launcher::Wsl {
                    distro,
                    user: self.user,
                })
            }
            LauncherKind::Ssh => {
                not_for(self.program.is_some(), "program", "`shell`")?;
                not_for(self.args.is_some(), "args", "`shell`")?;
                not_for(self.login.is_some(), "login", "`shell`")?;
                not_for(self.distro.is_some(), "distro", "`wsl`")?;
                let host = self
                    .host
                    .ok_or_else(|| "an `ssh` profile needs `host`".to_owned())?;
                Ok(Launcher::Ssh {
                    host,
                    user: self.user,
                    port: self.port,
                    extra_args: self.extra_args.unwrap_or_default(),
                    client: self.client,
                })
            }
        }
    }

    /// The complete profile this entry describes.
    pub fn resolve(self, origin: ProfileOrigin, detail: Option<String>) -> Result<Profile, String> {
        check_profile_id(&self.id)?;
        let id = self.id.clone();
        let name = self.name.clone().filter(|name| !name.trim().is_empty());
        let icon = self.icon.clone();
        let cwd = self.cwd.clone().filter(|cwd| !cwd.is_empty());
        let env = self.env.clone().unwrap_or_default();
        let theme = self.theme.clone();
        let font_size = self.font_size;
        let shell_integration = self.shell_integration.unwrap_or_default();
        let close_on_exit = self.close_on_exit;
        let hidden = self.hidden.unwrap_or(false);
        let launcher = self.into_launcher()?;
        Ok(Profile {
            name: name.unwrap_or_else(|| id.clone()),
            id,
            icon,
            launcher,
            cwd,
            env,
            theme,
            font_size,
            shell_integration,
            close_on_exit,
            hidden,
            origin,
            detail,
        })
    }
}

impl Profile {
    /// The profile as an entry that sets everything the profile has.
    pub fn to_entry(&self) -> ProfileEntry {
        let mut entry = ProfileEntry {
            id: self.id.clone(),
            name: Some(self.name.clone()),
            icon: self.icon.clone(),
            launcher: Some(LauncherKind::of(&self.launcher)),
            cwd: self.cwd.clone(),
            env: (!self.env.is_empty()).then(|| self.env.clone()),
            theme: self.theme.clone(),
            font_size: self.font_size,
            shell_integration: Some(self.shell_integration),
            close_on_exit: self.close_on_exit,
            hidden: Some(self.hidden),
            ..ProfileEntry::default()
        };
        match &self.launcher {
            Launcher::Wsl { distro, user } => {
                entry.distro = Some(distro.clone());
                entry.user.clone_from(user);
            }
            Launcher::Ssh {
                host,
                user,
                port,
                extra_args,
                client,
            } => {
                entry.host = Some(host.clone());
                entry.user.clone_from(user);
                entry.port = *port;
                entry.extra_args = (!extra_args.is_empty()).then(|| extra_args.clone());
                entry.client.clone_from(client);
            }
            Launcher::Shell {
                program,
                args,
                login,
            } => {
                entry.program = (!program.is_empty()).then(|| program.clone());
                entry.args = (!args.is_empty()).then(|| args.clone());
                entry.login = login.then_some(true);
            }
            // A launcher this build does not know has no fields an entry could hold.
            _ => {}
        }
        entry
    }
}

/// The profiles in effect: the ones the application made with the user's entries laid
/// over them, and the user's own.
///
/// The user's own come first, in the order they were written; the generated ones
/// follow in the order they were found. An entry that cannot be made into a profile is
/// left out and said in the second part of the answer, as `(id, why)`.
pub fn merge_profiles(
    generated: &[Profile],
    entries: &[ProfileEntry],
) -> (Vec<Profile>, Vec<(String, String)>) {
    let mut profiles = Vec::new();
    let mut problems = Vec::new();
    let is_generated = |id: &str| generated.iter().any(|profile| profile.id == id);

    for entry in entries.iter().filter(|entry| !is_generated(&entry.id)) {
        match entry.clone().resolve(ProfileOrigin::User, None) {
            Ok(profile) => profiles.push(profile),
            Err(why) => problems.push((entry.id.clone(), why)),
        }
    }
    for base in generated {
        let Some(over) = entries.iter().find(|entry| entry.id == base.id) else {
            profiles.push(base.clone());
            continue;
        };
        let mut entry = base.to_entry();
        entry.overlay(over);
        match entry.resolve(ProfileOrigin::Generated, base.detail.clone()) {
            Ok(profile) => profiles.push(profile),
            Err(why) => {
                // What the user wrote does not fit; the profile stays as it was found.
                problems.push((base.id.clone(), why));
                profiles.push(base.clone());
            }
        }
    }
    (profiles, problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(toml: &str) -> ProfileEntry {
        toml::from_str(toml).unwrap()
    }

    fn generated(id: &str, launcher: Launcher) -> Profile {
        Profile {
            id: id.to_owned(),
            name: id.to_owned(),
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
            detail: Some("found".to_owned()),
        }
    }

    #[test]
    fn a_shell_profile_needs_only_its_launcher() {
        let profile = entry("id = 'mine'\nlauncher = 'shell'")
            .resolve(ProfileOrigin::User, None)
            .unwrap();
        assert_eq!(profile.name, "mine");
        assert_eq!(
            profile.launcher,
            Launcher::Shell {
                program: String::new(),
                args: vec![],
                login: false
            }
        );
        assert_eq!(profile.shell_integration, ShellIntegration::Auto);
        assert!(!profile.hidden);
        assert_eq!(profile.close_on_exit, None);
    }

    #[test]
    fn every_field_of_an_ssh_profile_arrives() {
        let profile = entry(
            "id = 'box'\nname = 'Work box'\nlauncher = 'ssh'\nhost = 'work'\nuser = 'me'\n\
             port = 2222\nextra-args = ['-A']\nclient = 'C:/ssh.exe'\ncwd = 'D:/'\n\
             env = { A = 'b' }\ntheme = 'lume-light'\nfont-size = 13\n\
             shell-integration = 'off'\nclose-on-exit = 'never'\nhidden = true\nicon = 'server'",
        )
        .resolve(ProfileOrigin::User, None)
        .unwrap();
        assert_eq!(profile.name, "Work box");
        assert_eq!(
            profile.launcher,
            Launcher::Ssh {
                host: "work".to_owned(),
                user: Some("me".to_owned()),
                port: Some(2222),
                extra_args: vec!["-A".to_owned()],
                client: Some("C:/ssh.exe".to_owned()),
            }
        );
        assert_eq!(profile.env["A"], "b");
        assert_eq!(profile.theme.as_deref(), Some("lume-light"));
        assert_eq!(profile.font_size, Some(13.0));
        assert_eq!(profile.shell_integration, ShellIntegration::Off);
        assert_eq!(profile.close_on_exit, Some(CloseOnExit::Never));
        assert!(profile.hidden);
        assert_eq!(profile.icon.as_deref(), Some("server"));
    }

    #[test]
    fn what_is_missing_or_misplaced_is_said() {
        let why = |toml: &str| entry(toml).check().unwrap_err();
        assert!(why("id = 'a'").contains("`launcher` is missing"));
        assert!(why("id = 'a'\nlauncher = 'wsl'").contains("needs `distro`"));
        assert!(why("id = 'a'\nlauncher = 'ssh'").contains("needs `host`"));
        assert!(why("id = 'a'\nlauncher = 'shell'\nhost = 'x'").contains("`host` is for"));
        assert!(why("id = 'a'\nlauncher = 'ssh'\nhost = 'x'\nprogram = 'p'").contains("`program`"));
        assert!(why("id = 'a b'\nlauncher = 'shell'").contains("not ` `"));
        assert!(why("id = ''\nlauncher = 'shell'").contains("empty"));
    }

    #[test]
    fn a_word_that_would_be_read_as_an_option_is_refused() {
        let why = |toml: &str| entry(toml).check().unwrap_err();
        assert!(
            why("id = 'a'\nlauncher = 'ssh'\nhost = '-oProxyCommand=calc'").contains("start with")
        );
        assert!(why("id = 'a'\nlauncher = 'ssh'\nhost = 'h'\nuser = '-l'").contains("start with"));
        assert!(why("id = 'a'\nlauncher = 'wsl'\ndistro = '--exec'").contains("start with"));
        // The same holds for an entry that only changes a generated profile.
        assert!(why("id = 'auto.ssh.h'\nhost = '-x'").contains("start with"));
    }

    #[test]
    fn an_unknown_key_is_refused_by_the_type() {
        assert!(
            toml::from_str::<ProfileEntry>("id = 'a'\nlauncher = 'shell'\nprogam = 'x'").is_err()
        );
    }

    #[test]
    fn an_entry_for_a_generated_id_may_set_one_thing() {
        assert_eq!(entry("id = 'auto.pwsh'\nhidden = true").check(), Ok(()));
        let found = [
            generated(
                "auto.pwsh",
                Launcher::Shell {
                    program: "C:/pwsh.exe".to_owned(),
                    args: vec![],
                    login: false,
                },
            ),
            generated(
                "auto.wsl.Ubuntu",
                Launcher::Wsl {
                    distro: "Ubuntu".to_owned(),
                    user: None,
                },
            ),
        ];
        let written = [
            entry("id = 'auto.wsl.Ubuntu'\nuser = 'root'\nname = 'Ubuntu as root'"),
            entry("id = 'mine'\nlauncher = 'shell'\nprogram = 'nu'"),
            entry("id = 'auto.pwsh'\nhidden = true"),
        ];
        let (profiles, problems) = merge_profiles(&found, &written);
        assert_eq!(problems, vec![]);
        let ids: Vec<&str> = profiles.iter().map(|profile| profile.id.as_str()).collect();
        assert_eq!(ids, ["mine", "auto.pwsh", "auto.wsl.Ubuntu"]);
        assert_eq!(profiles[0].origin, ProfileOrigin::User);
        assert!(profiles[1].hidden);
        assert_eq!(profiles[1].origin, ProfileOrigin::Generated);
        assert_eq!(profiles[2].name, "Ubuntu as root");
        assert_eq!(profiles[2].detail.as_deref(), Some("found"));
        assert_eq!(
            profiles[2].launcher,
            Launcher::Wsl {
                distro: "Ubuntu".to_owned(),
                user: Some("root".to_owned()),
            }
        );
    }

    #[test]
    fn an_entry_that_does_not_fit_leaves_the_generated_profile_as_found() {
        let found = [generated(
            "auto.wsl.Ubuntu",
            Launcher::Wsl {
                distro: "Ubuntu".to_owned(),
                user: None,
            },
        )];
        let written = [
            entry("id = 'auto.wsl.Ubuntu'\nhost = 'x'"),
            entry("id = 'auto.wsl.Gone'\nuser = 'root'"),
        ];
        let (profiles, problems) = merge_profiles(&found, &written);
        assert_eq!(profiles, found);
        assert_eq!(problems.len(), 2);
        assert_eq!(problems[0].0, "auto.wsl.Gone");
        assert!(problems[0].1.contains("`launcher` is missing"));
        assert!(problems[1].1.contains("`host` is for"));
    }

    #[test]
    fn a_profile_goes_to_an_entry_and_back_unchanged() {
        let profile = entry(
            "id = 'box'\nname = 'Box'\nlauncher = 'ssh'\nhost = 'work'\nport = 22\n\
             env = { A = 'b' }\nclose-on-exit = 'always'",
        )
        .resolve(ProfileOrigin::User, None)
        .unwrap();
        assert_eq!(
            profile
                .to_entry()
                .resolve(ProfileOrigin::User, None)
                .unwrap(),
            profile
        );
    }

    #[test]
    fn an_upper_entry_replaces_only_what_it_sets() {
        let mut lower = entry("id = 'a'\nlauncher = 'shell'\nprogram = 'bash'\nargs = ['-l']");
        lower.overlay(&entry("id = 'a'\nprogram = 'zsh'\ncwd = '/tmp'"));
        assert_eq!(lower.program.as_deref(), Some("zsh"));
        assert_eq!(lower.args, Some(vec!["-l".to_owned()]));
        assert_eq!(lower.cwd.as_deref(), Some("/tmp"));
        assert_eq!(lower.launcher, Some(LauncherKind::Shell));
    }
}
