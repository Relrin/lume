//! `cargo xtask latency`: the time from a key pressed at the desktop to its echo on screen.
//!
//! The benchmarks time each part of a key's way inside the application: the page, the
//! session, the frame. This tool stands outside any terminal, as a person at the keyboard
//! does. It presses a key through the operating system's input queue and watches the
//! screen until the pixels at the cursor change, so what it measures includes the window
//! system, the webview or toolkit, the terminal's drawing and the compositor. It knows
//! nothing of the terminal, so the same run measures Lume, another terminal or a native
//! Rex client, and the reports it keeps can be set side by side.
//!
//! The terminal to measure must have the keyboard focus, run a program that echoes what
//! is typed (`cat` on Linux, `cmd.exe /d` on Windows) and have its cursor's blink and any
//! cursor effects turned off. The tool types `x` and Backspace in turn, so the line ends as it started. It stops
//! as soon as the focus moves to another window, so that nothing is typed there.
//!
//! The numbers are recorded, not held to a limit: they depend on the display's refresh
//! rate and the compositor as much as on the terminal, and only a comparison on the same
//! machine says anything.

#[cfg(target_os = "linux")]
mod x11;

#[cfg(windows)]
mod win32;

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::paths;

/// How many keys are timed unless another number is asked for.
const SAMPLES: usize = 200;

/// How long there is to bring the terminal to the front unless another time is asked for.
const WAIT: Duration = Duration::from_secs(5);

/// How long the window is watched, before anything is typed, for changes it makes by itself.
const QUIET: Duration = Duration::from_millis(1200);

/// How long the area at the cursor must stay as it is for the screen to count as settled.
const SETTLED: Duration = Duration::from_millis(60);

/// How long a key may take to show before it counts as missed.
const SAMPLE_TIMEOUT: Duration = Duration::from_secs(1);

/// How many keys may be missed before the run stops.
const MISSES: usize = 3;

/// The shortest and longest pause before a key. The pause is drawn at random, so that keys
/// do not keep the same place relative to the display's refresh.
const GAP_MS: (u64, u64) = (40, 140);

/// An area of the screen, in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    /// This area made larger by `margin` on every side and cut to `within`.
    fn grown(self, margin: i32, within: Rect) -> Rect {
        let left = (self.x - margin).max(within.x);
        let top = (self.y - margin).max(within.y);
        let right = (self.x + self.width as i32 + margin).min(within.x + within.width as i32);
        let bottom = (self.y + self.height as i32 + margin).min(within.y + within.height as i32);
        Rect {
            x: left,
            y: top,
            width: (right - left).max(0) as u32,
            height: (bottom - top).max(0) as u32,
        }
    }
}

/// The keys the tool types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    X,
    Backspace,
}

/// What the tool needs of the desktop: where the focused window is, its pixels, and keys.
pub trait Desktop {
    /// The window with the keyboard focus: something that tells it from other windows,
    /// and its content area on screen.
    fn focused(&mut self) -> Result<(u64, Rect)>;

    /// The title of a window that [`Desktop::focused`] named.
    fn title(&mut self, window: u64) -> Result<String>;

    /// The pixels of an area of the screen, four bytes each, row by row from the top. The
    /// first three bytes of a pixel are its colour; the fourth is not looked at.
    fn capture(&mut self, area: Rect) -> Result<Vec<u8>>;

    /// Presses and releases a key, as the keyboard would.
    fn press(&mut self, key: Key) -> Result<()>;
}

/// The desktop of the platform the tool runs on.
fn open() -> Result<Box<dyn Desktop>> {
    #[cfg(windows)]
    {
        Ok(Box::new(win32::Win32::open()?))
    }
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(x11::X11::open()?))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        bail!("the latency tool works on Windows and on Linux under X11, not on this system")
    }
}

/// What the command was asked to do.
#[derive(Debug, PartialEq)]
struct Options {
    /// The name the report is kept under, such as `lume-webgl2` or `ghostty`.
    label: String,
    samples: usize,
    wait: Duration,
    /// Text the focused window's title must contain before anything is typed into it.
    title: Option<String>,
    /// Print the kept reports side by side instead of measuring.
    table: bool,
}

