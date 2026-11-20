//! The keys that are bound: the defaults of each platform with the user's laid over.
//!
//! The defaults are tables in this file, one for a Mac and one for Windows and Linux,
//! so that every platform's keys can be listed and tested on any of them. On a Mac the
//! application's keys are on Command, which no program in a terminal can receive anyway.
//! Elsewhere they are on Ctrl+Shift, so that Ctrl alone stays with the shell.
//!
//! The user's bindings come from `[keybindings]`, then from the table of their
//! platform. A binding replaces a lower one on the same key in the same context, and
//! `"unbound"` takes every lower binding of a key away.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value, json};

use crate::keys::{Chord, KeySequence, WhenTerm, parse_when};
use crate::paths::Platform;
use crate::schema::{BindingEntry, BindingValue, Config, UNBOUND};
use crate::source::{Problem, ProblemKind, Source, join_path};

/// A key bound to a command.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Binding {
    pub keys: KeySequence,
    /// The id of the command the keys run.
    pub command: String,
    /// What the command is given. Null when it is given nothing.
    pub args: Value,
    /// Where the binding holds. Empty is everywhere.
    pub when: Vec<WhenTerm>,
    /// The layer the binding came from.
    pub source: Source,
}

/// A default binding that the user took away.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Removed {
    pub binding: Binding,
    /// What took it away.
    pub by: Source,
}

/// The keys in effect.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Keymap {
    /// The bindings, lowest first: of two that match a key equally well, the later
    /// one wins.
    pub bindings: Vec<Binding>,
    /// The bindings that were unbound or replaced.
    pub removed: Vec<Removed>,
    /// What was wrong with the user's bindings. Such a binding is left out.
    #[serde(skip)]
    pub problems: Vec<Problem>,
}

