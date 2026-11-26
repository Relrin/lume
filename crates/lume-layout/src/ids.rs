//! The names of the parts of a layout.
//!
//! Panes, tabs, splits, groups and windows are numbered from one counter that the workspace
//! keeps, so a number is used once in a workspace's life and applying the same intents
//! to the same workspace always gives the same ids. In JSON an id is a letter for its
//! kind and the number: `"p12"`, `"t3"`, `"s7"`, `"g4"`, `"w1"`.

use std::borrow::Cow;
use std::fmt;
use std::num::NonZeroU64;
use std::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

macro_rules! counted_id {
    ($(#[$doc:meta])* $name:ident, $prefix:literal, $what:literal) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(u32);

        impl $name {
            /// Makes an id from its number.
            pub const fn new(number: u32) -> Self {
                Self(number)
            }

            /// The id's number.
            pub const fn get(self) -> u32 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefix, self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(self, f)
            }
        }

        impl FromStr for $name {
            type Err = &'static str;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                const WHAT: &str = $what;
                let digits = text.strip_prefix($prefix).ok_or(WHAT)?;
                // One spelling for each id: no sign, no spaces, no leading zeros.
                let plain = !digits.is_empty()
                    && digits.bytes().all(|byte| byte.is_ascii_digit())
                    && (digits.len() == 1 || !digits.starts_with('0'));
                if !plain {
                    return Err(WHAT);
                }
                digits.parse().map(Self).map_err(|_| WHAT)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                text.parse().map_err(de::Error::custom)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({
                    "description": concat!("An id: ", $what, ", as in `", $prefix, "12`."),
                    "type": "string",
                    "pattern": concat!("^", $prefix, "(0|[1-9][0-9]*)$")
                })
            }
        }
    };
}

counted_id!(
    /// Names a pane: a leaf of a tab's tree, or a pane that floats.
    PaneId,
    "p",
    "a pane id is `p` followed by a number"
);
counted_id!(
    /// Names a tab.
    TabId,
    "t",
    "a tab id is `t` followed by a number"
);
counted_id!(
    /// Names a split: a node of a tab's tree that divides its space among children.
    SplitId,
    "s",
    "a split id is `s` followed by a number"
);
counted_id!(
    /// Names a group of tabs in a window's tab strip.
    GroupId,
    "g",
    "a group id is `g` followed by a number"
);
counted_id!(
    /// Names a window of a workspace.
    WindowId,
    "w",
    "a window id is `w` followed by a number"
);

/// Names a panel within a window. A panel is named by whoever contributes it, so the
/// name is text: lowercase letters, digits, dots and hyphens, as in `lume.tabs`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PanelId(String);

impl PanelId {
    /// The longest a panel's name may be, in bytes.
    pub const MAX_LEN: usize = 64;

    /// Makes a panel id, or says why the text is not one.
    pub fn new(name: impl Into<String>) -> Result<Self, &'static str> {
        const WHAT: &str = "a panel id is 1 to 64 lowercase letters, digits, dots or hyphens";
        let name = name.into();
        let allowed = |byte: u8| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-');
        if name.is_empty() || name.len() > Self::MAX_LEN || !name.bytes().all(allowed) {
            return Err(WHAT);
        }
        Ok(Self(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl JsonSchema for PanelId {
    fn schema_name() -> Cow<'static, str> {
        "PanelId".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "Names a panel: a dotted name, as in `lume.tabs`.",
            "type": "string",
            "pattern": "^[a-z0-9.-]{1,64}$"
        })
    }
}

impl fmt::Display for PanelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for PanelId {
    type Error = &'static str;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        Self::new(name)
    }
}

impl From<PanelId> for String {
    fn from(id: PanelId) -> Self {
        id.0
    }
}

/// Identifies a workspace for as long as it exists, whatever it is called.
///
/// A workspace's name can change and its id cannot, so what refers to a workspace
/// across a rename refers to it by id. Ids are random, not counted: two workspaces made
/// on different days must not share one. In JSON an id is sixteen lowercase hexadecimal
/// digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WorkspaceId(NonZeroU64);

impl WorkspaceId {
    /// Makes an id from its numeric value. Zero is not an id.
    pub fn new(raw: u64) -> Option<Self> {
        NonZeroU64::new(raw).map(Self)
    }

    pub fn get(self) -> u64 {
        self.0.get()
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl fmt::Debug for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WorkspaceId({self})")
    }
}

impl FromStr for WorkspaceId {
    type Err = &'static str;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        const WHAT: &str = "a workspace id is sixteen lowercase hexadecimal digits and not zero";
        let well_formed = text.len() == 16
            && text
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
        if !well_formed {
            return Err(WHAT);
        }
        u64::from_str_radix(text, 16)
            .ok()
            .and_then(Self::new)
            .ok_or(WHAT)
    }
}

impl JsonSchema for WorkspaceId {
    fn schema_name() -> Cow<'static, str> {
        "WorkspaceId".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "Identifies a workspace whatever it is called: sixteen lowercase hexadecimal digits.",
            "type": "string",
            "pattern": "^[0-9a-f]{16}$"
        })
    }
}

impl Serialize for WorkspaceId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for WorkspaceId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_counted_id_is_its_letter_and_its_number() {
        assert_eq!(PaneId::new(12).to_string(), "p12");
        assert_eq!("t3".parse::<TabId>(), Ok(TabId::new(3)));
        assert_eq!(serde_json::to_string(&SplitId::new(7)).unwrap(), "\"s7\"");
        assert_eq!(
            serde_json::from_str::<WindowId>("\"w1\"").unwrap(),
            WindowId::new(1)
        );
    }

    #[test]
    fn an_id_has_one_spelling() {
        for text in [
            "", "p", "12", "p-1", "p+1", "p 1", "p01", "t1", "p1.0", "P1",
        ] {
            assert!(text.parse::<PaneId>().is_err(), "{text:?} was taken");
        }
        assert_eq!("p0".parse::<PaneId>(), Ok(PaneId::new(0)));
        assert!("p4294967296".parse::<PaneId>().is_err());
    }

    #[test]
    fn a_panel_id_is_a_plain_dotted_name() {
        assert_eq!(PanelId::new("lume.tabs").unwrap().as_str(), "lume.tabs");
        for name in ["", "Lume.Tabs", "a b", "a/b", &"x".repeat(65)] {
            assert!(PanelId::new(name).is_err(), "{name:?} was taken");
        }
        assert!(serde_json::from_str::<PanelId>("\"a_b\"").is_err());
    }

    #[test]
    fn a_workspace_id_is_sixteen_hexadecimal_digits() {
        let id = WorkspaceId::new(0xab).unwrap();
        assert_eq!(id.to_string(), "00000000000000ab");
        assert_eq!("00000000000000ab".parse::<WorkspaceId>(), Ok(id));
        for text in ["", "ab", "00000000000000AB", "0000000000000000"] {
            assert!(text.parse::<WorkspaceId>().is_err(), "{text:?} was taken");
        }
    }
}
