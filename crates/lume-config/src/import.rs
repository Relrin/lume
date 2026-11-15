//! Making a theme from another terminal's colour scheme.
//!
//! A colour scheme says what a terminal's colours are and nothing about the window
//! around it. An imported theme takes the terminal's colours as they are and makes the
//! few colours of the window from them by fixed mixes: a window a little darker than
//! the terminal, borders and quiet text between its foreground and background, and
//! its blue as the accent. Everything else comes from the built-in theme of the same
//! scheme, dark or light, which the imported one is told apart from by how light its
//! background is.
//!
//! The format read is Ghostty's theme file: lines of `key = value`, with `palette`
//! given once for each colour as `palette = 4=#7fb4f0`.

use lume_proto::ColorScheme;

use crate::theme::{DARK, LIGHT, Themes, parse_theme};
use crate::tokens::{Color, PALETTE, PALETTE_LEN, TokenValue};
use crate::tuning::{
    IMPORT_BORDER_MIX, IMPORT_DARK_SURFACE_MIX, IMPORT_DROP_PREVIEW_ALPHA, IMPORT_HOVER_MIX,
    IMPORT_LIGHT_LUMINANCE, IMPORT_LIGHT_SURFACE_MIX, IMPORT_MUTED_MIX, IMPORT_PRESSED_MIX,
    IMPORT_SELECTION_MIX,
};

/// The palette entry a scheme's blue is in, which becomes the accent.
const BLUE: usize = 4;

/// A theme made from a colour scheme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Imported {
    /// What the settings will call the theme, and the name of its file.
    pub id: String,
    /// The theme file's text.
    pub text: String,
    /// What in the scheme was passed over, for whoever imported it to know.
    pub notes: Vec<String>,
}

/// A name as a theme's id: lower case, with everything but letters, digits, dots and
/// dashes turned into dashes.
pub fn theme_id(name: &str) -> Option<String> {
    let mut id = String::new();
    for character in name.trim().chars() {
        let character = character.to_ascii_lowercase();
        if character.is_ascii_alphanumeric() || character == '.' {
            id.push(character);
        } else if !id.ends_with('-') {
            id.push('-');
        }
    }
    let id = id.trim_matches(['-', '.']).to_owned();
    (!id.is_empty()).then_some(id)
}

/// Reads a colour as Ghostty writes one: six hexadecimal digits, with or without `#`.
fn colour(written: &str) -> Option<Color> {
    let written = written.trim().trim_matches('"');
    let digits = written.strip_prefix('#').unwrap_or(written);
    if digits.len() != 6 {
        return None;
    }
    Color::parse(&format!("#{digits}"))
}

#[derive(Default)]
struct Scheme {
    background: Option<Color>,
    foreground: Option<Color>,
    cursor: Option<Color>,
    cursor_text: Option<Color>,
    selection: Option<Color>,
    selection_text: Option<Color>,
    palette: [Option<Color>; PALETTE_LEN],
}

fn read(source: &str, notes: &mut Vec<String>) -> Scheme {
    let mut scheme = Scheme::default();
    for (number, line) in source.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let number = number + 1;
        let Some((key, value)) = line.split_once('=') else {
            notes.push(format!(
                "line {number} is not `key = value` and was passed over"
            ));
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if key == "palette" {
            let entry = value
                .split_once('=')
                .and_then(|(index, written)| Some((index.trim().parse::<usize>().ok()?, written)));
            match entry {
                // A scheme may give all 256 colours; only the first sixteen are a
                // theme's to set.
                Some((index, _)) if index >= PALETTE_LEN => {}
                Some((index, written)) => match colour(written) {
                    Some(read) => scheme.palette[index] = Some(read),
                    None => notes.push(format!(
                        "line {number}: `{written}` is not a colour as six hexadecimal digits"
                    )),
                },
                None => notes.push(format!(
                    "line {number}: a palette entry is written `palette = 4=#7fb4f0`"
                )),
            }
            continue;
        }
        let slot = match key {
            "background" => &mut scheme.background,
            "foreground" => &mut scheme.foreground,
            "cursor-color" => &mut scheme.cursor,
            "cursor-text" => &mut scheme.cursor_text,
            "selection-background" => &mut scheme.selection,
            "selection-foreground" => &mut scheme.selection_text,
            _ => {
                notes.push(format!(
                    "line {number}: `{key}` is not a colour of the scheme and was passed over"
                ));
                continue;
            }
        };
        match colour(value) {
            Some(read) => *slot = Some(read),
            // Ghostty lets the cursor and the selection follow the cell they are on.
            None => notes.push(format!(
                "line {number}: `{key} = {value}` is not a colour as six hexadecimal digits; \
                 the theme's own is used"
            )),
        }
    }
    scheme
}

/// A string as TOML writes it.
fn quoted(text: &str) -> String {
    toml::Value::String(text.to_owned()).to_string()
}