impl Options {
    fn parse(args: &[&str]) -> Result<Options> {
        let mut options = Options {
            label: "terminal".to_owned(),
            samples: SAMPLES,
            wait: WAIT,
            title: None,
            table: false,
        };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let mut value = || {
                args.next()
                    .copied()
                    .with_context(|| format!("{arg} needs a value"))
            };
            match *arg {
                "--label" => {
                    let label = value()?;
                    let allowed = |c: char| c.is_ascii_alphanumeric() || "._-".contains(c);
                    if label.is_empty() || !label.chars().all(allowed) {
                        bail!("a label is letters, digits, `.`, `_` and `-`: {label}");
                    }
                    options.label = label.to_owned();
                }
                "--samples" => {
                    let samples: usize = value()?.parse().context("--samples takes a number")?;
                    if samples < 2 {
                        bail!("--samples must be at least 2");
                    }
                    // An even number, so that every `x` typed is taken back.
                    options.samples = samples.next_multiple_of(2);
                }
                "--wait" => {
                    let seconds: u64 = value()?.parse().context("--wait takes seconds")?;
                    options.wait = Duration::from_secs(seconds);
                }
                "--title" => options.title = Some(value()?.to_owned()),
                "--table" => options.table = true,
                other => bail!("unknown option for latency: {other}"),
            }
        }
        Ok(options)
    }
}

/// Where the reports are kept, one file per label.
fn reports_dir() -> PathBuf {
    paths::root().join("target").join("bench").join("latency")
}

/// The changed part of two captures of the same area, relative to that area.
fn changed_box(before: &[u8], after: &[u8], width: u32) -> Option<Rect> {
    let width = width as usize;
    let mut bounds: Option<(usize, usize, usize, usize)> = None;
    for (index, (a, b)) in before.chunks(4).zip(after.chunks(4)).enumerate() {
        if a[..3] == b[..3] {
            continue;
        }
        let (x, y) = (index % width, index / width);
        bounds = Some(match bounds {
            None => (x, y, x, y),
            Some((left, top, right, bottom)) => {
                (left.min(x), top.min(y), right.max(x), bottom.max(y))
            }
        });
    }
    bounds.map(|(left, top, right, bottom)| Rect {
        x: left as i32,
        y: top as i32,
        width: (right - left + 1) as u32,
        height: (bottom - top + 1) as u32,
    })
}

/// Whether two captures show the same colours.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.chunks(4).zip(b.chunks(4)).all(|(a, b)| a[..3] == b[..3])
}

/// Captures `area` until it differs from `before`: how long that took, the capture that
/// differed, and how many captures were made. `None` if it did not change in `timeout`.
fn until_changed(
    desktop: &mut dyn Desktop,
    area: Rect,
    before: &[u8],
    since: Instant,
    timeout: Duration,
) -> Result<Option<(Duration, Vec<u8>, u32)>> {
    let mut captures = 0;
    loop {
        let now = desktop.capture(area)?;
        captures += 1;
        if !same(&now, before) {
            return Ok(Some((since.elapsed(), now, captures)));
        }
        if since.elapsed() > timeout {
            return Ok(None);
        }
    }
}

/// Captures `area` until it has stayed the same for [`SETTLED`], and returns that capture.
fn settle(desktop: &mut dyn Desktop, area: Rect) -> Result<Vec<u8>> {
    let started = Instant::now();
    let mut last = desktop.capture(area)?;
    let mut since = Instant::now();
    loop {
        if since.elapsed() >= SETTLED {
            return Ok(last);
        }
        if started.elapsed() > Duration::from_secs(3) {
            bail!(
                "the area at the cursor kept changing; is the cursor's blink off, and is the \
                 program quiet?"
            );
        }
        let now = desktop.capture(area)?;
        if !same(&now, &last) {
            last = now;
            since = Instant::now();
        }
    }
}

/// Finds the cursor: types one `x` into the window and takes it back, and returns the
/// area that changed, with a margin, as the area to watch.
fn find_cursor(desktop: &mut dyn Desktop, window: Rect) -> Result<Rect> {
    let first = desktop.capture(window)?;
    let quiet_since = Instant::now();
    while quiet_since.elapsed() < QUIET {
        std::thread::sleep(Duration::from_millis(100));
        if !same(&desktop.capture(window)?, &first) {
            bail!(
                "the window changed while nothing was typed: turn the cursor's blink off and \
                 run a program that prints nothing by itself, such as `cat` or `cmd.exe /d`"
            );
        }
    }
    desktop.press(Key::X)?;
    let pressed = Instant::now();
    if until_changed(desktop, window, &first, pressed, Duration::from_secs(2))?.is_none() {
        bail!("typing `x` changed nothing in the window; does the program echo what is typed?");
    }
    let typed = settle(desktop, window)?;
    let changed = changed_box(&first, &typed, window.width)
        .context("the window changed and changed back before it could be looked at")?;
    desktop.press(Key::Backspace)?;
    settle(desktop, window)?;

    let on_screen = Rect {
        x: window.x + changed.x,
        y: window.y + changed.y,
        ..changed
    };
    // A cell's height either side catches the cursor moving, whichever way it is drawn.
    let margin = (changed.height as i32).max(4);
    Ok(on_screen.grown(margin, window))
}

