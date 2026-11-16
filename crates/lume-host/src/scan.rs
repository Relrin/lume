//! Finds what a program reports about itself in the bytes it prints.
//!
//! A shell says where it is, when a command starts and how it ended; a program sets
//! the window's title, rings the bell, or says how its work stands. All of that
//! travels in the terminal's output as control strings. The scanner follows just
//! enough of the output's structure to find those strings wherever a read happened to
//! cut them, and to pass over the other kinds of string a terminal knows, so that what
//! is inside one is not mistaken for a report.
//!
//! It only looks. The bytes are not its to change, delay or drop: they go to the
//! client as they came, and the client's terminal makes of them what it will. What the
//! scanner finds is a hint for display and nothing more, since whatever runs in a
//! terminal, on this machine or another, can print any of it.

use lume_proto::{BlockKind, ProgramState, ProgramStatus};

use crate::tuning::{
    COMMAND_MAX_CHARS, CWD_MAX_BYTES, OSC_MAX_BYTES, STATUS_APP_MAX_BYTES, STATUS_ID_DEPTH_MAX,
    STATUS_ID_MAX_BYTES, STATUS_KEY_MAX_BYTES, STATUS_MESSAGE_MAX_BYTES,
    STATUS_MESSAGE_MAX_ENCODED, STATUS_SEGMENT_MAX_BYTES, STATUS_SEQUENCE_MAX_BYTES,
    STATUS_TITLE_MAX_BYTES, STATUS_TITLE_MAX_ENCODED, TITLE_MAX_CHARS,
};

const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;
const ESC: u8 = 0x1b;

/// One thing found in the output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Report {
    Title(String),
    /// The working directory, as the URL the program gave.
    Cwd(String),
    Bell,
    /// The shell is about to show its prompt.
    Prompt,
    /// The shell is about to run a command line, which it may have named.
    CommandStarted(Option<String>),
    /// The command line has ended, with this exit code if the shell gave one.
    CommandFinished(Option<i32>),
    Status(ProgramStatus),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    /// After an escape character.
    Escape,
    /// Inside an operating-system command, whose text is being kept.
    Osc,
    /// After an escape character inside an operating-system command.
    OscEscape,
    /// Inside a string of another kind, whose text is of no interest.
    Skip,
    /// After an escape character inside such a string.
    SkipEscape,
}

/// Follows one terminal's output. Feed it every byte, in order.
#[derive(Debug)]
pub(crate) struct Scanner {
    state: State,
    /// The text of the command being read, up to [`OSC_MAX_BYTES`] of it.
    text: Vec<u8>,
    /// The command being read is longer than is kept, and will be passed over.
    oversized: bool,
}

/// The place of the first escape or bell character in `bytes`.
///
/// Nearly everything a program prints is neither, and all of it passes through here,
/// so the bytes are looked at eight at a time.
fn next_control(bytes: &[u8]) -> Option<usize> {
    const ONES: u64 = 0x0101_0101_0101_0101;
    const HIGHS: u64 = 0x8080_8080_8080_8080;
    // Sets the high bit of the lowest byte of `word` that equals `byte`. Bytes above
    // it may be marked wrongly, which does not matter to a search for the first.
    let marks = |word: u64, byte: u8| {
        let differs = word ^ (ONES * u64::from(byte));
        differs.wrapping_sub(ONES) & !differs & HIGHS
    };
    let (words, rest) = bytes.as_chunks::<8>();
    for (index, word) in words.iter().enumerate() {
        let word = u64::from_le_bytes(*word);
        let found = marks(word, ESC) | marks(word, BEL);
        if found != 0 {
            return Some(index * 8 + found.trailing_zeros() as usize / 8);
        }
    }
    rest.iter()
        .position(|&byte| byte == ESC || byte == BEL)
        .map(|at| words.len() * 8 + at)
}

impl Scanner {
    pub(crate) fn new() -> Self {
        Self {
            state: State::Ground,
            text: Vec::new(),
            oversized: false,
        }
    }

