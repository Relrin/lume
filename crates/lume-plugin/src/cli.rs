//! Verbs a plugin adds to the command line: `lume <words> [arguments]`.
//!
//! A verb is one to three words. The first must not be a word the command line has a
//! meaning of its own for; the list of those is here, so that the manifest check and
//! the command line's own reading agree.

/// The first words the command line keeps for itself.
pub const RESERVED_WORDS: [&str; 8] = [
    "help",
    "ctl",
    "host",
    "plugin",
    "config",
    "keymap",
    "theme",
    "shell-integration",
];

/// The most words a verb has.
pub const VERB_WORDS_MAX: usize = 3;

/// The longest a verb's word is.
pub const VERB_WORD_MAX_CHARS: usize = 32;

/// Whether a word can be part of a verb: lower-case letters, digits and hyphens,
/// beginning with a letter.
pub fn word_ok(word: &str) -> bool {
    let mut characters = word.chars();
    word.len() <= VERB_WORD_MAX_CHARS
        && characters
            .next()
            .is_some_and(|first| first.is_ascii_lowercase())
        && characters.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Whether the command line keeps a word for itself.
pub fn is_reserved(word: &str) -> bool {
    RESERVED_WORDS.contains(&word)
}

/// The words of a verb as a manifest writes it, or what is wrong with it.
pub fn parse_verb(written: &str) -> Result<Vec<String>, String> {
    let words: Vec<&str> = written.split(' ').collect();
    if words.len() > VERB_WORDS_MAX {
        return Err(format!("`{written}` has more than {VERB_WORDS_MAX} words"));
    }
    if let Some(word) = words.iter().find(|word| !word_ok(word)) {
        return Err(format!(
            "`{written}` is not a verb: one to {VERB_WORDS_MAX} words with one space between \
             them, each of lower-case letters, digits and hyphens, beginning with a letter \
             (`{word}` is not)"
        ));
    }
    if is_reserved(words[0]) {
        return Err(format!(
            "`{written}`: `{}` is one of lume's own words; they are {}",
            words[0],
            RESERVED_WORDS.join(", ")
        ));
    }
    Ok(words.into_iter().map(str::to_owned).collect())
}

/// The verb that `words` begin with: of the verbs given, the one with the most words
/// that are all there, in order. Returns its place in `verbs`.
pub fn longest_match<'a, I>(verbs: I, words: &[String]) -> Option<usize>
where
    I: IntoIterator<Item = &'a [String]>,
{
    verbs
        .into_iter()
        .enumerate()
        .filter(|(_, verb)| !verb.is_empty() && words.starts_with(verb))
        .max_by_key(|(_, verb)| verb.len())
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(written: &[&str]) -> Vec<String> {
        written.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn a_verb_is_one_to_three_plain_words() {
        assert_eq!(parse_verb("review"), Ok(words(&["review"])));
        assert_eq!(parse_verb("agent run"), Ok(words(&["agent", "run"])));
        assert_eq!(parse_verb("a b-2 c"), Ok(words(&["a", "b-2", "c"])));
        for bad in [
            "",
            "Agent",
            "agent  run",
            " agent",
            "a b c d",
            "2go",
            "--x",
            "a_b",
        ] {
            assert!(parse_verb(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_first_word_is_not_one_of_the_command_lines_own() {
        for word in RESERVED_WORDS {
            assert!(parse_verb(word).is_err(), "{word}");
            assert!(parse_verb(&format!("{word} x")).is_err(), "{word}");
            assert!(parse_verb(&format!("x {word}")).is_ok(), "{word}");
        }
    }

    #[test]
    fn the_verb_with_the_most_words_is_the_one_meant() {
        let verbs = [
            words(&["agent"]),
            words(&["agent", "run"]),
            words(&["review"]),
        ];
        let find = |given: &[&str]| longest_match(verbs.iter().map(Vec::as_slice), &words(given));
        assert_eq!(find(&["agent", "run", "--profile", "x"]), Some(1));
        assert_eq!(find(&["agent", "list"]), Some(0));
        assert_eq!(find(&["review"]), Some(2));
        assert_eq!(find(&["run", "agent"]), None);
        assert_eq!(find(&[]), None);
    }
}
