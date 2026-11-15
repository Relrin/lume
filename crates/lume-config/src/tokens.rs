//! The design tokens: every colour, size and font the interface is drawn with.
//!
//! The table here is the one definition of the set. A theme sets tokens, the settings
//! override some, and what results is handed to the page, which writes each token as a
//! CSS variable and passes the terminal's to its renderer. The TypeScript types of the
//! set and the stylesheet a page starts from are generated from this table, so the two
//! sides cannot drift apart.

use std::collections::BTreeMap;
use std::fmt;

use lume_proto::ColorScheme;
use serde_json::{Map, Value};

/// What kind of value a token holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    /// A colour: `#rrggbb`, or `#rrggbbaa` with transparency.
    Color,
    /// A length in CSS pixels.
    Length,
    /// A plain number: a ratio, an opacity, a font weight.
    Number,
    /// The name of one font.
    FontFamily,
    /// The names of several fonts, most preferred first.
    FontList,
    /// One or more shadows: `x y blur spread colour`, separated by commas.
    Shadow,
    /// The path of a picture, or none.
    Image,
}

impl TokenKind {
    /// The kind's name in the generated TypeScript.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Color => "color",
            Self::Length => "length",
            Self::Number => "number",
            Self::FontFamily => "font-family",
            Self::FontList => "font-list",
            Self::Shadow => "shadow",
            Self::Image => "image",
        }
    }

    /// The TypeScript type of a token of this kind.
    fn typescript(self) -> &'static str {
        match self {
            Self::Color | Self::FontFamily | Self::Shadow => "string",
            Self::Length | Self::Number => "number",
            Self::FontList => "readonly string[]",
            Self::Image => "string | null",
        }
    }
}

/// One token: its name, what it holds and what it is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenDef {
    /// The name, as themes and settings write it: `color.surface`.
    pub name: &'static str,
    pub kind: TokenKind,
    pub doc: &'static str,
}

impl TokenDef {
    /// The CSS variable the token is written as: `--lume-color-surface`.
    pub fn css_variable(&self) -> String {
        format!("--lume-{}", self.name.replace('.', "-"))
    }

    /// The token's group: the part of its name before the first dot.
    pub fn group(&self) -> &'static str {
        self.name.split('.').next().unwrap_or(self.name)
    }
}

const fn token(name: &'static str, kind: TokenKind, doc: &'static str) -> TokenDef {
    TokenDef { name, kind, doc }
}

use TokenKind::{Color as C, FontFamily, FontList, Image, Length as L, Number as N, Shadow as S};

/// The group whose tokens come from the `[font]` settings and never from a theme.
pub const FONT_GROUP: &str = "font";

/// The token that holds the sixteen ANSI colours, which a theme writes as one list.
pub const PALETTE: &str = "terminal.palette";

/// How many colours the palette has.
pub const PALETTE_LEN: usize = 16;