    /// Takes the next bytes of the output and calls `found` for each report that is
    /// complete with them, in order.
    pub(crate) fn feed(&mut self, bytes: &[u8], mut found: impl FnMut(Report)) {
        let mut at = 0;
        while at < bytes.len() {
            match self.state {
                State::Ground => {
                    let Some(control) = next_control(&bytes[at..]) else {
                        return;
                    };
                    at += control;
                    if bytes[at] == BEL {
                        found(Report::Bell);
                    } else {
                        self.state = State::Escape;
                    }
                    at += 1;
                }
                State::Escape => {
                    let byte = bytes[at];
                    at += 1;
                    match byte {
                        b']' => {
                            self.text.clear();
                            self.oversized = false;
                            self.state = State::Osc;
                        }
                        // Device control, application command, privacy message and
                        // start of string: each runs to a string terminator.
                        b'P' | b'_' | b'^' | b'X' => self.state = State::Skip,
                        // A control character in the middle of an escape sequence is
                        // acted on, and the sequence carries on after it.
                        BEL => found(Report::Bell),
                        ESC => {}
                        CAN | SUB => self.state = State::Ground,
                        0x00..=0x1f => {}
                        // Any other sequence is of no interest. Its remaining bytes
                        // are passed over as text would be, which finds the same
                        // bells and escapes a terminal acts on while inside one.
                        _ => self.state = State::Ground,
                    }
                }
                State::Osc => {
                    let rest = &bytes[at..];
                    let run = rest
                        .iter()
                        .position(|&byte| byte < 0x20)
                        .unwrap_or(rest.len());
                    self.keep(&rest[..run]);
                    at += run;
                    let Some(&control) = bytes.get(at) else {
                        return;
                    };
                    at += 1;
                    match control {
                        BEL => {
                            self.finish(&mut found);
                            self.state = State::Ground;
                        }
                        ESC => self.state = State::OscEscape,
                        CAN | SUB => self.state = State::Ground,
                        // Other control characters are not part of the text.
                        _ => {}
                    }
                }
                State::OscEscape => {
                    if bytes[at] == b'\\' {
                        at += 1;
                        self.finish(&mut found);
                        self.state = State::Ground;
                    } else {
                        // The command was cut off by the start of something else,
                        // which is read as that and not as the command's end.
                        self.state = State::Escape;
                    }
                }
                State::Skip => {
                    let rest = &bytes[at..];
                    let Some(control) = rest
                        .iter()
                        .position(|&byte| matches!(byte, ESC | CAN | SUB))
                    else {
                        return;
                    };
                    at += control + 1;
                    self.state = if rest[control] == ESC {
                        State::SkipEscape
                    } else {
                        State::Ground
                    };
                }
                State::SkipEscape => {
                    if bytes[at] == b'\\' {
                        at += 1;
                        self.state = State::Ground;
                    } else {
                        self.state = State::Escape;
                    }
                }
            }
        }
    }

    fn keep(&mut self, run: &[u8]) {
        if self.oversized {
            return;
        }
        if self.text.len() + run.len() > OSC_MAX_BYTES {
            self.oversized = true;
            self.text.clear();
        } else {
            self.text.extend_from_slice(run);
        }
    }

    /// Reads the command that has just ended.
    fn finish(&mut self, found: &mut impl FnMut(Report)) {
        if !self.oversized
            && let Some(report) = read_command(&self.text)
        {
            found(report);
        }
        self.text.clear();
        self.oversized = false;
    }

    /// How much text the scanner holds on to between reads.
    #[cfg(test)]
    fn held(&self) -> usize {
        self.text.capacity()
    }
}

/// Text fit to show: what `bytes` say as UTF-8, with no control characters in it and
/// no more than `max_chars` characters. Line breaks and tabs become spaces.
fn clean(bytes: &[u8], max_chars: usize) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .take(max_chars)
        .collect()
}

/// The bytes a percent-encoded text stands for. A percent sign that begins no
/// encoding stands for itself.
fn percent_decode(text: &[u8]) -> Vec<u8> {
    let hex = |byte: u8| char::from(byte).to_digit(16).map(|digit| digit as u8);
    let mut out = Vec::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        let pair = (text[at] == b'%')
            .then(|| Some((hex(*text.get(at + 1)?)?, hex(*text.get(at + 2)?)?)))
            .flatten();
        match pair {
            Some((high, low)) => {
                out.push(high << 4 | low);
                at += 3;
            }
            None => {
                out.push(text[at]);
                at += 1;
            }
        }
    }
    out
}

