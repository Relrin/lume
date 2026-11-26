//! Brings fresh terminals up to the state of another one, part-way through every
//! recording under `tests/corpus/`, and checks that they show the same thing, then and
//! after the rest of the recording.
//!
//! A snapshot must bring back everything. Replayed output must bring back the screen in
//! use, the cursor, the modes and the title; what it cannot bring back is listed in
//! [`REPLAY_GAPS`] and checked to be the only difference.
//!
//! `cargo test -p lume-vt --release --test hydrate -- --ignored --nocapture` prints how
//! long saving and loading take with deep scrollback.

use std::path::{Path, PathBuf};
use std::time::Instant;

use lume_vt::corpus::{Recording, dump, recordings};
use lume_vt::{Format, Terminal};

/// The points of each recording at which a copy is made, as fractions of its input.
const CUTS: [f64; 4] = [0.25, 0.5, 0.75, 1.0];

/// What replayed output is known not to bring back, by recording: the reason, and the
/// dump lines allowed to differ (by prefix) once the rest of the recording has been fed
/// to both terminals. Right after the copy is made nothing may differ.
const REPLAY_GAPS: &[(&str, &str, &[&str])] = &[(
    "05-resize-reflow",
    "a line wrapped onto several rows comes back as several lines, so a later resize \
     does not flow it again",
    &["cursor ", "0"],
)];

fn corpus() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives at <root>/crates/lume-vt")
        .join("tests")
        .join("corpus");
    recordings(&root).expect("list recordings")
}

/// A terminal fed `input[..end]` of a recording, with its resizes, and the resizes still
/// to come.
fn fed_to(recording: &Recording, end: usize) -> Terminal {
    let script = &recording.script;
    let mut term = Terminal::new(script.cols, script.rows).unwrap();
    let mut fed = 0;
    for r in script.resize.iter().filter(|r| r.at <= end) {
        term.write(&recording.input[fed..r.at]);
        term.resize(r.cols, r.rows).unwrap();
        fed = r.at;
    }
    term.write(&recording.input[fed..end]);
    term
}

/// Feeds `input[start..]` with the resizes that fall in it.
fn feed_rest(term: &mut Terminal, recording: &Recording, start: usize) {
    let mut fed = start;
    for r in recording.script.resize.iter().filter(|r| r.at > start) {
        term.write(&recording.input[fed..r.at]);
        term.resize(r.cols, r.rows).unwrap();
        fed = r.at;
    }
    term.write(&recording.input[fed..]);
}

/// A dump without the replies section: what a terminal answered is not part of what it
/// shows, and a copy has answered nothing.
fn state(term: &Terminal) -> String {
    let full = dump(term).unwrap();
    let cut = full.find("--- replies").unwrap_or(full.len());
    full[..cut].to_owned()
}

/// The lines of `a` and `b` that differ, other than ones starting with an allowed prefix.
fn differences(a: &str, b: &str, allowed: &[&str]) -> Vec<String> {
    let lines_a: Vec<&str> = a.lines().collect();
    let lines_b: Vec<&str> = b.lines().collect();
    let mut out = Vec::new();
    for i in 0..lines_a.len().max(lines_b.len()) {
        let (x, y) = (lines_a.get(i).copied(), lines_b.get(i).copied());
        if x == y {
            continue;
        }
        let excused = |line: Option<&str>| {
            line.is_some_and(|line| allowed.iter().any(|prefix| line.starts_with(prefix)))
        };
        if excused(x) && excused(y) {
            continue;
        }
        out.push(format!("  line {i}: {x:?} vs {y:?}"));
    }
    out
}

fn name(dir: &Path) -> String {
    dir.file_name().unwrap().to_string_lossy().into_owned()
}

/// Whether this build of the library lays out its pages otherwise than the builds that
/// write the snapshot files. A page of the terminal is sized in pages of memory, which are
/// four times as large on Apple silicon, so its snapshots of a screen with scrollback are
/// cut into pages elsewhere and differ in their bytes. They are not written there.
const PAGES_OF_ITS_OWN: bool = cfg!(all(target_os = "macos", target_arch = "aarch64"));