/// Every token.
pub const TOKENS: &[TokenDef] = &[
    token("color.surface", C, "The window behind the panes."),
    token("color.card", C, "A pane's background around its terminal."),
    token("color.card-header", C, "A pane's header."),
    token("color.border", C, "Lines between and around things."),
    token(
        "color.border-focus",
        C,
        "The border of the pane that has the focus.",
    ),
    token("color.text", C, "Text."),
    token("color.text-muted", C, "Text that matters less."),
    token("color.text-inverse", C, "Text on a strong colour."),
    token("color.accent", C, "What is selected or active."),
    token("color.accent-text", C, "Text on the accent colour."),
    token("color.control", C, "A button or field at rest."),
    token(
        "color.control-hover",
        C,
        "A button or field under the pointer.",
    ),
    token(
        "color.control-active",
        C,
        "A button or field being pressed.",
    ),
    token(
        "color.popover",
        C,
        "Menus, the command palette and other things that float.",
    ),
    token("color.overlay", C, "What dims the window behind a dialog."),
    token(
        "color.drop-preview",
        C,
        "Where something being dragged would land.",
    ),
    token("color.status.info", C, "Something worth knowing."),
    token("color.status.success", C, "Something that went well."),
    token("color.status.warning", C, "Something to look at."),
    token("color.status.danger", C, "Something that went wrong."),
    token("terminal.fg", C, "Terminal text."),
    token("terminal.bg", C, "The terminal's background."),
    token("terminal.cursor", C, "The cursor."),
    token(
        "terminal.cursor-text",
        C,
        "The character under a block cursor.",
    ),
    token("terminal.selection", C, "The background of selected text."),
    token("terminal.selection-text", C, "Selected text."),
    token(
        "terminal.search-match",
        C,
        "The background of text a search found.",
    ),
    token(
        "terminal.search-active",
        C,
        "The background of the match a search is at.",
    ),
    token("terminal.palette.0", C, "ANSI black."),
    token("terminal.palette.1", C, "ANSI red."),
    token("terminal.palette.2", C, "ANSI green."),
    token("terminal.palette.3", C, "ANSI yellow."),
    token("terminal.palette.4", C, "ANSI blue."),
    token("terminal.palette.5", C, "ANSI magenta."),
    token("terminal.palette.6", C, "ANSI cyan."),
    token("terminal.palette.7", C, "ANSI white."),
    token("terminal.palette.8", C, "ANSI bright black."),
    token("terminal.palette.9", C, "ANSI bright red."),
    token("terminal.palette.10", C, "ANSI bright green."),
    token("terminal.palette.11", C, "ANSI bright yellow."),
    token("terminal.palette.12", C, "ANSI bright blue."),
    token("terminal.palette.13", C, "ANSI bright magenta."),
    token("terminal.palette.14", C, "ANSI bright cyan."),
    token("terminal.palette.15", C, "ANSI bright white."),
    token("font.family", FontFamily, "The font of terminals."),
    token("font.fallback", FontList, "The fonts tried after it."),
    token("font.size", L, "The size of terminal text."),
    token(
        "font.line-height",
        N,
        "The height of a line, as a multiple of the size.",
    ),
    token("font.weight", N, "The weight of ordinary terminal text."),
    token("font.bold-weight", N, "The weight of bold terminal text."),
    token(
        "font.ui-family",
        FontFamily,
        "The font of everything but terminals.",
    ),
    token("font.ui-size", L, "The size of that text."),
    token("space.gap", L, "The space between panes and around them."),
    token(
        "space.padding",
        L,
        "The space between a pane's edge and its text.",
    ),
    token("radius.card", L, "The radius of a pane's corners."),
    token(
        "radius.control",
        L,
        "The radius of a button's or field's corners.",
    ),
    token("size.header", L, "The height of a pane's header."),
    token(
        "size.topbar",
        L,
        "The height of the bar at the top of a window.",
    ),
    token("size.tab", L, "The height of a tab."),
    token("size.border", L, "The width of a border."),
    token(
        "size.focus-ring",
        L,
        "The width of the ring around what has the focus.",
    ),
    token(
        "opacity.dim",
        N,
        "How much a pane without the focus is dimmed.",
    ),
    token(
        "opacity.disabled",
        N,
        "How opaque something that cannot be used is.",
    ),
    token(
        "opacity.background-image",
        N,
        "How opaque the picture behind the panes is.",
    ),
    token("shadow.card", S, "The shadow of a pane."),
    token(
        "shadow.card-focus",
        S,
        "The shadow of the pane that has the focus.",
    ),
    token("shadow.floating", S, "The shadow of a floating pane."),
    token("shadow.popover", S, "The shadow of a menu or dialog."),
    token("background.image", Image, "A picture behind the panes."),
    token("background.blur", L, "How much that picture is blurred."),
];

/// The token called `name`.
pub fn token_def(name: &str) -> Option<&'static TokenDef> {
    TOKENS.iter().find(|token| token.name == name)
}

/// A colour with transparency.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// 255 is opaque.
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Reads `#rrggbb` or `#rrggbbaa`, in either case of letter.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('#')?;
        if !matches!(digits.len(), 6 | 8) || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
        Some(Self {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a: if digits.len() == 8 { byte(6)? } else { 255 },
        })
    }

    /// The colour between this one and `other`: `amount` 0 is this one, 1 the other.
    pub fn mix(self, other: Self, amount: f64) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        let channel = |from: u8, to: u8| {
            let mixed = f64::from(from) + (f64::from(to) - f64::from(from)) * amount;
            // Within 0..=255 by construction, so the cast cannot truncate.
            mixed.round() as u8
        };
        Self {
            r: channel(self.r, other.r),
            g: channel(self.g, other.g),
            b: channel(self.b, other.b),
            a: channel(self.a, other.a),
        }
    }

    /// How light the colour looks, from 0 (black) to 1 (white).
    pub fn luminance(self) -> f64 {
        let linear = |channel: u8| {
            let value = f64::from(channel) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(self.r) + 0.7152 * linear(self.g) + 0.0722 * linear(self.b)
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)?;
        if self.a != 255 {
            write!(f, "{:02x}", self.a)?;
        }
        Ok(())
    }
}

