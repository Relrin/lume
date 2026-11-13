//! `tools.toml`: the pinned versions and checksums of external tools.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::paths;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tools {
    #[serde(rename = "ghostty-vt")]
    pub ghostty_vt: GhosttyVt,
    pub zig: Zig,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct GhosttyVt {
    pub version: String,
    pub base_url: String,
    pub source_sha256: String,
    pub deps_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zig {
    pub version: String,
    pub archives: BTreeMap<String, Archive>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Archive {
    pub url: String,
    pub sha256: String,
}

impl Tools {
    pub fn load() -> Result<Self> {
        let path = paths::root().join("tools.toml");
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }
}

impl GhosttyVt {
    pub fn source_name(&self) -> String {
        format!("libghostty-vt-{}", self.version)
    }

    pub fn deps_name(&self) -> String {
        format!("libghostty-vt-{}-deps", self.version)
    }

    pub fn source_url(&self) -> String {
        format!(
            "{}/{}/{}.tar.gz",
            self.base_url,
            self.version,
            self.source_name()
        )
    }

    pub fn deps_url(&self) -> String {
        format!(
            "{}/{}/{}.tar.gz",
            self.base_url,
            self.version,
            self.deps_name()
        )
    }
}
