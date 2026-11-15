//! How keys and the places they hold are written.
//!
//! A chord is modifiers and one key, joined by `+`: `ctrl+shift+t`. The key is the
//! character the key types without Shift on the keyboard's layout, or a name such as
//! `enter` or `pageup`; Shift is always written out, so `ctrl+shift+1` means the same
//! key whatever it types with Shift held. A sequence is chords separated by spaces,
//! pressed one after another: `ctrl+a c`.
//!
//! Where a binding holds is written as context names joined by `&&`, each of which may
//! be negated with `!`: `terminal && !alt-screen`.

use std::fmt;

use serde::{Serialize, Serializer};

use crate::paths::Platform;
use crate::tuning::KEY_SEQUENCE_MAX;

/// The keys that have names. A key that types a character is written as the character.
const NAMED_KEYS: [&str; 16] = [
    "enter",
    "tab",
    "space",
    "escape",
    "backspace",
    "delete",
    "insert",
    "home",
    "end",
    "pageup",
    "pagedown",
    "up",
    "down",
    "left",
    "right",
    "contextmenu",
];

/// Other spellings of the names above.
const KEY_ALIASES: [(&str, &str); 12] = [
    ("return", "enter"),
    ("esc", "escape"),
    ("del", "delete"),
    ("ins", "insert"),
    ("pgup", "pageup"),
    ("pgdn", "pagedown"),
    ("arrowup", "up"),
    ("arrowdown", "down"),
    ("arrowleft", "left"),
    ("arrowright", "right"),
    ("plus", "+"),
    ("menu", "contextmenu"),
];

/// The highest function key there is a name for.
const FUNCTION_KEY_MAX: u8 = 24;

/// Modifiers and one key, pressed together.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// The Command key of a Mac, the Windows key, the Super key.
    pub logo: bool,
    /// The key: a name, or the one character it types without Shift, in lower case.
    pub key: String,
}

/// The key a name or a character stands for, if it stands for one.
fn key_of(written: &str) -> Option<String> {
    let lower = written.to_lowercase();
    if let Some((_, name)) = KEY_ALIASES.iter().find(|(alias, _)| *alias == lower) {
        return Some((*name).to_owned());
    }
    if NAMED_KEYS.contains(&lower.as_str()) {
        return Some(lower);
    }
    if let Some(number) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok())
        && (1..=FUNCTION_KEY_MAX).contains(&number)
        && lower == format!("f{number}")
    {
        return Some(lower);
    }
    let mut characters = lower.chars();
    match (characters.next(), characters.next()) {
        (Some(only), None) if !only.is_whitespace() && !only.is_control() => Some(lower),
        _ => None,
    }
}

impl Chord {
    /// Reads a chord. The error says what is wrong with it.
    pub fn parse(written: &str) -> Result<Self, String> {
        let mut chord = Self {
            ctrl: false,
            alt: false,
            shift: false,
            logo: false,
            key: String::new(),
        };
        // The key that types `+` is written as itself, after the `+` that joins.
        let (modifiers, key) = match written.strip_suffix('+') {
            Some(rest) if rest.is_empty() || rest.ends_with('+') => {
                (rest.strip_suffix('+').unwrap_or(rest), "+")
            }
            _ => match written.rsplit_once('+') {
                Some((modifiers, key)) => (modifiers, key),
                None => ("", written),
            },
        };
        if !modifiers.is_empty() && modifiers.split('+').any(str::is_empty) {
            return Err(format!("`{written}`: two `+` with nothing between them"));
        }
        for modifier in modifiers.split('+').filter(|part| !part.is_empty()) {
            match modifier.to_lowercase().as_str() {
                "ctrl" | "control" => chord.ctrl = true,
                "alt" | "option" | "opt" => chord.alt = true,
                "shift" => chord.shift = true,
                "super" | "cmd" | "command" | "meta" | "win" => chord.logo = true,
                _ => {
                    return Err(format!(
                        "`{written}`: `{modifier}` is not a modifier (ctrl, alt, shift or super)"
                    ));
                }
            }
        }
        if key.is_empty() {
            return Err(format!("`{written}` has no key"));
        }
        chord.key = match key.to_lowercase().as_str() {
            "ctrl" | "control" | "alt" | "option" | "opt" | "shift" | "super" | "cmd"
            | "command" | "meta" | "win" => {
                return Err(format!(
                    "`{written}` has only modifiers; it needs a key as well"
                ));
            }
            _ => key_of(key).ok_or_else(|| {
                format!(
                    "`{written}`: `{key}` is not a key (one character, or a name such as \
                     enter, tab, space, escape, pageup, left, f5)"
                )
            })?,
        };
        Ok(chord)
    }

    /// Whether the chord is a key alone, or with Shift alone: something a person types.
    pub fn is_typing(&self) -> bool {
        !self.ctrl && !self.alt && !self.logo
    }