/// One shadow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    /// Drawn inside the box rather than behind it.
    pub inset: bool,
    pub x: f64,
    pub y: f64,
    pub blur: f64,
    pub spread: f64,
    pub color: Color,
}

impl Shadow {
    /// Reads `[inset] x y blur spread colour`, lengths in pixels without a unit.
    fn parse(text: &str) -> Option<Self> {
        let mut words: Vec<&str> = text.split_whitespace().collect();
        let inset = words.first() == Some(&"inset");
        if inset {
            words.remove(0);
        }
        let [x, y, blur, spread, color] = words.as_slice() else {
            return None;
        };
        let length = |word: &str| word.parse::<f64>().ok().filter(|value| value.is_finite());
        Some(Self {
            inset,
            x: length(x)?,
            y: length(y)?,
            blur: length(blur).filter(|blur| *blur >= 0.0)?,
            spread: length(spread)?,
            color: Color::parse(color)?,
        })
    }
}

impl fmt::Display for Shadow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.inset {
            f.write_str("inset ")?;
        }
        write!(
            f,
            "{}px {}px {}px {}px {}",
            self.x, self.y, self.blur, self.spread, self.color
        )
    }
}

/// The value of a token.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenValue {
    Color(Color),
    Length(f64),
    Number(f64),
    FontFamily(String),
    FontList(Vec<String>),
    /// No shadows is no shadow at all.
    Shadow(Vec<Shadow>),
    Image(Option<String>),
}

/// The font names CSS knows without quotes.
const GENERIC_FAMILIES: [&str; 13] = [
    "serif",
    "sans-serif",
    "monospace",
    "cursive",
    "fantasy",
    "system-ui",
    "ui-serif",
    "ui-sans-serif",
    "ui-monospace",
    "ui-rounded",
    "emoji",
    "math",
    "fangsong",
];