/// One timed key.
struct Sample {
    key: Key,
    ms: f64,
    /// The average time one capture took while this key was waited for: how finely it
    /// was timed.
    capture_ms: f64,
}

/// A small random number source: the pauses between keys need no more.
struct Pauses(u64);

impl Pauses {
    fn new() -> Pauses {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0x9e37_79b9, |time| time.as_nanos() as u64);
        Pauses(seed | 1)
    }

    fn next(&mut self) -> Duration {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        let (low, high) = GAP_MS;
        Duration::from_millis(low + self.0 % (high - low + 1))
    }
}

fn time_keys(
    desktop: &mut dyn Desktop,
    window: u64,
    watch: Rect,
    samples: usize,
) -> Result<Vec<Sample>> {
    let mut pauses = Pauses::new();
    let mut timed = Vec::with_capacity(samples);
    let mut missed = 0;
    let mut before = settle(desktop, watch)?;
    for index in 0..samples {
        if desktop.focused()?.0 != window {
            bail!("the focus moved to another window; stopped so that nothing is typed there");
        }
        std::thread::sleep(pauses.next());
        // The pause may have let something change, such as a late part of the last echo.
        let now = desktop.capture(watch)?;
        if !same(&now, &before) {
            before = settle(desktop, watch)?;
        }
        let key = if index % 2 == 0 {
            Key::X
        } else {
            Key::Backspace
        };
        let pressed = Instant::now();
        desktop.press(key)?;
        match until_changed(desktop, watch, &before, pressed, SAMPLE_TIMEOUT)? {
            Some((elapsed, _, captures)) => {
                let ms = elapsed.as_secs_f64() * 1000.0;
                timed.push(Sample {
                    key,
                    ms,
                    capture_ms: ms / f64::from(captures),
                });
            }
            None => {
                missed += 1;
                println!("latency: key {} showed nothing in a second", index + 1);
                if missed >= MISSES {
                    bail!("{missed} keys showed nothing; is the terminal still echoing?");
                }
            }
        }
        before = settle(desktop, watch)?;
        if (index + 1) % 50 == 0 {
            println!("latency: {} of {samples} keys", index + 1);
        }
    }
    Ok(timed)
}

/// The value `p` samples in a hundred stay at or under.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn summary(mut ms: Vec<f64>) -> Value {
    ms.sort_by(f64::total_cmp);
    let mean = ms.iter().sum::<f64>() / ms.len().max(1) as f64;
    json!({
        "samples": ms.len(),
        "min": round(ms.first().copied().unwrap_or(0.0)),
        "p50": round(percentile(&ms, 50.0)),
        "p95": round(percentile(&ms, 95.0)),
        "p99": round(percentile(&ms, 99.0)),
        "max": round(ms.last().copied().unwrap_or(0.0)),
        "mean": round(mean),
    })
}

fn report(label: &str, watch: Rect, samples: &[Sample]) -> Value {
    let of = |key: Key| -> Vec<f64> {
        samples
            .iter()
            .filter(|sample| sample.key == key)
            .map(|sample| sample.ms)
            .collect()
    };
    let capture: Vec<f64> = samples.iter().map(|sample| sample.capture_ms).collect();
    json!({
        "label": label,
        "platform": std::env::consts::OS,
        "session": std::env::var("XDG_SESSION_TYPE").ok(),
        "measuredAt": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |time| time.as_secs()),
        "watched": { "width": watch.width, "height": watch.height },
        "keyToScreen": summary(samples.iter().map(|sample| sample.ms).collect()),
        "x": summary(of(Key::X)),
        "backspace": summary(of(Key::Backspace)),
        "captureMs": summary(capture),
        "all": samples.iter().map(|sample| round(sample.ms)).collect::<Vec<f64>>(),
    })
}

