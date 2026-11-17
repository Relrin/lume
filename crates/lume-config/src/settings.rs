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

/// The schema keyword under which a setting says how it is shown.
pub const SETTINGS_KEY: &str = "x-lume-settings";

/// The schema keyword, at the root, under which the pages are listed.
pub const PAGES_KEY: &str = "x-lume-pages";

/// The group of pages the application's own are listed under.
const APPLICATION: &str = "Application";

/// The unit of a length, of a share of a whole, and of a count of lines.
const PIXELS: &str = "px";
const PERCENT: &str = "%";
const LINES: &str = "lines";
const SECONDS: &str = "seconds";

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
    /// `unset` or `set`: whether that setting must be left out of the file, or be in it.
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
    /// What the choice that takes the setting out of the file is called. A setting
    /// without one is always written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unset: Option<&'static str>,
    /// The design token that holds the value in effect while the setting is left out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<&'static str>,
    /// What the value is counted in: `px`, `lines`, or `%` for a share written from 0
    /// to 1 and shown from 0 to 100.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<When>,
}

/// A whole number as one, and any other as it is: `10`, not `10.0`.
fn number(value: f64) -> Value {
    // Rounded to what the steps are written to, so 0.1 added three times is 0.3.
    let value = (value * 1000.0).round() / 1000.0;
    if value.fract() == 0.0 && value.abs() < 1e15 {
        #[allow(clippy::cast_possible_truncation)]
        Value::from(value as i64)
    } else {
        Value::from(value)
    }
}

/// The values from `first` to `last` in steps of `step`, both ends included.
fn steps((first, last, step): (f64, f64, f64)) -> Vec<f64> {
    let mut values = Vec::new();
    let mut count = 0.0;
    loop {
        let value = first + count * step;
        // A step's worth of rounding does not drop the last value.
        if value > last + step / 1000.0 {
            return values;
        }
        values.push((value * 1000.0).round() / 1000.0);
        count += 1.0;
    }
}

/// A count with its thousands set apart: `10,000`.
fn grouped(count: u32) -> String {
    let digits = count.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
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
            unset: None,
            token: None,
            unit: None,
            when: None,
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

    /// Lengths in pixels to choose from: the first, the last and the step between.
    #[must_use]
    pub fn lengths(mut self, range: (f64, f64, f64)) -> Self {
        self.options = steps(range)
            .into_iter()
            .map(|value| Choice {
                value: number(value),
                label: format!("{} {PIXELS}", number(value)),
            })
            .collect();
        self.unit = Some(PIXELS);
        self
    }

    /// Shares of a whole to choose from, written from 0 to 1 and called by their
    /// hundredths. `none` is what a share of nothing is called, if it has a name.
    #[must_use]
    pub fn shares(mut self, range: (f64, f64, f64), none: Option<&str>) -> Self {
        self.options = steps(range)
            .into_iter()
            .map(|value| Choice {
                value: number(value),
                label: match none {
                    Some(none) if value == 0.0 => none.to_owned(),
                    _ => format!("{}{PERCENT}", number(value * 100.0)),
                },
            })
            .collect();
        self.unit = Some(PERCENT);
        self
    }

    /// Plain numbers to choose from, in steps of tenths.
    #[must_use]
    pub fn numbers(mut self, range: (f64, f64, f64)) -> Self {
        self.options = steps(range)
            .into_iter()
            .map(|value| Choice {
                value: number(value),
                // One decimal place throughout: 1.0 beside 1.1.
                label: format!("{value:.1}"),
            })
            .collect();
        self
    }

    /// Counts of lines to choose from.
    #[must_use]
    pub fn lines(mut self, counts: &[u32]) -> Self {
        self.options = counts
            .iter()
            .map(|count| Choice {
                value: Value::from(*count),
                label: format!("{} {LINES}", grouped(*count)),
            })
            .collect();
        self.unit = Some(LINES);
        self
    }

    /// Durations in whole seconds to choose from.
    #[must_use]
    pub fn seconds(mut self, counts: &[u32]) -> Self {
        self.options = counts
            .iter()
            .map(|count| Choice {
                value: Value::from(*count),
                label: format!("{count} {SECONDS}"),
            })
            .collect();
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
            blurb: "Theme, type and panes.",
            groups: &["Theme", "Type", "Panes", "Background"],
        },
        Page {
            id: "terminal",
            label: "Terminal",
            icon: "terminal",
            group: APPLICATION,
            blurb: "Cursor, scrollback and what happens when a program ends.",
            groups: &["Cursor", "Buffer", "Closing"],
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
    use crate::tokens::token_def;

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
    fn whole_numbers_are_written_whole_and_steps_do_not_drift() {
        assert_eq!(number(10.0), json!(10));
        assert_eq!(number(1.2), json!(1.2));
        assert_eq!(steps((0.0, 0.5, 0.1)), [0.0, 0.1, 0.2, 0.3, 0.4, 0.5]);
        assert_eq!(steps((1.0, 1.6, 0.1)).len(), 7);
        assert_eq!(steps((0.0, 18.0, 2.0)).last(), Some(&18.0));
        assert_eq!(grouped(100_000), "100,000");
        assert_eq!(grouped(5000), "5,000");
        assert_eq!(grouped(500), "500");
    }

    #[test]
    fn choices_are_called_by_their_unit() {
        let lengths = Shown::select("p", "g", 1, "l").lengths((0.0, 4.0, 2.0));
        assert_eq!(
            serde_json::to_value(&lengths.options).unwrap(),
            json!([
                { "value": 0, "label": "0 px" },
                { "value": 2, "label": "2 px" },
                { "value": 4, "label": "4 px" },
            ])
        );
        let shares = Shown::select("p", "g", 1, "l").shares((0.0, 0.2, 0.1), Some("Off"));
        assert_eq!(
            serde_json::to_value(&shares.options).unwrap(),
            json!([
                { "value": 0, "label": "Off" },
                { "value": 0.1, "label": "10%" },
                { "value": 0.2, "label": "20%" },
            ])
        );
        let numbers = Shown::select("p", "g", 1, "l").numbers((1.0, 1.1, 0.1));
        assert_eq!(numbers.options[0].label, "1.0");
        assert_eq!(numbers.options[1].label, "1.1");
        assert_eq!(numbers.unit, None);
        let lines = Shown::select("p", "g", 1, "l").lines(&[1000]);
        assert_eq!(lines.options[0].label, "1,000 lines");
        assert_eq!(lines.unit, Some("lines"));
        let seconds = Shown::select("p", "g", 1, "l").seconds(&[30]);
        assert_eq!(
            serde_json::to_value(&seconds.options).unwrap(),
            json!([{ "value": 30, "label": "30 seconds" }])
        );
        assert_eq!(seconds.unit, Some("seconds"));
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
                other => panic!("{path:?} is set by `{other}`"),
            }
            if let Some(list) = says.get("list") {
                let known = ["themes", "fonts", "shells", "workspaces", "profiles"];
                assert!(known.contains(&list.as_str().unwrap()), "{path:?}");
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
                assert!(["unset", "set"].contains(&when["is"].as_str().unwrap()));
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
