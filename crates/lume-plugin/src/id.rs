//! What plugins and the things they add are called.
//!
//! A plugin's id is dotted, lower-case words, such as `acme.metrics`. It is the plugin's
//! namespace everywhere: its commands are `acme.metrics.<name>`, its settings are under
//! `[plugins."acme.metrics"]`, its stored data is kept under the same name. Inside its
//! manifest a plugin names its own things without that prefix (`refresh`, not
//! `acme.metrics.refresh`), because the name it is installed under may not be the one its
//! author gave it: two plugins with the same id can be installed side by side when one
//! of them is given another.
//!
//! No plugin may be called `lume` or have a name under it. Everything under `lume.` is
//! the application's own, so nothing a plugin adds can pass for part of it.

/// The longest id a plugin may have. A command id is the plugin's id and a name, and
/// those may have 96 characters together.
pub const PLUGIN_ID_MAX_LEN: usize = 64;

/// The longest name a plugin may give one of its own things.
pub const NAME_MAX_LEN: usize = 31;

/// The application's own prefix, which no plugin may have.
pub const RESERVED: &str = "lume";

/// Whether `part` is one lower-case word: letters, digits and hyphens, beginning with a
/// letter and not ending with a hyphen.
fn word(part: &str) -> bool {
    part.starts_with(|first: char| first.is_ascii_lowercase())
        && !part.ends_with('-')
        && part
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Whether `text` is dotted lower-case words.
fn dotted(text: &str) -> bool {
    !text.is_empty() && text.split('.').all(word)
}

/// Checks that `id` can be a plugin's: dotted lower-case words, not too long, and not the
/// application's.
pub fn check_plugin_id(id: &str) -> Result<(), String> {
    if !dotted(id) {
        return Err(format!(
            "`{id}` is not a plugin id: lower-case words joined by dots, as in `acme.metrics`"
        ));
    }
    if id.len() > PLUGIN_ID_MAX_LEN {
        return Err(format!(
            "`{id}` is too long for a plugin id: {PLUGIN_ID_MAX_LEN} characters at most"
        ));
    }
    if is_reserved(id) {
        return Err(format!(
            "a plugin cannot be called `{id}`: names beginning with `{RESERVED}.` are the \
             application's"
        ));
    }
    Ok(())
}

/// Whether an id is the application's prefix or under it.
pub fn is_reserved(id: &str) -> bool {
    id.split('.').next() == Some(RESERVED)
}

/// Checks that `name` can name one of a plugin's own things, such as a command: dotted
/// lower-case words, short enough.
pub fn check_name(name: &str) -> Result<(), String> {
    if !dotted(name) {
        return Err(format!(
            "`{name}` is not a name a plugin can give: lower-case words, letters, digits \
             and hyphens, as in `refresh` or `show-all`"
        ));
    }
    if name.len() > NAME_MAX_LEN {
        return Err(format!(
            "`{name}` is too long: {NAME_MAX_LEN} characters at most"
        ));
    }
    Ok(())
}

/// The full id of one of a plugin's things: the id it is installed under and the name.
pub fn full_id(plugin: &str, name: &str) -> String {
    format!("{plugin}.{name}")
}

/// Whether two plugin ids would share names: the same, or one inside the other, as
/// `acme` and `acme.tools` are. Two installs may not, or the commands of one could take
/// the names of the other's.
pub fn overlap(one: &str, other: &str) -> bool {
    let inside = |inner: &str, outer: &str| {
        inner
            .strip_prefix(outer)
            .is_some_and(|rest| rest.starts_with('.'))
    };
    one == other || inside(one, other) || inside(other, one)
}

/// How a manifest names a thing: one of its own, one of the application's, or one of
/// another plugin's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reference {
    /// The plugin's own: `refresh`.
    Own(String),
    /// The application's: `lume:<key>` for a place in its menus and bars, or a command
    /// id beginning with `lume.`.
    Builtin(String),
    /// Another plugin's, by the id its manifest gives it: `acme.core:open`.
    Other { plugin: String, name: String },
}

impl Reference {
    /// Reads a reference. Command ids of the application (`lume.tab.new`) are read as
    /// built-in; so is `lume:<key>`.
    pub fn parse(written: &str) -> Result<Self, String> {
        if let Some((plugin, name)) = written.split_once(':') {
            if plugin == RESERVED {
                return if dotted(name) {
                    Ok(Self::Builtin(name.to_owned()))
                } else {
                    Err(format!("`{written}` names nothing of the application's"))
                };
            }
            check_plugin_id(plugin)
                .map_err(|problem| format!("`{written}` names another plugin's: {problem}"))?;
            check_name(name).map_err(|problem| format!("`{written}`: {problem}"))?;
            return Ok(Self::Other {
                plugin: plugin.to_owned(),
                name: name.to_owned(),
            });
        }
        if is_reserved(written) {
            return if dotted(written) && written.contains('.') {
                Ok(Self::Builtin(written.to_owned()))
            } else {
                Err(format!("`{written}` names nothing of the application's"))
            };
        }
        check_name(written)?;
        Ok(Self::Own(written.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plugin_id_is_dotted_lower_case_words() {
        for good in ["acme", "acme.metrics", "a1.b-2", "x.y.z"] {
            assert_eq!(check_plugin_id(good), Ok(()), "{good}");
        }
        for bad in [
            "", "Acme", "acme.", ".acme", "acme..x", "acme_x", "9acme", "acme-", "a b", "acme/x",
        ] {
            assert!(check_plugin_id(bad).is_err(), "{bad:?}");
        }
        assert!(check_plugin_id(&"a".repeat(PLUGIN_ID_MAX_LEN)).is_ok());
        assert!(check_plugin_id(&"a".repeat(PLUGIN_ID_MAX_LEN + 1)).is_err());
    }

    #[test]
    fn nothing_under_the_applications_name_can_be_a_plugin() {
        for reserved in ["lume", "lume.metrics", "lume.pane.extras"] {
            assert!(check_plugin_id(reserved).is_err(), "{reserved}");
        }
        assert_eq!(check_plugin_id("lumen"), Ok(()));
        assert_eq!(check_plugin_id("my.lume"), Ok(()));
    }

    #[test]
    fn ids_overlap_when_one_is_inside_the_other() {
        assert!(overlap("acme", "acme"));
        assert!(overlap("acme", "acme.tools"));
        assert!(overlap("acme.tools.x", "acme.tools"));
        assert!(!overlap("acme", "acmex"));
        assert!(!overlap("acme.tools", "acme.toolsx"));
        assert!(!overlap("acme.a", "acme.b"));
    }

    #[test]
    fn references_say_whose_thing_they_name() {
        assert_eq!(
            Reference::parse("refresh"),
            Ok(Reference::Own("refresh".to_owned()))
        );
        assert_eq!(
            Reference::parse("lume.tab.new"),
            Ok(Reference::Builtin("lume.tab.new".to_owned()))
        );
        assert_eq!(
            Reference::parse("lume:new-tab-with"),
            Ok(Reference::Builtin("new-tab-with".to_owned()))
        );
        assert_eq!(
            Reference::parse("acme.core:open"),
            Ok(Reference::Other {
                plugin: "acme.core".to_owned(),
                name: "open".to_owned()
            })
        );
        for bad in [
            "lume",
            "lume:",
            "Refresh",
            "acme:",
            ":x",
            "lume.metrics:x",
            "a b",
        ] {
            assert!(Reference::parse(bad).is_err(), "{bad:?}");
        }
    }
}
