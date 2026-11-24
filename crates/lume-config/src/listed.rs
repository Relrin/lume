//! Changing one table of a list in a file the user wrote, such as one `[[profile]]`.
//!
//! Like the changes of single settings, a change here touches only what it is about. A
//! table that is there has its values replaced where they stand. A new one is written
//! after the others of its list, or after the section it belongs to, and a comment
//! stays with the header it stands right above.

use std::ops::Range;

use serde_json::Value;
use toml_edit::{Document, DocumentMut, Item, Table, TableLike};

use crate::write::toml_value;

/// What one key of a table is to hold: a value, or nothing for a key that is left out.
pub type Field<'a> = (&'a str, Option<Value>);

/// A header in the file, and how far what stands under it goes.
struct Section {
    path: Vec<String>,
    /// Which of its list it is, for a table of a list.
    index: Option<usize>,
    header: Range<usize>,
    end: usize,
}

fn syntax(error: &toml_edit::TomlError) -> String {
    error.message().to_owned()
}

/// Takes down the headers of a table and of those in it. Values extend the section they
/// stand in, whose number is `own`.
fn sections_of(table: &Table, path: &[String], own: Option<usize>, found: &mut Vec<Section>) {
    for (key, item) in table {
        let mut inner = path.to_vec();
        inner.push(key.to_owned());
        match item {
            Item::Table(table) => {
                let section = table.span().filter(|span| !span.is_empty()).map(|header| {
                    found.push(Section {
                        path: inner.clone(),
                        index: None,
                        end: header.end,
                        header,
                    });
                    found.len() - 1
                });
                sections_of(table, &inner, section.or(own), found);
            }
            Item::ArrayOfTables(tables) => {
                for (index, table) in tables.iter().enumerate() {
                    let Some(header) = table.span() else {
                        continue;
                    };
                    found.push(Section {
                        path: inner.clone(),
                        index: Some(index),
                        end: header.end,
                        header,
                    });
                    let section = found.len() - 1;
                    sections_of(table, &inner, Some(section), found);
                }
            }
            Item::Value(value) => {
                if let (Some(own), Some(span)) = (own, value.span()) {
                    found[own].end = found[own].end.max(span.end);
                }
            }
            Item::None => {}
        }
    }
}

/// The headers of a file, in the order they stand in it.
fn sections(text: &str) -> Result<Vec<Section>, String> {
    let document = Document::parse(text).map_err(|error| syntax(&error))?;
    let mut found = Vec::new();
    sections_of(document.as_table(), &[], None, &mut found);
    found.sort_by_key(|section| section.header.start);
    Ok(found)
}

/// Where the line that holds `at` begins.
fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |end| end + 1)
}

/// Where the line after the one that holds `at` begins.
fn next_line(text: &str, at: usize) -> usize {
    text[at..].find('\n').map_or(text.len(), |end| at + end + 1)
}

/// Where the comment begins that stands right above a line, or the line itself.
fn above(text: &str, mut start: usize, floor: usize) -> usize {
    while start > floor {
        let before = line_start(text, start - 1);
        if before < floor || !text[before..start].trim_start().starts_with('#') {
            break;
        }
        start = before;
    }
    start
}

