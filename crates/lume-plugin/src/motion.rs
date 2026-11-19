//! Motion a plugin brings: effects for fixed places in the window, written as data.
//!
//! A plugin never gives the window styles of its own. It gives keyframes of a few
//! properties (opacity, parts of a transform, the blur and brightness filters, colours
//! named by design tokens), a length, a curve and, for the mark of the tab on show, the
//! springs it moves with. Everything is checked here before the window sees it, and
//! what the window is given is the checked form: lengths and curves it can hand
//! straight to the browser's animations, colours as the tokens' variables.
//!
//! An effect that brings something in ends where the thing rests, so that nothing
//! jumps when it is over; one that takes something away begins there. An effect plays
//! once, never longer than a second, and leaves nothing behind.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// The longest an effect may last, and wait before it starts, in milliseconds.
pub const DURATION_MAX_MS: u32 = 1000;
pub const DELAY_MAX_MS: u32 = 500;

/// How many keyframes an effect may have.
const KEYFRAMES_MIN: usize = 2;
const KEYFRAMES_MAX: usize = 8;

/// The furthest a curve may overshoot, below and above.
const BEZIER_Y_MIN: f64 = -2.0;
const BEZIER_Y_MAX: f64 = 3.0;

/// The most steps a stepped curve may have.
const STEPS_MAX: u32 = 60;

/// What a spring may be: its stiffness and its damping ratio.
const STIFFNESS: (f64, f64) = (50.0, 2000.0);
const DAMPING: (f64, f64) = (0.1, 2.0);

/// How far a transform may move, scale, turn and slant something.
const TRANSLATE_PX_MAX: f64 = 200.0;
const TRANSLATE_PERCENT_MAX: f64 = 100.0;
const SCALE: (f64, f64) = (0.0, 4.0);
const ROTATE_DEG_MAX: f64 = 360.0;
const SKEW_DEG_MAX: f64 = 45.0;

/// How far the filters may go.
const BLUR_PX_MAX: f64 = 20.0;
const BRIGHTNESS_MAX: f64 = 3.0;

/// The curves of the application's own motion, by the names of their tokens.
pub const EASING_TOKENS: [&str; 3] = ["standard", "emphasized", "exit"];

/// The curves CSS names.
const NAMED_EASINGS: [&str; 5] = ["linear", "ease", "ease-in", "ease-out", "ease-in-out"];

/// The places that take something away rather than bring it in.
const EXITS: [&str; 1] = ["toast.exit"];

/// One keyframe, as a manifest writes it.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Keyframe {
    /// Where in the effect the keyframe is, from 0 to 1; spread evenly if not given.
    #[serde(default)]
    pub offset: Option<f64>,
    /// From 0, unseen, to 1.
    #[serde(default)]
    pub opacity: Option<f64>,
    /// Moves, scales, turns or slants: `translateY(-4px) scale(0.96)`.
    #[serde(default)]
    pub transform: Option<String>,
    /// `blur(2px)`, `brightness(1.4)`, or both.
    #[serde(default)]
    pub filter: Option<String>,
    /// The text's colour, as a colour token: `token(color.accent)`.
    #[serde(default)]
    pub color: Option<String>,
    /// The background's colour, as a colour token.
    #[serde(default)]
    pub background: Option<String>,
}

/// How tabs slide to their new places: along a spring, or along a curve for a time.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Slide {
    /// `"spring"`.
    Spring(String),
    Curve(SlideCurve),
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct SlideCurve {
    pub duration: u32,
    pub easing: String,
}

/// How a new tab comes in.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Enter {
    pub keyframes: Vec<Keyframe>,
    /// How long it takes, for one that is not moved by a spring.
    #[serde(default)]
    pub duration: Option<u32>,
    /// The spring it is moved by: its stiffness and damping.
    #[serde(default)]
    pub spring: Option<[f64; 2]>,
}

/// How the mark of the tab on show moves, as the window's own styles do.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Highlight {
    /// `pill`, `underline` or `glow`.
    pub shape: String,
    /// The spring of the end that leads: its stiffness and damping.
    pub lead: [f64; 2],
    /// The spring of the end that trails.
    pub trail: [f64; 2],
    pub slide: Slide,
    pub enter: Enter,
    /// How a renamed label settles; nothing if not given.
    #[serde(default)]
    pub renamed: Option<Vec<Keyframe>>,
}

