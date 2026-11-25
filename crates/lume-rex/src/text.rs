//! Text a server wrote, made safe to show.
//!
//! A server's labels, titles, versions and reasons are shown beside the application's
//! own words, and among them in its questions. What they may hold is only what is seen:
//! no character that changes the order the text around it is read in, or that is there
//! and not seen, and no pile of marks that covers the lines above and below.

use crate::tuning::{MARKS_MAX, TEXT_MAX_BYTES};

/// Cuts a text to so many bytes, at a character's edge.
pub(crate) fn cut(text: &mut String, max: usize) {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

/// Whether a character changes the direction text is laid out in.
fn turns(char: char) -> bool {
    matches!(char, '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Whether a character takes part in text without being seen, and is not one that
/// scripts and emoji are joined with.
fn unseen(char: char) -> bool {
    matches!(char,
        '\u{ad}' | '\u{34f}'
        // Fillers that stand for a letter and show nothing.
        | '\u{115f}' | '\u{1160}' | '\u{3164}' | '\u{ffa0}'
        | '\u{17b4}' | '\u{17b5}' | '\u{180b}'..='\u{180f}'
        | '\u{200b}' | '\u{2060}'..='\u{2065}' | '\u{206a}'..='\u{206f}'
        | '\u{feff}' | '\u{fff0}'..='\u{fffb}'
        // Musical beams, tags and the selectors of further variants.
        | '\u{1d173}'..='\u{1d17a}' | '\u{e0000}'..='\u{e0fff}')
}

/// What joins or varies the characters beside it, and is nothing by itself.
fn joins(char: char) -> bool {
    matches!(char, '\u{200c}' | '\u{200d}' | '\u{fe00}'..='\u{fe0f}')
}

/// Whether a character is a mark on the one before it, of those that pile up.
fn marks(char: char) -> bool {
    matches!(char,
        '\u{300}'..='\u{36f}' | '\u{483}'..='\u{489}'
        | '\u{e31}' | '\u{e34}'..='\u{e3a}' | '\u{e47}'..='\u{e4e}'
        | '\u{eb1}' | '\u{eb4}'..='\u{ebc}' | '\u{ec8}'..='\u{ece}'
        | '\u{1ab0}'..='\u{1aff}' | '\u{1dc0}'..='\u{1dff}'
        | '\u{20d0}'..='\u{20ff}' | '\u{fe20}'..='\u{fe2f}')
}

/// A text as it may be shown: one line, at most `max` bytes, with nothing in it that is
/// not seen. A text of which nothing would be seen is empty.
pub(crate) fn plain_within(text: &str, max: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max));
    let mut piled = 0;
    let mut seen = false;
    for char in text.chars() {
        let char = match char {
            // A line's end is a space: the words on either side are not one word.
            '\t' | '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}' => ' ',
            char if char.is_control() || turns(char) || unseen(char) => continue,
            char => char,
        };
        let alone = !(marks(char) || joins(char));
        piled = if alone { 0 } else { piled + 1 };
        if piled > MARKS_MAX {
            continue;
        }
        if out.len() + char.len_utf8() > max {
            break;
        }
        seen |= alone && !char.is_whitespace();
        out.push(char);
    }
    if seen { out } else { String::new() }
}

/// A text as it may be shown, within the longest this client takes.
pub(crate) fn plain(text: &str) -> String {
    plain_within(text, TEXT_MAX_BYTES)
}

/// Makes a text one that may be shown.
pub(crate) fn make_plain(text: &mut String) {
    *text = plain(text);
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    /// The rule as it is written down for the core and the page alike.
    #[derive(serde::Deserialize)]
    struct Rule {
        max_bytes: usize,
        marks_max: usize,
        spaces: Vec<(String, String)>,
        dropped: Vec<(String, String)>,
        piled: Vec<(String, String)>,
        blank: Vec<(String, String)>,
        cases: Vec<Case>,
    }

    #[derive(serde::Deserialize)]
    struct Case {
        why: String,
        text: String,
        plain: String,
        max: Option<usize>,
    }

    fn rule() -> Rule {
        let written = include_str!("../../../tests/text/plain.json");
        serde_json::from_str(written).unwrap()
    }

    /// The characters of some ranges.
    fn chars(ranges: &[(String, String)]) -> HashSet<char> {
        let point = |written: &str| u32::from_str_radix(written, 16).unwrap();
        ranges
            .iter()
            .flat_map(|(first, last)| point(first)..=point(last))
            .filter_map(char::from_u32)
            .collect()
    }

    #[test]
    fn a_text_is_what_the_rules_cases_say() {
        let rule = rule();
        assert_eq!(rule.max_bytes, TEXT_MAX_BYTES);
        assert!(rule.cases.len() > 20);
        for case in rule.cases {
            let shown = plain_within(&case.text, case.max.unwrap_or(TEXT_MAX_BYTES));
            assert_eq!(shown, case.plain, "{}: {:?}", case.why, case.text);
        }
    }

    #[test]
    fn each_character_there_is_is_held_to_the_rule() {
        let rule = rule();
        let spaces = chars(&rule.spaces);
        let dropped = chars(&rule.dropped);
        let piled = chars(&rule.piled);
        let blank = chars(&rule.blank);
        let times = MARKS_MAX + 2;
        assert_eq!(rule.marks_max, MARKS_MAX);
        for char in '\0'..=char::MAX {
            let one = char.to_string();
            let within = if spaces.contains(&char) {
                " ".repeat(times)
            } else if dropped.contains(&char) {
                String::new()
            } else if piled.contains(&char) {
                one.repeat(MARKS_MAX)
            } else {
                one.repeat(times)
            };
            let given = format!("a{}b", one.repeat(times));
            assert_eq!(plain(&given), format!("a{within}b"), "{char:?}");
            let unseen = [&spaces, &dropped, &piled, &blank]
                .iter()
                .any(|of| of.contains(&char));
            assert_eq!(plain(&one), if unseen { "" } else { &one }, "{char:?}");
        }
    }

    #[test]
    fn marks_do_not_pile_up() {
        let piled = format!("a{}b", "\u{30d}".repeat(500));
        let shown = plain(&piled);
        assert_eq!(shown.chars().count(), MARKS_MAX + 2);
        assert!(shown.ends_with('b'));
    }

    #[test]
    fn what_is_too_long_is_cut_at_a_characters_edge() {
        assert_eq!(plain(&"é".repeat(TEXT_MAX_BYTES)).len(), TEXT_MAX_BYTES);
        assert_eq!(
            plain(&format!("a{}", "é".repeat(TEXT_MAX_BYTES))).len(),
            TEXT_MAX_BYTES - 1
        );
        assert_eq!(plain_within("日本", 4), "日");
        let mut id = "é".repeat(3);
        cut(&mut id, 5);
        assert_eq!(id, "éé");
    }
}