fn read_command(text: &[u8]) -> Option<Report> {
    // A command is a number, a semicolon, and what the number is about.
    let separator = text.iter().position(|&byte| byte == b';')?;
    let (number, rest) = (&text[..separator], &text[separator + 1..]);
    match number {
        b"0" | b"2" => Some(Report::Title(clean(rest, TITLE_MAX_CHARS))),
        b"7" => {
            let url = clean(rest, CWD_MAX_BYTES + 1);
            // A directory that is cut short names another directory, so one that is
            // too long is no report at all.
            (!url.is_empty() && url.len() <= CWD_MAX_BYTES).then_some(Report::Cwd(url))
        }
        b"133" => read_mark(rest),
        b"7501" => read_status(rest).map(Report::Status),
        _ => None,
    }
}

/// A shell's mark: the prompt begins (`A`) and ends (`B`), a command's output begins
/// (`C`), the command has ended (`D`).
fn read_mark(rest: &[u8]) -> Option<Report> {
    let mut fields = rest.split(|&byte| byte == b';');
    match fields.next()? {
        b"A" => Some(Report::Prompt),
        b"C" => {
            let command = fields
                .find_map(|field| field.strip_prefix(b"cmdline_url="))
                .map(|encoded| clean(&percent_decode(encoded), COMMAND_MAX_CHARS))
                .map(|command| command.trim().to_owned())
                .filter(|command| !command.is_empty());
            Some(Report::CommandStarted(command))
        }
        b"D" => {
            let exit = fields
                .next()
                .and_then(|field| std::str::from_utf8(field).ok())
                .and_then(|field| field.trim().parse().ok());
            Some(Report::CommandFinished(exit))
        }
        _ => None,
    }
}

/// The bytes standard base64 text stands for, with or without its padding.
fn base64_decode(text: &[u8]) -> Option<Vec<u8>> {
    let text = match text {
        [body @ .., b'=', b'='] | [body @ .., b'='] => body,
        body => body,
    };
    if text.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3 + 2);
    let (mut bits, mut held) = (0u32, 0u32);
    for &byte in text {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        bits = bits << 6 | u32::from(value);
        held += 6;
        if held >= 8 {
            held -= 8;
            out.push((bits >> held) as u8);
        }
    }
    Some(out)
}

/// One line of text a program sent encoded. `None` when it is not what it must be:
/// base64 of UTF-8, within its limits, with no control character in it.
fn encoded_line(value: &[u8], max_encoded: usize, max_decoded: usize) -> Option<String> {
    if value.len() > max_encoded {
        return None;
    }
    let text = String::from_utf8(base64_decode(value)?).ok()?;
    (text.len() <= max_decoded && !text.chars().any(char::is_control)).then_some(text)
}

/// Whether `word` is made of the characters a program's name or one step of a record's
/// path may have.
fn is_name(word: &[u8], max: usize) -> bool {
    (1..=max).contains(&word.len())
        && word
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'+' | b'-'))
}

fn text(word: &[u8]) -> String {
    String::from_utf8_lossy(word).into_owned()
}

