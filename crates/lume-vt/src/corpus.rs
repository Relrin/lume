//! The golden corpus: recorded terminal byte streams with expected screen dumps.
//!
//! A recording is a directory holding:
//!
//! - `input.bin`: the bytes a program wrote to the terminal.
//! - `script.toml`: the initial grid size, and resizes placed at byte offsets of the input.
//! - `expect.txt`: the expected screen dump after the whole input has been fed.
//!
//! The dump format is plain text and deliberately simple, so that the WebAssembly build
//! can produce the same text from TypeScript and the two can be compared byte for
//! byte.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Attrs, Color, Terminal, Wide};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// `script.toml` of a recording.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Script {
    /// One line saying what the recording exercises.
    pub description: String,
    pub cols: u16,
    pub rows: u16,
    /// Resizes, in order of `at`.
    #[serde(default)]
    pub resize: Vec<Resize>,
}

/// A resize applied when `at` bytes of the input have been fed.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resize {
    pub at: usize,
    pub cols: u16,
    pub rows: u16,
}

/// A loaded recording.
#[derive(Debug)]
pub struct Recording {
    pub dir: PathBuf,
    pub script: Script,
    pub input: Vec<u8>,
}

impl Recording {
    pub fn load(dir: &Path) -> Result<Self, BoxError> {
        let script_path = dir.join("script.toml");
        let script: Script = toml::from_str(&fs::read_to_string(&script_path)?)
            .map_err(|e| format!("{}: {e}", script_path.display()))?;
        let input = fs::read(dir.join("input.bin"))?;

        let mut previous = 0;
        for r in &script.resize {
            if r.at < previous || r.at > input.len() {
                return Err(format!(
                    "{}: resize offsets must be ascending and within the input ({} bytes)",
                    script_path.display(),
                    input.len()
                )
                .into());
            }
            previous = r.at;
        }

        Ok(Self {
            dir: dir.to_path_buf(),
            script,
            input,
        })
    }

    /// Feed the recording into a fresh terminal, applying each resize at its offset.
    pub fn replay(&self) -> Result<Terminal, BoxError> {
        let mut term = Terminal::new(self.script.cols, self.script.rows)?;
        let mut fed = 0;
        for r in &self.script.resize {
            term.write(&self.input[fed..r.at]);
            term.resize(r.cols, r.rows)?;
            fed = r.at;
        }
        term.write(&self.input[fed..]);
        Ok(term)
    }

    /// Replay and compare with `expect.txt`.
    ///
    /// With the environment variable `UPDATE_GOLDEN=1`, writes `expect.txt` instead. A
    /// golden written this way records what the library did, not what is correct: read it
    /// before committing it.
    pub fn check(&self) -> Result<(), BoxError> {
        let actual = dump(&self.replay()?)?;
        let expect_path = self.dir.join("expect.txt");

        if std::env::var_os("UPDATE_GOLDEN").is_some_and(|v| v == "1") {
            fs::write(&expect_path, &actual)?;
            return Ok(());
        }

        let expected = fs::read_to_string(&expect_path)
            .map_err(|e| {
                format!(
                    "{}: {e} (run with UPDATE_GOLDEN=1 to create it)",
                    expect_path.display()
                )
            })?
            .replace("\r\n", "\n");
        if expected == actual {
            return Ok(());
        }
        Err(format!(
            "{} does not match.\n{}",
            expect_path.display(),
            first_difference(&expected, &actual)
        )
        .into())
    }
}

/// Every recording directory under `root`, sorted by name.
pub fn recordings(root: &Path) -> Result<Vec<PathBuf>, BoxError> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.join("script.toml").is_file())
        .collect();
    dirs.sort();
    Ok(dirs)
}

/// The screen dump of a terminal's active area.
///
/// ```text
/// size 80x24
/// cursor 3,1 visible
/// screen primary
/// --- text
/// 00|hello
/// 01|red
/// ...
/// --- attrs
/// 01 0..3 fg=palette:1
/// ```
///
/// Text rows have trailing blanks removed. The attrs section lists runs of adjacent cells
/// that share a style other than the default; `a..b` is a half-open column range.
pub fn dump(term: &Terminal) -> Result<String, BoxError> {
    let mut out = String::new();
    writeln!(out, "size {}x{}", term.cols(), term.rows())?;

    let cursor = term.cursor()?;
    write!(out, "cursor {},{}", cursor.x, cursor.y)?;
    write!(
        out,
        " {}",
        if cursor.visible { "visible" } else { "hidden" }
    )?;
    if cursor.pending_wrap {
        out.push_str(" pending-wrap");
    }
    out.push('\n');

    let screen = if term.alternate_screen()? {
        "alternate"
    } else {
        "primary"
    };
    writeln!(out, "screen {screen}")?;

    let mut attrs = String::new();
    writeln!(out, "--- text")?;
    for row in 0..term.rows() {
        let mut line = String::new();
        let mut run: Option<(u16, String)> = None;
        for col in 0..term.cols() {
            let cell = term.cell(col, row)?;
            match cell.wide {
                Wide::SpacerTail | Wide::SpacerHead => {}
                _ if cell.text.is_empty() => line.push(' '),
                _ => line.push_str(&cell.text),
            }

            let style = describe_style(cell.fg, cell.bg, cell.attrs);
            match &run {
                Some((_, current)) if *current == style => {}
                _ => {
                    flush_run(&mut attrs, row, run.take(), col)?;
                    if !style.is_empty() {
                        run = Some((col, style));
                    }
                }
            }
        }
        flush_run(&mut attrs, row, run.take(), term.cols())?;
        writeln!(out, "{row:02}|{}", line.trim_end_matches(' '))?;
    }
    writeln!(out, "--- attrs")?;
    out.push_str(&attrs);
    Ok(out)
}

fn flush_run(
    attrs: &mut String,
    row: u16,
    run: Option<(u16, String)>,
    end: u16,
) -> std::fmt::Result {
    if let Some((start, style)) = run {
        writeln!(attrs, "{row:02} {start}..{end} {style}")?;
    }
    Ok(())
}

/// A stable, space-separated description of a non-default style; empty for the default.
fn describe_style(fg: Color, bg: Color, attrs: Attrs) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(c) = describe_color(fg) {
        parts.push(format!("fg={c}"));
    }
    if let Some(c) = describe_color(bg) {
        parts.push(format!("bg={c}"));
    }
    for (on, name) in [
        (attrs.bold, "bold"),
        (attrs.faint, "faint"),
        (attrs.italic, "italic"),
        (attrs.inverse, "inverse"),
        (attrs.strikethrough, "strikethrough"),
    ] {
        if on {
            parts.push(name.to_owned());
        }
    }
    if attrs.underline != 0 {
        parts.push(format!("underline:{}", attrs.underline));
    }
    parts.join(" ")
}

fn describe_color(color: Color) -> Option<String> {
    match color {
        Color::Default => None,
        Color::Palette(i) => Some(format!("palette:{i}")),
        Color::Rgb(r, g, b) => Some(format!("rgb:{r:02x}{g:02x}{b:02x}")),
    }
}

fn first_difference(expected: &str, actual: &str) -> String {
    let mut exp = expected.lines();
    let mut act = actual.lines();
    let mut line_no = 1;
    loop {
        match (exp.next(), act.next()) {
            (Some(e), Some(a)) if e == a => line_no += 1,
            (e, a) => {
                return format!(
                    "first difference at line {line_no}:\n  expected: {}\n  actual:   {}",
                    e.unwrap_or("<end of file>"),
                    a.unwrap_or("<end of file>")
                );
            }
        }
    }
}