/// A font's name as CSS writes it: quoted, unless it is one of CSS's own names.
pub fn css_family(name: &str) -> String {
    if GENERIC_FAMILIES.contains(&name) {
        name.to_owned()
    } else {
        format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

impl TokenValue {
    /// Reads a value of `kind` as a theme or the settings write it. The error says
    /// what was expected.
    pub fn parse(kind: TokenKind, value: &Value) -> Result<Self, String> {
        let number = |what: &str| {
            value
                .as_f64()
                .filter(|number| number.is_finite())
                .ok_or_else(|| format!("expected {what}"))
        };
        let text = |what: &str| value.as_str().ok_or_else(|| format!("expected {what}"));
        match kind {
            TokenKind::Color => {
                let what = "a colour such as \"#1c1f2a\", or \"#1c1f2a80\" with transparency";
                Color::parse(text(what)?)
                    .map(Self::Color)
                    .ok_or_else(|| format!("expected {what}"))
            }
            TokenKind::Length => {
                let length = number("a length in pixels, as a number")?;
                if length < 0.0 {
                    return Err("a length cannot be negative".to_owned());
                }
                Ok(Self::Length(length))
            }
            TokenKind::Number => Ok(Self::Number(number("a number")?)),
            TokenKind::FontFamily => {
                let family = text("the name of a font")?.trim();
                if family.is_empty() {
                    return Err("expected the name of a font".to_owned());
                }
                Ok(Self::FontFamily(family.to_owned()))
            }
            TokenKind::FontList => {
                let what = "a list of font names";
                let names = value.as_array().ok_or_else(|| format!("expected {what}"))?;
                names
                    .iter()
                    .map(|name| name.as_str().map(str::to_owned))
                    .collect::<Option<Vec<String>>>()
                    .map(Self::FontList)
                    .ok_or_else(|| format!("expected {what}"))
            }
            TokenKind::Shadow => {
                let what = "shadows written as \"x y blur spread #colour\", separated by \
                            commas, or \"none\"";
                let written = text(what)?.trim();
                if written.is_empty() || written == "none" {
                    return Ok(Self::Shadow(Vec::new()));
                }
                written
                    .split(',')
                    .map(Shadow::parse)
                    .collect::<Option<Vec<Shadow>>>()
                    .map(Self::Shadow)
                    .ok_or_else(|| format!("expected {what}"))
            }
            TokenKind::Image => {
                let path = text("the path of a picture, or \"\" for none")?;
                Ok(Self::Image(
                    Some(path.to_owned()).filter(|path| !path.is_empty()),
                ))
            }
        }
    }

    /// The value as the page is sent it.
    pub fn to_json(&self) -> Value {
        match self {
            Self::Color(color) => Value::String(color.to_string()),
            Self::Length(number) | Self::Number(number) => Value::from(*number),
            Self::FontFamily(family) => Value::String(family.clone()),
            Self::FontList(families) => Value::from(families.clone()),
            Self::Shadow(_) => Value::String(self.to_css()),
            Self::Image(path) => path.clone().map_or(Value::Null, Value::String),
        }
    }

    /// The value as a CSS variable holds it.
    pub fn to_css(&self) -> String {
        match self {
            Self::Color(color) => color.to_string(),
            Self::Length(length) => format!("{length}px"),
            Self::Number(number) => number.to_string(),
            Self::FontFamily(family) => css_family(family),
            Self::FontList(families) => families
                .iter()
                .map(|family| css_family(family))
                .collect::<Vec<_>>()
                .join(", "),
            Self::Shadow(shadows) if shadows.is_empty() => "none".to_owned(),
            Self::Shadow(shadows) => shadows
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            Self::Image(None) => "none".to_owned(),
            Self::Image(Some(path)) => format!(
                "url(\"{}\")",
                path.replace('\\', "\\\\").replace('"', "\\\"")
            ),
        }
    }
}

/// Every token with its value: what a theme and the settings come to.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedTokens {
    /// The id of the theme the values start from.
    pub theme: String,
    /// Whether that theme is a light or a dark one.
    pub scheme: ColorScheme,
    pub values: BTreeMap<&'static str, TokenValue>,
}

impl ResolvedTokens {
    /// The value of a token.
    pub fn get(&self, name: &str) -> Option<&TokenValue> {
        self.values.get(name)
    }

    /// The colour a token holds, for a token that holds one.
    pub fn color(&self, name: &str) -> Option<Color> {
        match self.get(name)? {
            TokenValue::Color(color) => Some(*color),
            _ => None,
        }
    }

    /// The tokens as the page is sent them: each name with its value.
    pub fn to_json(&self) -> Map<String, Value> {
        TOKENS
            .iter()
            .filter_map(|token| Some((token.name.to_owned(), self.get(token.name)?.to_json())))
            .collect()
    }

    /// A stylesheet that sets every token as a CSS variable on the document's root.
    pub fn css(&self) -> String {
        let mut css = String::from(GENERATED_NOTE_CSS);
        css.push_str(":root {\n");
        for token in TOKENS {
            if let Some(value) = self.get(token.name) {
                css.push_str(&format!(
                    "  {}: {};\n",
                    token.css_variable(),
                    value.to_css()
                ));
            }
        }
        css.push_str("}\n");
        css
    }
}

const GENERATED_NOTE_CSS: &str = "\
/*
 * Generated by `cargo xtask tokens` from the token table and the built-in dark theme.
 * Do not edit: change the table or the theme and generate again.
 *
 * These are the values a page starts with, before the application has told it which
 * theme and settings are in use.
 */
";

const GENERATED_NOTE_TS: &str = "\
/**
 * The design tokens: their names, kinds and CSS variables, and the values of the
 * built-in dark theme.
 *
 * Generated by `cargo xtask tokens` from the token table of the configuration crate.
 * Do not edit: change the table and generate again.
 */
";

/// A value as a TypeScript literal.
fn typescript_literal(value: &Value) -> String {
    match value {
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(typescript_literal).collect();
            format!("[{}]", items.join(", "))
        }
        // JSON's literals are TypeScript's, strings and their escapes included.
        other => other.to_string(),
    }
}