    /// The chord as a user of `platform` would write it: the logo key has a different
    /// name on each.
    pub fn display_for(&self, platform: Platform) -> String {
        let logo = match platform {
            Platform::Macos => "cmd",
            Platform::Windows => "win",
            Platform::Linux => "super",
        };
        self.written(logo)
    }

    fn written(&self, logo: &str) -> String {
        let mut written = String::new();
        for (held, name) in [
            (self.ctrl, "ctrl"),
            (self.alt, "alt"),
            (self.shift, "shift"),
            (self.logo, logo),
        ] {
            if held {
                written.push_str(name);
                written.push('+');
            }
        }
        written.push_str(&self.key);
        written
    }
}

impl fmt::Display for Chord {
    /// The chord in its one canonical spelling, the same on every platform.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.written("super"))
    }
}

impl Serialize for Chord {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// One chord, or several pressed one after another.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct KeySequence(pub Vec<Chord>);

impl KeySequence {
    /// Reads chords separated by spaces.
    pub fn parse(written: &str) -> Result<Self, String> {
        let chords = written
            .split_whitespace()
            .map(Chord::parse)
            .collect::<Result<Vec<Chord>, String>>()?;
        if chords.is_empty() {
            return Err("a binding needs a key".to_owned());
        }
        if chords.len() > KEY_SEQUENCE_MAX {
            return Err(format!(
                "`{written}` is {} keys one after another; {KEY_SEQUENCE_MAX} is the most",
                chords.len()
            ));
        }
        Ok(Self(chords))
    }

    /// The chord pressed first.
    pub fn first(&self) -> Option<&Chord> {
        self.0.first()
    }

    /// The sequence as a user of `platform` would write it.
    pub fn display_for(&self, platform: Platform) -> String {
        self.0
            .iter()
            .map(|chord| chord.display_for(platform))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl fmt::Display for KeySequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let chords: Vec<String> = self.0.iter().map(ToString::to_string).collect();
        f.write_str(&chords.join(" "))
    }
}

/// The places a binding can be said to hold in.
pub const CONTEXTS: [&str; 6] = [
    "terminal",
    "alt-screen",
    "floating",
    "palette",
    "search",
    "settings",
];

/// What a context that names a panel begins with: `panel:lume.tabs`.
pub const PANEL_CONTEXT: &str = "panel:";

/// One condition on where a binding holds.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WhenTerm {
    /// The context's name.
    pub name: String,
    /// Whether the binding holds where the context does not.
    pub negated: bool,
}

impl fmt::Display for WhenTerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.negated {
            f.write_str("!")?;
        }
        f.write_str(&self.name)
    }
}

impl Serialize for WhenTerm {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

fn is_context(name: &str) -> bool {
    if CONTEXTS.contains(&name) {
        return true;
    }
    name.strip_prefix(PANEL_CONTEXT).is_some_and(|panel| {
        !panel.is_empty()
            && panel
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    })
}

/// Reads where a binding holds: context names joined by `&&`. The terms come back in
/// one order whatever order they were written in, without repeats.
pub fn parse_when(written: &str) -> Result<Vec<WhenTerm>, String> {
    let mut terms = Vec::new();
    for part in written.split("&&") {
        let part = part.trim();
        let (negated, name) = match part.strip_prefix('!') {
            Some(name) => (true, name.trim()),
            None => (false, part),
        };
        if name.is_empty() {
            return Err(format!("`{written}`: a context is missing"));
        }
        if !is_context(name) {
            return Err(format!(
                "`{written}`: `{name}` is not a context ({}, or panel:<id>)",
                CONTEXTS.join(", ")
            ));
        }
        terms.push(WhenTerm {
            name: name.to_owned(),
            negated,
        });
    }
    terms.sort();
    terms.dedup();
    let contradicts = terms
        .windows(2)
        .any(|pair| pair[0].name == pair[1].name && pair[0].negated != pair[1].negated);
    if contradicts {
        return Err(format!(
            "`{written}` asks for a context and for its absence, which never holds"
        ));
    }
    Ok(terms)
}

/// Writes where a binding holds, as it is read.
pub fn write_when(terms: &[WhenTerm]) -> String {
    terms
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" && ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(written: &str) -> Chord {
        Chord::parse(written).unwrap()
    }

    #[test]
    fn a_chord_is_modifiers_and_a_key_in_any_order_and_case() {
        let parsed = chord("Shift+CTRL+T");
        assert!(parsed.ctrl && parsed.shift && !parsed.alt && !parsed.logo);
        assert_eq!(parsed.key, "t");
        assert_eq!(parsed.to_string(), "ctrl+shift+t");
        assert_eq!(chord("ctrl+shift+t"), parsed);
        assert_eq!(chord("a").to_string(), "a");
        assert_eq!(
            chord("ctrl+alt+shift+super+f12").to_string(),
            "ctrl+alt+shift+super+f12"
        );
    }