/// The table of a list whose `key` is `name`, by its number.
fn index_of(document: &dyn TableLike, list: &[&str], key: &str, name: &str) -> Found {
    let Some((first, rest)) = list.split_first() else {
        return Found::Missing;
    };
    match (document.get(first), rest.is_empty()) {
        (None, _) => Found::Missing,
        (Some(Item::ArrayOfTables(tables)), true) => tables
            .iter()
            .position(|table| table.get(key).and_then(Item::as_str) == Some(name))
            .map_or(Found::Missing, Found::At),
        (Some(_), true) => Found::NotTables,
        (Some(inner), false) => match inner.as_table_like() {
            Some(inner) => index_of(inner, rest, key, name),
            None => Found::NotTables,
        },
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Found {
    At(usize),
    Missing,
    /// The list is written, but not as tables with headers of their own.
    NotTables,
}

fn find(text: &str, list: &[&str], key: &str, name: &str) -> Result<Found, String> {
    let document = text
        .parse::<DocumentMut>()
        .map_err(|error| syntax(&error))?;
    Ok(index_of(document.as_table(), list, key, name))
}

fn not_tables(list: &[&str]) -> String {
    format!(
        "`{0}` is written as a list in one value, which is left to you to change; a list of \
         `[[{0}]]` tables can be changed here",
        list.join(".")
    )
}

fn missing(list: &[&str], name: &str) -> String {
    format!("the file has no `[[{}]]` called `{name}`", list.join("."))
}

/// Whether a file has a table in this list whose `key` is `name`.
pub fn holds(text: &str, list: &[&str], key: &str, name: &str) -> Result<bool, String> {
    Ok(matches!(find(text, list, key, name)?, Found::At(_)))
}

/// The keys of the table of a list whose `key` is `name`.
pub fn keys(text: &str, list: &[&str], key: &str, name: &str) -> Result<Vec<String>, String> {
    let mut document = text
        .parse::<DocumentMut>()
        .map_err(|error| syntax(&error))?;
    let Found::At(index) = index_of(document.as_table(), list, key, name) else {
        return Err(missing(list, name));
    };
    let table = tables_mut(document.as_table_mut(), list)
        .and_then(|tables| tables.get_mut(index))
        .ok_or_else(|| missing(list, name))?;
    Ok(table.iter().map(|(key, _)| key.to_owned()).collect())
}

fn tables_mut<'a>(
    table: &'a mut dyn TableLike,
    list: &[&str],
) -> Option<&'a mut toml_edit::ArrayOfTables> {
    let (first, rest) = list.split_first()?;
    let item = table.get_mut(first)?;
    if rest.is_empty() {
        item.as_array_of_tables_mut()
    } else {
        tables_mut(item.as_table_like_mut()?, rest)
    }
}

