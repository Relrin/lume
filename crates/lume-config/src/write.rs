//! Changing a setting in a file the user wrote, without disturbing the rest of it.
//!
//! A settings view and a command both change settings, and the file they change is one
//! a person also edits by hand. So a change touches the one key it is about: comments,
//! blank lines, the order of things and the way other values are written all stay.
//!
//! A file is replaced in one step. The new text is written beside the old file and
//! moved over it, so a reader sees the old file or the new one and never half of one.

use std::io::Write as _;
use std::path::Path;

use serde_json::Value;
use toml_edit::{DocumentMut, InlineTable, Item, Table, TableLike};

use crate::schema::WORKSPACE_ROOT_KEY;

/// A JSON value as a TOML one. TOML has no way to write nothing.
pub(crate) fn toml_value(value: &Value) -> Result<toml_edit::Value, String> {
    Ok(match value {
        Value::Null => {
            return Err("a setting cannot be set to nothing; unset it instead".to_owned());
        }
        Value::Bool(boolean) => (*boolean).into(),
        Value::Number(number) => match (number.as_i64(), number.as_f64()) {
            (Some(integer), _) => integer.into(),
            (None, Some(float)) if float.is_finite() => float.into(),
            _ => return Err(format!("{number} is not a number a file can hold")),
        },
        Value::String(text) => text.as_str().into(),
        Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(toml_value(item)?);
            }
            array.into()
        }
        Value::Object(entries) => {
            let mut table = InlineTable::new();
            for (key, entry) in entries {
                table.insert(key, toml_value(entry)?);
            }
            table.into()
        }
    })
}

fn parse(text: &str) -> Result<DocumentMut, String> {
    text.parse::<DocumentMut>()
        .map_err(|error| error.message().to_owned())
}

/// The table a path leads to, making the tables on the way if `make` says so.
fn table_at<'a>(
    table: &'a mut dyn TableLike,
    path: &[String],
    make: bool,
) -> Result<Option<&'a mut dyn TableLike>, String> {
    let Some((first, rest)) = path.split_first() else {
        return Ok(Some(table));
    };
    if !table.contains_key(first) {
        if !make {
            return Ok(None);
        }
        // A table made only to hold another has no header of its own in the file.
        let mut made = Table::new();
        made.set_implicit(true);
        table.insert(first, Item::Table(made));
    }
    match table.get_mut(first).and_then(Item::as_table_like_mut) {
        Some(inner) => table_at(inner, rest, make),
        None => Err(format!("`{first}` is not a table in the file")),
    }
}

/// Sets the value at `path` in a TOML file's text and returns the new text. Everything
/// else in the file is as it was.
pub fn set_value(text: &str, path: &[String], value: &Value) -> Result<String, String> {
    let (key, tables) = path
        .split_last()
        .ok_or_else(|| "a setting needs a name".to_owned())?;
    let mut document = parse(text)?;
    let new = toml_value(value)?;
    let table = table_at(document.as_table_mut(), tables, true)?
        .ok_or_else(|| "the table could not be made".to_owned())?;
    match table.get_mut(key).and_then(Item::as_value_mut) {
        // The value is replaced where it stands, so a comment after it stays.
        Some(old) => {
            let decor = old.decor().clone();
            *old = new;
            *old.decor_mut() = decor;
        }
        None => {
            table.insert(key, Item::Value(new));
        }
    }
    Ok(document.to_string())
}

/// Takes the value at `path` out of a TOML file's text and returns the new text. A
/// file that does not set it comes back unchanged.
pub fn unset_value(text: &str, path: &[String]) -> Result<String, String> {
    let (key, tables) = path
        .split_last()
        .ok_or_else(|| "a setting needs a name".to_owned())?;
    let mut document = parse(text)?;
    let removed = match table_at(document.as_table_mut(), tables, false)? {
        Some(table) => table.remove(key).is_some(),
        None => false,
    };
    if removed {
        Ok(document.to_string())
    } else {
        Ok(text.to_owned())
    }
}

/// Sets or clears the folder in a workspace file's text.
pub fn set_workspace_root(text: &str, root: Option<&str>) -> Result<String, String> {
    let path = [WORKSPACE_ROOT_KEY.to_owned()];
    match root {
        Some(root) => set_value(text, &path, &Value::from(root)),
        None => unset_value(text, &path),
    }
}