/// One effect a plugin offers for one of the window's places.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Motion {
    pub id: String,
    /// What the settings call it.
    pub title: String,
    /// The place it plays in: `tab.highlight`, `menu.enter`, `toast.enter`,
    /// `toast.exit`, `palette.enter`, `panel.enter`, `float.enter`, `pill.enter` or
    /// `status.change`.
    pub slot: String,
    #[serde(default)]
    pub keyframes: Vec<Keyframe>,
    /// How long it takes, in milliseconds: 1000 at most.
    #[serde(default)]
    pub duration: Option<u32>,
    /// Its curve: `linear`, `ease-out`, one of the application's (`standard`,
    /// `emphasized`, `exit`), `cubic-bezier(…)`, `steps(…)` or `spring(stiffness,
    /// damping)`.
    #[serde(default)]
    pub easing: Option<String>,
    /// How long it waits before it starts, in milliseconds: 500 at most.
    #[serde(default)]
    pub delay: Option<u32>,
    /// For `tab.highlight`: how the mark moves.
    #[serde(default)]
    pub highlight: Option<Highlight>,
}

/// A curve, read.
#[derive(Clone, Debug, PartialEq)]
pub enum Easing {
    /// What CSS takes as it is.
    Css(String),
    /// One of the application's curves, by the name of its token.
    Token(String),
    Spring(f64, f64),
}

impl Easing {
    /// As the window is sent it.
    pub fn to_json(&self) -> Value {
        match self {
            Self::Css(css) => json!({ "css": css }),
            Self::Token(name) => json!({ "token": name }),
            Self::Spring(stiffness, damping) => json!({ "spring": [stiffness, damping] }),
        }
    }
}

/// Reads a curve.
pub fn parse_easing(written: &str) -> Result<Easing, String> {
    let written = written.trim();
    if NAMED_EASINGS.contains(&written) {
        return Ok(Easing::Css(written.to_owned()));
    }
    if EASING_TOKENS.contains(&written) {
        return Ok(Easing::Token(written.to_owned()));
    }
    let (name, args) = function(written).ok_or_else(|| {
        format!(
            "`{written}` is not a curve: linear, ease, ease-in, ease-out, ease-in-out, \
             standard, emphasized, exit, cubic-bezier(…), steps(…) or spring(…)"
        )
    })?;
    match name {
        "cubic-bezier" => {
            let numbers = numbers(&args, 4, written)?;
            let (x1, y1, x2, y2) = (numbers[0], numbers[1], numbers[2], numbers[3]);
            if !(0.0..=1.0).contains(&x1) || !(0.0..=1.0).contains(&x2) {
                return Err(format!(
                    "`{written}`: the first and third numbers are between 0 and 1"
                ));
            }
            let y = |value: f64| value.clamp(BEZIER_Y_MIN, BEZIER_Y_MAX);
            Ok(Easing::Css(format!(
                "cubic-bezier({}, {}, {}, {})",
                plain(x1),
                plain(y(y1)),
                plain(x2),
                plain(y(y2))
            )))
        }
        "steps" => {
            let parts: Vec<&str> = args.split(',').map(str::trim).collect();
            let count: u32 = parts
                .first()
                .and_then(|count| count.parse().ok())
                .filter(|count| (1..=STEPS_MAX).contains(count))
                .ok_or_else(|| format!("`{written}`: from 1 to {STEPS_MAX} steps"))?;
            let position = match parts.get(1).copied() {
                None => None,
                Some(
                    position @ ("start" | "end" | "jump-start" | "jump-end" | "jump-none"
                    | "jump-both"),
                ) => Some(position),
                Some(_) => return Err(format!("`{written}` is not a stepped curve")),
            };
            if parts.len() > 2 {
                return Err(format!("`{written}` is not a stepped curve"));
            }
            Ok(Easing::Css(match position {
                Some(position) => format!("steps({count}, {position})"),
                None => format!("steps({count})"),
            }))
        }
        "spring" => {
            let numbers = numbers(&args, 2, written)?;
            check_spring([numbers[0], numbers[1]]).map_err(|why| format!("`{written}`: {why}"))?;
            Ok(Easing::Spring(numbers[0], numbers[1]))
        }
        _ => Err(format!("`{written}` is not a curve")),
    }
}