/// The TypeScript module that describes the token set, with `defaults` as the values
/// a page starts from.
pub fn typescript(defaults: &ResolvedTokens) -> String {
    let mut ts = String::from(GENERATED_NOTE_TS);

    ts.push_str("\n/** What kind of value a token holds. */\nexport type TokenKind =\n");
    let kinds = [C, L, N, FontFamily, FontList, S, Image];
    for (index, kind) in kinds.iter().enumerate() {
        let end = if index + 1 == kinds.len() { ";" } else { "" };
        ts.push_str(&format!("  | \"{}\"{end}\n", kind.as_str()));
    }

    ts.push_str(
        "\n/** The kind of every token, by the token's name. */\nexport const TOKEN_KINDS = {\n",
    );
    for token in TOKENS {
        ts.push_str(&format!(
            "  \"{}\": \"{}\",\n",
            token.name,
            token.kind.as_str()
        ));
    }
    ts.push_str("} as const satisfies Record<string, TokenKind>;\n");

    ts.push_str(
        "\n/** The name of a token. */\nexport type TokenName = keyof typeof TOKEN_KINDS;\n",
    );

    ts.push_str("\n/** Every token's name, in the order of the token table. */\n");
    ts.push_str("export const TOKEN_NAMES = Object.keys(TOKEN_KINDS) as readonly TokenName[];\n");

    ts.push_str("\n/** The CSS variable each token is written as. */\n");
    ts.push_str("export const CSS_VARIABLES: Readonly<Record<TokenName, string>> = {\n");
    for token in TOKENS {
        ts.push_str(&format!(
            "  \"{}\": \"{}\",\n",
            token.name,
            token.css_variable()
        ));
    }
    ts.push_str("};\n");

    ts.push_str("\n/** Every token with its value, as the application sends them. */\n");
    ts.push_str("export interface Tokens {\n");
    for token in TOKENS {
        ts.push_str(&format!("  /** {} */\n", token.doc));
        ts.push_str(&format!(
            "  readonly \"{}\": {};\n",
            token.name,
            token.kind.typescript()
        ));
    }
    ts.push_str("}\n");

    ts.push_str("\n/** The built-in dark theme with the default settings. */\n");
    ts.push_str("export const DEFAULT_TOKENS: Tokens = {\n");
    for token in TOKENS {
        if let Some(value) = defaults.get(token.name) {
            ts.push_str(&format!(
                "  \"{}\": {},\n",
                token.name,
                typescript_literal(&value.to_json())
            ));
        }
    }
    ts.push_str("};\n");

    ts.push_str("\n/** Whether the built-in dark theme is a light or a dark one. */\n");
    let scheme = match defaults.scheme {
        ColorScheme::Light => "light",
        ColorScheme::Dark => "dark",
    };
    ts.push_str(&format!(
        "export const DEFAULT_SCHEME: \"light\" | \"dark\" = \"{scheme}\";\n"
    ));
    ts
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::json;

    use super::*;

    #[test]
    fn every_token_has_a_name_of_its_own_and_a_variable_of_its_own() {
        let names: HashSet<&str> = TOKENS.iter().map(|token| token.name).collect();
        assert_eq!(names.len(), TOKENS.len());
        let variables: HashSet<String> = TOKENS.iter().map(TokenDef::css_variable).collect();
        assert_eq!(variables.len(), TOKENS.len());
        assert_eq!(
            token_def("color.status.info").unwrap().css_variable(),
            "--lume-color-status-info"
        );
        assert_eq!(
            token_def("terminal.palette.15").unwrap().group(),
            "terminal"
        );
        for token in TOKENS {
            assert!(!token.doc.is_empty(), "{}", token.name);
            assert!(
                token
                    .name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-'),
                "{}",
                token.name
            );
        }
    }

    #[test]
    fn the_palette_has_sixteen_colours_in_order() {
        for index in 0..PALETTE_LEN {
            let token = token_def(&format!("{PALETTE}.{index}")).unwrap();
            assert_eq!(token.kind, TokenKind::Color);
        }
        assert!(token_def(&format!("{PALETTE}.{PALETTE_LEN}")).is_none());
    }

    #[test]
    fn a_colour_is_read_with_or_without_transparency_and_written_in_lower_case() {
        assert_eq!(Color::parse("#1C1f2a"), Some(Color::rgb(0x1c, 0x1f, 0x2a)));
        let faint = Color::parse("#00000040").unwrap();
        assert_eq!(faint.a, 0x40);
        assert_eq!(faint.to_string(), "#00000040");
        assert_eq!(Color::rgb(255, 0, 170).to_string(), "#ff00aa");
        for bad in ["1c1f2a", "#1c1f2", "#1c1f2g", "#fff", "red", "#1c1f2a8"] {
            assert_eq!(Color::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn colours_mix_and_know_how_light_they_are() {
        let black = Color::rgb(0, 0, 0);
        let white = Color::rgb(255, 255, 255);
        assert_eq!(black.mix(white, 0.5), Color::rgb(128, 128, 128));
        assert_eq!(black.mix(white, 0.0), black);
        assert_eq!(black.mix(white, 7.0), white);
        assert!(black.luminance() < 0.01);
        assert!(white.luminance() > 0.99);
        assert!(Color::rgb(0x14, 0x16, 0x1d).luminance() < 0.5);
    }

    #[test]
    fn each_kind_of_value_is_read_and_written_for_the_page_and_for_css() {
        let read = |kind, value: Value| TokenValue::parse(kind, &value).unwrap();

        let length = read(TokenKind::Length, json!(12));
        assert_eq!(length.to_json(), json!(12.0));
        assert_eq!(length.to_css(), "12px");
        assert_eq!(read(TokenKind::Length, json!(0.5)).to_css(), "0.5px");
        assert_eq!(read(TokenKind::Number, json!(1.2)).to_css(), "1.2");
        assert_eq!(read(TokenKind::Number, json!(400)).to_css(), "400");

        assert_eq!(
            read(TokenKind::FontFamily, json!("JetBrains Mono")).to_css(),
            "\"JetBrains Mono\""
        );
        assert_eq!(
            read(TokenKind::FontFamily, json!("system-ui")).to_css(),
            "system-ui"
        );
        let list = read(TokenKind::FontList, json!(["Cascadia Mono", "monospace"]));
        assert_eq!(list.to_css(), "\"Cascadia Mono\", monospace");
        assert_eq!(list.to_json(), json!(["Cascadia Mono", "monospace"]));

        let shadow = read(
            TokenKind::Shadow,
            json!("0 1 3 0 #00000040, inset 0 0 0 2 #7fb4f0"),
        );
        assert_eq!(
            shadow.to_css(),
            "0px 1px 3px 0px #00000040, inset 0px 0px 0px 2px #7fb4f0"
        );
        assert_eq!(shadow.to_json(), json!(shadow.to_css()));
        assert_eq!(read(TokenKind::Shadow, json!("none")).to_css(), "none");

        assert_eq!(read(TokenKind::Image, json!("")).to_json(), Value::Null);
        assert_eq!(read(TokenKind::Image, json!("")).to_css(), "none");
        assert_eq!(
            read(TokenKind::Image, json!("C:\\pics\\a \"b\".png")).to_css(),
            "url(\"C:\\\\pics\\\\a \\\"b\\\".png\")"
        );
    }

    #[test]
    fn a_value_of_the_wrong_kind_says_what_was_expected() {
        let refused = |kind, value: Value| TokenValue::parse(kind, &value).unwrap_err();
        assert!(refused(TokenKind::Color, json!("red")).contains("a colour such as"));
        assert!(refused(TokenKind::Color, json!(7)).contains("a colour such as"));
        assert_eq!(
            refused(TokenKind::Length, json!(-1)),
            "a length cannot be negative"
        );
        assert!(refused(TokenKind::Length, json!("12px")).contains("as a number"));
        assert!(refused(TokenKind::FontList, json!("Menlo")).contains("a list"));
        assert!(refused(TokenKind::FontList, json!(["Menlo", 3])).contains("a list"));
        assert!(refused(TokenKind::Shadow, json!("0 1 3 #000000")).contains("x y blur spread"));
        assert!(refused(TokenKind::Shadow, json!("0 1 -3 0 #000000")).contains("x y blur spread"));
        assert!(refused(TokenKind::FontFamily, json!("  ")).contains("the name of a font"));
    }
}
