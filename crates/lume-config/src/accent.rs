//! The accent colour chosen over the theme's own.
//!
//! A theme draws focus, the selected tab, the window's glow and often the cursor in its
//! accent colour. The setting `appearance.accent` changes that one colour: it is the
//! name of one of [`ACCENT_PRESETS`], which has a colour for dark themes and one for
//! light ones, or a colour of its own written `#rrggbb`. Every token the theme draws in
//! its accent follows the chosen one, keeping its own transparency.

use std::collections::BTreeMap;

use lume_proto::ColorScheme;

use crate::tokens::{Color, TokenValue};
use crate::tuning::{
    ACCENT_PRESETS, ACCENT_TEXT_DARK, ACCENT_TEXT_LIGHT, SELECTION_ACCENT_MIX_DARK,
    SELECTION_ACCENT_MIX_LIGHT,
};

/// The schema keyword that marks a string setting as an accent colour.
pub const FORMAT_KEY: &str = "x-lume-format";

/// The value of [`FORMAT_KEY`] for an accent colour.
pub const ACCENT_FORMAT: &str = "accent";

/// The colour an accent setting stands for on a theme of `scheme`, or `None` when it is
/// neither a preset's name nor a colour.
pub fn accent_color(value: &str, scheme: ColorScheme) -> Option<Color> {
    if let Some(preset) = ACCENT_PRESETS.iter().find(|preset| preset.name == value) {
        let text = match scheme {
            ColorScheme::Dark => preset.dark,
            ColorScheme::Light => preset.light,
        };
        return Color::parse(text);
    }
    // An accent is opaque: what is drawn in it adds its own transparency.
    value
        .strip_prefix('#')
        .filter(|digits| digits.len() == 6)
        .and(Color::parse(value))
}

/// Why `value` is not an accent colour, in words for the person who wrote it.
pub fn check_accent(value: &str) -> Result<(), String> {
    if accent_color(value, ColorScheme::Dark).is_some() {
        return Ok(());
    }
    let names: Vec<String> = ACCENT_PRESETS
        .iter()
        .map(|preset| format!("`{}`", preset.name))
        .collect();
    Err(format!(
        "expected a colour written `#rrggbb` or one of {}",
        names.join(", ")
    ))
}

/// Draws in `accent` what the theme draws in its own accent.
///
/// A colour or a shadow of the theme's accent takes the new one, keeping its
/// transparency; the text on the accent becomes whichever of dark and light reads
/// better on it, and selected text's background is the terminal's background tinted
/// with it.
pub(crate) fn apply_accent(
    values: &mut BTreeMap<&'static str, TokenValue>,
    accent: Color,
    scheme: ColorScheme,
) {
    let Some(TokenValue::Color(old)) = values.get("color.accent").cloned() else {
        return;
    };
    let tied = |color: &Color| (color.r, color.g, color.b) == (old.r, old.g, old.b);
    let recolor = |color: &mut Color| {
        if tied(color) {
            *color = Color {
                a: color.a,
                ..accent
            };
        }
    };
    for value in values.values_mut() {
        match value {
            TokenValue::Color(color) => recolor(color),
            TokenValue::Shadow(shadows) => {
                for shadow in shadows {
                    recolor(&mut shadow.color);
                }
            }
            _ => {}
        }
    }

    let text = [ACCENT_TEXT_DARK, ACCENT_TEXT_LIGHT]
        .into_iter()
        .filter_map(Color::parse)
        .max_by(|one, other| contrast(*one, accent).total_cmp(&contrast(*other, accent)));
    if let Some(text) = text {
        values.insert("color.accent-text", TokenValue::Color(text));
    }

    if let Some(TokenValue::Color(background)) = values.get("terminal.bg").cloned() {
        let amount = match scheme {
            ColorScheme::Dark => SELECTION_ACCENT_MIX_DARK,
            ColorScheme::Light => SELECTION_ACCENT_MIX_LIGHT,
        };
        let selection = background.mix(accent, amount);
        values.insert("terminal.selection", TokenValue::Color(selection));
    }
}