/// Prints the kept reports side by side, fastest first.
fn print_table() -> Result<()> {
    let dir = reports_dir();
    let mut reports: Vec<Value> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                let text = std::fs::read_to_string(&path)?;
                reports.push(
                    serde_json::from_str(&text)
                        .with_context(|| format!("{} is not a report", path.display()))?,
                );
            }
        }
    }
    if reports.is_empty() {
        println!("latency: no reports in {} yet", dir.display());
        return Ok(());
    }
    let p50 = |report: &Value| report["keyToScreen"]["p50"].as_f64().unwrap_or(f64::MAX);
    reports.sort_by(|a, b| p50(a).total_cmp(&p50(b)));
    let width = reports
        .iter()
        .filter_map(|report| report["label"].as_str())
        .map(str::len)
        .max()
        .unwrap_or(5)
        .max(5);
    println!(
        "{:width$}  {:>8}  {:>7}  {:>7}  {:>7}  {:>7}  {:>7}  {:>9}",
        "label", "platform", "keys", "p50", "p95", "p99", "max", "capture"
    );
    for report in &reports {
        let all = &report["keyToScreen"];
        let number = |value: &Value| value.as_f64().map_or("-".to_owned(), |n| format!("{n:.1}"));
        let platform = match report["session"].as_str() {
            Some(session) => format!("{}/{session}", report["platform"].as_str().unwrap_or("?")),
            None => report["platform"].as_str().unwrap_or("?").to_owned(),
        };
        println!(
            "{:width$}  {:>8}  {:>7}  {:>7}  {:>7}  {:>7}  {:>7}  {:>9}",
            report["label"].as_str().unwrap_or("?"),
            platform,
            all["samples"].as_u64().unwrap_or(0),
            number(&all["p50"]),
            number(&all["p95"]),
            number(&all["p99"]),
            number(&all["max"]),
            number(&report["captureMs"]["mean"]),
        );
    }
    println!("\nlatency: times in ms; `capture` is how long one look at the screen took");
    Ok(())
}

