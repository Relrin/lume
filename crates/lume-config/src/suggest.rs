//! Finding the key somebody probably meant.

use crate::tuning::SUGGEST_MAX_DISTANCE;

/// How many single-character edits (insert, delete, replace, or swap two neighbours)
/// turn `a` into `b`.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    // Three rows are enough: the one being filled, the one before, and the one before
    // that for a swap.
    let mut before: Vec<usize> = vec![0; b.len() + 1];
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current: Vec<usize> = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        current[0] = i;
        for j in 1..=b.len() {
            let replace = previous[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            let mut best = replace.min(previous[j] + 1).min(current[j - 1] + 1);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(before[j - 2] + 1);
            }
            current[j] = best;
        }
        std::mem::swap(&mut before, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

/// The candidate closest to `key`, if one is close enough to be what was meant. Among
/// equally close candidates the first wins.
pub fn nearest<'a>(key: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let key = key.to_lowercase();
    candidates
        .into_iter()
        .map(|candidate| (edit_distance(&key, &candidate.to_lowercase()), candidate))
        .filter(|(distance, candidate)| {
            // A short key is close to everything; it must share more than its length.
            *distance <= SUGGEST_MAX_DISTANCE && *distance < candidate.chars().count()
        })
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_distance_counts_inserts_deletes_replacements_and_swaps() {
        assert_eq!(edit_distance("size", "size"), 0);
        assert_eq!(edit_distance("size", "sizee"), 1);
        assert_eq!(edit_distance("size", "sze"), 1);
        assert_eq!(edit_distance("size", "sise"), 1);
        assert_eq!(edit_distance("size", "szie"), 1);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn a_misspelt_key_finds_the_one_that_was_meant() {
        let keys = ["family", "fallback", "size", "line-height"];
        assert_eq!(nearest("sizee", keys), Some("size"));
        assert_eq!(nearest("Family", keys), Some("family"));
        assert_eq!(nearest("lineheight", keys), Some("line-height"));
    }

    #[test]
    fn a_key_that_resembles_nothing_gets_no_suggestion() {
        let keys = ["family", "fallback", "size", "line-height"];
        assert_eq!(nearest("colour", keys), None);
        // Two letters are two edits from any two-letter key, which says nothing.
        assert_eq!(nearest("xy", ["ab", "gap"]), None);
    }
}