fn lay(table: &mut Table, fields: &[Field<'_>]) -> Result<(), String> {
    for (key, value) in fields {
        let Some(value) = value else {
            table.remove(key);
            continue;
        };
        let mut new = toml_value(value)?;
        match table.get_mut(key).and_then(Item::as_value_mut) {
            // Replaced where it stands, so a comment after it stays.
            Some(old) => {
                *new.decor_mut() = old.decor().clone();
                *old = new;
            }
            None => {
                table.insert(key, Item::Value(new));
            }
        }
    }
    Ok(())
}

/// Changes the table of a list whose `key` is `name`: each field is set, or left out
/// where it has no value. Everything else in the file is as it was.
pub fn change(
    text: &str,
    list: &[&str],
    key: &str,
    name: &str,
    fields: &[Field<'_>],
) -> Result<String, String> {
    let mut document = text
        .parse::<DocumentMut>()
        .map_err(|error| syntax(&error))?;
    let index = match index_of(document.as_table(), list, key, name) {
        Found::At(index) => index,
        Found::Missing => return Err(missing(list, name)),
        Found::NotTables => return Err(not_tables(list)),
    };
    let table = tables_mut(document.as_table_mut(), list)
        .and_then(|tables| tables.get_mut(index))
        .ok_or_else(|| missing(list, name))?;
    lay(table, fields)?;
    Ok(document.to_string())
}

/// Writes a new table at the end of a list, which may not be there yet.
pub fn add(text: &str, list: &[&str], fields: &[Field<'_>]) -> Result<String, String> {
    if find(text, list, "", "")? == Found::NotTables {
        return Err(not_tables(list));
    }
    let mut table = Table::new();
    lay(&mut table, fields)?;
    let mut block = format!("[[{}]]\n{table}", list.join("."));
    let line = if text.contains("\r\n") { "\r\n" } else { "\n" };
    block = block.replace('\n', line);

    let sections = sections(text)?;
    let under =
        |section: &Section| section.path.first().map(String::as_str) == list.first().copied();
    let after = sections
        .iter()
        .rposition(|section| under(section) && section.path.len() <= list.len())
        // With what stands in the last of them.
        .map(|last| {
            let mut last = last;
            while sections.get(last + 1).is_some_and(under) {
                last += 1;
            }
            last
        });
    let next = after.and_then(|after| sections.get(after + 1).map(|next| (after, next)));
    let added = match next {
        // Before the next section, and before the comment that stands right above it.
        Some((after, next)) => {
            let floor = next_line(text, sections[after].end);
            let at = above(text, line_start(text, next.header.start), floor);
            let blank = text[..at].ends_with("\n\n") || text[..at].ends_with("\r\n\r\n");
            let gap = if blank { "" } else { line };
            format!("{}{gap}{block}{line}{}", &text[..at], &text[at..])
        }
        None if text.trim().is_empty() => format!("{text}{block}"),
        None => {
            let end = if text.ends_with('\n') { "" } else { line };
            let blank = text.ends_with("\n\n") || text.ends_with("\r\n\r\n");
            let gap = if blank { "" } else { line };
            format!("{text}{end}{gap}{block}")
        }
    };
    // A file may leave no room for a list of this name.
    added
        .parse::<DocumentMut>()
        .map_err(|error| syntax(&error))?;
    Ok(added)
}

/// Takes the table of a list whose `key` is `name` out of a file, with the tables in
/// it. Comments above it stay: what they are about is the user's to know.
pub fn remove(text: &str, list: &[&str], key: &str, name: &str) -> Result<String, String> {
    let index = match find(text, list, key, name)? {
        Found::At(index) => index,
        Found::Missing => return Err(missing(list, name)),
        Found::NotTables => return Err(not_tables(list)),
    };
    let sections = sections(text)?;
    let is_list = |section: &Section| {
        section
            .path
            .iter()
            .map(String::as_str)
            .eq(list.iter().copied())
    };
    let first = sections
        .iter()
        .position(|section| is_list(section) && section.index == Some(index))
        .ok_or_else(|| missing(list, name))?;
    let within = sections[first + 1..]
        .iter()
        .take_while(|section| {
            section.path.len() > list.len()
                && section
                    .path
                    .iter()
                    .map(String::as_str)
                    .zip(list)
                    .all(|(one, other)| one == *other)
        })
        .last();
    let start = line_start(text, sections[first].header.start);
    let end = next_line(text, within.map_or(sections[first].end, |last| last.end));
    let (before, after) = (&text[..start], &text[end..]);
    // One blank line where there were two, and none at the end.
    let after = if before.is_empty() || before.ends_with("\n\n") || before.ends_with("\r\n\r\n") {
        after
            .strip_prefix("\r\n")
            .or_else(|| after.strip_prefix('\n'))
            .unwrap_or(after)
    } else {
        after
    };
    let left = if after.is_empty() {
        let kept = before.trim_end_matches(['\r', '\n']);
        // The end of its last line.
        let line = before[kept.len()..].find('\n').map_or(0, |end| end + 1);
        before[..kept.len() + line].to_owned()
    } else {
        format!("{before}{after}")
    };
    left.parse::<DocumentMut>()
        .map_err(|error| syntax(&error))?;
    Ok(left)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::schema::default_config_text;

    const HOSTS: &[&str] = &["rex", "host"];
    const PROFILES: &[&str] = &["profile"];

    const FILE: &str = "\
default-profile = \"work\"

[rex]
# The one at home.
local = false

# The build machine.
[[rex.host]]
label = \"build\"  # fast
address = \"https://build\"

[[rex.host]]
label = \"spare\"
address = \"https://spare\"

# My keys.
[keybindings]
\"ctrl+x\" = \"lume.tab.new\"

[[profile]]
id = \"work\"
launcher = \"ssh\"
host = \"work\"

[profile.env]
A = \"1\"
";

    fn host(label: &str, address: &str) -> [Field<'static>; 2] {
        [
            ("label", Some(json!(label))),
            ("address", Some(json!(address))),
        ]
    }

    #[test]
    fn a_table_is_changed_where_it_stands() {
        let text = change(
            FILE,
            HOSTS,
            "label",
            "build",
            &host("builder", "ssh://build"),
        )
        .unwrap();
        assert_eq!(
            text,
            FILE.replace("label = \"build\"  # fast", "label = \"builder\"  # fast")
                .replace("https://build", "ssh://build")
        );
        // A key that is left out goes, and one that was not there comes.
        let text = change(
            FILE,
            PROFILES,
            "id",
            "work",
            &[("host", None), ("distro", Some(json!("Ubuntu")))],
        )
        .unwrap();
        assert_eq!(
            text,
            FILE.replace("host = \"work\"\n", "distro = \"Ubuntu\"\n")
        );
        assert!(change(FILE, HOSTS, "label", "other", &host("a", "b")).is_err());
    }

    #[test]
    fn a_new_table_follows_the_others_of_its_list() {
        let text = add(FILE, HOSTS, &host("nas", "nas.lan")).unwrap();
        assert_eq!(
            text,
            FILE.replace(
                "# My keys.",
                "[[rex.host]]\nlabel = \"nas\"\naddress = \"nas.lan\"\n\n# My keys."
            )
        );
        // After what stands in the last of them.
        let text = add(FILE, PROFILES, &[("id", Some(json!("b")))]).unwrap();
        assert_eq!(text, format!("{FILE}\n[[profile]]\nid = \"b\"\n"));
    }

    #[test]
    fn the_first_table_of_a_list_goes_into_its_section_or_to_the_end() {
        let default = default_config_text();
        let text = add(default, HOSTS, &host("nas", "nas.lan")).unwrap();
        let block = "[[rex.host]]\nlabel = \"nas\"\naddress = \"nas.lan\"\n\n";
        // What the file says of the section stays with it.
        assert_eq!(
            text,
            default.replace("[keybindings]\n", &format!("{block}[keybindings]\n"))
        );
        assert_eq!(remove(&text, HOSTS, "label", "nas").unwrap(), default);

        let text = add(default, PROFILES, &[("id", Some(json!("b")))]).unwrap();
        assert_eq!(text, format!("{default}\n[[profile]]\nid = \"b\"\n"));
        assert_eq!(remove(&text, PROFILES, "id", "b").unwrap(), default);

        assert_eq!(
            add("", HOSTS, &host("a", "b")).unwrap(),
            "[[rex.host]]\nlabel = \"a\"\naddress = \"b\"\n"
        );
        assert_eq!(
            add("font.size = 3", HOSTS, &host("a", "b")).unwrap(),
            "font.size = 3\n\n[[rex.host]]\nlabel = \"a\"\naddress = \"b\"\n"
        );
    }

    #[test]
    fn a_table_is_taken_out_with_what_is_in_it() {
        let text = remove(FILE, HOSTS, "label", "spare").unwrap();
        assert_eq!(
            text,
            FILE.replace(
                "[[rex.host]]\nlabel = \"spare\"\naddress = \"https://spare\"\n\n",
                ""
            )
        );
        let text = remove(FILE, HOSTS, "label", "build").unwrap();
        assert!(text.contains("# The build machine.\n"), "{text}");
        assert!(!text.contains("https://build"), "{text}");
        let text = remove(FILE, PROFILES, "id", "work").unwrap();
        assert!(text.ends_with("\"ctrl+x\" = \"lume.tab.new\"\n"), "{text}");
        assert!(remove(FILE, HOSTS, "label", "Build").is_err());
    }

    #[test]
    fn line_ends_are_those_of_the_file() {
        let file = FILE.replace('\n', "\r\n");
        let text = add(&file, HOSTS, &host("nas", "nas.lan")).unwrap();
        assert!(!text.replace("\r\n", "").contains('\n'), "{text:?}");
        assert_eq!(remove(&text, HOSTS, "label", "nas").unwrap(), file);
        let text = change(&file, HOSTS, "label", "spare", &host("other", "x")).unwrap();
        assert_eq!(text.lines().count(), file.lines().count());
    }

    #[test]
    fn a_list_in_one_value_and_a_file_that_is_not_toml_are_left_alone() {
        let inline = "[rex]\nhost = [{ label = \"a\", address = \"b\" }]\n";
        assert!(
            add(inline, HOSTS, &host("c", "d"))
                .unwrap_err()
                .contains("[[rex.host]]")
        );
        assert!(change(inline, HOSTS, "label", "a", &host("c", "d")).is_err());
        assert!(remove(inline, HOSTS, "label", "a").is_err());
        assert!(add("rex = { local = true }\n", HOSTS, &host("c", "d")).is_err());
        assert!(add("[rex\n", HOSTS, &host("c", "d")).is_err());
        assert!(holds(FILE, HOSTS, "label", "spare").unwrap());
        assert!(!holds(FILE, HOSTS, "label", "other").unwrap());
        assert!(!holds("", HOSTS, "label", "other").unwrap());
    }
}
