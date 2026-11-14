//! The environment a child starts with, and its encoding as a Windows environment block.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::OsStrExt;

use crate::{CommandSpec, IDENTITY_ENV};

/// Variables that describe the terminal the caller itself runs in. A child must not
/// inherit them: `TERM` means nothing to Windows programs and misleads ported ones, and
/// the others would make programs believe they run in a different terminal application.
const INHERITED_TERMINAL_ENV: [&str; 3] = ["TERM", "WT_SESSION", "WT_PROFILE_ID"];

/// A child's environment, in the order an environment block wants it.
pub(crate) struct Environment {
    /// Keyed by the name as Windows compares it. The value keeps the original spelling.
    vars: BTreeMap<SortKey, (OsString, OsString)>,
}

/// Orders variables the way Windows keeps them: the hidden per-drive entries whose names
/// start with `=` first, then by name without regard to case.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SortKey {
    visible: bool,
    upper: String,
}

impl SortKey {
    fn new(name: &OsStr) -> Self {
        let upper = name.to_string_lossy().to_uppercase();
        Self {
            visible: !upper.starts_with('='),
            upper,
        }
    }
}

impl Environment {
    /// Builds the environment for `command` on top of `inherited`.
    ///
    /// Later steps win: the inherited variables, minus those describing the caller's own
    /// terminal, plus the ones identifying this terminal, minus what the command removes,
    /// plus what the command sets.
    pub(crate) fn for_command(
        inherited: impl IntoIterator<Item = (OsString, OsString)>,
        command: &CommandSpec,
    ) -> Self {
        let mut env = Self {
            vars: BTreeMap::new(),
        };
        for (name, value) in inherited {
            env.set(name, value);
        }
        for name in INHERITED_TERMINAL_ENV {
            env.remove(OsStr::new(name));
        }
        for (name, value) in IDENTITY_ENV {
            env.set(name.into(), value.into());
        }
        for name in &command.env_remove {
            env.remove(name);
        }
        for (name, value) in &command.env {
            env.set(name.clone(), value.clone());
        }
        env
    }

    fn set(&mut self, name: OsString, value: OsString) {
        self.vars.insert(SortKey::new(&name), (name, value));
    }

    fn remove(&mut self, name: &OsStr) {
        self.vars.remove(&SortKey::new(name));
    }

    /// The value of a variable, matched without regard to case.
    pub(crate) fn get(&self, name: &str) -> Option<&OsStr> {
        self.vars
            .get(&SortKey::new(OsStr::new(name)))
            .map(|(_, value)| value.as_os_str())
    }

    /// Encodes the environment as `NAME=value\0` entries followed by a final `\0`.
    pub(crate) fn block(&self) -> io::Result<Vec<u16>> {
        let invalid = |what: &str| io::Error::new(io::ErrorKind::InvalidInput, what.to_owned());
        let mut block = Vec::new();
        for (name, value) in self.vars.values() {
            let name: Vec<u16> = name.encode_wide().collect();
            let value: Vec<u16> = value.encode_wide().collect();
            // A name may start with `=` (the per-drive entries do) but not contain one
            // anywhere else: the first `=` after the first character ends the name.
            let equals = u16::from(b'=');
            if name.is_empty() || name.iter().skip(1).any(|&unit| unit == equals) {
                return Err(invalid(
                    "an environment variable name is empty or contains `=`",
                ));
            }
            if name.contains(&0) || value.contains(&0) {
                return Err(invalid("an environment variable contains a NUL"));
            }
            block.extend_from_slice(&name);
            block.push(equals);
            block.extend_from_slice(&value);
            block.push(0);
        }
        if block.is_empty() {
            // An empty block is two terminators, not one.
            block.push(0);
        }
        block.push(0);
        Ok(block)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inherited() -> Vec<(OsString, OsString)> {
        [
            ("Path", r"C:\Windows\System32"),
            ("TERM", "xterm-256color"),
            ("WT_SESSION", "abc"),
            ("=C:", r"C:\work"),
            ("alpha", "1"),
            ("Beta", "2"),
        ]
        .into_iter()
        .map(|(name, value)| (name.into(), value.into()))
        .collect()
    }

    fn entries(block: &[u16]) -> Vec<String> {
        assert_eq!(block.last(), Some(&0), "the block ends with a terminator");
        block[..block.len() - 1]
            .split(|&unit| unit == 0)
            .filter(|entry| !entry.is_empty())
            .map(String::from_utf16_lossy)
            .collect()
    }

    #[test]
    fn the_callers_terminal_variables_are_replaced_by_this_terminals() {
        let env = Environment::for_command(inherited(), &CommandSpec::new("cmd.exe"));
        assert_eq!(env.get("TERM"), None);
        assert_eq!(env.get("WT_SESSION"), None);
        assert_eq!(env.get("COLORTERM"), Some(OsStr::new("truecolor")));
        assert_eq!(env.get("TERM_PROGRAM"), Some(OsStr::new("lume")));
        assert_eq!(env.get("path"), Some(OsStr::new(r"C:\Windows\System32")));
    }

    #[test]
    fn the_commands_own_variables_win_and_names_match_without_regard_to_case() {
        let mut command = CommandSpec::new("cmd.exe")
            .env("PATH", r"D:\bin")
            .env("TERM", "dumb");
        command.env_remove.push("ALPHA".into());
        let env = Environment::for_command(inherited(), &command);
        assert_eq!(env.get("Path"), Some(OsStr::new(r"D:\bin")));
        assert_eq!(env.get("TERM"), Some(OsStr::new("dumb")));
        assert_eq!(env.get("alpha"), None);
        // One entry for the variable, however its name was spelt before.
        let names = entries(&env.block().unwrap());
        assert_eq!(
            names
                .iter()
                .filter(|e| e.to_uppercase().starts_with("PATH="))
                .count(),
            1
        );
    }

    #[test]
    fn the_block_is_sorted_with_drive_entries_first_and_doubly_terminated() {
        let env = Environment::for_command(inherited(), &CommandSpec::new("cmd.exe"));
        let block = env.block().unwrap();
        assert_eq!(block[block.len() - 2..], [0, 0]);
        let names: Vec<String> = entries(&block)
            .into_iter()
            .map(|entry| match entry.strip_prefix('=') {
                Some(rest) => format!("={}", rest.split('=').next().unwrap()),
                None => entry.split('=').next().unwrap().to_owned(),
            })
            .collect();
        assert_eq!(
            names,
            [
                "=C:",
                "alpha",
                "Beta",
                "COLORTERM",
                "Path",
                "TERM_PROGRAM",
                "TERM_PROGRAM_VERSION"
            ]
        );
    }

    #[test]
    fn an_empty_environment_is_two_terminators() {
        let env = Environment {
            vars: BTreeMap::new(),
        };
        assert_eq!(env.block().unwrap(), [0, 0]);
    }

    #[test]
    fn names_and_values_that_cannot_be_encoded_are_refused() {
        for (name, value) in [("A=B", "x"), ("", "x"), ("A\0B", "x"), ("A", "x\0y")] {
            let command = CommandSpec::new("cmd.exe").env(name, value);
            let env = Environment::for_command(Vec::new(), &command);
            assert!(env.block().is_err(), "{name:?}={value:?} was encoded");
        }
    }
}