/// One row of a table of defaults: the keys, the command, where it holds.
type Row = (&'static str, &'static str, &'static str);

/// Where the keys that scroll back hold: in a terminal whose program has not taken
/// the whole screen, since such a program scrolls itself.
const SCROLLBACK: &str = "terminal && !alt-screen";

const TERMINAL: &str = "terminal";

/// Where the keyboard is in the bar that searches a terminal. The key that opens the
/// bar holds there too, and goes back to what was typed.
const SEARCH: &str = "search";

/// The defaults on Windows and Linux.
const DEFAULTS: &[Row] = &[
    ("ctrl+shift+c", "lume.terminal.copy", TERMINAL),
    ("ctrl+insert", "lume.terminal.copy", TERMINAL),
    ("ctrl+shift+v", "lume.terminal.paste", TERMINAL),
    ("shift+insert", "lume.terminal.paste", TERMINAL),
    ("ctrl+shift+k", "lume.terminal.clear", TERMINAL),
    ("ctrl+=", "lume.view.zoom-in", ""),
    ("ctrl+shift+=", "lume.view.zoom-in", ""),
    ("ctrl+-", "lume.view.zoom-out", ""),
    ("ctrl+0", "lume.view.zoom-reset", ""),
    ("shift+pageup", "lume.terminal.scroll-page-up", SCROLLBACK),
    (
        "shift+pagedown",
        "lume.terminal.scroll-page-down",
        SCROLLBACK,
    ),
    ("shift+home", "lume.terminal.scroll-top", SCROLLBACK),
    ("shift+end", "lume.terminal.scroll-bottom", SCROLLBACK),
    ("ctrl+shift+t", "lume.tab.new", ""),
    ("ctrl+shift+r", "lume.tab.rename-start", ""),
    ("ctrl+shift+w", "lume.pane.close", ""),
    ("ctrl+tab", "lume.tab.next", ""),
    ("ctrl+shift+tab", "lume.tab.previous", ""),
    ("ctrl+shift+e", "lume.pane.split-right", ""),
    ("ctrl+shift+o", "lume.pane.split-down", ""),
    ("ctrl+shift+left", "lume.pane.focus-left", ""),
    ("ctrl+shift+right", "lume.pane.focus-right", ""),
    ("ctrl+shift+up", "lume.pane.focus-up", ""),
    ("ctrl+shift+down", "lume.pane.focus-down", ""),
    ("ctrl+shift+enter", "lume.pane.zoom", ""),
    ("ctrl+shift+g", "lume.pane.float", ""),
    ("ctrl+shift+space", "lume.pane.scratch", ""),
    ("ctrl+shift+p", "lume.palette.open", ""),
    ("ctrl+shift+l", "lume.connect.open", ""),
    ("ctrl+shift+f", "lume.search.open", TERMINAL),
    ("ctrl+shift+f", "lume.search.open", SEARCH),
    ("ctrl+shift+,", "lume.settings.open", ""),
    ("ctrl+shift+n", "lume.window.new", ""),
    ("ctrl+shift+b", "lume.panel.toggle", ""),
];

/// The defaults on a Mac.
const MAC_DEFAULTS: &[Row] = &[
    ("cmd+c", "lume.terminal.copy", TERMINAL),
    ("cmd+v", "lume.terminal.paste", TERMINAL),
    ("cmd+k", "lume.terminal.clear", TERMINAL),
    ("cmd+a", "lume.terminal.select-all", TERMINAL),
    ("cmd+=", "lume.view.zoom-in", ""),
    ("cmd+shift+=", "lume.view.zoom-in", ""),
    ("cmd+-", "lume.view.zoom-out", ""),
    ("cmd+0", "lume.view.zoom-reset", ""),
    ("shift+pageup", "lume.terminal.scroll-page-up", SCROLLBACK),
    (
        "shift+pagedown",
        "lume.terminal.scroll-page-down",
        SCROLLBACK,
    ),
    ("shift+home", "lume.terminal.scroll-top", SCROLLBACK),
    ("shift+end", "lume.terminal.scroll-bottom", SCROLLBACK),
    ("cmd+pageup", "lume.terminal.scroll-page-up", SCROLLBACK),
    ("cmd+pagedown", "lume.terminal.scroll-page-down", SCROLLBACK),
    ("cmd+home", "lume.terminal.scroll-top", SCROLLBACK),
    ("cmd+end", "lume.terminal.scroll-bottom", SCROLLBACK),
    ("cmd+t", "lume.tab.new", ""),
    ("cmd+shift+r", "lume.tab.rename-start", ""),
    ("cmd+w", "lume.pane.close", ""),
    ("ctrl+tab", "lume.tab.next", ""),
    ("ctrl+shift+tab", "lume.tab.previous", ""),
    ("cmd+shift+]", "lume.tab.next", ""),
    ("cmd+shift+[", "lume.tab.previous", ""),
    ("cmd+d", "lume.pane.split-right", ""),
    ("cmd+shift+d", "lume.pane.split-down", ""),
    ("cmd+alt+left", "lume.pane.focus-left", ""),
    ("cmd+alt+right", "lume.pane.focus-right", ""),
    ("cmd+alt+up", "lume.pane.focus-up", ""),
    ("cmd+alt+down", "lume.pane.focus-down", ""),
    ("cmd+shift+enter", "lume.pane.zoom", ""),
    ("cmd+shift+g", "lume.pane.float", ""),
    ("cmd+shift+space", "lume.pane.scratch", ""),
    ("cmd+shift+p", "lume.palette.open", ""),
    ("cmd+shift+t", "lume.connect.open", ""),
    ("cmd+f", "lume.search.open", TERMINAL),
    ("cmd+f", "lume.search.open", SEARCH),
    ("cmd+,", "lume.settings.open", ""),
    ("cmd+n", "lume.window.new", ""),
    ("cmd+o", "lume.workspace.open-folder", ""),
    ("cmd+shift+b", "lume.panel.toggle", ""),
];

/// The command that shows the tab with a number, and the modifiers its keys have: the
/// digit is added to them.
const TAB_COMMAND: &str = "lume.tab.activate";
const TAB_MODIFIERS: &str = "ctrl+alt";
const MAC_TAB_MODIFIERS: &str = "cmd";

/// How many tabs have a key of their own.
const TAB_KEYS: u32 = 9;

/// The command that shows or hides a dock area, and the area its default key is for.
const PANEL_COMMAND: &str = "lume.panel.toggle";
const PANEL_AREA: &str = "left";

/// The keys a shell or an editor in a terminal expects to get, and so no default may
/// take: a key alone or with Shift, anything with Ctrl and no Shift, anything with
/// Alt, and Ctrl+Alt, which is how many keyboards type their third level of characters.
pub fn is_shell_key(chord: &Chord) -> bool {
    if chord.logo {
        return false;
    }
    chord.is_typing() || chord.alt || !chord.shift
}

/// The defaults that take a key the rule above keeps for programs, each with the
/// reason it is taken all the same.
pub const SHELL_KEY_EXCEPTIONS: &[(&str, &str)] = &[
    (
        "ctrl+=",
        "the font size; terminals have used these three keys for it for a long time, and \
         programs do not",
    ),
    ("ctrl+-", "the font size"),
    ("ctrl+0", "the font size"),
    (
        "shift+pageup",
        "scrolling back; it holds only while no full-screen program has the terminal",
    ),
    ("shift+pagedown", "scrolling back"),
    ("shift+home", "scrolling back"),
    ("shift+end", "scrolling back"),
    (
        "ctrl+insert",
        "copy, as terminals had it before Ctrl+Shift+C; no shell binds it",
    ),
    ("shift+insert", "paste, in the same way"),
    (
        "ctrl+tab",
        "the next tab; a program receives the same bytes for it as for Tab, so none relies on it",
    ),
    (
        "ctrl+alt+1",
        "tabs by number; the key that types a third-level character is told apart from \
         Ctrl and Alt held together",
    ),
    ("ctrl+alt+2", "tabs by number"),
    ("ctrl+alt+3", "tabs by number"),
    ("ctrl+alt+4", "tabs by number"),
    ("ctrl+alt+5", "tabs by number"),
    ("ctrl+alt+6", "tabs by number"),
    ("ctrl+alt+7", "tabs by number"),
    ("ctrl+alt+8", "tabs by number"),
    ("ctrl+alt+9", "tabs by number"),
];

fn default_binding(keys: &str, command: &str, when: &str, args: Value) -> Option<Binding> {
    let when = if when.is_empty() {
        Vec::new()
    } else {
        parse_when(when).ok()?
    };
    Some(Binding {
        keys: KeySequence::parse(keys).ok()?,
        command: command.to_owned(),
        args,
        when,
        source: Source::Default,
    })
}

/// The bindings a platform has before the user changes any.
pub fn default_bindings(platform: Platform) -> Vec<Binding> {
    let (rows, tab_modifiers) = match platform {
        Platform::Macos => (MAC_DEFAULTS, MAC_TAB_MODIFIERS),
        Platform::Windows | Platform::Linux => (DEFAULTS, TAB_MODIFIERS),
    };
    // A row that could not be read would be a mistake in the tables above; a test
    // holds every row to being read.
    let mut bindings: Vec<Binding> = rows
        .iter()
        .filter_map(|(keys, command, when)| {
            let args = if *command == PANEL_COMMAND {
                json!({ "area": PANEL_AREA })
            } else {
                Value::Null
            };
            default_binding(keys, command, when, args)
        })
        .collect();
    for number in 1..=TAB_KEYS {
        let keys = format!("{tab_modifiers}+{number}");
        bindings.extend(default_binding(
            &keys,
            TAB_COMMAND,
            "",
            json!({ "index": number }),
        ));
    }
    bindings
}

/// How many bindings the tables above hold for a platform.
#[cfg(test)]
fn default_count(platform: Platform) -> usize {
    let rows = match platform {
        Platform::Macos => MAC_DEFAULTS.len(),
        Platform::Windows | Platform::Linux => DEFAULTS.len(),
    };
    rows + TAB_KEYS as usize
}

/// One binding as the user wrote it, read.
struct Written {
    command: String,
    when: Vec<WhenTerm>,
    args: Value,
}

fn read_entry(entry: &BindingEntry) -> Result<Written, String> {
    let when = match entry.when.as_deref() {
        None => Vec::new(),
        Some(when) => parse_when(when)?,
    };
    if entry.command.trim().is_empty() {
        return Err("a binding needs a command".to_owned());
    }
    Ok(Written {
        command: entry.command.trim().to_owned(),
        when,
        args: entry.args.clone().unwrap_or(Value::Null),
    })
}

fn read_value(value: &BindingValue) -> Result<Vec<Written>, String> {
    let command = |command: &str| BindingEntry {
        command: command.to_owned(),
        when: None,
        args: None,
    };
    match value {
        BindingValue::Command(id) => Ok(vec![read_entry(&command(id))?]),
        BindingValue::Entry(entry) => Ok(vec![read_entry(entry)?]),
        BindingValue::Entries(entries) => entries.iter().map(read_entry).collect(),
    }
}

/// The defaults of `platform` with the user's bindings laid over. `sources` says where
/// each of the user's bindings was written.
pub fn resolve_keymap(
    config: &Config,
    sources: &BTreeMap<String, Source>,
    platform: Platform,
) -> Keymap {
    resolve_keymap_with(config, sources, platform, &[]).0
}

/// A key a plugin binds by default, as its manifest gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginBinding {
    /// The plugin, as installed.
    pub plugin: String,
    /// The keys, and the keys on a Mac where they differ.
    pub keys: String,
    pub macos: Option<String>,
    pub command: String,
    pub args: Value,
    pub when: Option<String>,
}