fn check_spring([stiffness, damping]: [f64; 2]) -> Result<(), String> {
    if !(STIFFNESS.0..=STIFFNESS.1).contains(&stiffness) {
        return Err(format!(
            "a spring's stiffness is from {} to {}",
            STIFFNESS.0, STIFFNESS.1
        ));
    }
    if !(DAMPING.0..=DAMPING.1).contains(&damping) {
        return Err(format!(
            "a spring's damping is from {} to {}",
            DAMPING.0, DAMPING.1
        ));
    }
    Ok(())
}

/// A number as CSS writes it, without needless digits.
fn plain(number: f64) -> String {
    let rounded = (number * 10_000.0).round() / 10_000.0;
    let text = format!("{rounded}");
    if text == "-0" { "0".to_owned() } else { text }
}

/// A function written as `name(args)`.
fn function(written: &str) -> Option<(&str, String)> {
    let open = written.find('(')?;
    let name = written[..open].trim();
    let rest = written[open + 1..].trim_end();
    let args = rest.strip_suffix(')')?;
    if name.is_empty() || args.contains('(') || args.contains(')') {
        return None;
    }
    Some((name, args.to_owned()))
}

fn numbers(args: &str, count: usize, written: &str) -> Result<Vec<f64>, String> {
    let numbers: Option<Vec<f64>> = args
        .split(',')
        .map(|part| part.trim().parse::<f64>().ok().filter(|n| n.is_finite()))
        .collect();
    match numbers {
        Some(numbers) if numbers.len() == count => Ok(numbers),
        _ => Err(format!("`{written}` takes {count} numbers")),
    }
}

/// The functions of a list such as a transform: `name(args) name(args)`.
fn functions(written: &str) -> Option<Vec<(String, String)>> {
    let mut found = Vec::new();
    let mut rest = written.trim();
    while !rest.is_empty() {
        let open = rest.find('(')?;
        let close = rest.find(')')?;
        if close < open {
            return None;
        }
        let name = rest[..open].trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        found.push((name.to_owned(), rest[open + 1..close].trim().to_owned()));
        rest = rest[close + 1..].trim_start();
    }
    Some(found)
}

/// A length with its unit: pixels or a share of the thing's own size.
fn length(text: &str) -> Option<(f64, &'static str)> {
    let text = text.trim();
    if let Some(number) = text.strip_suffix("px") {
        return number.trim().parse().ok().map(|n| (n, "px"));
    }
    if let Some(number) = text.strip_suffix('%') {
        return number.trim().parse().ok().map(|n| (n, "%"));
    }
    // A bare nought is a length too.
    (text == "0").then_some((0.0, "px"))
}

fn angle(text: &str) -> Option<f64> {
    let text = text.trim();
    text.strip_suffix("deg")
        .and_then(|number| number.trim().parse().ok())
        .or_else(|| (text == "0").then_some(0.0))
}

/// Checks a transform, and says whether it leaves the thing as it is.
fn check_transform(written: &str) -> Result<bool, String> {
    if written.trim() == "none" {
        return Ok(true);
    }
    let parts = functions(written).ok_or_else(|| format!("`{written}` is not a transform"))?;
    if parts.is_empty() {
        return Err(format!("`{written}` is not a transform"));
    }
    let mut identity = true;
    for (name, args) in parts {
        let args: Vec<&str> = args.split(',').map(str::trim).collect();
        let fits_length = |text: &str| -> Result<f64, String> {
            let (value, unit) = length(text)
                .filter(|(value, _)| value.is_finite())
                .ok_or_else(|| format!("`{text}` in `{written}` is not a length in px or %"))?;
            let most = if unit == "px" {
                TRANSLATE_PX_MAX
            } else {
                TRANSLATE_PERCENT_MAX
            };
            if value.abs() > most {
                return Err(format!(
                    "`{text}` in `{written}` moves further than {most}{unit}"
                ));
            }
            Ok(value)
        };
        let fits_scale = |text: &str| -> Result<f64, String> {
            text.parse::<f64>()
                .ok()
                .filter(|value| (SCALE.0..=SCALE.1).contains(value))
                .ok_or_else(|| {
                    format!(
                        "`{text}` in `{written}` is not a scale from {} to {}",
                        SCALE.0, SCALE.1
                    )
                })
        };
        let fits_angle = |text: &str, most: f64| -> Result<f64, String> {
            angle(text)
                .filter(|value| value.abs() <= most)
                .ok_or_else(|| format!("`{text}` in `{written}` is not an angle within {most}deg"))
        };
        let count = args.len();
        match (name.as_str(), count) {
            ("translate", 1 | 2) => {
                for arg in &args {
                    identity &= fits_length(arg)? == 0.0;
                }
            }
            ("translateX" | "translateY", 1) => identity &= fits_length(args[0])? == 0.0,
            ("scale", 1 | 2) => {
                for arg in &args {
                    identity &= fits_scale(arg)? == 1.0;
                }
            }
            ("scaleX" | "scaleY", 1) => identity &= fits_scale(args[0])? == 1.0,
            ("rotate", 1) => identity &= fits_angle(args[0], ROTATE_DEG_MAX)? == 0.0,
            ("skewX" | "skewY", 1) => identity &= fits_angle(args[0], SKEW_DEG_MAX)? == 0.0,
            _ => {
                return Err(format!(
                    "`{name}` in `{written}` cannot be used: translate, translateX, translateY, \
                     scale, scaleX, scaleY, rotate, skewX and skewY can"
                ));
            }
        }
    }
    Ok(identity)
}

