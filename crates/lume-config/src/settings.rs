//! How the settings window shows the settings it shows.
//!
//! Most settings are only ever written in the file. The ones a window offers controls
//! for say so here, beside their definition: which page and group they are on, what
//! they are called there, and what kind of control sets them. That description travels
//! in the JSON Schema of the settings, under [`SETTINGS_KEY`] on a setting and
//! [`PAGES_KEY`] at the root, so the page that draws the window is given one document
//! and needs no list of its own to keep in step with this one.
//!
//! A page of settings contributed by something other than the application is described
//! the same way, which is why a page is data and not code.

use serde::Serialize;
use serde_json::Value;

use crate::tuning::AccentPreset;

/// The schema keyword under which a setting says how it is shown.
pub const SETTINGS_KEY: &str = "x-lume-settings";

/// The schema keyword, at the root, under which the pages are listed.
pub const PAGES_KEY: &str = "x-lume-pages";

/// The group of pages the application's own are listed under.
const APPLICATION: &str = "Application";

/// The unit of a length, of a share of a whole, of a count of lines and of a time.
const PIXELS: &str = "px";
const PERCENT: &str = "%";
const LINES: &str = "lines";
const SECONDS: &str = "seconds";

/// How many times bigger a share of a whole is shown than it is written: from 0 to 100
/// for one written from 0 to 1.
const PERCENT_SCALE: f64 = 100.0;

/// The platforms a setting may be shown on alone, as the configuration names them.
pub const PLATFORMS: [&str; 3] = ["windows", "macos", "linux"];

/// The kind of control that sets a setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Control {
    /// On or off.
    Switch,
    /// One of a list, which may be long.
    Select,
    /// One of two or three, all on show.
    Segmented,
    /// Whatever is typed.
    Text,
    /// A number that is typed, held between the least and the greatest the setting
    /// takes.
    Number,
    /// A colour: one of a row of swatches, or one of the user's own.
    Swatch,
}

/// A colour offered as a swatch: the value written, what it is called, and how it looks
/// on a dark theme and on a light one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Swatch {
    pub value: &'static str,
    pub label: &'static str,
    pub dark: &'static str,
    pub light: &'static str,
}

/// One thing a setting can be set to, and what that is called.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Choice {
    pub value: Value,
    pub label: String,
}

/// A condition on another setting, for a setting shown only some of the time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct When {
    /// The other setting's path.
    pub path: &'static str,
    /// `unset` or `set`: whether that setting must be left out of the file, or be in it;
    /// or `on`: that switch must be on.
    pub is: &'static str,
}

/// How one setting is shown.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Shown {
    /// The id of the page the setting is on.
    pub page: &'static str,
    /// The group of that page it is in.
    pub group: &'static str,
    /// Where in the group: lower comes first.
    pub order: u32,
    /// What the setting is called.
    pub label: &'static str,
    /// A line said under the label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<&'static str>,
    pub control: Control,
    /// The choices that are always the same.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Choice>,
    /// The choices that depend on the machine: `themes`, `fonts`, `shells` or
    /// `workspaces`. They follow the fixed ones.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list: Option<&'static str>,
    /// The colours a swatch control offers.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub swatches: Vec<Swatch>,
    /// What the choice that takes the setting out of the file is called. A setting
    /// without one is always written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unset: Option<&'static str>,
    /// The design token that holds the value in effect while the setting is left out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<&'static str>,
    /// What the value is counted in: `px`, `lines`, `seconds`, or `%` for a share
    /// written from 0 to 1 and shown from 0 to 100.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<&'static str>,
    /// How far the arrow keys move a typed number, in what it is shown in. The least
    /// and the greatest it may be are the `minimum` and `maximum` of the schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    /// How many times bigger a typed number is shown than it is written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<When>,
    /// The only platform the setting is shown on, for one that does nothing elsewhere.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<&'static str>,
}

impl Shown {
    fn new(
        page: &'static str,
        group: &'static str,
        order: u32,
        label: &'static str,
        control: Control,
    ) -> Self {
        Self {
            page,
            group,
            order,
            label,
            help: None,
            control,
            options: Vec::new(),
            list: None,
            swatches: Vec::new(),
            unset: None,
            token: None,
            unit: None,
            step: None,
            scale: None,
            when: None,
            platform: None,
        }
    }

    /// A setting that is on or off.
    pub fn switch(
        page: &'static str,
        group: &'static str,
        order: u32,
        label: &'static str,
    ) -> Self {
        Self::new(page, group, order, label, Control::Switch)
    }

