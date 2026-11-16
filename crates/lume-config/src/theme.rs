//! Themes, and how a theme and the settings come to one value for every token.
//!
//! A theme is a TOML file that sets tokens. Two are built in, one dark and one light;
//! the user's own are in the `themes` folder of the configuration directory, each named
//! by its file. A theme need not set every token: what it leaves out comes from the
//! theme it names with `inherits`, and in the end from the built-in theme of its scheme.
//!
//! The settings come after the theme. Fonts are the settings' alone, since a theme
//! that changed the user's font would surprise them. The handful of appearance
//! settings (radius, gap, and so on) override a token each when they are set, and
//! `[appearance.tokens]` overrides any token at all.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lume_proto::ColorScheme;
use serde_json::{Map, Value, json};

use crate::layers::parse_document;
use crate::schema::Config;
use crate::source::{Problem, ProblemKind, Source, join_path};
use crate::suggest::nearest;
use crate::tokens::{
    FONT_GROUP, PALETTE, PALETTE_LEN, ResolvedTokens, TOKENS, TokenKind, TokenValue, token_def,
};
use crate::tuning::{FILE_MAX_BYTES, THEME_INHERIT_MAX};

/// The id of the built-in dark theme.
pub const DARK: &str = "lume-dark";

/// The id of the built-in light theme.
pub const LIGHT: &str = "lume-light";

const DARK_TEXT: &str = include_str!("themes/lume-dark.toml");
const LIGHT_TEXT: &str = include_str!("themes/lume-light.toml");

/// The keys at the top of a theme file that are about the theme, not tokens.
const NAME_KEY: &str = "name";
const SCHEME_KEY: &str = "scheme";
const INHERITS_KEY: &str = "inherits";

/// One theme.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    /// What the settings call it: the name of its file without `.toml`.
    pub id: String,
    /// What a person calls it.
    pub name: String,
    pub scheme: ColorScheme,
    /// The theme whose tokens this one starts from.
    pub inherits: Option<String>,
    /// The tokens the theme sets itself.
    pub values: BTreeMap<&'static str, TokenValue>,
    /// The file it was read from. The built-in themes have none.
    pub file: Option<PathBuf>,
    /// Whether the theme says which scheme it is. One that does not is what the theme
    /// it inherits from is.
    pub(crate) says_scheme: bool,
}

/// Something wrong with a token while its table is read: the token's path inside the
/// table, what kind of thing is wrong, the message, and the name that was meant.
type Report<'a> = &'a mut dyn FnMut(&[String], ProblemKind, String, Option<String>);

/// Whether tokens begin with `prefix` and a dot: whether it names a group of tokens.
pub(crate) fn is_group(prefix: &str) -> bool {
    TOKENS.iter().any(|token| {
        token
            .name
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('.'))
    })
}

/// The token or group that was probably meant by `name`.
pub(crate) fn meant(name: &str) -> Option<String> {
    let mut known: Vec<&str> = Vec::new();
    for token in TOKENS {
        known.push(token.name);
        let mut prefix = token.name;
        while let Some((group, _)) = prefix.rsplit_once('.') {
            if !known.contains(&group) {
                known.push(group);
            }
            prefix = group;
        }
    }
    nearest(name, known).map(str::to_owned)
}

fn read_palette(
    value: &Value,
    path: &[String],
    values: &mut BTreeMap<&'static str, TokenValue>,
    report: Report<'_>,
) {
    let Value::Array(colours) = value else {
        report(
            path,
            ProblemKind::InvalidValue,
            format!("`{PALETTE}`: expected a list of {PALETTE_LEN} colours"),
            None,
        );
        return;
    };
    if colours.len() > PALETTE_LEN {
        report(
            path,
            ProblemKind::InvalidValue,
            format!(
                "`{PALETTE}` has {} colours; only the first {PALETTE_LEN} are used",
                colours.len()
            ),
            None,
        );
    }
    for (index, colour) in colours.iter().take(PALETTE_LEN).enumerate() {
        let name = format!("{PALETTE}.{index}");
        let Some(token) = token_def(&name) else {
            continue;
        };
        match TokenValue::parse(TokenKind::Color, colour) {
            Ok(value) => {
                values.insert(token.name, value);
            }
            Err(message) => report(
                path,
                ProblemKind::InvalidValue,
                format!("`{PALETTE}`, colour {}: {message}", index + 1),
                None,
            ),
        }
    }
}

