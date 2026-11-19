//! Turning places in a plugin's bundle back into places in its source.
//!
//! A plugin is written in TypeScript and shipped bundled into one JavaScript file. An
//! error's stack names lines of the bundle, which say little to the author; when the
//! package ships the bundle's source map, the stack is written with the source's files
//! and lines instead. Only what that takes is read: the version 3 format's `sources` and
//! `mappings`.

use serde::Deserialize;

/// One place in the source a stretch of a generated line comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Segment {
    column: u32,
    source: u32,
    line: u32,
    source_column: u32,
}

/// A source map, read.
#[derive(Debug, Default)]
pub struct SourceMap {
    sources: Vec<String>,
    /// For each generated line, its segments by column.
    lines: Vec<Vec<Segment>>,
}

#[derive(Deserialize)]
struct Raw {
    version: u32,
    #[serde(default)]
    sources: Vec<String>,
    #[serde(default, rename = "sourceRoot")]
    source_root: Option<String>,
    mappings: String,
}

/// The value of one base-64 digit of a variable-length quantity.
fn digit(byte: u8) -> Option<i64> {
    Some(match byte {
        b'A'..=b'Z' => i64::from(byte - b'A'),
        b'a'..=b'z' => i64::from(byte - b'a') + 26,
        b'0'..=b'9' => i64::from(byte - b'0') + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    })
}

/// The numbers of one segment.
fn numbers(text: &str) -> Option<Vec<i64>> {
    let mut found = Vec::new();
    let mut value: i64 = 0;
    let mut shift = 0;
    for byte in text.bytes() {
        let digit = digit(byte)?;
        value += (digit & 31) << shift;
        if digit & 32 != 0 {
            shift += 5;
            if shift > 60 {
                return None;
            }
        } else {
            let negative = value & 1 == 1;
            let magnitude = value >> 1;
            found.push(if negative { -magnitude } else { magnitude });
            value = 0;
            shift = 0;
        }
    }
    (shift == 0).then_some(found)
}

impl SourceMap {
    /// Reads a source map; one that cannot be read gives none.
    pub fn parse(text: &str) -> Option<Self> {
        let raw: Raw = serde_json::from_str(text).ok()?;
        if raw.version != 3 {
            return None;
        }
        let root = raw.source_root.unwrap_or_default();
        let sources = raw
            .sources
            .into_iter()
            .map(|source| {
                let joined = if root.is_empty() {
                    source
                } else {
                    format!("{}/{source}", root.trim_end_matches('/'))
                };
                // Bundlers write paths relative to the bundle: what follows the climbs is
                // what the author recognises.
                joined
                    .trim_start_matches("webpack://")
                    .split('/')
                    .filter(|part| *part != ".." && *part != ".")
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .collect();
        let (mut source, mut line, mut column) = (0i64, 0i64, 0i64);
        let mut lines = Vec::new();
        for generated in raw.mappings.split(';') {
            let mut segments = Vec::new();
            let mut generated_column = 0i64;
            for part in generated.split(',').filter(|part| !part.is_empty()) {
                let numbers = numbers(part)?;
                generated_column += numbers[0];
                if numbers.len() >= 4 {
                    source += numbers[1];
                    line += numbers[2];
                    column += numbers[3];
                    segments.push(Segment {
                        column: u32::try_from(generated_column).ok()?,
                        source: u32::try_from(source).ok()?,
                        line: u32::try_from(line).ok()?,
                        source_column: u32::try_from(column).ok()?,
                    });
                }
            }
            lines.push(segments);
        }
        Some(Self { sources, lines })
    }

    /// The source file, line and column (counted from 1) of a place in the bundle
    /// (counted from 1).
    pub fn find(&self, line: u32, column: u32) -> Option<(&str, u32, u32)> {
        let segments = self
            .lines
            .get(usize::try_from(line.checked_sub(1)?).ok()?)?;
        let column = column.saturating_sub(1);
        let segment = segments
            .iter()
            .rev()
            .find(|segment| segment.column <= column)
            .or_else(|| segments.first())?;
        let source = self.sources.get(usize::try_from(segment.source).ok()?)?;
        Some((source, segment.line + 1, segment.source_column + 1))
    }

    /// A stack with each place in `file` written as its place in the source.
    pub fn rewrite(&self, stack: &str, file: &str) -> String {
        let needle = format!("{file}:");
        let mut out = String::with_capacity(stack.len());
        let mut rest = stack;
        while let Some(at) = rest.find(&needle) {
            out.push_str(&rest[..at]);
            let after = &rest[at + needle.len()..];
            let digits = |text: &str| text.bytes().take_while(u8::is_ascii_digit).count();
            let line_len = digits(after);
            let line: Option<u32> = after[..line_len].parse().ok();
            let (column, used) = match after[line_len..].strip_prefix(':') {
                Some(more) => {
                    let column_len = digits(more);
                    (more[..column_len].parse().ok(), line_len + 1 + column_len)
                }
                None => (Some(1), line_len),
            };
            match line
                .zip(column)
                .and_then(|(line, column)| self.find(line, column))
            {
                Some((source, line, column)) => out.push_str(&format!("{source}:{line}:{column}")),
                None => out.push_str(&rest[at..at + needle.len() + used]),
            }
            rest = &after[used..];
        }
        out.push_str(rest);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two lines of bundle: line 1 maps column 0 to src/a.ts 1:1 and column 10 to
    // src/a.ts 3:5; line 2 maps column 2 to src/b.ts 10:1.
    const MAP: &str = r#"{
        "version": 3,
        "sources": ["../src/a.ts", "../src/b.ts"],
        "mappings": "AAAA,UAEI;ECOJ"
    }"#;

    #[test]
    fn places_in_the_bundle_are_found_in_the_source() {
        let map = SourceMap::parse(MAP).unwrap();
        assert_eq!(map.find(1, 1), Some(("src/a.ts", 1, 1)));
        assert_eq!(map.find(1, 12), Some(("src/a.ts", 3, 5)));
        assert_eq!(map.find(2, 5), Some(("src/b.ts", 10, 1)));
        assert_eq!(map.find(9, 1), None);
    }

    #[test]
    fn a_stack_is_written_with_the_sources_places() {
        let map = SourceMap::parse(MAP).unwrap();
        let stack = "    at refresh (dist/host.js:1:12)\n    at <anonymous> (dist/host.js:2:3)\n    at native";
        assert_eq!(
            map.rewrite(stack, "dist/host.js"),
            "    at refresh (src/a.ts:3:5)\n    at <anonymous> (src/b.ts:10:1)\n    at native"
        );
        // A place the map does not know is left as it was.
        assert_eq!(
            map.rewrite("at x (dist/host.js:40:1)", "dist/host.js"),
            "at x (dist/host.js:40:1)"
        );
    }

    #[test]
    fn a_map_that_is_not_one_is_none() {
        assert!(SourceMap::parse("{}").is_none());
        assert!(SourceMap::parse(r#"{"version": 2, "mappings": ""}"#).is_none());
        assert!(SourceMap::parse(r#"{"version": 3, "mappings": "!!"}"#).is_none());
    }
}
