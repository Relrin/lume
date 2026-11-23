//! The Rex servers the application can show sessions of.
//!
//! The user writes each server as a `[[rex.host]]` table in `config.toml`: what it is
//! called and where it is reached. A workspace's file has none. Which servers there are
//! is the user's to say, not that of a file that may come with a project.
//!
//! What an address means is not known here. It is held to being a line of text, and
//! whoever reaches servers says whether it names one.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::tuning::{REX_ADDRESS_MAX_CHARS, REX_LABEL_MAX_CHARS};

/// The Rex servers the application can show sessions of.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case")]
pub struct RexConfig {
    /// Look for the Rex server of this account on this machine, when the places that
    /// list servers are on show. One that runs is listed; none is ever started.
    pub local: bool,
    /// Rex servers. Each is a `[[rex.host]]` table with a `label` and an `address`.
    #[serde(rename = "host", skip_serializing_if = "Vec::is_empty")]
    pub hosts: Vec<RexHostEntry>,
}

impl Default for RexConfig {
    fn default() -> Self {
        Self {
            local: true,
            hosts: Vec::new(),
        }
    }
}

/// One `[[rex.host]]` table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct RexHostEntry {
    /// What the server is called in lists. No two servers have the same label, whatever
    /// the case of its letters.
    pub label: String,
    /// Where the server is reached: `https://HOST[:PORT]`, `http://HOST[:PORT]`,
    /// `tailnet://HOST:PORT`, `ssh://[USER@]HOST[:PORT][/SOCKET]`, or a host alone,
    /// which is tried the safest way first.
    pub address: String,
}

/// A line of text a person can have meant: something, not too long, and nothing in it
/// that is not shown.
fn check_line(what: &str, text: &str, most: usize) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err(format!("`{what}` is empty"));
    }
    if text.trim() != text {
        return Err(format!("`{what}` begins or ends with a space"));
    }
    if text.chars().count() > most {
        return Err(format!("`{what}` is longer than {most} characters"));
    }
    if text.chars().any(char::is_control) {
        return Err(format!("`{what}` holds a character that cannot be shown"));
    }
    Ok(())
}

impl RexHostEntry {
    /// What is wrong with the entry on its own, if anything.
    pub fn check(&self) -> Result<(), String> {
        check_line("label", &self.label, REX_LABEL_MAX_CHARS)?;
        check_line("address", &self.address, REX_ADDRESS_MAX_CHARS)
    }

    /// Whether two entries have the same label to a person who reads them.
    pub fn same_label(&self, label: &str) -> bool {
        self.label.to_lowercase() == label.to_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(label: &str, address: &str) -> RexHostEntry {
        RexHostEntry {
            label: label.to_owned(),
            address: address.to_owned(),
        }
    }

    #[test]
    fn a_host_is_two_lines_of_text() {
        assert_eq!(entry("Build box", "https://build").check(), Ok(()));
        for (label, address) in [
            ("", "https://build"),
            ("build", "  "),
            (" build", "https://build"),
            ("build", "https://build\u{1b}[31m"),
            ("build", &"x".repeat(REX_ADDRESS_MAX_CHARS + 1)),
        ] {
            assert!(
                entry(label, address).check().is_err(),
                "{label:?} {address:?}"
            );
        }
        assert!(entry("Build", "a").same_label("bUILD"));
    }
}