fn read_token(
    value: &Value,
    path: &mut Vec<String>,
    by_theme: bool,
    values: &mut BTreeMap<&'static str, TokenValue>,
    report: Report<'_>,
) {
    let name = path.join(".");
    if name == PALETTE && !value.is_object() {
        read_palette(value, path, values, &mut *report);
        return;
    }
    if let Some(token) = token_def(&name) {
        if by_theme && token.group() == FONT_GROUP {
            report(
                path,
                ProblemKind::InvalidValue,
                format!("`{name}`: fonts are set in [font] of config.toml, not by a theme"),
                None,
            );
            return;
        }
        match TokenValue::parse(token.kind, value) {
            Ok(value) => {
                values.insert(token.name, value);
            }
            Err(message) => report(
                path,
                ProblemKind::InvalidValue,
                format!("`{name}`: {message}"),
                None,
            ),
        }
        return;
    }
    match value {
        Value::Object(entries) if is_group(&name) => {
            for (key, entry) in entries {
                path.push(key.clone());
                read_token(entry, path, by_theme, values, &mut *report);
                path.pop();
            }
        }
        _ => report(
            path,
            ProblemKind::UnknownKey,
            format!("unknown token `{name}`"),
            meant(&name),
        ),
    }
}

/// Reads tokens out of a table written as a theme writes them: a table for each group,
/// the palette as one list. `by_theme` says whether the table is a theme's, which may
/// not set fonts.
fn read_tokens(
    tree: &Map<String, Value>,
    by_theme: bool,
    report: Report<'_>,
) -> BTreeMap<&'static str, TokenValue> {
    let mut values = BTreeMap::new();
    let mut path = Vec::new();
    for (key, value) in tree {
        path.push(key.clone());
        read_token(value, &mut path, by_theme, &mut values, &mut *report);
        path.pop();
    }
    values
}

fn scheme_of(name: &str) -> Option<ColorScheme> {
    match name {
        "light" => Some(ColorScheme::Light),
        "dark" => Some(ColorScheme::Dark),
        _ => None,
    }
}

/// Reads a theme file. `file` is where the text came from, for the messages. The
/// error is a file that is not TOML; anything else wrong is reported beside the theme,
/// which holds what was right.
pub fn parse_theme(
    id: &str,
    text: &str,
    file: Option<&Path>,
) -> Result<(Theme, Vec<Problem>), Problem> {
    let shown = file.map_or_else(|| PathBuf::from(format!("{id}.toml")), Path::to_owned);
    let mut document = parse_document(text).map_err(|error| error.problem(&shown))?;
    let positions = std::mem::take(&mut document.positions);
    let mut problems = Vec::new();
    let mut problem = |path: &[String], kind, message: String, suggestion: Option<String>| {
        let path = join_path(path);
        let position = positions.get(&path).copied().unwrap_or_default();
        problems.push(Problem {
            kind,
            path,
            file: Some(shown.clone()),
            line: position.line,
            column: position.column,
            message,
            suggestion,
        });
    };

    let mut string = |key: &str, tree: &mut Map<String, Value>| match tree.remove(key)? {
        Value::String(text) => Some(text),
        _ => {
            problem(
                &[key.to_owned()],
                ProblemKind::InvalidValue,
                format!("`{key}`: expected a string"),
                None,
            );
            None
        }
    };
    let name = string(NAME_KEY, &mut document.tree);
    let scheme = string(SCHEME_KEY, &mut document.tree);
    let inherits = string(INHERITS_KEY, &mut document.tree);
    let scheme = match scheme {
        None => None,
        Some(written) => {
            let scheme = scheme_of(&written);
            if scheme.is_none() {
                problem(
                    &[SCHEME_KEY.to_owned()],
                    ProblemKind::InvalidValue,
                    "`scheme`: expected \"light\" or \"dark\"".to_owned(),
                    None,
                );
            }
            scheme
        }
    };

    let values = read_tokens(&document.tree, true, &mut problem);
    let theme = Theme {
        id: id.to_owned(),
        name: name.unwrap_or_else(|| id.to_owned()),
        // A theme that does not say is taken for a dark one unless what it inherits
        // from says otherwise; that is settled when the themes are put together.
        scheme: scheme.unwrap_or(ColorScheme::Dark),
        inherits,
        values,
        file: file.map(Path::to_owned),
        says_scheme: scheme.is_some(),
    };
    Ok((theme, problems))
}

/// The themes there are to choose from.
#[derive(Clone, Debug, PartialEq)]
pub struct Themes {
    themes: BTreeMap<String, Arc<Theme>>,
}

impl Default for Themes {
    fn default() -> Self {
        Self::builtin()
    }
}

impl Themes {
    /// The two built-in themes.
    pub fn builtin() -> Self {
        let mut themes = BTreeMap::new();
        for (id, text) in [(DARK, DARK_TEXT), (LIGHT, LIGHT_TEXT)] {
            // The built-in themes are part of the program; a test holds them to being
            // complete and without fault.
            if let Ok((theme, _)) = parse_theme(id, text, None) {
                themes.insert(id.to_owned(), Arc::new(theme));
            }
        }
        Self { themes }
    }