/// A program's report of how its work stands: pairs of `key=value` with colons
/// between. A report that breaks a rule is no report; a pair that makes no sense is
/// passed over, as is a key this does not know.
fn read_status(rest: &[u8]) -> Option<ProgramStatus> {
    // A question about what the terminal understands is not a report.
    if rest.first() == Some(&b'?') || rest.len() > STATUS_SEQUENCE_MAX_BYTES {
        return None;
    }
    let mut state = None;
    let mut status = ProgramStatus::new(ProgramState::Idle, None);
    for pair in rest.split(|&byte| byte == b':') {
        let Some(equals) = pair.iter().position(|&byte| byte == b'=') else {
            continue;
        };
        let (key, value) = (pair[..equals].trim_ascii(), pair[equals + 1..].trim_ascii());
        let well_formed = (1..=STATUS_KEY_MAX_BYTES).contains(&key.len())
            && key.iter().all(u8::is_ascii_lowercase)
            && value.iter().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(byte, b'_' | b'.' | b',' | b'+' | b'/' | b'=' | b'-')
            });
        if !well_formed {
            continue;
        }
        match key {
            b"state" => {
                state = Some(match value {
                    b"idle" => ProgramState::Idle,
                    b"working" => ProgramState::Working,
                    b"done" => ProgramState::Done,
                    b"blocked" => ProgramState::Blocked,
                    b"error" => ProgramState::Error,
                    b"clear" => ProgramState::Clear,
                    _ => return None,
                });
            }
            b"id" => {
                let steps = || value.split(|&byte| byte == b'/');
                let sound = value.len() <= STATUS_ID_MAX_BYTES
                    && steps().count() <= STATUS_ID_DEPTH_MAX
                    && steps().all(|step| is_name(step, STATUS_SEGMENT_MAX_BYTES));
                if !sound {
                    return None;
                }
                status.id = Some(text(value));
            }
            b"kind" => {
                status.kind = match value {
                    b"permission" => Some(BlockKind::Permission),
                    b"question" => Some(BlockKind::Question),
                    b"auth" => Some(BlockKind::Auth),
                    _ => None,
                };
            }
            b"progress" => {
                status.progress = std::str::from_utf8(value)
                    .ok()
                    .and_then(|digits| digits.parse::<u8>().ok())
                    .filter(|progress| *progress <= 100);
            }
            b"app" => {
                if value.len() > STATUS_APP_MAX_BYTES {
                    return None;
                }
                status.app = is_name(value, STATUS_APP_MAX_BYTES).then(|| text(value));
            }
            b"title" => {
                status.title = Some(encoded_line(
                    value,
                    STATUS_TITLE_MAX_ENCODED,
                    STATUS_TITLE_MAX_BYTES,
                )?);
            }
            b"msg" => {
                status.message = Some(encoded_line(
                    value,
                    STATUS_MESSAGE_MAX_ENCODED,
                    STATUS_MESSAGE_MAX_BYTES,
                )?);
            }
            _ => {}
        }
    }
    status.state = state?;
    // What a blocked program waits for and how far the work has come mean something
    // only in the states that have them.
    if status.state != ProgramState::Blocked {
        status.kind = None;
    }
    if !matches!(status.state, ProgramState::Working | ProgramState::Blocked) {
        status.progress = None;
    }
    Some(status)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn scan_in(chunks: &[&[u8]]) -> Vec<Report> {
        let mut scanner = Scanner::new();
        let mut reports = Vec::new();
        for chunk in chunks {
            scanner.feed(chunk, |report| reports.push(report));
        }
        reports
    }

    fn scan(bytes: &[u8]) -> Vec<Report> {
        scan_in(&[bytes])
    }

    fn title(text: &str) -> Report {
        Report::Title(text.to_owned())
    }

    fn status(state: ProgramState) -> ProgramStatus {
        ProgramStatus::new(state, None)
    }

    /// The recording of a shell that reports everything the scanner knows, with both
    /// ways of ending a string.
    const SHELL: &[u8] = include_bytes!("../../../tests/corpus/09-shell-integration/input.bin");

    fn shell_reports() -> Vec<Report> {
        vec![
            Report::Cwd("file://host/home/user/project".to_owned()),
            title("user@host: ~/project"),
            Report::Prompt,
            Report::CommandStarted(None),
            Report::CommandFinished(Some(0)),
            Report::Status(ProgramStatus {
                app: Some("make".to_owned()),
                message: Some("building".to_owned()),
                ..status(ProgramState::Working)
            }),
            Report::Prompt,
            Report::CommandStarted(None),
            Report::CommandFinished(Some(1)),
            Report::Status(ProgramStatus {
                app: Some("make".to_owned()),
                ..status(ProgramState::Done)
            }),
            Report::Cwd("file://host/tmp".to_owned()),
            Report::Prompt,
        ]
    }

    #[test]
    fn a_shells_reports_are_found_in_order() {
        assert_eq!(scan(SHELL), shell_reports());
    }

    #[test]
    fn a_report_is_found_wherever_the_output_is_cut() {
        for cut in 0..=SHELL.len() {
            let (first, second) = SHELL.split_at(cut);
            assert_eq!(scan_in(&[first, second]), shell_reports(), "cut at {cut}");
        }
        let bytes: Vec<&[u8]> = SHELL.chunks(1).collect();
        assert_eq!(scan_in(&bytes), shell_reports());
    }

    /// The recording of output that says what a shell says without being one.
    const HOSTILE: &[u8] = include_bytes!("../../../tests/corpus/18-hostile-reports/input.bin");

    #[test]
    fn of_hostile_reports_only_the_well_formed_are_found_and_those_are_cleaned() {
        let cwd = |url: &str| Report::Cwd(url.to_owned());
        let expected = vec![
            // A directory is passed on as it was said, whatever it names. What it is
            // good for is decided where it would be used, not here.
            cwd("file://elsewhere/etc"),
            cwd("file://localhost/"),
            cwd("file://localhost/tmp/%0a%1b%5d2;taken"),
            cwd("javascript:alert(1)"),
            // The line break is a blank and the escape character is gone.
            Report::CommandStarted(Some("rm -rf ~ [2J".to_owned())),
            Report::CommandFinished(Some(0)),
            Report::CommandFinished(None),
            Report::Status(ProgramStatus {
                kind: Some(BlockKind::Auth),
                app: Some("sudo".to_owned()),
                message: Some("Password:".to_owned()),
                ..status(ProgramState::Blocked)
            }),
            // Text that is not base64 is a pair that makes no sense, which is passed
            // over; the report stands without it.
            Report::Status(status(ProgramState::Error)),
            // The rest broke a rule each and are no reports: a line break in the text,
            // a path that is no path of records, a name too long, a report too long,
            // a directory too long, a string longer than is kept, a question.
        ];
        assert_eq!(scan(HOSTILE), expected);
        for report in &expected {
            within_limits(report);
        }
        // However the output is cut.
        for size in [1, 7, 4096] {
            let pieces: Vec<&[u8]> = HOSTILE.chunks(size).collect();
            assert_eq!(scan_in(&pieces), expected, "in pieces of {size}");
        }
    }

    #[test]
    fn the_first_escape_or_bell_is_found_at_any_place() {
        for len in 0..40 {
            let plain = vec![b'x'; len];
            assert_eq!(next_control(&plain), None, "{len} plain bytes");
            for at in 0..len {
                for control in [ESC, BEL] {
                    let mut bytes = plain.clone();
                    bytes[at] = control;
                    // One further on is not the first.
                    if let Some(later) = bytes.get_mut(at + 3) {
                        *later = BEL;
                    }
                    assert_eq!(next_control(&bytes), Some(at), "{len} bytes, one at {at}");
                }
            }
        }
        // Bytes that differ from the two in one bit only, and bytes above ASCII.
        assert_eq!(
            next_control(&[0x06, 0x1a, 0x9b, 0x87, 0x1f, 0x0f, 0xff, 0x00, 0x1c]),
            None
        );
    }

    #[test]
    fn titles_come_from_commands_0_and_2_and_end_with_either_terminator() {
        assert_eq!(scan(b"\x1b]0;one\x07"), [title("one")]);
        assert_eq!(scan(b"\x1b]2;two\x1b\\"), [title("two")]);
        assert_eq!(scan(b"\x1b]2;a;b\x07"), [title("a;b")]);
        assert_eq!(scan(b"\x1b]2;\x07"), [title("")]);
        // The icon's name, a colour and a link are not titles.
        assert_eq!(
            scan(b"\x1b]1;icon\x07\x1b]11;?\x07\x1b]8;;http://x\x07"),
            []
        );
        // Nor is a command with nothing after its number.
        assert_eq!(scan(b"\x1b]2\x07\x1b]\x07\x1b];\x07"), []);
    }

    #[test]
    fn a_bell_is_one_outside_a_string_and_not_inside() {
        assert_eq!(scan(b"a\x07b"), [Report::Bell]);
        assert_eq!(scan(b"\x1b[1\x07m"), [Report::Bell]);
        assert_eq!(scan(b"\x1b\x07[m"), [Report::Bell]);
        // Ending a command is not ringing.
        assert_eq!(scan(b"\x1b]2;x\x07"), [title("x")]);
        // Inside the other strings a bell is just a byte.
        for start in *b"P_^X" {
            let mut bytes = vec![ESC, start];
            bytes.extend_from_slice(b"q\x07]2;no\x07\x1b\\\x07");
            assert_eq!(scan(&bytes), [Report::Bell], "{}", char::from(start));
        }
    }

    #[test]
    fn what_is_inside_another_kind_of_string_is_not_read() {
        assert_eq!(
            scan(b"\x1bP]2;no\x07 133;A\x1b\\\x1b]2;yes\x07"),
            [title("yes")]
        );
        assert_eq!(scan(b"\x1b_2;no\x1b\\\x1b]2;yes\x1b\\"), [title("yes")]);
        // An escape ends such a string wherever it comes, as it does in a terminal, so
        // a command that starts there is a command.
        assert_eq!(scan(b"\x1bPq\x1b]2;inner\x07"), [title("inner")]);
        // A string that is cancelled is over.
        assert_eq!(scan(b"\x1bPabc\x18\x1b]2;yes\x07"), [title("yes")]);
    }

    #[test]
    fn a_command_that_does_not_end_properly_is_not_acted_on() {
        // Cancelled.
        assert_eq!(scan(b"\x1b]2;no\x18\x07"), [Report::Bell]);
        assert_eq!(scan(b"\x1b]2;no\x1a"), []);
        // Cut off by another sequence, which is read for what it is.
        assert_eq!(scan(b"\x1b]2;no\x1b[m"), []);
        assert_eq!(scan(b"\x1b]2;no\x1b]2;yes\x07"), [title("yes")]);
        assert_eq!(scan(b"\x1b]2;no\x1b\x1b]2;yes\x07"), [title("yes")]);
        // Never ended at all.
        assert_eq!(scan(b"\x1b]2;no"), []);
        // Control characters inside are not part of the text.
        assert_eq!(scan(b"\x1b]2;a\r\nb\x00c\x07"), [title("abc")]);
    }

    #[test]
    fn a_command_longer_than_is_kept_is_passed_over_whole() {
        let mut bytes = b"\x1b]2;".to_vec();
        bytes.extend(std::iter::repeat_n(b'x', OSC_MAX_BYTES * 3));
        bytes.extend_from_slice(b"\x07\x1b]2;after\x07");
        let mut scanner = Scanner::new();
        let mut reports = Vec::new();
        for chunk in bytes.chunks(1000) {
            scanner.feed(chunk, |report| reports.push(report));
            assert!(scanner.held() <= OSC_MAX_BYTES * 2, "{}", scanner.held());
        }
        assert_eq!(reports, [title("after")]);

        // Right at the limit it is still read.
        let fits = OSC_MAX_BYTES - 2;
        let mut bytes = b"\x1b]2;".to_vec();
        bytes.extend(std::iter::repeat_n(b'y', fits));
        bytes.push(BEL);
        let expected: String = std::iter::repeat_n('y', TITLE_MAX_CHARS).collect();
        assert_eq!(scan(&bytes), [title(&expected)]);
    }

    #[test]
    fn text_is_cleaned_before_it_leaves() {
        // Not UTF-8, a character from the second control set, a deleted character.
        assert_eq!(
            scan(b"\x1b]2;a\xffb\xc2\x9bc\x7fd\x07"),
            [title("a\u{fffd}bcd")]
        );
        let long: String = std::iter::repeat_n('é', TITLE_MAX_CHARS + 50).collect();
        let Report::Title(kept) = &scan(format!("\x1b]0;{long}\x07").as_bytes())[0] else {
            panic!("no title");
        };
        assert_eq!(kept.chars().count(), TITLE_MAX_CHARS);
    }

    #[test]
    fn a_directory_is_a_url_and_one_too_long_is_none() {
        assert_eq!(
            scan(b"\x1b]7;file://box/home/me/a%20b\x1b\\"),
            [Report::Cwd("file://box/home/me/a%20b".to_owned())]
        );
        assert_eq!(scan(b"\x1b]7;\x07"), []);
        let long = format!("\x1b]7;file://box/{}\x07", "d".repeat(CWD_MAX_BYTES));
        assert_eq!(scan(long.as_bytes()), []);
    }

    #[test]
    fn a_shells_marks_carry_the_command_line_and_the_exit_code() {
        assert_eq!(
            scan(b"\x1b]133;A\x07\x1b]133;B\x07\x1b]133;C\x07\x1b]133;D\x07"),
            [
                Report::Prompt,
                Report::CommandStarted(None),
                Report::CommandFinished(None),
            ]
        );
        assert_eq!(
            scan(b"\x1b]133;C;cmdline_url=git%20log%20%2D%2Doneline%0A%09x\x07"),
            [Report::CommandStarted(Some(
                "git log --oneline  x".to_owned()
            ))]
        );
        // Other things a shell may say with its marks are passed over.
        assert_eq!(
            scan(b"\x1b]133;A;cl=m;aid=7\x07\x1b]133;C;x=1;cmdline_url=ls;y\x07"),
            [
                Report::Prompt,
                Report::CommandStarted(Some("ls".to_owned()))
            ]
        );
        assert_eq!(
            scan(b"\x1b]133;C;cmdline_url=%20\x07\x1b]133;C;cmdline_url=100%\x07"),
            [
                Report::CommandStarted(None),
                Report::CommandStarted(Some("100%".to_owned())),
            ]
        );
        assert_eq!(
            scan(b"\x1b]133;D;130\x07\x1b]133;D;-1\x07\x1b]133;D;x\x07\x1b]133;D;0;aid=7\x07"),
            [
                Report::CommandFinished(Some(130)),
                Report::CommandFinished(Some(-1)),
                Report::CommandFinished(None),
                Report::CommandFinished(Some(0)),
            ]
        );
        assert_eq!(
            scan(b"\x1b]133\x07\x1b]133;\x07\x1b]133;Z\x07\x1b]133;AA\x07"),
            []
        );
    }

    fn reported(pairs: &str) -> Option<ProgramStatus> {
        match scan(format!("\x1b]7501;{pairs}\x1b\\").as_bytes()).as_slice() {
            [] => None,
            [Report::Status(status)] => Some(status.clone()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_programs_status_is_read_pair_by_pair() {
        assert_eq!(
            reported(
                "state=blocked:kind=permission:app=terraform:progress=40:id=us-east/plan:title=UGxhbg:msg=QXBwbHkgMyByZXNvdXJjZXM/"
            ),
            Some(ProgramStatus {
                state: ProgramState::Blocked,
                id: Some("us-east/plan".to_owned()),
                kind: Some(BlockKind::Permission),
                progress: Some(40),
                app: Some("terraform".to_owned()),
                title: Some("Plan".to_owned()),
                message: Some("Apply 3 resources?".to_owned()),
            })
        );
        // Spaces round a key or a value are not part of it; a key given twice has its
        // last value; a key nobody knows and a pair that is none are passed over.
        assert_eq!(
            reported(" state = working : state=done:colour=red:nonsense:=x:App=y"),
            Some(status(ProgramState::Done))
        );
        assert_eq!(
            reported("state=clear:id=us-east"),
            Some(ProgramStatus::new(
                ProgramState::Clear,
                Some("us-east".to_owned())
            ))
        );
        // Padding may be there or not.
        for message in ["YQ==", "YQ", "YWI=", "YWI", "YWJj"] {
            let status = reported(&format!("state=idle:msg={message}")).unwrap();
            assert!(["a", "ab", "abc"].contains(&status.message.as_deref().unwrap()));
        }
    }

    #[test]
    fn what_a_state_cannot_have_is_left_out() {
        let working = reported("state=working:kind=auth:progress=7").unwrap();
        assert_eq!((working.kind, working.progress), (None, Some(7)));
        let done = reported("state=done:kind=auth:progress=7").unwrap();
        assert_eq!((done.kind, done.progress), (None, None));
        // A value that is not one of the choices is as good as none.
        let blocked = reported("state=blocked:kind=mood:progress=101:app=a/b").unwrap();
        assert_eq!(
            (blocked.kind, blocked.progress, blocked.app),
            (None, None, None)
        );
    }

    #[test]
    fn a_status_that_breaks_a_rule_is_no_status() {
        let long_app = "a".repeat(STATUS_APP_MAX_BYTES + 1);
        let long_step = "s".repeat(STATUS_SEGMENT_MAX_BYTES + 1);
        let deep = ["a"; STATUS_ID_DEPTH_MAX + 1].join("/");
        let long_message = "QUFB".repeat(STATUS_MESSAGE_MAX_ENCODED / 4 + 1);
        let refused = [
            // No state, or one that is not known.
            "app=make".to_owned(),
            "state=paused".to_owned(),
            String::new(),
            // A question about what the terminal understands.
            "?".to_owned(),
            "?:states=idle".to_owned(),
            // A record's path with an empty step, a step too long, too many steps.
            "state=idle:id=a//b".to_owned(),
            "state=idle:id=/a".to_owned(),
            format!("state=idle:id={long_step}"),
            format!("state=idle:id={deep}"),
            format!("state=idle:app={long_app}"),
            // Text that is not base64, not UTF-8, has a control character, is too long.
            "state=idle:msg=a".to_owned(),
            "state=idle:msg=/w==".to_owned(),
            "state=idle:msg=YQph".to_owned(),
            "state=idle:title=YQlh".to_owned(),
            format!("state=idle:msg={long_message}"),
        ];
        for pairs in refused {
            assert_eq!(reported(&pairs), None, "{pairs}");
        }
        // The whole of it has a limit too.
        let padding = ":x=".to_owned() + &"y".repeat(STATUS_SEQUENCE_MAX_BYTES);
        assert_eq!(reported(&format!("state=idle{padding}")), None);
    }

    /// Bytes that are mostly the makings of control strings, so that a random stream
    /// is full of strings that begin, end, nest and break off.
    fn noise() -> impl Strategy<Value = Vec<u8>> {
        let piece = prop_oneof![
            4 => Just(b"\x1b]".to_vec()),
            3 => Just(b"\x07".to_vec()),
            3 => Just(b"\x1b\\".to_vec()),
            2 => Just(b"\x1bP".to_vec()),
            1 => Just(b"\x1b_".to_vec()),
            1 => Just(b"\x18".to_vec()),
            2 => Just(b"2;".to_vec()),
            2 => Just(b"7;file://".to_vec()),
            2 => Just(b"133;".to_vec()),
            2 => Just(b"7501;state=".to_vec()),
            2 => Just(b"cmdline_url=".to_vec()),
            1 => Just(b":msg=".to_vec()),
            1 => Just(b":id=".to_vec()),
            6 => proptest::collection::vec(any::<u8>(), 0..12),
            3 => "[A-Dab=:;%/+0-9]{0,10}".prop_map(String::into_bytes),
            1 => (any::<u8>(), 0..OSC_MAX_BYTES * 2)
                .prop_map(|(byte, len)| vec![byte | 0x20; len]),
        ];
        proptest::collection::vec(piece, 0..60).prop_map(|pieces| pieces.concat())
    }

    fn within_limits(report: &Report) {
        let sound = |text: &str, max_chars: usize| {
            assert!(
                text.chars().count() <= max_chars,
                "{} characters",
                text.chars().count()
            );
            assert!(!text.chars().any(char::is_control), "{text:?}");
        };
        match report {
            Report::Title(text) => sound(text, TITLE_MAX_CHARS),
            Report::Cwd(url) => {
                sound(url, CWD_MAX_BYTES);
                assert!(url.len() <= CWD_MAX_BYTES);
            }
            Report::CommandStarted(Some(command)) => sound(command, COMMAND_MAX_CHARS),
            Report::Status(status) => {
                for (text, max) in [
                    (&status.id, STATUS_ID_MAX_BYTES),
                    (&status.app, STATUS_APP_MAX_BYTES),
                    (&status.title, STATUS_TITLE_MAX_BYTES),
                    (&status.message, STATUS_MESSAGE_MAX_BYTES),
                ] {
                    if let Some(text) = text {
                        sound(text, max);
                        assert!(text.len() <= max);
                    }
                }
                assert!(status.progress.is_none_or(|progress| progress <= 100));
            }
            _ => {}
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        /// Whatever the bytes and however they are cut, the scanner finds the same
        /// reports, each within its limits, and holds on to no more than one string.
        #[test]
        fn any_bytes_in_any_pieces_give_the_same_reports_within_their_limits(
            bytes in noise(),
            cuts in proptest::collection::vec(any::<proptest::sample::Index>(), 0..16),
        ) {
            let whole = scan(&bytes);
            for report in &whole {
                within_limits(report);
            }

            let mut places: Vec<usize> = cuts.iter().map(|cut| cut.index(bytes.len() + 1)).collect();
            places.push(bytes.len());
            places.sort_unstable();
            let mut scanner = Scanner::new();
            let mut pieces = Vec::new();
            let mut start = 0;
            for end in places {
                scanner.feed(&bytes[start..end], |report| pieces.push(report));
                prop_assert!(scanner.held() <= OSC_MAX_BYTES * 2);
                start = end;
            }
            prop_assert_eq!(pieces, whole);
        }
    }
}
