//! Where a setting came from, and what was wrong with a file.
//!
//! Every value of the resolved configuration remembers the layer that set it, so that a
//! settings view can say "set in your workspace file, line 12" and a problem can point
//! at the line it is about.

use std::fmt;
use std::path::PathBuf;

use serde::Serialize;

/// The layer a value came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "layer", rename_all = "snake_case")]
pub enum Source {
    /// The built-in default.
    Default,
    /// The user's `config.toml`.
    User { file: PathBuf, line: u32 },
    /// The file of the active workspace.
    Workspace {
        name: String,
        file: PathBuf,
        line: u32,
    },
    /// A profile's overrides.
    Profile { id: String },
}

impl Source {
    /// The file and line the value was written at, for a value that was written.
    pub fn location(&self) -> Option<(&PathBuf, u32)> {
        match self {
            Self::User { file, line } | Self::Workspace { file, line, .. } => Some((file, *line)),
            Self::Default | Self::Profile { .. } => None,
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => f.write_str("default"),
            Self::User { file, line } => write!(f, "user:{}:{line}", file.display()),
            Self::Workspace { name, file, line } => {
                write!(f, "workspace {name}:{}:{line}", file.display())
            }
            Self::Profile { id } => write!(f, "profile {id}"),
        }
    }
}

/// What kind of thing is wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKind {
    /// The file is not TOML. Nothing in it can be used.
    Syntax,
    /// The file could not be read.
    Io,
    /// A key nothing knows. It is ignored.
    UnknownKey,
    /// A value of the wrong type or outside what the key allows. The value a lower
    /// layer gave stands.
    InvalidValue,
    /// A value that is understood but does nothing yet.
    NotAvailable,
}

/// One thing wrong with a configuration or theme file, and where.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub kind: ProblemKind,
    /// The key the problem is about, as a dotted path. Empty for a problem with the
    /// whole file.
    pub path: String,
    pub file: Option<PathBuf>,
    /// The line in the file, counted from 1. Zero when the problem has no place.
    pub line: u32,
    /// The column on that line, counted from 1 in characters. Zero when unknown.
    pub column: u32,
    pub message: String,
    /// The key that was probably meant, for an unknown key that resembles one.
    pub suggestion: Option<String>,
}

impl Problem {
    /// Whether the problem leaves its whole file unusable.
    pub fn is_fatal(&self) -> bool {
        matches!(self.kind, ProblemKind::Syntax | ProblemKind::Io)
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(file) = &self.file {
            write!(f, "{}", file.display())?;
            if self.line > 0 {
                write!(f, ":{}", self.line)?;
                if self.column > 0 {
                    write!(f, ":{}", self.column)?;
                }
            }
            f.write_str(": ")?;
        }
        f.write_str(&self.message)?;
        if let Some(suggestion) = &self.suggestion {
            write!(f, " (did you mean `{suggestion}`?)")?;
        }
        Ok(())
    }
}

/// A line and a column in a file, both counted from 1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

impl Position {
    /// The position of a byte offset in `text`.
    pub fn of(text: &str, offset: usize) -> Self {
        let mut offset = offset.min(text.len());
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        let before = &text[..offset];
        let line_start = before.rfind('\n').map_or(0, |at| at + 1);
        let line = before.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let column = before[line_start..].chars().count() + 1;
        Self {
            line: u32::try_from(line).unwrap_or(u32::MAX),
            column: u32::try_from(column).unwrap_or(u32::MAX),
        }
    }
}

/// Whether a path segment can be written without quotes.
fn is_bare(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Writes a path as its segments joined by dots. A segment that holds anything but
/// letters, digits, `-` and `_` is quoted, as TOML would quote it: `keybindings."ctrl+."`.
pub fn join_path<S: AsRef<str>>(segments: &[S]) -> String {
    let mut path = String::new();
    for segment in segments {
        let segment = segment.as_ref();
        if !path.is_empty() {
            path.push('.');
        }
        if is_bare(segment) {
            path.push_str(segment);
        } else {
            path.push('"');
            for character in segment.chars() {
                if character == '"' || character == '\\' {
                    path.push('\\');
                }
                path.push(character);
            }
            path.push('"');
        }
    }
    path
}

/// Reads a path written by [`join_path`], or by hand in the same way.
pub fn split_path(path: &str) -> Result<Vec<String>, String> {
    let mut segments = Vec::new();
    let mut chars = path.chars().peekable();
    loop {
        let mut segment = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            loop {
                match chars.next() {
                    Some('"') => break,
                    Some('\\') => match chars.next() {
                        Some(escaped) => segment.push(escaped),
                        None => return Err(format!("`{path}` ends inside a quoted key")),
                    },
                    Some(character) => segment.push(character),
                    None => return Err(format!("`{path}` ends inside a quoted key")),
                }
            }
        } else {
            while let Some(character) = chars.peek() {
                if *character == '.' {
                    break;
                }
                segment.push(*character);
                chars.next();
            }
            if segment.is_empty() {
                return Err(format!("`{path}` has an empty key in it"));
            }
        }
        segments.push(segment);
        match chars.next() {
            None => return Ok(segments),
            Some('.') => {}
            Some(other) => {
                return Err(format!(
                    "`{path}` has `{other}` where a dot or the end was expected"
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_position_counts_lines_and_characters_from_one() {
        let text = "a = 1\nbé = 2\n\nc = 3";
        assert_eq!(Position::of(text, 0), Position { line: 1, column: 1 });
        assert_eq!(Position::of(text, 4), Position { line: 1, column: 5 });
        // The `=` of the second line, after a two-byte letter.
        let at = text.find("= 2").unwrap();
        assert_eq!(Position::of(text, at), Position { line: 2, column: 4 });
        assert_eq!(
            Position::of(text, text.len()),
            Position { line: 4, column: 6 }
        );
        assert_eq!(Position::of(text, 9999), Position { line: 4, column: 6 });
    }

    #[test]
    fn a_path_round_trips_with_keys_that_need_quoting() {
        let segments = ["keybindings", "ctrl+.", "a \"b\"", "plain-key_1"];
        let path = join_path(&segments);
        assert_eq!(path, r#"keybindings."ctrl+."."a \"b\"".plain-key_1"#);
        assert_eq!(split_path(&path).unwrap(), segments);
        assert_eq!(split_path("font.size").unwrap(), ["font", "size"]);
    }

    #[test]
    fn a_malformed_path_is_refused() {
        assert!(split_path("").is_err());
        assert!(split_path("font.").is_err());
        assert!(split_path(".size").is_err());
        assert!(split_path("\"open").is_err());
        assert!(split_path("\"a\"b").is_err());
    }

    #[test]
    fn a_problem_reads_as_a_compiler_message() {
        let problem = Problem {
            kind: ProblemKind::UnknownKey,
            path: "font.sizee".to_owned(),
            file: Some(PathBuf::from("config.toml")),
            line: 3,
            column: 1,
            message: "unknown key `font.sizee`".to_owned(),
            suggestion: Some("size".to_owned()),
        };
        assert_eq!(
            problem.to_string(),
            "config.toml:3:1: unknown key `font.sizee` (did you mean `size`?)"
        );
    }
}