    /// The built-in themes and the user's, from the `themes` folder at `dir`. A file
    /// that cannot be used is reported and left out.
    pub fn load(dir: &Path) -> (Self, Vec<Problem>) {
        let mut all = Self::builtin();
        let mut problems = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            // No folder is no themes of the user's own.
            return (all, problems);
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "toml")
            })
            .collect();
        files.sort();
        for file in files {
            let Some(id) = file.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let whole_file = |kind, message: String| Problem {
                kind,
                path: String::new(),
                file: Some(file.clone()),
                line: 0,
                column: 0,
                message,
                suggestion: None,
            };
            if id == DARK || id == LIGHT {
                problems.push(whole_file(
                    ProblemKind::InvalidValue,
                    format!(
                        "a theme cannot be called `{id}`, which is a built-in theme; give \
                         the file another name and let it inherit from `{id}`"
                    ),
                ));
                continue;
            }
            let text = match read_limited(&file) {
                Ok(text) => text,
                Err(error) => {
                    problems.push(whole_file(ProblemKind::Io, error));
                    continue;
                }
            };
            match parse_theme(id, &text, Some(&file)) {
                Ok((theme, found)) => {
                    problems.extend(found);
                    all.themes.insert(id.to_owned(), Arc::new(theme));
                }
                Err(problem) => problems.push(problem),
            }
        }
        all.settle_schemes();
        (all, problems)
    }

    /// Adds a theme, or replaces the one with its id.
    pub fn insert(&mut self, theme: Theme) {
        self.themes.insert(theme.id.clone(), Arc::new(theme));
        self.settle_schemes();
    }

    /// The theme called `id`.
    pub fn get(&self, id: &str) -> Option<&Theme> {
        self.themes.get(id).map(Arc::as_ref)
    }

    /// Every theme, in the order of their ids.
    pub fn iter(&self) -> impl Iterator<Item = &Theme> {
        self.themes.values().map(Arc::as_ref)
    }

    /// The built-in theme of a scheme.
    fn base(&self, scheme: ColorScheme) -> Option<&Theme> {
        self.get(match scheme {
            ColorScheme::Light => LIGHT,
            ColorScheme::Dark => DARK,
        })
    }

    /// Gives each theme that does not say which scheme it is the scheme of the theme
    /// it inherits from.
    fn settle_schemes(&mut self) {
        // Once for each step of the longest line of inheritance that is followed.
        for _ in 0..=THEME_INHERIT_MAX {
            let inherited: Vec<(String, ColorScheme)> = self
                .themes
                .values()
                .filter(|theme| !theme.says_scheme)
                .filter_map(|theme| {
                    let parent = self.get(theme.inherits.as_deref()?)?;
                    (parent.scheme != theme.scheme).then(|| (theme.id.clone(), parent.scheme))
                })
                .collect();
            if inherited.is_empty() {
                break;
            }
            for (id, scheme) in inherited {
                if let Some(theme) = self.themes.get_mut(&id) {
                    Arc::make_mut(theme).scheme = scheme;
                }
            }
        }
    }
}

/// Reads a file that somebody wrote by hand: text, and not enormous.
pub(crate) fn read_limited(file: &Path) -> Result<String, String> {
    let size = std::fs::metadata(file)
        .map_err(|error| error.to_string())?
        .len();
    if size > FILE_MAX_BYTES {
        return Err(format!(
            "the file is {size} bytes, which is more than a file written by hand is"
        ));
    }
    std::fs::read_to_string(file).map_err(|error| error.to_string())
}

/// A problem with the theme the settings name, placed where the settings name it.
fn at_setting(
    path: &str,
    sources: &BTreeMap<String, Source>,
    kind: ProblemKind,
    message: String,
    suggestion: Option<String>,
) -> Problem {
    let location = sources.get(path).and_then(Source::location);
    Problem {
        kind,
        path: path.to_owned(),
        file: location.map(|(file, _)| file.clone()),
        line: location.map_or(0, |(_, line)| line),
        column: 0,
        message,
        suggestion,
    }
}

/// The theme and everything it inherits from, the theme itself first.
fn lineage<'a>(
    theme: &'a Theme,
    themes: &'a Themes,
    problems: &mut Vec<Problem>,
) -> Vec<&'a Theme> {
    let mut chain = vec![theme];
    let mut current = theme;
    while let Some(parent) = current.inherits.as_deref() {
        let mut stop = |message: String| {
            problems.push(Problem {
                kind: ProblemKind::InvalidValue,
                path: INHERITS_KEY.to_owned(),
                file: current.file.clone(),
                line: 0,
                column: 0,
                message,
                suggestion: None,
            });
        };
        if chain.iter().any(|seen| seen.id == parent) {
            stop(format!(
                "the theme `{}` inherits from `{parent}`, which inherits from it",
                current.id
            ));
            break;
        }
        if chain.len() > THEME_INHERIT_MAX {
            stop(format!(
                "the theme `{}` inherits through more than {THEME_INHERIT_MAX} themes; \
                 the rest are left out",
                theme.id
            ));
            break;
        }
        let Some(found) = themes.get(parent) else {
            stop(format!(
                "the theme `{}` inherits from `{parent}`, and there is no such theme",
                current.id
            ));
            break;
        };
        chain.push(found);
        current = found;
    }
    chain
}