    /// A setting chosen from a list.
    pub fn select(
        page: &'static str,
        group: &'static str,
        order: u32,
        label: &'static str,
    ) -> Self {
        Self::new(page, group, order, label, Control::Select)
    }

    /// A setting chosen from two or three things, all on show.
    pub fn segmented(
        page: &'static str,
        group: &'static str,
        order: u32,
        label: &'static str,
    ) -> Self {
        Self::new(page, group, order, label, Control::Segmented)
    }

    /// A setting that is typed.
    pub fn text(page: &'static str, group: &'static str, order: u32, label: &'static str) -> Self {
        Self::new(page, group, order, label, Control::Text)
    }

    /// A number that is typed, which the arrow keys move by `step`.
    pub fn number(
        page: &'static str,
        group: &'static str,
        order: u32,
        label: &'static str,
        step: f64,
    ) -> Self {
        Self {
            step: Some(step),
            ..Self::new(page, group, order, label, Control::Number)
        }
    }

    /// A colour, chosen from `presets` or picked.
    pub fn swatch(
        page: &'static str,
        group: &'static str,
        order: u32,
        label: &'static str,
        presets: &[AccentPreset],
    ) -> Self {
        Self {
            swatches: presets
                .iter()
                .map(|preset| Swatch {
                    value: preset.name,
                    label: preset.label,
                    dark: preset.dark,
                    light: preset.light,
                })
                .collect(),
            ..Self::new(page, group, order, label, Control::Swatch)
        }
    }

    /// A line said under the label.
    #[must_use]
    pub fn help(mut self, help: &'static str) -> Self {
        self.help = Some(help);
        self
    }

    /// The choices, each as the value written and what it is called.
    #[must_use]
    pub fn options(mut self, options: &[(&str, &str)]) -> Self {
        self.options = options
            .iter()
            .map(|(value, label)| Choice {
                value: Value::from(*value),
                label: (*label).to_owned(),
            })
            .collect();
        self
    }

    /// A length in pixels.
    #[must_use]
    pub fn pixels(mut self) -> Self {
        self.unit = Some(PIXELS);
        self
    }

    /// A share of a whole, written from 0 to 1 and shown in hundredths.
    #[must_use]
    pub fn percent(mut self) -> Self {
        self.unit = Some(PERCENT);
        self.scale = Some(PERCENT_SCALE);
        self
    }

    /// A count of lines.
    #[must_use]
    pub fn lines(mut self) -> Self {
        self.unit = Some(LINES);
        self
    }

    /// A time in whole seconds.
    #[must_use]
    pub fn seconds(mut self) -> Self {
        self.unit = Some(SECONDS);
        self
    }

    /// Choices that depend on the machine, after the fixed ones.
    #[must_use]
    pub fn list(mut self, list: &'static str) -> Self {
        self.list = Some(list);
        self
    }

    /// What the choice that takes the setting out of the file is called.
    #[must_use]
    pub fn unset(mut self, label: &'static str) -> Self {
        self.unset = Some(label);
        self
    }

    /// The token that holds the value in effect while the setting is left out.
    #[must_use]
    pub fn token(mut self, token: &'static str) -> Self {
        self.token = Some(token);
        self
    }

    /// Shown only while another setting is left out of the file.
    #[must_use]
    pub fn when_unset(mut self, path: &'static str) -> Self {
        self.when = Some(When { path, is: "unset" });
        self
    }

    /// Shown only while a switch is on.
    #[must_use]
    pub fn when_on(mut self, path: &'static str) -> Self {
        self.when = Some(When { path, is: "on" });
        self
    }

    /// Shown only on one platform, for a setting that does nothing on the others.
    #[must_use]
    pub fn only_on(mut self, platform: &'static str) -> Self {
        self.platform = Some(platform);
        self
    }
}

/// One page of the settings window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Page {
    pub id: &'static str,
    pub label: &'static str,
    /// The name of the picture beside the label.
    pub icon: &'static str,
    /// The heading the page is listed under.
    pub group: &'static str,
    /// A line said at the top of the page.
    pub blurb: &'static str,
    /// The page's groups, in the order they are shown.
    pub groups: &'static [&'static str],
}