/// Checks a filter, and says whether it leaves the thing as it is. Only blurring and
/// brightening are allowed: nothing that names another document.
fn check_filter(written: &str) -> Result<bool, String> {
    if written.trim() == "none" {
        return Ok(true);
    }
    let parts = functions(written).ok_or_else(|| format!("`{written}` is not a filter"))?;
    if parts.is_empty() {
        return Err(format!("`{written}` is not a filter"));
    }
    let mut identity = true;
    for (name, args) in parts {
        match name.as_str() {
            "blur" => {
                let (value, unit) = length(&args)
                    .filter(|(value, unit)| *unit == "px" && (0.0..=BLUR_PX_MAX).contains(value))
                    .ok_or_else(|| format!("`blur({args})` blurs from 0px to {BLUR_PX_MAX}px"))?;
                let _ = unit;
                identity &= value == 0.0;
            }
            "brightness" => {
                let value = args
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|value| (0.0..=BRIGHTNESS_MAX).contains(value))
                    .ok_or_else(|| format!("`brightness({args})` is from 0 to {BRIGHTNESS_MAX}"))?;
                identity &= value == 1.0;
            }
            _ => {
                return Err(format!(
                    "`{name}` cannot be used in a filter: blur and brightness can"
                ));
            }
        }
    }
    Ok(identity)
}

/// A colour named by a token, as its CSS variable.
fn token_color(written: &str) -> Result<String, String> {
    let name = function(written.trim())
        .filter(|(name, _)| *name == "token")
        .map(|(_, args)| args.trim().to_owned())
        .ok_or_else(|| {
            format!("`{written}` is not a colour of the theme, written as token(color.accent)")
        })?;
    match lume_config::tokens::token_def(&name) {
        Some(token) if token.kind == lume_config::tokens::TokenKind::Color => {
            Ok(format!("var({})", token.css_variable()))
        }
        _ => Err(format!("`{name}` is not one of the theme's colours")),
    }
}