/// How far apart two colours are in lightness, as the contrast ratio of WCAG: from 1
/// (the same) to 21 (black on white).
pub fn contrast(one: Color, other: Color) -> f64 {
    let (one, other) = (one.luminance() + 0.05, other.luminance() + 0.05);
    one.max(other) / one.min(other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::default_tokens;

    #[test]
    fn a_preset_has_a_colour_for_each_scheme_and_a_colour_of_its_own_is_kept() {
        let mint = accent_color("mint", ColorScheme::Dark).unwrap();
        assert_eq!(mint.to_string(), "#7fe3b8");
        let deep = accent_color("mint", ColorScheme::Light).unwrap();
        assert_eq!(deep.to_string(), "#147a55");
        let own = accent_color("#FF8800", ColorScheme::Light).unwrap();
        assert_eq!(own.to_string(), "#ff8800");
    }

    #[test]
    fn the_setting_is_marked_as_an_accent_in_the_schema() {
        let accent =
            &crate::schema::config_schema()["properties"]["appearance"]["properties"]["accent"];
        assert_eq!(accent[FORMAT_KEY], ACCENT_FORMAT);
    }

    #[test]
    fn anything_else_is_refused_with_the_names_offered() {
        for wrong in ["", "red", "#ff8800aa", "#ff88", "ff8800", "#gg8800"] {
            let message = check_accent(wrong).unwrap_err();
            assert!(message.contains("`mint`"), "{wrong}: {message}");
        }
        assert!(check_accent("graphite").is_ok());
    }

    #[test]
    fn every_preset_stands_out_from_the_panes_of_its_scheme() {
        // The card colours of the built-in dark and light themes.
        let dark_card = Color::parse("#171a1e").unwrap();
        let light_card = Color::parse("#ffffff").unwrap();
        for preset in ACCENT_PRESETS {
            let dark = Color::parse(preset.dark).unwrap();
            let light = Color::parse(preset.light).unwrap();
            assert!(contrast(dark, dark_card) >= 3.0, "{}", preset.name);
            assert!(contrast(light, light_card) >= 3.0, "{}", preset.name);
        }
    }

    #[test]
    fn what_the_theme_draws_in_its_accent_follows_the_chosen_one() {
        let mut values = default_tokens().values;
        let blue = Color::parse("#7aa7ff").unwrap();
        apply_accent(&mut values, blue, ColorScheme::Dark);
        let color = |name: &str| match &values[name] {
            TokenValue::Color(color) => color.to_string(),
            other => panic!("{name} is {other:?}"),
        };
        assert_eq!(color("color.accent"), "#7aa7ff");
        // Transparency is the theme's.
        assert_eq!(color("color.border-focus"), "#7aa7ff99");
        assert_eq!(color("color.drop-preview"), "#7aa7ff14");
        assert_eq!(color("terminal.cursor"), "#7aa7ff");
        // A colour that merely looks alike is left alone.
        assert_eq!(color("color.tab.teal"), "#7fdbca");
        assert_eq!(color("color.accent-text"), "#0f1113");
        assert_ne!(color("terminal.selection"), "#2b4a40");
        let TokenValue::Shadow(focus) = &values["shadow.card-focus"] else {
            panic!("not a shadow");
        };
        assert_eq!(focus[0].color.to_string(), "#7aa7ff33");
    }

    #[test]
    fn a_dark_accent_is_written_on_in_white() {
        let mut values = default_tokens().values;
        let deep = Color::parse("#2f6fd0").unwrap();
        apply_accent(&mut values, deep, ColorScheme::Light);
        assert_eq!(
            values["color.accent-text"],
            TokenValue::Color(Color::parse("#ffffff").unwrap())
        );
    }
}
