//! The fonts installed on this machine that a terminal can use.
//!
//! A webview can use an installed font by name but cannot list the fonts there are, so
//! the list comes from the core.

use tokio::sync::OnceCell;

use crate::error::CommandError;

/// The names of the installed monospace font families, found once and kept.
#[derive(Default)]
pub struct Fonts {
    families: OnceCell<Vec<String>>,
}

impl Fonts {
    /// The families, in alphabetical order. The first call reads every installed font,
    /// which takes a moment, so it runs off the threads that serve commands.
    pub async fn families(&self) -> Result<Vec<String>, CommandError> {
        self.families
            .get_or_try_init(|| async {
                tokio::task::spawn_blocking(monospace_families)
                    .await
                    .map_err(|error| CommandError::internal(error.to_string()))
            })
            .await
            .cloned()
    }
}

fn monospace_families() -> Vec<String> {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    let mut families: Vec<String> = database
        .faces()
        .filter(|face| face.monospaced)
        .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
        .collect();
    families.sort_unstable();
    families.dedup();
    families
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_families_are_sorted_and_listed_once_each() {
        let fonts = Fonts::default();
        let families = fonts.families().await.unwrap();
        let mut expected = families.clone();
        expected.sort_unstable();
        expected.dedup();
        assert_eq!(families, expected);
        // Asked again, the answer is the one that was kept.
        assert_eq!(fonts.families().await.unwrap(), families);
    }
}