/// A plugin's binding that was left out, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dropped {
    pub plugin: String,
    pub message: String,
}

/// Whether a chord is one a plugin's binding may begin with: with a modifier other than
/// Shift alone, or a function key, and not one a shell or an editor expects.
fn fit_for_plugin(chord: &Chord) -> bool {
    let function = chord
        .key
        .strip_prefix('f')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (1..=24).contains(&number));
    let modified = chord.ctrl || chord.alt || chord.logo;
    (modified || function) && (!is_shell_key(chord) || function)
}

/// Whether one sequence of keys is the other or begins it.
fn shares_keys(one: &KeySequence, other: &KeySequence) -> bool {
    let (one, other) = (&one.0, &other.0);
    let shorter = one.len().min(other.len());
    one[..shorter] == other[..shorter]
}

/// The defaults of `platform`, the plugins' defaults over them, and the user's bindings
/// over both. A plugin's binding is left out when its keys are, or begin, or are begun
/// by, keys the application or the user binds; when they have no modifier and are no
/// function key; and when a shell or an editor expects them. The second value says
/// which were left out, and why.
pub fn resolve_keymap_with(
    config: &Config,
    sources: &BTreeMap<String, Source>,
    platform: Platform,
    plugins: &[PluginBinding],
) -> (Keymap, Vec<Dropped>) {
    let mut bindings = default_bindings(platform);
    let mut removed = Vec::new();
    let mut problems = Vec::new();
    let mut dropped = Vec::new();

    // The keys the user binds, on this platform, for the plugins' to keep clear of.
    let mut users: Vec<KeySequence> = config
        .keybindings
        .all
        .keys()
        .chain(config.keybindings.on(platform).keys())
        .filter_map(|written| KeySequence::parse(written).ok())
        .collect();
    users.dedup();
    let defaults: Vec<KeySequence> = bindings
        .iter()
        .map(|binding| binding.keys.clone())
        .collect();
    let mut taken: Vec<KeySequence> = Vec::new();
    for plugin in plugins {
        let written = match (&plugin.macos, platform) {
            (Some(macos), Platform::Macos) => macos,
            _ => &plugin.keys,
        };
        let drop = |message: String, dropped: &mut Vec<Dropped>| {
            dropped.push(Dropped {
                plugin: plugin.plugin.clone(),
                message,
            });
        };
        let keys = match KeySequence::parse(written) {
            Ok(keys) => keys,
            Err(problem) => {
                drop(
                    format!("the key {written} is not bound: {problem}"),
                    &mut dropped,
                );
                continue;
            }
        };
        let when = match plugin.when.as_deref().map(parse_when).transpose() {
            Ok(when) => when.unwrap_or_default(),
            Err(problem) => {
                drop(
                    format!("the key {written} is not bound: {problem}"),
                    &mut dropped,
                );
                continue;
            }
        };
        if !keys.first().is_some_and(fit_for_plugin) {
            drop(
                format!(
                    "the key {written} is not bound: a plugin's keys begin with Ctrl+Shift, \
                     the logo key or a function key; programs in a terminal expect the rest"
                ),
                &mut dropped,
            );
            continue;
        }
        if let Some(clash) = defaults.iter().find(|other| shares_keys(&keys, other)) {
            drop(
                format!("the key {written} is not bound: Lume binds {clash}"),
                &mut dropped,
            );
            continue;
        }
        if let Some(clash) = users.iter().find(|other| shares_keys(&keys, other)) {
            drop(
                format!("the key {written} is not bound: you bind {clash}"),
                &mut dropped,
            );
            continue;
        }
        if let Some(clash) = taken.iter().find(|other| shares_keys(&keys, other)) {
            drop(
                format!("the key {written} is not bound: a plugin before it binds {clash}"),
                &mut dropped,
            );
            continue;
        }
        taken.push(keys.clone());
        bindings.push(Binding {
            keys,
            command: plugin.command.clone(),
            args: plugin.args.clone(),
            when,
            source: Source::Plugin {
                id: plugin.plugin.clone(),
            },
        });
    }

    // Everywhere first, then this platform's own. The other platforms' tables change
    // nothing here, but what is wrong in them is said on every platform.
    let mut tables: Vec<(Vec<&str>, &BTreeMap<String, BindingValue>, bool)> =
        vec![(vec!["keybindings"], &config.keybindings.all, true)];
    for other in Platform::ALL {
        tables.push((
            vec!["keybindings", other.as_str()],
            config.keybindings.on(other),
            other == platform,
        ));
    }

    for (prefix, table, applies) in tables {
        for (written, value) in table {
            let mut path: Vec<&str> = prefix.clone();
            path.push(written);
            let path = join_path(&path);
            let source = sources.get(&path).cloned().unwrap_or(Source::Default);
            let read = KeySequence::parse(written).and_then(|keys| Ok((keys, read_value(value)?)));
            let (keys, entries) = match read {
                Ok(read) => read,
                Err(message) => {
                    let location = source.location();
                    problems.push(Problem {
                        kind: ProblemKind::InvalidValue,
                        path,
                        file: location.map(|(file, _)| file.clone()),
                        line: location.map_or(0, |(_, line)| line),
                        column: 0,
                        message,
                        suggestion: None,
                    });
                    continue;
                }
            };
            if !applies {
                continue;
            }
            for entry in entries {
                // A binding gives way to a later one on its key in its context, and
                // to `unbound` on its key in any context when none is named.
                let unbinds = entry.command == UNBOUND;
                let everywhere = unbinds && entry.when.is_empty();
                let mut kept = Vec::with_capacity(bindings.len());
                for binding in bindings {
                    if binding.keys == keys && (everywhere || binding.when == entry.when) {
                        removed.push(Removed {
                            binding,
                            by: source.clone(),
                        });
                    } else {
                        kept.push(binding);
                    }
                }
                bindings = kept;
                if !unbinds {
                    bindings.push(Binding {
                        keys: keys.clone(),
                        command: entry.command,
                        args: entry.args,
                        when: entry.when,
                        source: source.clone(),
                    });
                }
            }
        }
    }
    (
        Keymap {
            bindings,
            removed,
            problems,
        },
        dropped,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::PathBuf;

    use super::*;
    use crate::keys::write_when;
    use crate::layers::{LayerInput, LayerKind, resolve_layers};

    fn keymap(text: &str, platform: Platform) -> Keymap {
        let layered = resolve_layers(&[LayerInput {
            kind: LayerKind::User,
            file: PathBuf::from("config.toml"),
            text: text.to_owned(),
        }]);
        assert_eq!(layered.problems, vec![], "{text}");
        resolve_keymap(&layered.config, &layered.sources, platform)
    }

    fn bound<'a>(keymap: &'a Keymap, keys: &str) -> Vec<&'a Binding> {
        let keys = KeySequence::parse(keys).unwrap();
        keymap
            .bindings
            .iter()
            .filter(|binding| binding.keys == keys)
            .collect()
    }

    #[test]
    fn every_row_of_the_default_tables_is_a_binding() {
        for platform in Platform::ALL {
            let bindings = default_bindings(platform);
            assert_eq!(bindings.len(), default_count(platform), "{platform:?}");
            for binding in &bindings {
                assert!(binding.command.starts_with("lume."), "{}", binding.command);
                assert_eq!(binding.source, Source::Default);
            }
        }
        assert_eq!(
            default_bindings(Platform::Windows),
            default_bindings(Platform::Linux)
        );
    }

    #[test]
    fn no_default_takes_a_key_that_shells_and_editors_need() {
        let excepted: HashSet<&str> = SHELL_KEY_EXCEPTIONS.iter().map(|(key, _)| *key).collect();
        let mut used = HashSet::new();
        for platform in Platform::ALL {
            for binding in default_bindings(platform) {
                let first = binding.keys.first().unwrap();
                if !is_shell_key(first) {
                    continue;
                }
                let written = first.to_string();
                assert!(
                    excepted.contains(written.as_str()),
                    "{platform:?}: {written} is a key programs in a terminal use"
                );
                used.insert(written);
            }
        }
        // An exception nothing uses is a key given away for nothing.
        for (key, reason) in SHELL_KEY_EXCEPTIONS {
            assert!(used.contains(*key), "{key} is excepted and not bound");
            assert!(!reason.is_empty());
            assert_eq!(Chord::parse(key).unwrap().to_string(), *key);
        }
    }

    #[test]
    fn the_rule_keeps_for_programs_what_programs_use() {
        let shell = |written: &str| is_shell_key(&Chord::parse(written).unwrap());
        for key in [
            "a",
            "shift+a",
            "enter",
            "tab",
            "shift+tab",
            "escape",
            "f5",
            "shift+f5",
            "up",
            "ctrl+c",
            "ctrl+space",
            "ctrl+left",
            "ctrl+[",
            "alt+b",
            "alt+shift+f",
            "alt+enter",
            "ctrl+alt+q",
        ] {
            assert!(shell(key), "{key}");
        }
        for key in [
            "ctrl+shift+c",
            "ctrl+shift+left",
            "cmd+c",
            "cmd+shift+p",
            "cmd+alt+left",
        ] {
            assert!(!shell(key), "{key}");
        }
    }

    #[test]
    fn windows_and_linux_leave_the_logo_key_to_the_desktop_and_a_mac_uses_it() {
        for platform in [Platform::Windows, Platform::Linux] {
            for binding in default_bindings(platform) {
                assert!(
                    binding.keys.0.iter().all(|chord| !chord.logo),
                    "{}",
                    binding.keys
                );
            }
        }
        let mac = default_bindings(Platform::Macos);
        assert!(mac.iter().filter(|binding| binding.keys.0[0].logo).count() > 20);
    }

    #[test]
    fn no_two_defaults_share_a_key_in_the_same_place() {
        for platform in Platform::ALL {
            let mut seen = HashSet::new();
            for binding in default_bindings(platform) {
                let place = (binding.keys.to_string(), write_when(&binding.when));
                assert!(seen.insert(place.clone()), "{platform:?}: {place:?} twice");
            }
            // A key bound everywhere is not also bound somewhere in particular: the
            // one in particular would always win, and the other never run there.
            let everywhere: HashSet<String> = seen
                .iter()
                .filter(|(_, when)| when.is_empty())
                .map(|(keys, _)| keys.clone())
                .collect();
            for (keys, when) in &seen {
                assert!(
                    when.is_empty() || !everywhere.contains(keys),
                    "{platform:?}: {keys}"
                );
            }
        }
    }

    #[test]
    fn copy_and_paste_are_where_the_shell_does_not_look() {
        let windows = default_bindings(Platform::Windows);
        let command = |keys: &str| {
            let keys = KeySequence::parse(keys).unwrap();
            windows
                .iter()
                .find(|binding| binding.keys == keys)
                .map(|binding| binding.command.as_str())
        };
        assert_eq!(command("ctrl+shift+c"), Some("lume.terminal.copy"));
        assert_eq!(command("ctrl+insert"), Some("lume.terminal.copy"));
        assert_eq!(command("ctrl+shift+v"), Some("lume.terminal.paste"));
        assert_eq!(command("shift+insert"), Some("lume.terminal.paste"));
        assert_eq!(command("ctrl+c"), None);
        assert_eq!(command("ctrl+v"), None);
        assert_eq!(command("ctrl+alt+3"), Some("lume.tab.activate"));

        let mac = keymap("", Platform::Macos);
        assert_eq!(bound(&mac, "cmd+c")[0].command, "lume.terminal.copy");
        assert_eq!(bound(&mac, "cmd+3")[0].args, json!({ "index": 3 }));
        assert_eq!(
            write_when(&bound(&mac, "shift+pageup")[0].when),
            "!alt-screen && terminal"
        );
    }

    #[test]
    fn the_users_binding_replaces_the_default_on_its_key_and_says_where_it_is_from() {
        let text = "[keybindings]\n\"Ctrl+Shift+T\" = \"lume.window.new\"\n\"ctrl+a c\" = \"lume.tab.new\"\n";
        let keymap = keymap(text, Platform::Linux);
        assert_eq!(keymap.problems, vec![]);

        let tab = bound(&keymap, "ctrl+shift+t");
        assert_eq!(tab.len(), 1);
        assert_eq!(tab[0].command, "lume.window.new");
        assert!(matches!(tab[0].source, Source::User { line: 2, .. }));
        assert_eq!(bound(&keymap, "ctrl+a c")[0].command, "lume.tab.new");

        assert_eq!(keymap.removed.len(), 1);
        assert_eq!(keymap.removed[0].binding.command, "lume.tab.new");
        assert!(matches!(keymap.removed[0].by, Source::User { line: 2, .. }));
        // The user's bindings come after the defaults, so they win a tie.
        let last_two = &keymap.bindings[keymap.bindings.len() - 2..];
        assert!(
            last_two
                .iter()
                .all(|binding| matches!(binding.source, Source::User { .. }))
        );
    }

    #[test]
    fn unbound_takes_a_default_away() {
        let keymap = keymap(
            "[keybindings]\n\"ctrl+shift+k\" = \"unbound\"\n\"shift+pageup\" = \"unbound\"\n",
            Platform::Windows,
        );
        assert_eq!(bound(&keymap, "ctrl+shift+k").len(), 0);
        // Whatever context the default had.
        assert_eq!(bound(&keymap, "shift+pageup").len(), 0);
        assert_eq!(keymap.removed.len(), 2);
        assert_eq!(keymap.bindings.len(), default_count(Platform::Windows) - 2);
    }

    #[test]
    fn a_key_can_do_different_things_in_different_places() {
        let text = "\
[keybindings]\n\
\"ctrl+shift+k\" = [\n  { command = \"lume.terminal.clear\", when = \"terminal\" },\n  \
{ command = \"lume.palette.open\", when = \"settings\" },\n]\n\
\"ctrl+shift+p\" = { command = \"lume.tab.activate\", args = { index = 2 }, when = \"floating\" }\n";
        let keymap = keymap(text, Platform::Windows);
        assert_eq!(keymap.problems, vec![]);
        let clear = bound(&keymap, "ctrl+shift+k");
        assert_eq!(clear.len(), 2);
        assert_eq!(clear[0].command, "lume.terminal.clear");
        assert_eq!(write_when(&clear[1].when), "settings");
        // The default in the terminal was replaced by the same binding of the user's.
        assert!(matches!(clear[0].source, Source::User { .. }));

        // In another context the default stays beside the user's.
        let palette = bound(&keymap, "ctrl+shift+p");
        assert_eq!(palette.len(), 2);
        assert_eq!(palette[0].source, Source::Default);
        assert_eq!(palette[1].args, json!({ "index": 2 }));
    }

    #[test]
    fn a_platforms_own_table_holds_on_that_platform_only_and_goes_last() {
        let text = "\
[keybindings]\n\"ctrl+shift+e\" = \"lume.window.new\"\n\
[keybindings.linux]\n\"ctrl+shift+e\" = \"lume.tab.new\"\n\
[keybindings.macos]\n\"cmd+d\" = \"unbound\"\n";
        let linux = keymap(text, Platform::Linux);
        assert_eq!(bound(&linux, "ctrl+shift+e")[0].command, "lume.tab.new");
        let windows = keymap(text, Platform::Windows);
        assert_eq!(
            bound(&windows, "ctrl+shift+e")[0].command,
            "lume.window.new"
        );
        let mac = keymap(text, Platform::Macos);
        assert_eq!(bound(&mac, "cmd+d").len(), 0);
        assert_eq!(bound(&mac, "ctrl+shift+e")[0].command, "lume.window.new");
    }

    #[test]
    fn a_binding_that_cannot_be_read_is_reported_at_its_line_on_every_platform() {
        let text = "\
[keybindings]\n\"ctrl+shift\" = \"lume.tab.new\"\n\
\"ctrl+shift+y\" = { command = \"lume.tab.new\", when = \"editor\" }\n\
\"ctrl+shift+u\" = \"  \"\n\
[keybindings.macos]\n\"hyper+d\" = \"lume.tab.new\"\n";
        let keymap = keymap(text, Platform::Windows);
        let said: Vec<(u32, &str)> = keymap
            .problems
            .iter()
            .map(|problem| (problem.line, problem.message.as_str()))
            .collect();
        assert_eq!(said.len(), 4, "{said:?}");
        assert!(
            said.iter()
                .any(|(line, message)| *line == 2 && message.contains("only modifiers"))
        );
        assert!(
            said.iter()
                .any(|(line, message)| *line == 3 && message.contains("`editor` is not a context"))
        );
        assert!(
            said.iter()
                .any(|(line, message)| *line == 4 && message.contains("needs a command"))
        );
        assert!(
            said.iter()
                .any(|(line, message)| *line == 6 && message.contains("`hyper` is not a modifier"))
        );
        assert_eq!(keymap.bindings.len(), default_count(Platform::Windows));
        assert_eq!(keymap.problems[0].path, "keybindings.\"ctrl+shift\"");
    }

    #[test]
    fn a_binding_is_sent_with_its_keys_and_contexts_as_text() {
        let keymap = keymap("", Platform::Windows);
        let copy = serde_json::to_value(bound(&keymap, "ctrl+shift+c")[0]).unwrap();
        assert_eq!(
            copy,
            json!({
                "keys": ["ctrl+shift+c"],
                "command": "lume.terminal.copy",
                "args": null,
                "when": ["terminal"],
                "source": { "layer": "default" },
            })
        );
    }

    #[test]
    fn plugins_bind_keys_between_the_defaults_and_the_users_and_only_keys_no_one_has() {
        let layered = resolve_layers(&[LayerInput {
            kind: LayerKind::User,
            file: PathBuf::from("config.toml"),
            text: "[keybindings]
\"ctrl+shift+u\" = \"lume.tab.new\"
\"ctrl+shift+j\" = \"unbound\"
"
            .to_owned(),
        }]);
        let binding = |keys: &str| PluginBinding {
            plugin: "acme.x".to_owned(),
            keys: keys.to_owned(),
            macos: Some("cmd+shift+y".to_owned()),
            command: "acme.x.go".to_owned(),
            args: Value::Null,
            when: Some("plugin:acme.x.busy && terminal".to_owned()),
        };
        let plugins = [
            binding("ctrl+shift+y"),
            binding("ctrl+shift+t"),
            binding("ctrl+shift+u"),
            binding("ctrl+shift+j"),
            binding("ctrl+alt+m"),
            binding("m"),
            binding("f7"),
            binding("ctrl+shift+y ctrl+shift+z"),
        ];
        let (keymap, dropped) = resolve_keymap_with(
            &layered.config,
            &layered.sources,
            Platform::Windows,
            &plugins,
        );
        let ours: Vec<String> = keymap
            .bindings
            .iter()
            .filter(|binding| binding.command == "acme.x.go")
            .map(|binding| binding.keys.to_string())
            .collect();
        assert_eq!(ours, ["ctrl+shift+y", "f7"]);
        let first = keymap
            .bindings
            .iter()
            .find(|binding| binding.command == "acme.x.go")
            .unwrap();
        assert_eq!(
            first.source,
            Source::Plugin {
                id: "acme.x".to_owned()
            }
        );
        assert_eq!(write_when(&first.when), "plugin:acme.x.busy && terminal");
        let why: Vec<&str> = dropped.iter().map(|one| one.message.as_str()).collect();
        assert_eq!(why.len(), 6, "{why:#?}");
        assert!(why[0].contains("Lume binds ctrl+shift+t"), "{why:#?}");
        assert!(why[1].contains("you bind ctrl+shift+u"), "{why:#?}");
        // A key the user unbinds is the user's too.
        assert!(why[2].contains("you bind ctrl+shift+j"), "{why:#?}");
        assert!(why[3].contains("Ctrl+Shift"), "{why:#?}");
        assert!(why[5].contains("a plugin before it binds"), "{why:#?}");
        let (mac, dropped) = resolve_keymap_with(
            &layered.config,
            &layered.sources,
            Platform::Macos,
            &plugins[..1],
        );
        let wanted = KeySequence::parse("cmd+shift+y").unwrap();
        assert!(
            mac.bindings
                .iter()
                .any(|binding| binding.command == "acme.x.go" && binding.keys == wanted),
            "{dropped:#?}"
        );
    }
}