/// Replaces a file with `text` in one step, making its directory if it has none.
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("the path names no file"))?
        .to_owned();
    name.push(format!(".{}.tmp", std::process::id()));
    let beside = path.with_file_name(name);
    let written = (|| {
        let mut file = std::fs::File::create(&beside)?;
        file.write_all(text.as_bytes())?;
        // On disk before it takes the old file's place.
        file.sync_all()?;
        drop(file);
        std::fs::rename(&beside, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&beside);
    }
    written
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::testing::TempDir;

    fn path(dotted: &str) -> Vec<String> {
        dotted.split('.').map(str::to_owned).collect()
    }

    const FILE: &str = "\
# My settings.

[font]
# Bigger on the big screen.
size = 14  # was 12
family = \"Iosevka\"

[terminal]
cursor-style = \"bar\"
";

    #[test]
    fn a_value_is_changed_where_it_stands_and_its_comments_stay() {
        let text = set_value(FILE, &path("font.size"), &json!(16.5)).unwrap();
        assert_eq!(
            text,
            FILE.replace("size = 14  # was 12", "size = 16.5  # was 12")
        );
    }

    #[test]
    fn a_new_key_goes_into_its_section_and_a_new_section_at_the_end() {
        let text = set_value(FILE, &path("font.weight"), &json!(500)).unwrap();
        assert!(
            text.contains("family = \"Iosevka\"\nweight = 500\n\n[terminal]"),
            "{text}"
        );

        let text = set_value(FILE, &path("layout.tab-placement"), &json!("left")).unwrap();
        assert!(text.starts_with(FILE), "{text}");
        assert!(
            text.ends_with("\n[layout]\ntab-placement = \"left\"\n"),
            "{text}"
        );
    }

    #[test]
    fn tables_made_on_the_way_have_no_header_of_their_own() {
        let text = set_value(
            "",
            &path("appearance.tokens.color.accent"),
            &json!("#ff8800"),
        )
        .unwrap();
        assert_eq!(text, "[appearance.tokens.color]\naccent = \"#ff8800\"\n");
    }

    #[test]
    fn every_kind_of_value_can_be_written() {
        let mut text = String::new();
        for (key, value) in [
            ("a", json!(true)),
            ("b", json!(3)),
            ("c", json!(1.5)),
            ("d", json!("x \"y\"")),
            ("e", json!(["Menlo", "monospace"])),
            (
                "f",
                json!({ "command": "lume.tab.new", "when": "terminal" }),
            ),
        ] {
            text = set_value(&text, &[key.to_owned()], &value).unwrap();
        }
        let read: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(read["a"].as_bool(), Some(true));
        assert_eq!(read["b"].as_integer(), Some(3));
        assert_eq!(read["c"].as_float(), Some(1.5));
        assert_eq!(read["d"].as_str(), Some("x \"y\""));
        assert_eq!(read["e"].as_array().unwrap().len(), 2);
        assert_eq!(read["f"]["when"].as_str(), Some("terminal"));
        assert!(set_value("", &path("a"), &Value::Null).is_err());
    }

    #[test]
    fn a_key_with_a_dot_in_it_is_one_key() {
        let key = vec!["keybindings".to_owned(), "ctrl+.".to_owned()];
        let text = set_value("", &key, &json!("lume.tab.new")).unwrap();
        assert_eq!(text, "[keybindings]\n\"ctrl+.\" = \"lume.tab.new\"\n");
        assert_eq!(unset_value(&text, &key).unwrap(), "[keybindings]\n");
    }

    #[test]
    fn unsetting_takes_one_key_out_and_leaves_a_file_without_it_alone() {
        let text = unset_value(FILE, &path("font.size")).unwrap();
        assert!(!text.contains("size = 14"));
        assert!(text.contains("family = \"Iosevka\""));
        assert!(text.contains("# My settings."));
        assert_eq!(unset_value(FILE, &path("font.weight")).unwrap(), FILE);
        assert_eq!(
            unset_value(FILE, &path("layout.tab-placement")).unwrap(),
            FILE
        );
    }

    #[test]
    fn a_file_that_is_not_toml_is_not_changed() {
        assert!(set_value("[font\n", &path("font.size"), &json!(1)).is_err());
        assert!(unset_value("[font\n", &path("font.size")).is_err());
        // A value where a table is needed cannot be written into.
        assert!(
            set_value("font = 3\n", &path("font.size"), &json!(1))
                .unwrap_err()
                .contains("`font` is not a table")
        );
    }

    #[test]
    fn a_workspaces_folder_is_set_and_cleared_above_its_sections() {
        let file = "# Work.\n\n[font]\nsize = 12\n";
        let set = set_workspace_root(file, Some("D:\\code\\lume")).unwrap();
        assert!(
            set.contains("root = 'D:\\code\\lume'")
                || set.contains("root = \"D:\\\\code\\\\lume\""),
            "{set}"
        );
        let read: toml::Table = toml::from_str(&set).unwrap();
        assert_eq!(read["root"].as_str(), Some("D:\\code\\lume"));
        assert!(set.find("root").unwrap() < set.find("[font]").unwrap());
        assert!(set.contains("# Work."));

        let cleared = set_workspace_root(&set, None).unwrap();
        let read: toml::Table = toml::from_str(&cleared).unwrap();
        assert!(!read.contains_key("root"));
        assert_eq!(read["font"]["size"].as_integer(), Some(12));
    }

    #[test]
    fn a_file_is_replaced_whole_and_its_directory_made() {
        let dir = TempDir::new("write");
        let file = dir.path().join("deep").join("config.toml");
        write_atomic(&file, "a = 1\n").unwrap();
        write_atomic(&file, "a = 2\n").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "a = 2\n");
        // Nothing is left beside it.
        let left: Vec<_> = std::fs::read_dir(file.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(left, ["config.toml"]);
    }
}