pub fn run(args: &[&str]) -> Result<()> {
    let options = Options::parse(args)?;
    if options.table {
        return print_table();
    }
    let mut desktop = open()?;

    println!(
        "latency: bring the terminal to measure to the front now. It should run `cat` (or \
         `cmd.exe /d` on Windows), with the cursor's blink and effects off. Timing starts \
         in {} seconds.",
        options.wait.as_secs()
    );
    std::thread::sleep(options.wait);
    let (window, area) = desktop.focused()?;
    if area.width == 0 || area.height == 0 {
        bail!("the focused window has no area on screen");
    }
    let title = desktop.title(window)?;
    if let Some(expected) = &options.title
        && !title.contains(expected.as_str())
    {
        bail!("the focused window is `{title}`, not one with `{expected}` in its title");
    }
    println!(
        "latency: finding the cursor in `{title}`, a window of {} by {} pixels",
        area.width, area.height
    );
    let watch = find_cursor(desktop.as_mut(), area)?;
    println!(
        "latency: watching {} by {} pixels at the cursor; typing {} keys",
        watch.width, watch.height, options.samples
    );
    let samples = time_keys(desktop.as_mut(), window, watch, options.samples)?;
    if samples.is_empty() {
        bail!("no key was timed");
    }

    let report = report(&options.label, watch, &samples);
    let dir = reports_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", options.label));
    std::fs::write(&path, serde_json::to_string_pretty(&report)?)
        .with_context(|| format!("writing {}", path.display()))?;

    let all = &report["keyToScreen"];
    println!(
        "\nlatency: {}: key to screen p50 {} ms, p95 {} ms, p99 {} ms, max {} ms over {} keys",
        options.label, all["p50"], all["p95"], all["p99"], all["max"], all["samples"]
    );
    println!(
        "latency: one look at the screen took {} ms on average, so times are that fine",
        report["captureMs"]["mean"]
    );
    println!("latency: the report is in {}", path.display());
    println!("latency: `cargo xtask latency --table` sets every kept report side by side");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels(width: u32, height: u32, lit: &[(u32, u32)]) -> Vec<u8> {
        let mut buffer = vec![0; (width * height * 4) as usize];
        for &(x, y) in lit {
            let at = ((y * width + x) * 4) as usize;
            buffer[at..at + 3].copy_from_slice(&[255, 255, 255]);
        }
        buffer
    }

    #[test]
    fn the_changed_part_of_two_captures_is_found() {
        let before = pixels(10, 5, &[]);
        let after = pixels(10, 5, &[(3, 1), (5, 3)]);
        assert_eq!(
            changed_box(&before, &after, 10),
            Some(Rect {
                x: 3,
                y: 1,
                width: 3,
                height: 3
            })
        );
        assert_eq!(changed_box(&before, &before, 10), None);
    }

    #[test]
    fn the_fourth_byte_of_a_pixel_is_not_looked_at() {
        let before = pixels(2, 1, &[]);
        let mut after = before.clone();
        after[3] = 0xff;
        assert!(same(&before, &after));
        assert_eq!(changed_box(&before, &after, 2), None);
    }

    #[test]
    fn a_grown_area_stays_inside_its_window() {
        let window = Rect {
            x: 100,
            y: 100,
            width: 50,
            height: 40,
        };
        let cell = Rect {
            x: 102,
            y: 130,
            width: 8,
            height: 16,
        };
        assert_eq!(
            cell.grown(16, window),
            Rect {
                x: 100,
                y: 114,
                width: 26,
                height: 26
            }
        );
    }

    #[test]
    fn percentiles_are_ranks_of_the_sorted_samples() {
        let sorted: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&sorted, 50.0), 50.0);
        assert_eq!(percentile(&sorted, 99.0), 99.0);
        assert_eq!(percentile(&sorted, 100.0), 100.0);
        assert_eq!(percentile(&[7.0], 99.0), 7.0);
        assert_eq!(percentile(&[], 50.0), 0.0);
    }

    #[test]
    fn options_are_read_and_checked() {
        let options = Options::parse(&["--label", "lume-webgl2", "--samples", "51"]).unwrap();
        assert_eq!(options.label, "lume-webgl2");
        assert_eq!(options.samples, 52);
        assert_eq!(options.wait, WAIT);
        assert!(!options.table);
        assert!(Options::parse(&["--table"]).unwrap().table);
        let titled = Options::parse(&["--title", "Lume"]).unwrap();
        assert_eq!(titled.title.as_deref(), Some("Lume"));
        assert!(Options::parse(&["--label", "../x"]).is_err());
        assert!(Options::parse(&["--samples"]).is_err());
        assert!(Options::parse(&["--samples", "1"]).is_err());
        assert!(Options::parse(&["--fast"]).is_err());
    }

    #[test]
    fn pauses_stay_between_their_bounds() {
        let mut pauses = Pauses::new();
        for _ in 0..1000 {
            let pause = pauses.next().as_millis() as u64;
            assert!((GAP_MS.0..=GAP_MS.1).contains(&pause), "{pause}");
        }
    }

    /// A desktop whose screen shows a cell that echoes the keys typed into it.
    struct Echoing {
        width: u32,
        height: u32,
        typed: usize,
    }

    impl Desktop for Echoing {
        fn focused(&mut self) -> Result<(u64, Rect)> {
            Ok((
                1,
                Rect {
                    x: 0,
                    y: 0,
                    width: self.width,
                    height: self.height,
                },
            ))
        }

        fn title(&mut self, _window: u64) -> Result<String> {
            Ok("echo".to_owned())
        }

        fn capture(&mut self, area: Rect) -> Result<Vec<u8>> {
            let lit: Vec<(u32, u32)> = (0..self.typed as u32)
                .map(|n| (4 + n, 6))
                .filter(|&(x, y)| {
                    x as i32 >= area.x
                        && (x as i32) < area.x + area.width as i32
                        && y as i32 >= area.y
                        && (y as i32) < area.y + area.height as i32
                })
                .map(|(x, y)| (x - area.x as u32, y - area.y as u32))
                .collect();
            Ok(pixels(area.width, area.height, &lit))
        }

        fn press(&mut self, key: Key) -> Result<()> {
            match key {
                Key::X => self.typed += 1,
                Key::Backspace => self.typed -= 1,
            }
            Ok(())
        }
    }

    #[test]
    fn keys_are_timed_at_the_cursor_and_the_line_ends_as_it_started() {
        let mut desktop = Echoing {
            width: 40,
            height: 20,
            typed: 0,
        };
        let (window, area) = desktop.focused().unwrap();
        let watch = find_cursor(&mut desktop, area).unwrap();
        assert!(
            watch.x <= 4 && watch.x + watch.width as i32 > 4,
            "{watch:?}"
        );
        assert!(
            watch.y <= 6 && watch.y + watch.height as i32 > 6,
            "{watch:?}"
        );
        let samples = time_keys(&mut desktop, window, watch, 4).unwrap();
        assert_eq!(samples.len(), 4);
        assert_eq!(desktop.typed, 0);
        let report = report("test", watch, &samples);
        assert_eq!(report["keyToScreen"]["samples"], 4);
        assert_eq!(report["x"]["samples"], 2);
        assert_eq!(report["backspace"]["samples"], 2);
    }
}
