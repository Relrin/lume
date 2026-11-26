//! `cargo xtask release`: what the release workflow asks of the tree before it builds.
//!
//! - `release check --tag v<version>`: the tag names the workspace's version, and the
//!   version is valid.
//! - `release check`: the same for a run that publishes nothing: the version must be
//!   valid.
//!
//! `check` prints `version=`, `channel=` and `tag=` lines, which a workflow step appends
//! to its outputs. A version with a pre-release part (`0.2.0-preview.1`) is published on
//! the `preview` channel, one without on `stable`.

use std::fmt;

use anyhow::{Context, Result, bail};

use crate::paths;

/// Where updates of a version are offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Stable,
    Preview,
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stable => "stable",
            Self::Preview => "preview",
        })
    }
}

/// The channel a version belongs to.
pub fn channel(version: &semver::Version) -> Channel {
    if version.pre.is_empty() {
        Channel::Stable
    } else {
        Channel::Preview
    }
}

/// The product's version: the workspace's, which every crate inherits and the bundler
/// takes for the installers.
pub fn workspace_version() -> Result<semver::Version> {
    let path = paths::root().join("Cargo.toml");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    workspace_version_of(&text)
}

fn workspace_version_of(manifest: &str) -> Result<semver::Version> {
    let value: toml::Table = toml::from_str(manifest).context("Cargo.toml is not valid TOML")?;
    let version = value
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("version"))
        .and_then(|version| version.as_str())
        .context("Cargo.toml has no [workspace.package] version")?;
    let parsed = semver::Version::parse(version)
        .with_context(|| format!("the workspace version {version:?} is not a semantic version"))?;
    if !parsed.build.is_empty() {
        bail!("the workspace version {version} carries build metadata, which installers drop");
    }
    Ok(parsed)
}

/// The version a release tag names: `v` and the version.
fn tag_version(tag: &str) -> Result<semver::Version> {
    let Some(version) = tag.strip_prefix('v') else {
        bail!("the tag {tag:?} does not start with `v`");
    };
    semver::Version::parse(version)
        .with_context(|| format!("the tag {tag:?} does not name a semantic version"))
}

/// What a release of the tree would be, checked against the tag that asks for it.
#[derive(Debug, PartialEq, Eq)]
struct Checked {
    version: semver::Version,
    channel: Channel,
    tag: String,
}

fn check_of(manifest: &str, tag: Option<&str>) -> Result<Checked> {
    let version = workspace_version_of(manifest)?;
    if let Some(tag) = tag {
        let named = tag_version(tag)?;
        if named != version {
            bail!(
                "the tag {tag} names {named}, but the workspace version is {version}; \
                 change `version` in Cargo.toml's [workspace.package] or tag v{version}"
            );
        }
    }
    Ok(Checked {
        channel: channel(&version),
        tag: format!("v{version}"),
        version,
    })
}

pub fn run(args: &[&str]) -> Result<()> {
    let tag = match args {
        ["check"] => None,
        ["check", "--tag", tag] => Some(*tag),
        _ => bail!("usage: cargo xtask release check [--tag v<version>]"),
    };
    let manifest = std::fs::read_to_string(paths::root().join("Cargo.toml"))?;
    let checked = check_of(&manifest, tag)?;
    println!("version={}", checked.version);
    println!("channel={}", checked.channel);
    println!("tag={}", checked.tag);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = "[workspace.package]\nversion = \"0.3.0\"\n";
    const PREVIEW: &str = "[workspace.package]\nversion = \"0.4.0-preview.2\"\n";

    #[test]
    fn a_tag_of_the_workspace_version_is_released_on_stable() {
        let checked = check_of(MANIFEST, Some("v0.3.0")).unwrap();
        assert_eq!(checked.version, semver::Version::new(0, 3, 0));
        assert_eq!(checked.channel, Channel::Stable);
        assert_eq!(checked.tag, "v0.3.0");
    }

    #[test]
    fn a_pre_release_is_released_on_preview() {
        let checked = check_of(PREVIEW, Some("v0.4.0-preview.2")).unwrap();
        assert_eq!(checked.channel, Channel::Preview);
    }

    #[test]
    fn a_tag_of_another_version_is_refused() {
        let error = check_of(MANIFEST, Some("v0.3.1")).unwrap_err();
        assert!(
            error.to_string().contains("workspace version is 0.3.0"),
            "{error}"
        );
    }

    #[test]
    fn a_tag_without_v_or_a_version_is_refused() {
        assert!(check_of(MANIFEST, Some("0.3.0")).is_err());
        assert!(check_of(MANIFEST, Some("vnext")).is_err());
    }

    #[test]
    fn a_run_without_a_tag_checks_the_version_alone() {
        let checked = check_of(MANIFEST, None).unwrap();
        assert_eq!(checked.tag, "v0.3.0");
        assert!(check_of("[workspace.package]\nversion = \"next\"\n", None).is_err());
    }

    #[test]
    fn a_version_with_build_metadata_is_refused() {
        let manifest = "[workspace.package]\nversion = \"0.3.0+abc\"\n";
        assert!(workspace_version_of(manifest).is_err());
    }
}