/// Checks keyframes and gives them as the window takes them. `ends_still` says the last
/// must leave the thing as it is, `starts_still` the first.
fn keyframes(
    frames: &[Keyframe],
    ends_still: bool,
    starts_still: bool,
    key: &str,
) -> Result<Value, String> {
    if !(KEYFRAMES_MIN..=KEYFRAMES_MAX).contains(&frames.len()) {
        return Err(format!(
            "{key}: from {KEYFRAMES_MIN} to {KEYFRAMES_MAX} keyframes"
        ));
    }
    let mut out = Vec::new();
    let mut last_offset = -1.0;
    for (index, frame) in frames.iter().enumerate() {
        let at = format!("{key}[{index}]");
        let mut written = serde_json::Map::new();
        let mut still = true;
        if let Some(offset) = frame.offset {
            if !(0.0..=1.0).contains(&offset) || offset < last_offset {
                return Err(format!("{at}.offset: offsets go up from 0 to 1"));
            }
            last_offset = offset;
            written.insert("offset".to_owned(), json!(offset));
        }
        if let Some(opacity) = frame.opacity {
            if !(0.0..=1.0).contains(&opacity) {
                return Err(format!("{at}.opacity: from 0 to 1"));
            }
            still &= opacity == 1.0;
            written.insert("opacity".to_owned(), json!(opacity));
        }
        if let Some(transform) = &frame.transform {
            still &= check_transform(transform).map_err(|why| format!("{at}.transform: {why}"))?;
            written.insert("transform".to_owned(), json!(transform.trim()));
        }
        if let Some(filter) = &frame.filter {
            still &= check_filter(filter).map_err(|why| format!("{at}.filter: {why}"))?;
            written.insert("filter".to_owned(), json!(filter.trim()));
        }
        if let Some(color) = &frame.color {
            still = false;
            written.insert(
                "color".to_owned(),
                json!(token_color(color).map_err(|why| format!("{at}.color: {why}"))?),
            );
        }
        if let Some(background) = &frame.background {
            still = false;
            written.insert(
                "backgroundColor".to_owned(),
                json!(token_color(background).map_err(|why| format!("{at}.background: {why}"))?),
            );
        }
        let first = index == 0;
        let last = index + 1 == frames.len();
        if last && ends_still && !still {
            return Err(format!(
                "{at}: an effect that brings something in ends with it as it rests: opacity \
                 1, no transform, no filter and no colour"
            ));
        }
        if first && starts_still && !still {
            return Err(format!(
                "{at}: an effect that takes something away begins with it as it rests"
            ));
        }
        out.push(Value::Object(written));
    }
    Ok(Value::Array(out))
}

/// Checks a plugin's effect, and gives it as the window takes it: the place, the
/// keyframes, the length and the curve, or the springs of the tab's mark.
pub fn check(motion: &Motion, key: &str) -> Result<Value, String> {
    if !crate::places::MOTION_SLOTS.contains(&motion.slot.as_str()) {
        return Err(format!(
            "{key}.slot: `{}` is not a place for motion; they are {}",
            motion.slot,
            crate::places::MOTION_SLOTS.join(", ")
        ));
    }
    if motion.slot == "tab.highlight" {
        let highlight = motion.highlight.as_ref().ok_or_else(|| {
            format!("{key}: the tab's mark moves as `highlight` says, which is needed")
        })?;
        if !motion.keyframes.is_empty() || motion.duration.is_some() || motion.easing.is_some() {
            return Err(format!(
                "{key}: the tab's mark is described by `highlight` alone"
            ));
        }
        return check_highlight(highlight, &format!("{key}.highlight"));
    }
    if motion.highlight.is_some() {
        return Err(format!("{key}.highlight: only for the tab.highlight place"));
    }
    let duration = motion
        .duration
        .ok_or_else(|| format!("{key}.duration: how long it takes is needed"))?;
    if duration == 0 || duration > DURATION_MAX_MS {
        return Err(format!(
            "{key}.duration: from 1 to {DURATION_MAX_MS} milliseconds"
        ));
    }
    let delay = motion.delay.unwrap_or(0);
    if delay > DELAY_MAX_MS {
        return Err(format!("{key}.delay: {DELAY_MAX_MS} milliseconds at most"));
    }
    let easing = match &motion.easing {
        Some(easing) => parse_easing(easing).map_err(|why| format!("{key}.easing: {why}"))?,
        None => Easing::Token("standard".to_owned()),
    };
    let exits = EXITS.contains(&motion.slot.as_str());
    let frames = keyframes(
        &motion.keyframes,
        !exits,
        exits,
        &format!("{key}.keyframes"),
    )?;
    Ok(json!({
        "slot": motion.slot,
        "keyframes": frames,
        "duration": duration,
        "delay": delay,
        "easing": easing.to_json(),
    }))
}