/// Brings a theme and the settings to one value for every token.
///
/// `system` is the colour scheme of the operating system, which chooses between the
/// light and the dark theme unless the settings name one outright. `sources` says where
/// each setting was written, so that a problem can point there.
pub fn resolve_tokens(
    config: &Config,
    sources: &BTreeMap<String, Source>,
    themes: &Themes,
    system: ColorScheme,
) -> (ResolvedTokens, Vec<Problem>) {
    let mut problems = Vec::new();
    let appearance = &config.appearance;
    let (wanted, setting) = match (&appearance.theme, system) {
        (Some(theme), _) => (theme, "appearance.theme"),
        (None, ColorScheme::Light) => (&appearance.theme_light, "appearance.theme-light"),
        (None, ColorScheme::Dark) => (&appearance.theme_dark, "appearance.theme-dark"),
    };
    let theme = themes.get(wanted).or_else(|| {
        let known = themes.iter().map(|theme| theme.id.as_str());
        problems.push(at_setting(
            setting,
            sources,
            ProblemKind::InvalidValue,
            format!("`{setting}`: there is no theme called `{wanted}`"),
            nearest(wanted, known).map(str::to_owned),
        ));
        themes.base(system)
    });

    let mut values: BTreeMap<&'static str, TokenValue> = BTreeMap::new();
    let (id, scheme) = match theme {
        Some(theme) => {
            if let Some(base) = themes.base(theme.scheme) {
                values.extend(base.values.clone());
            }
            let chain = lineage(theme, themes, &mut problems);
            for ancestor in chain.iter().rev() {
                values.extend(ancestor.values.clone());
            }
            (theme.id.clone(), theme.scheme)
        }
        // Without even the built-in themes there is nothing to start from.
        None => (wanted.clone(), system),
    };

    let font = &config.font;
    values.insert("font.family", TokenValue::FontFamily(font.family.clone()));
    values.insert("font.fallback", TokenValue::FontList(font.fallback.clone()));
    values.insert("font.size", TokenValue::Length(font.size));
    values.insert("font.line-height", TokenValue::Number(font.line_height));
    values.insert("font.weight", TokenValue::Number(f64::from(font.weight)));
    values.insert(
        "font.bold-weight",
        TokenValue::Number(f64::from(font.bold_weight)),
    );
    values.insert(
        "font.ui-family",
        TokenValue::FontFamily(font.ui_family.clone()),
    );
    values.insert("font.ui-size", TokenValue::Length(font.ui_size));

    let lengths = [
        ("radius.card", appearance.card_radius),
        ("space.gap", appearance.gap),
        ("background.blur", appearance.background_blur),
        ("space.padding", config.terminal.padding),
    ];
    for (name, set) in lengths {
        if let Some(length) = set {
            values.insert(name, TokenValue::Length(length));
        }
    }
    let numbers = [
        ("opacity.dim", appearance.dim_unfocused),
        ("opacity.background-image", appearance.background_opacity),
    ];
    for (name, set) in numbers {
        if let Some(number) = set {
            values.insert(name, TokenValue::Number(number));
        }
    }
    if let Some(image) = &appearance.background_image {
        values.insert(
            "background.image",
            TokenValue::Image(Some(image.clone()).filter(|image| !image.is_empty())),
        );
    }

    let overrides: Map<String, Value> = appearance.tokens.clone().into_iter().collect();
    let set = read_tokens(&overrides, false, &mut |path, kind, message, suggestion| {
        let mut whole = vec!["appearance".to_owned(), "tokens".to_owned()];
        whole.extend_from_slice(path);
        problems.push(at_setting(
            &join_path(&whole),
            sources,
            kind,
            message,
            suggestion,
        ));
    });
    values.extend(set);

    let resolved = ResolvedTokens {
        theme: id,
        scheme,
        values,
    };
    (resolved, problems)
}

/// The built-in dark theme with the default settings: what a page starts from.
pub fn default_tokens() -> ResolvedTokens {
    resolve_tokens(
        &Config::default(),
        &BTreeMap::new(),
        &Themes::builtin(),
        ColorScheme::Dark,
    )
    .0
}

/// The built-in light theme with the default settings.
pub fn light_tokens() -> ResolvedTokens {
    resolve_tokens(
        &Config::default(),
        &BTreeMap::new(),
        &Themes::builtin(),
        ColorScheme::Light,
    )
    .0
}