/// Whether the file at `path` is the snapshot of `term` that this build wrote as
/// `written`: byte for byte, or, where the build lays out its pages otherwise, by
/// bringing back what `term` shows and the end of its scrollback. Scrollback past its
/// limit is let go of a page at a time, so pages of another size keep another number of
/// the oldest lines; what one of the two kept is the end of what the other kept.
fn is_current(path: &Path, written: &[u8], term: &Terminal) -> bool {
    let Ok(kept) = std::fs::read(path) else {
        return false;
    };
    if kept == written {
        return true;
    }
    if !PAGES_OF_ITS_OWN {
        return false;
    }
    let Ok(loaded) = Terminal::from_snapshot(&kept) else {
        return false;
    };
    let text = |term: &Terminal| term.format(Format::Plain).unwrap().bytes;
    let (kept, here) = (text(&loaded), text(term));
    let (shorter, longer) = if kept.len() <= here.len() {
        (&kept, &here)
    } else {
        (&here, &kept)
    };
    state(&loaded) == state(term) && !shorter.is_empty() && longer.ends_with(shorter)
}

#[test]
fn a_snapshot_reproduces_every_recording_at_every_cut() {
    let mut failures = Vec::new();
    for dir in corpus() {
        let recording = Recording::load(&dir).unwrap();
        for cut in CUTS {
            let end = (recording.input.len() as f64 * cut) as usize;
            let mut original = fed_to(&recording, end);
            let snapshot = original.snapshot().unwrap();
            let mut copy = Terminal::from_snapshot(&snapshot).unwrap();
            let found = differences(&state(&original), &state(&copy), &[]);
            if !found.is_empty() {
                failures.push(format!(
                    "{} at {end} bytes:\n{}",
                    name(&dir),
                    found.join("\n")
                ));
                continue;
            }
            feed_rest(&mut original, &recording, end);
            feed_rest(&mut copy, &recording, end);
            let found = differences(&state(&original), &state(&copy), &[]);
            if !found.is_empty() {
                failures.push(format!(
                    "{} after continuing from {end} bytes:\n{}",
                    name(&dir),
                    found.join("\n")
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

#[test]
fn replayed_output_reproduces_every_recording_at_every_cut() {
    let mut failures = Vec::new();
    for dir in corpus() {
        let recording = Recording::load(&dir).unwrap();
        let gap = REPLAY_GAPS.iter().find(|(n, ..)| *n == name(&dir));
        let allowed = gap.map_or(&[][..], |(_, _, lines)| *lines);
        for cut in CUTS {
            let end = (recording.input.len() as f64 * cut) as usize;
            let mut original = fed_to(&recording, end);
            let replay = original.format(Format::Replay).unwrap();
            let mut copy = Terminal::new(original.cols(), original.rows()).unwrap();
            copy.write(&replay.bytes);
            let found = differences(&state(&original), &state(&copy), &[]);
            if !found.is_empty() {
                failures.push(format!(
                    "{} at {end} bytes:\n{}",
                    name(&dir),
                    found.join("\n")
                ));
                continue;
            }
            feed_rest(&mut original, &recording, end);
            feed_rest(&mut copy, &recording, end);
            let found = differences(&state(&original), &state(&copy), allowed);
            if !found.is_empty() {
                failures.push(format!(
                    "{} after continuing from {end} bytes:\n{}",
                    name(&dir),
                    found.join("\n")
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

/// The snapshot of each whole recording that the webview's engine is tested against: it
/// must load there and show what `expect.txt` says. Written by this build of the
/// library with `UPDATE_GOLDEN=1`, and checked to be what this build writes (see
/// [`PAGES_OF_ITS_OWN`]).
#[test]
fn snapshots_for_the_other_build_are_current() {
    let mut stale = Vec::new();
    for dir in corpus() {
        let recording = Recording::load(&dir).unwrap();
        let term = recording.replay().unwrap();
        let snapshot = term.snapshot().unwrap();
        assert_eq!(
            snapshot,
            term.snapshot().unwrap(),
            "{}: a snapshot of the same terminal came out different",
            name(&dir)
        );
        let path = dir.join("snapshot.bin");
        if std::env::var_os("UPDATE_GOLDEN").is_some_and(|v| v == "1") {
            std::fs::write(&path, &snapshot).unwrap();
            continue;
        }
        if !is_current(&path, &snapshot, &term) {
            stale.push(name(&dir));
        }
    }
    assert!(
        stale.is_empty(),
        "snapshot.bin is missing or stale for {stale:?}; run with UPDATE_GOLDEN=1"
    );
}

/// The lines [`history_snapshot`] writes: `history-0001` and so on.
const HISTORY_LINES: usize = 3000;

/// A snapshot with deep scrollback, for the webview's build to restore part of:
/// `tests/hydrate/history.bin`, written and checked as the corpus snapshots are.
#[test]
fn the_history_snapshot_for_the_other_build_is_current() {
    let mut term = Terminal::new(40, 5).unwrap();
    term.set_scrollback_limits(HISTORY_LINES * 2, usize::MAX)
        .unwrap();
    for i in 1..=HISTORY_LINES {
        term.write(format!("history-{i:04}\r\n").as_bytes());
    }
    assert!(term.scrollback_rows().unwrap() >= HISTORY_LINES - 5);
    let snapshot = term.snapshot().unwrap();
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .join("tests")
        .join("hydrate")
        .join("history.bin");
    if std::env::var_os("UPDATE_GOLDEN").is_some_and(|v| v == "1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &snapshot).unwrap();
        return;
    }
    assert!(
        is_current(&path, &snapshot, &term),
        "{} is missing or stale; run with UPDATE_GOLDEN=1",
        path.display()
    );
}

#[test]
#[ignore = "a measurement: run in release with --nocapture"]
fn time_to_save_and_load_deep_scrollback() {
    for lines in [1_000usize, 10_000, 100_000] {
        let mut term = Terminal::new(120, 40).unwrap();
        term.set_scrollback_limits(lines, usize::MAX).unwrap();
        for i in 0..lines + 40 {
            term.write(
                format!(
                    "\x1b[3{}m{i:>8}\x1b[0m the quick brown fox jumps over the lazy dog\r\n",
                    i % 8
                )
                .as_bytes(),
            );
        }

        let started = Instant::now();
        let snapshot = term.snapshot().unwrap();
        let encoded = started.elapsed();
        // The webview's build is timed loading the same snapshot, by
        // `packages/vt/test/snapshot.bench.test.ts`.
        if let Some(dir) = std::env::var_os("LUME_SNAPSHOT_OUT") {
            std::fs::write(
                std::path::Path::new(&dir).join(format!("snapshot-{lines}.bin")),
                &snapshot,
            )
            .unwrap();
        }
        let started = Instant::now();
        let copy = Terminal::from_snapshot(&snapshot).unwrap();
        let decoded = started.elapsed();
        drop(copy);

        let started = Instant::now();
        let replay = term.format(Format::Replay).unwrap();
        let formatted = started.elapsed();
        let started = Instant::now();
        let mut rebuilt = Terminal::new(120, 40).unwrap();
        rebuilt.write(&replay.bytes);
        let replayed = started.elapsed();

        println!(
            "{lines:>7} lines (kept {:>7}): snapshot {:>9} B, save {encoded:?}, load {decoded:?}; \
             replay {:>9} B, format {formatted:?}, write {replayed:?}; memory {} B",
            term.scrollback_rows().unwrap(),
            snapshot.len(),
            replay.bytes.len(),
            term.memory_bytes().unwrap(),
        );
    }
}