/// Makes a theme called `name` from a Ghostty theme file's text.
pub fn import_ghostty(source: &str, name: &str) -> Result<Imported, String> {
    let id = theme_id(name)
        .ok_or_else(|| format!("`{name}` has nothing in it a theme could be called by"))?;
    if id == DARK || id == LIGHT {
        return Err(format!(
            "`{id}` is a built-in theme; give the imported one another name"
        ));
    }
    let mut notes = Vec::new();
    let scheme = read(source, &mut notes);
    let (Some(background), Some(foreground)) = (scheme.background, scheme.foreground) else {
        return Err(
            "this does not look like a Ghostty theme: it sets no `background` or no `foreground`"
                .to_owned(),
        );
    };

    let light = background.luminance() > IMPORT_LIGHT_LUMINANCE;
    let (kind, base_id) = if light {
        (ColorScheme::Light, LIGHT)
    } else {
        (ColorScheme::Dark, DARK)
    };
    let black = Color::rgb(0, 0, 0);
    let surface = background.mix(
        black,
        if light {
            IMPORT_LIGHT_SURFACE_MIX
        } else {
            IMPORT_DARK_SURFACE_MIX
        },
    );
    let accent = scheme.palette[BLUE].or(scheme.cursor).unwrap_or(foreground);
    let border = background.mix(foreground, IMPORT_BORDER_MIX);
    let drop_preview = Color {
        a: IMPORT_DROP_PREVIEW_ALPHA,
        ..accent
    };

    // The colours the scheme leaves out of its palette are the built-in theme's, so
    // that the list in the file has every colour in its place.
    let themes = Themes::builtin();
    let base = themes.get(base_id);
    let palette: Vec<String> = (0..PALETTE_LEN)
        .filter_map(|index| {
            scheme.palette[index].or_else(|| {
                match base?.values.get(format!("{PALETTE}.{index}").as_str())? {
                    TokenValue::Color(color) => Some(*color),
                    _ => None,
                }
            })
        })
        .map(|color| quoted(&color.to_string()))
        .collect();

    let mut text = String::new();
    text.push_str(
        "# Made by `lume theme import` from a Ghostty theme. The terminal's colours are\n",
    );
    text.push_str(
        "# the scheme's; the window's are mixed from them. Change any of it by hand.\n\n",
    );
    text.push_str(&format!("name = {}\n", quoted(name.trim())));
    text.push_str(&format!(
        "scheme = \"{}\"\n",
        if light { "light" } else { "dark" }
    ));

    let colors = [
        ("surface", surface),
        ("card", background),
        ("card-header", background),
        ("border", border),
        ("border-focus", accent),
        ("text", foreground),
        ("text-muted", foreground.mix(background, IMPORT_MUTED_MIX)),
        ("accent", accent),
        ("accent-text", background),
        ("control", background),
        (
            "control-hover",
            background.mix(foreground, IMPORT_HOVER_MIX),
        ),
        (
            "control-active",
            background.mix(foreground, IMPORT_PRESSED_MIX),
        ),
        ("popover", background),
        ("drop-preview", drop_preview),
    ];
    text.push_str("\n[color]\n");
    for (key, color) in colors {
        text.push_str(&format!("{key} = \"{color}\"\n"));
    }

    let terminal = [
        ("fg", foreground),
        ("bg", background),
        ("cursor", scheme.cursor.unwrap_or(foreground)),
        ("cursor-text", scheme.cursor_text.unwrap_or(background)),
        (
            "selection",
            scheme
                .selection
                .unwrap_or_else(|| background.mix(foreground, IMPORT_SELECTION_MIX)),
        ),
        (
            "selection-text",
            scheme.selection_text.unwrap_or(foreground),
        ),
    ];
    text.push_str("\n[terminal]\n");
    for (key, color) in terminal {
        text.push_str(&format!("{key} = \"{color}\"\n"));
    }
    if palette.len() == PALETTE_LEN {
        text.push_str("palette = [\n");
        for row in palette.chunks(PALETTE_LEN / 2) {
            text.push_str(&format!("    {},\n", row.join(", ")));
        }
        text.push_str("]\n");
    }

    // What was written must be a theme. If it is not, that is a fault here, and better
    // found now than when the theme is chosen.
    match parse_theme(&id, &text, None) {
        Ok((theme, problems)) if problems.is_empty() && theme.scheme == kind => {}
        Ok((_, problems)) => {
            let first = problems
                .first()
                .map(ToString::to_string)
                .unwrap_or_default();
            return Err(format!(
                "the theme made from the scheme is not right: {first}"
            ));
        }
        Err(problem) => {
            return Err(format!(
                "the theme made from the scheme is not right: {problem}"
            ));
        }
    }
    Ok(Imported { id, text, notes })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DARK_SCHEME: &str = "\
# A dark scheme.
palette = 0=#15161e
palette = 1=#f7768e
palette = 2=#9ece6a
palette = 3=#e0af68
palette = 4=#7aa2f7
palette = 5=#bb9af7
palette = 6=#7dcfff
palette = 7=#a9b1d6
palette = 8=#414868
palette = 9=#f7768e
palette = 10=#9ece6a
palette = 11=#e0af68
palette = 12=#7aa2f7
palette = 13=#bb9af7
palette = 14=#7dcfff
palette = 15=#c0caf5
palette = 16=#ff9e64
background = 1a1b26
foreground = #c0caf5
cursor-color = #c0caf5
cursor-text = #15161e
selection-background = #33467c
selection-foreground = #c0caf5
";

    fn theme(imported: &Imported) -> crate::theme::Theme {
        let (theme, problems) = parse_theme(&imported.id, &imported.text, None).unwrap();
        assert_eq!(problems, vec![]);
        theme
    }

    fn color(theme: &crate::theme::Theme, name: &str) -> String {
        match &theme.values[name] {
            TokenValue::Color(color) => color.to_string(),
            other => panic!("{name} is {other:?}"),
        }
    }

    #[test]
    fn a_dark_scheme_becomes_a_dark_theme_with_its_terminal_colours_kept() {
        let imported = import_ghostty(DARK_SCHEME, "Tokyo Night").unwrap();
        assert_eq!(imported.id, "tokyo-night");
        assert_eq!(imported.notes, Vec::<String>::new());

        let theme = theme(&imported);
        assert_eq!(theme.name, "Tokyo Night");
        assert_eq!(theme.scheme, ColorScheme::Dark);
        assert_eq!(color(&theme, "terminal.bg"), "#1a1b26");
        assert_eq!(color(&theme, "terminal.fg"), "#c0caf5");
        assert_eq!(color(&theme, "terminal.selection"), "#33467c");
        assert_eq!(color(&theme, "terminal.palette.1"), "#f7768e");
        assert_eq!(color(&theme, "terminal.palette.15"), "#c0caf5");
        // The window's colours are made from the scheme's.
        assert_eq!(color(&theme, "color.card"), "#1a1b26");
        assert_eq!(color(&theme, "color.accent"), "#7aa2f7");
        assert_eq!(color(&theme, "color.text"), "#c0caf5");
        let surface = Color::parse(&color(&theme, "color.surface")).unwrap();
        assert!(surface.luminance() < Color::parse("#1a1b26").unwrap().luminance());
        assert!(!theme.values.contains_key("color.status.danger"));
    }

    #[test]
    fn a_light_scheme_becomes_a_light_theme_and_what_it_leaves_out_is_filled_in() {
        let scheme = "background = #fdf6e3\nforeground = #586e75\npalette = 4=#268bd2\n";
        let imported = import_ghostty(scheme, "paper.light").unwrap();
        assert_eq!(imported.id, "paper.light");
        let theme = theme(&imported);
        assert_eq!(theme.scheme, ColorScheme::Light);
        assert_eq!(color(&theme, "terminal.cursor"), "#586e75");
        assert_eq!(color(&theme, "terminal.cursor-text"), "#fdf6e3");
        assert_eq!(color(&theme, "terminal.palette.4"), "#268bd2");
        // The red it does not give is the built-in light theme's.
        let builtin = Themes::builtin();
        assert_eq!(
            theme.values["terminal.palette.1"],
            builtin.get(LIGHT).unwrap().values["terminal.palette.1"]
        );
    }

    #[test]
    fn what_is_not_a_colour_of_the_scheme_is_noted_and_passed_over() {
        let scheme = "\
background = #000000\nforeground = #ffffff\ncursor-color = cell-foreground\n\
font-size = 13\npalette = red\npalette = 3=yellow\nnonsense\n";
        let imported = import_ghostty(scheme, "odd").unwrap();
        assert_eq!(imported.notes.len(), 5, "{:?}", imported.notes);
        assert!(imported.notes[0].contains("line 3"));
        assert!(imported.notes[1].contains("`font-size`"));
        assert!(imported.notes[4].contains("line 7"));
        assert_eq!(color(&theme(&imported), "terminal.cursor"), "#ffffff");
    }

    #[test]
    fn a_file_that_is_no_scheme_or_a_name_that_is_no_name_is_refused() {
        assert!(
            import_ghostty("font-size = 13\n", "x")
                .unwrap_err()
                .contains("does not look like a Ghostty theme")
        );
        assert!(import_ghostty(DARK_SCHEME, "  ///  ").is_err());
        assert!(
            import_ghostty(DARK_SCHEME, "Lume Dark")
                .unwrap_err()
                .contains("built-in theme")
        );
    }

    #[test]
    fn a_name_becomes_an_id_a_file_can_have() {
        assert_eq!(theme_id("Tokyo Night").as_deref(), Some("tokyo-night"));
        assert_eq!(
            theme_id("  Gruvbox (Dark) / Hard ").as_deref(),
            Some("gruvbox-dark-hard")
        );
        assert_eq!(
            theme_id("solarized.light").as_deref(),
            Some("solarized.light")
        );
        assert_eq!(theme_id("--").as_deref(), None);
    }

    #[test]
    fn a_name_with_quotes_in_it_is_written_safely() {
        let imported = import_ghostty(DARK_SCHEME, "The \"Best\" One").unwrap();
        assert_eq!(theme(&imported).name, "The \"Best\" One");
    }
}