/// The JSON Schema of one token's value.
fn value_schema(kind: TokenKind, doc: &str) -> Value {
    match kind {
        TokenKind::Color => json!({
            "description": doc,
            "type": "string",
            "pattern": "^#([0-9a-fA-F]{6}|[0-9a-fA-F]{8})$",
        }),
        TokenKind::Length => json!({ "description": doc, "type": "number", "minimum": 0 }),
        TokenKind::Number => json!({ "description": doc, "type": "number" }),
        TokenKind::FontFamily | TokenKind::Shadow | TokenKind::Image => {
            json!({ "description": doc, "type": "string" })
        }
        TokenKind::FontList => json!({
            "description": doc,
            "type": "array",
            "items": { "type": "string" },
        }),
    }
}

/// Puts a token's schema where a theme file writes the token: inside a table for each
/// part of its name but the last.
fn insert_token(properties: &mut Map<String, Value>, segments: &[&str], schema: Value) {
    match segments {
        [] => {}
        [leaf] => {
            properties.insert((*leaf).to_owned(), schema);
        }
        [group, rest @ ..] => {
            let table = properties.entry((*group).to_owned()).or_insert_with(
                || json!({ "type": "object", "additionalProperties": false, "properties": {} }),
            );
            if let Some(inner) = table.get_mut("properties").and_then(Value::as_object_mut) {
                insert_token(inner, rest, schema);
            }
        }
    }
}