/// The pages of the application's own settings, in the order they are listed.
pub fn pages() -> [Page; 3] {
    [
        Page {
            id: "general",
            label: "General",
            icon: "general",
            group: APPLICATION,
            blurb: "How Lume starts, how it gets your attention, and what closing does.",
            groups: &["Startup", "Windows", "Attention", "Closing"],
        },
        Page {
            id: "appearance",
            label: "Appearance",
            icon: "appearance",
            group: APPLICATION,
            blurb: "Theme, type, panes and tabs.",
            groups: &["Theme", "Type", "Panes", "Tabs", "Background"],
        },
        Page {
            id: "terminal",
            label: "Terminal",
            icon: "terminal",
            group: APPLICATION,
            blurb: "The terminal new tabs open, the cursor, scrollback and what happens when a program ends.",
            groups: &["New terminals", "Cursor", "History", "Buffer", "Closing"],
        },
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::*;
    use crate::schema::config_schema;
    use crate::store::check_setting;
    use crate::tokens::{Color, token_def};

    /// Every setting that says how it is shown: its path, its schema and what it says.
    fn shown() -> Vec<(Vec<String>, &'static Value, &'static Value)> {
        fn walk(
            path: &mut Vec<String>,
            schema: &'static Value,
            found: &mut Vec<(Vec<String>, &'static Value, &'static Value)>,
        ) {
            if let Some(says) = schema.get(SETTINGS_KEY) {
                found.push((path.clone(), schema, says));
            }
            let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
                return;
            };
            for (key, inner) in properties {
                path.push(key.clone());
                walk(path, inner, found);
                path.pop();
            }
        }
        let mut found = Vec::new();
        walk(&mut Vec::new(), config_schema(), &mut found);
        found
    }

    #[test]
    fn a_typed_number_says_its_step_unit_and_scale() {
        let length = Shown::number("p", "g", 1, "l", 1.0).pixels();
        assert_eq!(
            serde_json::to_value(&length).unwrap(),
            json!({
                "page": "p",
                "group": "g",
                "order": 1,
                "label": "l",
                "control": "number",
                "unit": "px",
                "step": 1.0,
            })
        );
        let share = Shown::number("p", "g", 1, "l", 1.0).percent();
        assert_eq!(share.unit, Some("%"));
        assert_eq!(share.scale, Some(100.0));
        assert_eq!(
            Shown::number("p", "g", 1, "l", 1000.0).lines().unit,
            Some("lines")
        );
        assert_eq!(
            Shown::number("p", "g", 1, "l", 1.0).seconds().unit,
            Some("seconds")
        );
        let windows = Shown::switch("p", "g", 1, "l").only_on("windows");
        assert_eq!(
            serde_json::to_value(&windows).unwrap()["platform"],
            "windows"
        );
    }

    #[test]
    fn what_is_not_said_is_left_out() {
        let plain = Shown::switch("general", "Windows", 20, "Restore window positions");
        assert_eq!(
            serde_json::to_value(&plain).unwrap(),
            json!({
                "page": "general",
                "group": "Windows",
                "order": 20,
                "label": "Restore window positions",
                "control": "switch",
            })
        );
    }

    #[test]
    fn the_pages_are_listed_at_the_root_of_the_schema() {
        let listed = &config_schema()[PAGES_KEY];
        assert_eq!(listed, &serde_json::to_value(pages()).unwrap());
        assert_eq!(listed[1]["id"], "appearance");
        assert_eq!(listed[1]["groups"][2], "Panes");
    }

    #[test]
    fn every_setting_shown_is_on_a_page_in_one_of_its_groups_and_every_group_is_used() {
        let pages = pages();
        let mut used = BTreeSet::new();
        let mut places = BTreeSet::new();
        for (path, _, says) in shown() {
            let page = says["page"].as_str().unwrap();
            let group = says["group"].as_str().unwrap();
            let found = pages.iter().find(|known| known.id == page);
            assert!(
                found.is_some_and(|page| page.groups.contains(&group)),
                "{path:?} is on {page} / {group}"
            );
            used.insert((page.to_owned(), group.to_owned()));
            // Two settings of a group are never told to stand in the same place.
            let place = (page.to_owned(), group.to_owned(), says["order"].as_u64());
            assert!(places.insert(place), "{path:?}");
        }
        for page in &pages {
            for group in page.groups {
                assert!(
                    used.contains(&(page.id.to_owned(), (*group).to_owned())),
                    "nothing is in {} / {group}",
                    page.id
                );
            }
        }
        assert!(!shown().is_empty());
    }

    #[test]
    fn every_choice_offered_is_a_value_the_setting_takes() {
        for (path, _, says) in shown() {
            for choice in says["options"].as_array().into_iter().flatten() {
                let taken = check_setting(&path, &choice["value"]);
                assert!(taken.is_ok(), "{path:?} = {}: {taken:?}", choice["value"]);
                assert!(!choice["label"].as_str().unwrap().is_empty(), "{path:?}");
            }
        }
    }

    #[test]
    fn a_setting_is_set_by_a_control_that_fits_what_it_is() {
        for (path, schema, says) in shown() {
            let control = says["control"].as_str().unwrap();
            let fixed = says["options"].as_array().map_or(0, Vec::len);
            let listed = says.get("list").is_some();
            match control {
                "switch" => assert_eq!(schema["type"], "boolean", "{path:?}"),
                "select" => assert!(fixed > 0 || listed, "{path:?} offers nothing"),
                "segmented" => assert!((2..=3).contains(&fixed) && !listed, "{path:?}"),
                "text" => assert!(fixed == 0 && !listed, "{path:?}"),
                "swatch" => {
                    assert!(fixed == 0 && !listed, "{path:?}");
                    let swatches = says["swatches"].as_array().expect("swatches");
                    assert!(!swatches.is_empty(), "{path:?} offers no colours");
                    for swatch in swatches {
                        let taken = check_setting(&path, &swatch["value"]);
                        assert!(taken.is_ok(), "{path:?} = {}: {taken:?}", swatch["value"]);
                        for look in ["dark", "light"] {
                            let color = swatch[look].as_str().unwrap();
                            assert!(Color::parse(color).is_some(), "{path:?}: {color}");
                        }
                    }
                    // A colour of the user's own is taken too.
                    assert!(check_setting(&path, &json!("#123456")).is_ok(), "{path:?}");
                    assert!(check_setting(&path, &json!("#12345")).is_err(), "{path:?}");
                }
                "number" => {
                    assert!(fixed == 0 && !listed, "{path:?}");
                    // A setting that may be left out is a number or nothing.
                    let types: Vec<&Value> = match &schema["type"] {
                        Value::Array(types) => types.iter().collect(),
                        one => vec![one],
                    };
                    assert!(
                        types
                            .iter()
                            .any(|kind| *kind == "number" || *kind == "integer"),
                        "{path:?} is not a number"
                    );
                    let least = schema["minimum"].as_f64().expect("a least value");
                    let most = schema["maximum"].as_f64().expect("a greatest value");
                    let scale = says["scale"].as_f64().unwrap_or(1.0);
                    let step = says["step"].as_f64().expect("a step");
                    assert!(least < most, "{path:?}");
                    assert!(step > 0.0 && step <= (most - least) * scale, "{path:?}");
                }
                other => panic!("{path:?} is set by `{other}`"),
            }
            if let Some(list) = says.get("list") {
                let known = ["themes", "fonts", "shells", "workspaces", "profiles"];
                assert!(known.contains(&list.as_str().unwrap()), "{path:?}");
            }
            if let Some(platform) = says.get("platform") {
                assert!(PLATFORMS.contains(&platform.as_str().unwrap()), "{path:?}");
            }
        }
    }

    #[test]
    fn a_token_named_is_a_token_and_a_condition_names_a_setting() {
        for (path, _, says) in shown() {
            if let Some(token) = says.get("token") {
                assert!(token_def(token.as_str().unwrap()).is_some(), "{path:?}");
                // Only a setting that can be left out has a value to fall back to.
                assert!(says.get("unset").is_some(), "{path:?}");
            }
            if let Some(when) = says.get("when") {
                let other: Vec<String> = when["path"]
                    .as_str()
                    .unwrap()
                    .split('.')
                    .map(str::to_owned)
                    .collect();
                assert!(
                    shown().iter().any(|(path, ..)| *path == other),
                    "{path:?} depends on {other:?}"
                );
                assert!(["unset", "set", "on"].contains(&when["is"].as_str().unwrap()));
            }
        }
    }

    #[test]
    fn a_setting_that_holds_anything_at_all_is_never_shown() {
        // A setting whose schema is `true` takes any value, and the layers tell such a
        // setting by its schema being exactly that.
        fn walk(path: &str, schema: &Value) {
            let open = schema.get("additionalProperties");
            if open.is_some_and(|inner| inner.get(SETTINGS_KEY).is_some()) {
                panic!("{path} says how its entries are shown");
            }
            if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
                for (key, inner) in properties {
                    walk(&format!("{path}.{key}"), inner);
                }
            }
        }
        let schema = config_schema();
        walk("", schema);
        let tokens = &schema["properties"]["appearance"]["properties"]["tokens"];
        assert_eq!(tokens["additionalProperties"], json!(true));
        assert!(tokens.get(SETTINGS_KEY).is_none());
        assert!(schema["properties"]["plugins"].get(SETTINGS_KEY).is_none());
    }
}