    #[test]
    fn the_logo_key_and_alt_have_several_names() {
        for written in ["cmd+d", "super+d", "command+d", "meta+d", "win+d"] {
            assert_eq!(chord(written).to_string(), "super+d", "{written}");
        }
        assert_eq!(chord("option+left").to_string(), "alt+left");
        assert_eq!(chord("cmd+d").display_for(Platform::Macos), "cmd+d");
        assert_eq!(chord("cmd+d").display_for(Platform::Windows), "win+d");
        assert_eq!(chord("cmd+d").display_for(Platform::Linux), "super+d");
    }

    #[test]
    fn keys_have_names_and_other_spellings_of_them() {
        for (written, key) in [
            ("ctrl+Enter", "enter"),
            ("ctrl+return", "enter"),
            ("esc", "escape"),
            ("shift+PageUp", "pageup"),
            ("shift+pgdn", "pagedown"),
            ("ctrl+ArrowLeft", "left"),
            ("ctrl+space", "space"),
            ("ctrl+f5", "f5"),
            ("ctrl+,", ","),
            ("ctrl+=", "="),
            ("ctrl+-", "-"),
            ("ctrl+[", "["),
            ("ctrl+plus", "+"),
            ("ctrl+é", "é"),
        ] {
            assert_eq!(chord(written).key, key, "{written}");
        }
    }

    #[test]
    fn the_key_that_types_a_plus_is_written_after_the_plus_that_joins() {
        assert_eq!(chord("ctrl++").to_string(), "ctrl++");
        assert_eq!(chord("+").key, "+");
        assert_eq!(chord("ctrl+shift++").to_string(), "ctrl+shift++");
    }

    #[test]
    fn what_is_not_a_chord_is_refused_with_the_reason() {
        for (written, reason) in [
            ("ctrl+shift", "only modifiers"),
            ("ctrl+", "has no key"),
            ("", "has no key"),
            ("hyper+t", "`hyper` is not a modifier"),
            ("ctrl+tt", "`tt` is not a key"),
            ("ctrl+f25", "`f25` is not a key"),
            ("ctrl+f01", "`f01` is not a key"),
            ("ctrl++t+", "nothing between"),
        ] {
            let error = Chord::parse(written).unwrap_err();
            assert!(error.contains(reason), "{written:?}: {error}");
        }
    }

    #[test]
    fn a_sequence_is_chords_separated_by_spaces() {
        let sequence = KeySequence::parse("ctrl+a  c").unwrap();
        assert_eq!(sequence.0.len(), 2);
        assert_eq!(sequence.to_string(), "ctrl+a c");
        assert_eq!(sequence.first().unwrap().to_string(), "ctrl+a");
        assert_eq!(
            KeySequence::parse("cmd+k cmd+s")
                .unwrap()
                .display_for(Platform::Macos),
            "cmd+k cmd+s"
        );
        assert!(KeySequence::parse("   ").is_err());
        assert!(
            KeySequence::parse("a b c d e")
                .unwrap_err()
                .contains("the most")
        );
        assert!(KeySequence::parse("ctrl+a nope+b").is_err());
    }

    #[test]
    fn chords_and_sequences_are_sent_as_their_canonical_text() {
        let sequence = KeySequence::parse("CMD+K Ctrl+S").unwrap();
        assert_eq!(
            serde_json::to_value(&sequence).unwrap(),
            serde_json::json!(["super+k", "ctrl+s"])
        );
    }

    #[test]
    fn a_context_is_a_known_name_or_a_panel_and_may_be_negated() {
        let terms = parse_when("terminal && !alt-screen").unwrap();
        assert_eq!(write_when(&terms), "!alt-screen && terminal");
        // Written the other way round, it is the same condition.
        assert_eq!(
            parse_when("!alt-screen&&terminal && terminal").unwrap(),
            terms
        );
        assert_eq!(
            write_when(&parse_when("panel:lume.tabs").unwrap()),
            "panel:lume.tabs"
        );
        assert_eq!(
            serde_json::to_value(&terms).unwrap(),
            serde_json::json!(["!alt-screen", "terminal"])
        );
    }

    #[test]
    fn a_condition_that_names_nothing_known_or_cannot_hold_is_refused() {
        for (written, reason) in [
            ("", "a context is missing"),
            ("terminal &&", "a context is missing"),
            ("editor", "`editor` is not a context"),
            ("panel:", "is not a context"),
            ("terminal && !terminal", "never holds"),
            ("terminal || palette", "is not a context"),
        ] {
            let error = parse_when(written).unwrap_err();
            assert!(error.contains(reason), "{written:?}: {error}");
        }
    }
}