fn check_highlight(highlight: &Highlight, key: &str) -> Result<Value, String> {
    if !["pill", "underline", "glow"].contains(&highlight.shape.as_str()) {
        return Err(format!(
            "{key}.shape: `{}` is not a shape: pill, underline or glow",
            highlight.shape
        ));
    }
    for (side, spring) in [("lead", highlight.lead), ("trail", highlight.trail)] {
        check_spring(spring).map_err(|why| format!("{key}.{side}: {why}"))?;
    }
    let slide = match &highlight.slide {
        Slide::Spring(word) if word == "spring" => json!("spring"),
        Slide::Spring(word) => {
            return Err(format!(
                "{key}.slide: `{word}` is not a slide: \"spring\", or a duration and an easing"
            ));
        }
        Slide::Curve(curve) => {
            if curve.duration == 0 || curve.duration > DURATION_MAX_MS {
                return Err(format!(
                    "{key}.slide.duration: from 1 to {DURATION_MAX_MS} milliseconds"
                ));
            }
            match parse_easing(&curve.easing).map_err(|why| format!("{key}.slide.easing: {why}"))? {
                Easing::Spring(..) => {
                    return Err(format!(
                        "{key}.slide.easing: a slide along a spring is written \"spring\""
                    ));
                }
                easing => json!({ "duration": curve.duration, "easing": easing.to_json() }),
            }
        }
    };
    let enter = &highlight.enter;
    let frames = keyframes(
        &enter.keyframes,
        true,
        false,
        &format!("{key}.enter.keyframes"),
    )?;
    let mut entering = json!({ "keyframes": frames });
    match (enter.duration, enter.spring) {
        (Some(_), Some(_)) => {
            return Err(format!(
                "{key}.enter: give a duration or a spring, not both"
            ));
        }
        (None, None) => {
            return Err(format!("{key}.enter: a duration or a spring is needed"));
        }
        (Some(duration), None) => {
            if duration == 0 || duration > DURATION_MAX_MS {
                return Err(format!(
                    "{key}.enter.duration: from 1 to {DURATION_MAX_MS} milliseconds"
                ));
            }
            entering["duration"] = json!(duration);
        }
        (None, Some(spring)) => {
            check_spring(spring).map_err(|why| format!("{key}.enter.spring: {why}"))?;
            entering["spring"] = json!(spring);
        }
    }
    let renamed = match &highlight.renamed {
        Some(frames) => keyframes(frames, true, false, &format!("{key}.renamed"))?,
        None => Value::Null,
    };
    Ok(json!({
        "slot": "tab.highlight",
        "highlight": {
            "shape": highlight.shape,
            "lead": highlight.lead,
            "trail": highlight.trail,
            "slide": slide,
            "enter": entering,
            "renamed": renamed,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(opacity: Option<f64>, transform: Option<&str>) -> Keyframe {
        Keyframe {
            opacity,
            transform: transform.map(str::to_owned),
            ..Keyframe::default()
        }
    }

    fn entering(frames: Vec<Keyframe>) -> Motion {
        Motion {
            id: "pop".to_owned(),
            title: "Pop".to_owned(),
            slot: "menu.enter".to_owned(),
            keyframes: frames,
            duration: Some(180),
            easing: Some("cubic-bezier(0.2, 4, 0.3, -5)".to_owned()),
            delay: None,
            highlight: None,
        }
    }

    #[test]
    fn curves_are_read_and_held_to_what_is_safe() {
        assert_eq!(
            parse_easing("ease-out"),
            Ok(Easing::Css("ease-out".to_owned()))
        );
        assert_eq!(
            parse_easing("emphasized"),
            Ok(Easing::Token("emphasized".to_owned()))
        );
        assert_eq!(
            parse_easing("cubic-bezier(0.2, 4, 0.3, -5)"),
            Ok(Easing::Css("cubic-bezier(0.2, 3, 0.3, -2)".to_owned()))
        );
        assert_eq!(
            parse_easing("steps(4, jump-end)"),
            Ok(Easing::Css("steps(4, jump-end)".to_owned()))
        );
        assert_eq!(
            parse_easing("spring(600, 0.7)"),
            Ok(Easing::Spring(600.0, 0.7))
        );
        for bad in [
            "bounce",
            "cubic-bezier(2, 0, 0, 1)",
            "cubic-bezier(0, 0, 1)",
            "steps(0)",
            "steps(500)",
            "spring(1, 1)",
            "url(x)",
        ] {
            assert!(parse_easing(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_effect_that_brings_something_in_ends_where_it_rests() {
        let good = entering(vec![
            frame(Some(0.0), Some("translateY(-4px) scale(0.96)")),
            frame(Some(1.0), Some("none")),
        ]);
        let checked = check(&good, "contributes.motions[0]").unwrap();
        assert_eq!(
            checked["keyframes"][0]["transform"],
            "translateY(-4px) scale(0.96)"
        );
        assert_eq!(checked["easing"]["css"], "cubic-bezier(0.2, 3, 0.3, -2)");

        let jumps = entering(vec![frame(Some(0.0), None), frame(Some(0.5), None)]);
        assert!(check(&jumps, "m").unwrap_err().contains("as it rests"));
        let still_scaled = entering(vec![
            frame(Some(0.0), None),
            frame(None, Some("scale(1.1)")),
        ]);
        assert!(check(&still_scaled, "m").is_err());
        let mut exit = entering(vec![
            frame(None, None),
            frame(Some(0.0), Some("translateY(8px)")),
        ]);
        exit.slot = "toast.exit".to_owned();
        assert!(check(&exit, "m").is_ok());
        exit.keyframes.reverse();
        assert!(check(&exit, "m").unwrap_err().contains("begins"));
    }

    #[test]
    fn only_safe_properties_and_values_are_taken() {
        let with = |frame: Keyframe| entering(vec![frame, Keyframe::default()]);
        let filter = |written: &str| Keyframe {
            filter: Some(written.to_owned()),
            ..Keyframe::default()
        };
        assert!(check(&with(filter("blur(2px) brightness(1.5)")), "m").is_ok());
        for bad in [
            "url(#x)",
            "blur(100px)",
            "drop-shadow(0 0 2px red)",
            "blur(2em)",
        ] {
            assert!(check(&with(filter(bad)), "m").is_err(), "{bad}");
        }
        for bad in [
            "matrix(1,0,0,1,0,0)",
            "translateX(5em)",
            "translateX(900px)",
            "rotate(2turn)",
            "scale(9)",
            "perspective(10px)",
        ] {
            assert!(check(&with(frame(None, Some(bad))), "m").is_err(), "{bad}");
        }
        let color = Keyframe {
            color: Some("token(color.accent)".to_owned()),
            ..Keyframe::default()
        };
        let checked = check(&with(color), "m").unwrap();
        assert_eq!(checked["keyframes"][0]["color"], "var(--lume-color-accent)");
        let foreign = Keyframe {
            color: Some("red".to_owned()),
            ..Keyframe::default()
        };
        assert!(check(&with(foreign), "m").is_err());
        let size = Keyframe {
            color: Some("token(font.size)".to_owned()),
            ..Keyframe::default()
        };
        assert!(check(&with(size), "m").is_err());
        let mut long = with(Keyframe::default());
        long.duration = Some(5000);
        assert!(check(&long, "m").unwrap_err().contains("1000"));
        let mut nowhere = with(Keyframe::default());
        nowhere.slot = "window.close".to_owned();
        assert!(check(&nowhere, "m").unwrap_err().contains("not a place"));
    }

    #[test]
    fn the_tabs_mark_is_described_by_its_springs() {
        let highlight = Highlight {
            shape: "pill".to_owned(),
            lead: [700.0, 0.9],
            trail: [300.0, 1.0],
            slide: Slide::Curve(SlideCurve {
                duration: 200,
                easing: "standard".to_owned(),
            }),
            enter: Enter {
                keyframes: vec![frame(Some(0.0), Some("scale(0.9)")), frame(Some(1.0), None)],
                duration: None,
                spring: Some([600.0, 0.7]),
            },
            renamed: None,
        };
        let motion = Motion {
            id: "bouncy".to_owned(),
            title: "Bouncy".to_owned(),
            slot: "tab.highlight".to_owned(),
            keyframes: Vec::new(),
            duration: None,
            easing: None,
            delay: None,
            highlight: Some(highlight.clone()),
        };
        let checked = check(&motion, "m").unwrap();
        assert_eq!(checked["highlight"]["slide"]["easing"]["token"], "standard");
        assert_eq!(checked["highlight"]["enter"]["spring"][0], 600.0);
        let mut stiff = motion.clone();
        stiff.highlight = Some(Highlight {
            lead: [1.0e6, 1.0],
            ..highlight.clone()
        });
        assert!(check(&stiff, "m").unwrap_err().contains("stiffness"));
        let mut without = motion;
        without.highlight = None;
        assert!(check(&without, "m").is_err());
    }
}
