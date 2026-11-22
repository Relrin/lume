//! The fonts installed on this machine: those a terminal can use, and all of them, for
//! the interface.
//!
//! A webview can use an installed font by name but cannot list the fonts there are, so
//! the list comes from the core.

use tokio::sync::OnceCell;

use crate::error::CommandError;

/// The names of the installed font families, found once and kept: the monospace ones,
/// and every one.
#[derive(Default)]
pub struct Fonts {
    families: OnceCell<Found>,
}

struct Found {
    monospace: Vec<String>,
    all: Vec<String>,
}

impl Fonts {
    /// The families, in alphabetical order: the monospace ones, or with `all` every
    /// one. The first call reads every installed font, which takes a moment, so it runs
    /// off the threads that serve commands.
    pub async fn families(&self, all: bool) -> Result<Vec<String>, CommandError> {
        let found = self
            .families
            .get_or_try_init(|| async {
                tokio::task::spawn_blocking(installed_families)
                    .await
                    .map_err(|error| CommandError::internal(error.to_string()))
            })
            .await?;
        Ok(if all {
            found.all.clone()
        } else {
            found.monospace.clone()
        })
    }
}

fn installed_families() -> Found {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    let named = |monospace_only: bool| {
        let mut families: Vec<String> = database
            .faces()
            .filter(|face| face.monospaced || !monospace_only)
            .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
            // A family whose name begins with a dot or an at sign is the system's own
            // business: hidden ones, and the sideways forms of others.
            .filter(|name| !name.starts_with(['.', '@']))
            .collect();
        families.sort_unstable();
        families.dedup();
        families
    };
    Found {
        monospace: named(true),
        all: named(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_families_are_sorted_and_listed_once_each() {
        let fonts = Fonts::default();
        let families = fonts.families(false).await.unwrap();
        let mut expected = families.clone();
        expected.sort_unstable();
        expected.dedup();
        assert_eq!(families, expected);
        // Asked again, the answer is the one that was kept.
        assert_eq!(fonts.families(false).await.unwrap(), families);
    }

    #[tokio::test]
    async fn every_family_holds_the_monospace_ones_and_none_of_the_systems_hidden() {
        let fonts = Fonts::default();
        let all = fonts.families(true).await.unwrap();
        let mut expected = all.clone();
        expected.sort_unstable();
        expected.dedup();
        assert_eq!(all, expected);
        for family in fonts.families(false).await.unwrap() {
            assert!(all.contains(&family), "{family}");
        }
        assert!(all.iter().all(|name| !name.starts_with(['.', '@'])));
    }
}