/// The JSON Schema of a theme file.
pub fn theme_schema() -> Value {
    let mut properties = Map::new();
    properties.insert(
        NAME_KEY.to_owned(),
        json!({ "description": "What the theme is called.", "type": "string" }),
    );
    properties.insert(
        SCHEME_KEY.to_owned(),
        json!({
            "description": "Whether the theme is a light or a dark one.",
            "type": "string",
            "enum": ["light", "dark"],
        }),
    );
    properties.insert(
        INHERITS_KEY.to_owned(),
        json!({
            "description": "The theme this one starts from. Left out, the built-in theme of its scheme.",
            "type": "string",
        }),
    );

    for token in TOKENS {
        if token.group() == FONT_GROUP || token.name.starts_with(&format!("{PALETTE}.")) {
            continue;
        }
        let segments: Vec<&str> = token.name.split('.').collect();
        insert_token(
            &mut properties,
            &segments,
            value_schema(token.kind, token.doc),
        );
    }
    if let Some(terminal) = properties
        .get_mut("terminal")
        .and_then(|terminal| terminal.get_mut("properties"))
        .and_then(Value::as_object_mut)
    {
        terminal.insert(
            "palette".to_owned(),
            json!({
                "description": "The sixteen ANSI colours: eight normal, then eight bright.",
                "type": "array",
                "items": value_schema(TokenKind::Color, "One colour."),
                "maxItems": PALETTE_LEN,
            }),
        );
    }

    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Lume theme",
        "description": "A theme: design tokens, written as a table for each group.",
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::{LayerInput, LayerKind, resolve_layers};
    use crate::testing::TempDir;
    use crate::tokens::Color;

    fn settings(text: &str) -> (Config, BTreeMap<String, Source>) {
        let layered = resolve_layers(&[LayerInput {
            kind: LayerKind::User,
            file: PathBuf::from("config.toml"),
            text: text.to_owned(),
        }]);
        assert_eq!(layered.problems, vec![], "{text}");
        assert_eq!(layered.fatal, vec![]);
        (layered.config, layered.sources)
    }

    fn resolve(text: &str, themes: &Themes, system: ColorScheme) -> (ResolvedTokens, Vec<Problem>) {
        let (config, sources) = settings(text);
        resolve_tokens(&config, &sources, themes, system)
    }

    fn color(tokens: &ResolvedTokens, name: &str) -> String {
        tokens.color(name).unwrap().to_string()
    }

    #[test]
    fn the_built_in_themes_are_without_fault_and_set_every_token_but_the_fonts() {
        for (id, text, scheme) in [
            (DARK, DARK_TEXT, ColorScheme::Dark),
            (LIGHT, LIGHT_TEXT, ColorScheme::Light),
        ] {
            let (theme, problems) = parse_theme(id, text, None).unwrap();
            assert_eq!(problems, vec![], "{id}");
            assert_eq!(theme.scheme, scheme);
            assert_eq!(theme.inherits, None);
            for token in TOKENS {
                let set = theme.values.contains_key(token.name);
                assert_eq!(set, token.group() != FONT_GROUP, "{id}: {}", token.name);
            }
        }
        assert_eq!(Themes::builtin().iter().count(), 2);
    }

    #[test]
    fn the_defaults_give_every_token_a_value() {
        let tokens = default_tokens();
        assert_eq!(tokens.theme, DARK);
        assert_eq!(tokens.scheme, ColorScheme::Dark);
        for token in TOKENS {
            assert!(tokens.get(token.name).is_some(), "{}", token.name);
        }
        assert_eq!(color(&tokens, "color.surface"), "#121416");
        assert_eq!(color(&tokens, "terminal.bg"), "#171a1e");
        assert_eq!(color(&tokens, "terminal.palette.9"), "#ffa59e");
        assert_eq!(
            tokens.get("font.family"),
            Some(&TokenValue::FontFamily("JetBrains Mono".to_owned()))
        );
        assert_eq!(tokens.get("space.gap"), Some(&TokenValue::Length(8.0)));
        assert_eq!(
            tokens.get("background.image"),
            Some(&TokenValue::Image(None))
        );
        assert_eq!(tokens.to_json().len(), TOKENS.len());
    }

    #[test]
    fn the_theme_follows_the_systems_scheme_unless_one_is_named() {
        let themes = Themes::builtin();
        let (dark, problems) = resolve("", &themes, ColorScheme::Dark);
        assert_eq!((dark.theme.as_str(), problems.len()), (DARK, 0));
        let (light, _) = resolve("", &themes, ColorScheme::Light);
        assert_eq!(light.theme, LIGHT);
        assert_eq!(light.scheme, ColorScheme::Light);
        assert_eq!(color(&light, "terminal.bg"), "#ffffff");

        let named = "[appearance]\ntheme = \"lume-light\"\n";
        let (forced, _) = resolve(named, &themes, ColorScheme::Dark);
        assert_eq!(forced.theme, LIGHT);

        let swapped = "[appearance]\ntheme-dark = \"lume-light\"\ntheme-light = \"lume-dark\"\n";
        let (swapped, _) = resolve(swapped, &themes, ColorScheme::Dark);
        assert_eq!(swapped.theme, LIGHT);
    }

    #[test]
    fn a_theme_that_does_not_exist_is_reported_where_it_is_named() {
        let (tokens, problems) = resolve(
            "[appearance]\ntheme = \"lume-drak\"\n",
            &Themes::builtin(),
            ColorScheme::Light,
        );
        // The built-in theme of the system's scheme stands in.
        assert_eq!(tokens.theme, LIGHT);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].path, "appearance.theme");
        assert_eq!(problems[0].line, 2);
        assert_eq!(problems[0].suggestion.as_deref(), Some(DARK));
        assert!(problems[0].message.contains("no theme called `lume-drak`"));
    }

    #[test]
    fn the_settings_override_the_theme_and_the_token_table_overrides_both() {
        let text = "\
[appearance]\ncard-radius = 4\ngap = 2\ndim-unfocused = 0.4\nbackground-image = \"wall.png\"\n\
background-blur = 10\nbackground-opacity = 0.5\n\
[appearance.tokens.radius]\ncard = 20\n\
[appearance.tokens.color]\naccent = \"#FF8800\"\n\
[appearance.tokens.color.status]\ndanger = \"#ff000080\"\n\
[appearance.tokens.terminal]\npalette = [\"#010101\", \"#020202\"]\n\
[font]\nfamily = \"Iosevka\"\nsize = 16\nweight = 500\n\
[terminal]\npadding = 9\n";
        let (tokens, problems) = resolve(text, &Themes::builtin(), ColorScheme::Dark);
        assert_eq!(problems, vec![]);
        assert_eq!(tokens.get("radius.card"), Some(&TokenValue::Length(20.0)));
        assert_eq!(tokens.get("space.gap"), Some(&TokenValue::Length(2.0)));
        assert_eq!(tokens.get("opacity.dim"), Some(&TokenValue::Number(0.4)));
        assert_eq!(
            tokens.get("background.blur"),
            Some(&TokenValue::Length(10.0))
        );
        assert_eq!(
            tokens.get("opacity.background-image"),
            Some(&TokenValue::Number(0.5))
        );
        assert_eq!(
            tokens.get("background.image"),
            Some(&TokenValue::Image(Some("wall.png".to_owned())))
        );
        assert_eq!(color(&tokens, "color.accent"), "#ff8800");
        assert_eq!(color(&tokens, "color.status.danger"), "#ff000080");
        assert_eq!(color(&tokens, "terminal.palette.1"), "#020202");
        // The colours the short list leaves out are still the theme's.
        assert_eq!(color(&tokens, "terminal.palette.2"), "#8fd694");
        assert_eq!(
            tokens.get("font.family"),
            Some(&TokenValue::FontFamily("Iosevka".to_owned()))
        );
        assert_eq!(tokens.get("font.size"), Some(&TokenValue::Length(16.0)));
        assert_eq!(tokens.get("font.weight"), Some(&TokenValue::Number(500.0)));
        assert_eq!(tokens.get("space.padding"), Some(&TokenValue::Length(9.0)));
    }

    #[test]
    fn a_token_override_that_is_wrong_is_reported_at_its_line_and_passed_over() {
        let text = "\
[appearance.tokens.color]\nacent = \"#ff8800\"\nsurface = \"blue\"\n\
[appearance.tokens.colour]\ntext = \"#000000\"\n";
        let (tokens, problems) = resolve(text, &Themes::builtin(), ColorScheme::Dark);
        assert_eq!(color(&tokens, "color.surface"), "#121416");
        assert_eq!(problems.len(), 3, "{problems:?}");

        let unknown = &problems[0];
        assert_eq!(unknown.kind, ProblemKind::UnknownKey);
        assert_eq!(unknown.path, "appearance.tokens.color.acent");
        assert_eq!(unknown.line, 2);
        assert_eq!(unknown.message, "unknown token `color.acent`");
        assert_eq!(unknown.suggestion.as_deref(), Some("color.accent"));

        let invalid = &problems[1];
        assert_eq!(invalid.kind, ProblemKind::InvalidValue);
        assert_eq!(invalid.line, 3);
        assert!(
            invalid
                .message
                .starts_with("`color.surface`: expected a colour")
        );

        let group = &problems[2];
        assert_eq!(group.message, "unknown token `colour`");
        assert_eq!(group.suggestion.as_deref(), Some("color"));
    }

    fn theme(id: &str, text: &str) -> Theme {
        let file = PathBuf::from(format!("{id}.toml"));
        let (theme, problems) = parse_theme(id, text, Some(&file)).unwrap();
        assert_eq!(problems, vec![], "{id}");
        theme
    }

    #[test]
    fn a_theme_of_the_users_needs_only_the_tokens_it_changes() {
        let mut themes = Themes::builtin();
        themes.insert(theme(
            "paper",
            "name = \"Paper\"\nscheme = \"light\"\n[color]\naccent = \"#aa5500\"\n\
             [terminal]\nbg = \"#fdf6e3\"\n",
        ));
        let named = "[appearance]\ntheme = \"paper\"\n";
        let (tokens, problems) = resolve(named, &themes, ColorScheme::Dark);
        assert_eq!(problems, vec![]);
        assert_eq!(tokens.theme, "paper");
        assert_eq!(tokens.scheme, ColorScheme::Light);
        assert_eq!(color(&tokens, "color.accent"), "#aa5500");
        assert_eq!(color(&tokens, "terminal.bg"), "#fdf6e3");
        // The rest is the built-in light theme's.
        assert_eq!(color(&tokens, "color.card"), "#ffffff");
        assert_eq!(themes.get("paper").unwrap().name, "Paper");
    }

    #[test]
    fn a_theme_starts_from_the_one_it_inherits_and_takes_its_scheme() {
        let dir = TempDir::new("inherits");
        dir.write(
            "themes/base.toml",
            "scheme = \"light\"\n[color]\naccent = \"#111111\"\ntext = \"#222222\"\n",
        );
        dir.write(
            "themes/child.toml",
            "inherits = \"base\"\n[color]\ntext = \"#333333\"\n",
        );
        let (themes, problems) = Themes::load(&dir.path().join("themes"));
        assert_eq!(problems, vec![]);
        // The child does not say, so it is what its parent is.
        assert_eq!(themes.get("child").unwrap().scheme, ColorScheme::Light);

        let named = "[appearance]\ntheme = \"child\"\n";
        let (tokens, problems) = resolve(named, &themes, ColorScheme::Dark);
        assert_eq!(problems, vec![]);
        assert_eq!(color(&tokens, "color.accent"), "#111111");
        assert_eq!(color(&tokens, "color.text"), "#333333");
        assert_eq!(color(&tokens, "color.card"), "#ffffff");
    }

    #[test]
    fn themes_that_inherit_in_a_circle_or_from_nothing_are_reported() {
        let mut themes = Themes::builtin();
        themes.insert(theme(
            "a",
            "inherits = \"b\"\n[color]\naccent = \"#111111\"\n",
        ));
        themes.insert(theme(
            "b",
            "inherits = \"a\"\n[color]\ntext = \"#222222\"\n",
        ));
        let (tokens, problems) =
            resolve("[appearance]\ntheme = \"a\"\n", &themes, ColorScheme::Dark);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("which inherits from it"));
        // Both still count, once each.
        assert_eq!(color(&tokens, "color.accent"), "#111111");
        assert_eq!(color(&tokens, "color.text"), "#222222");

        themes.insert(theme("orphan", "inherits = \"nobody\"\n"));
        let (_, problems) = resolve(
            "[appearance]\ntheme = \"orphan\"\n",
            &themes,
            ColorScheme::Dark,
        );
        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("there is no such theme"));
    }

    #[test]
    fn a_long_line_of_inheritance_is_cut_off_and_said_to_be() {
        let mut themes = Themes::builtin();
        let depth = THEME_INHERIT_MAX + 3;
        for index in 0..depth {
            let text = format!("inherits = \"t{}\"\n[space]\ngap = {index}\n", index + 1);
            themes.insert(theme(&format!("t{index}"), &text));
        }
        themes.insert(theme(&format!("t{depth}"), "[space]\ngap = 99\n"));
        let (tokens, problems) =
            resolve("[appearance]\ntheme = \"t0\"\n", &themes, ColorScheme::Dark);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("more than"));
        assert_eq!(tokens.get("space.gap"), Some(&TokenValue::Length(0.0)));
    }

    #[test]
    fn what_is_wrong_in_a_theme_file_is_reported_with_its_place() {
        let file = PathBuf::from("mine.toml");
        let text = "\
scheme = \"dusk\"\nname = 7\n\n[color]\nsurfase = \"#000000\"\ncard = \"black\"\n\n\
[font]\nfamily = \"Comic Sans MS\"\n\n[terminal]\npalette = [\"#000000\", \"nope\"]\n";
        let (theme, problems) = parse_theme("mine", text, Some(&file)).unwrap();
        assert_eq!(theme.name, "mine");
        assert_eq!(theme.scheme, ColorScheme::Dark);
        assert_eq!(theme.values.len(), 1, "{:?}", theme.values);

        let said: Vec<(u32, &str)> = problems
            .iter()
            .map(|problem| (problem.line, problem.message.as_str()))
            .collect();
        assert_eq!(said.len(), 6, "{said:?}");
        assert!(said.contains(&(2, "`name`: expected a string")));
        assert!(said.contains(&(1, "`scheme`: expected \"light\" or \"dark\"")));
        assert!(said.contains(&(5, "unknown token `color.surfase`")));
        assert!(
            said.iter()
                .any(|(line, message)| *line == 6 && message.starts_with("`color.card`: expected"))
        );
        assert!(said.contains(&(
            9,
            "`font.family`: fonts are set in [font] of config.toml, not by a theme"
        )));
        assert!(
            said.iter()
                .any(|(line, message)| *line == 12 && message.contains("colour 2"))
        );
        let unknown = problems
            .iter()
            .find(|problem| problem.kind == ProblemKind::UnknownKey)
            .unwrap();
        assert_eq!(unknown.suggestion.as_deref(), Some("color.surface"));
        assert_eq!(unknown.file.as_deref(), Some(file.as_path()));
    }

    #[test]
    fn a_theme_file_that_is_not_toml_is_no_theme() {
        let problem = parse_theme("bad", "[color\n", None).unwrap_err();
        assert_eq!(problem.kind, ProblemKind::Syntax);
        assert_eq!(problem.line, 1);
        assert_eq!(problem.file.as_deref(), Some(Path::new("bad.toml")));
    }

    #[test]
    fn the_users_folder_adds_themes_and_cannot_replace_the_built_in_ones() {
        let dir = TempDir::new("themes");
        dir.write(
            "themes/sea.toml",
            "name = \"Sea\"\n[color]\naccent = \"#00aaff\"\n",
        );
        dir.write("themes/broken.toml", "[color\n");
        dir.write("themes/lume-dark.toml", "[color]\naccent = \"#ff0000\"\n");
        dir.write("themes/notes.txt", "not a theme");
        let (themes, problems) = Themes::load(&dir.path().join("themes"));

        let ids: Vec<&str> = themes.iter().map(|theme| theme.id.as_str()).collect();
        assert_eq!(ids, [DARK, LIGHT, "sea"]);
        assert_eq!(
            themes.get(DARK).unwrap().values["color.accent"],
            TokenValue::Color(Color::parse("#7fe3b8").unwrap())
        );
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert_eq!(problems[0].kind, ProblemKind::Syntax);
        assert!(problems[1].message.contains("built-in theme"));

        // No folder at all is not a problem.
        let (none, problems) = Themes::load(&dir.path().join("nowhere"));
        assert_eq!(none, Themes::builtin());
        assert_eq!(problems, vec![]);
    }

    #[test]
    fn the_schema_of_a_theme_has_every_token_a_theme_may_set() {
        let schema = theme_schema();
        let properties = &schema["properties"];
        assert_eq!(
            properties["color"]["properties"]["surface"]["type"],
            "string"
        );
        assert_eq!(
            properties["color"]["properties"]["status"]["properties"]["danger"]["type"],
            "string"
        );
        assert_eq!(properties["space"]["properties"]["gap"]["minimum"], 0);
        assert_eq!(
            properties["terminal"]["properties"]["palette"]["maxItems"],
            PALETTE_LEN
        );
        assert!(properties.get("font").is_none());
        assert!(
            properties["terminal"]["properties"]
                .get("palette.0")
                .is_none()
        );
        assert_eq!(properties["scheme"]["enum"], json!(["light", "dark"]));
    }
}
