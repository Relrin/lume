//! A session's own copy of its screen: a terminal emulator fed every byte the program
//! prints, in the order it printed them.
//!
//! The emulator cannot be shared between threads, so each session has a thread that owns
//! its emulator and takes work from a queue. The order of that queue is what makes a
//! copy exact: a snapshot reflects every byte fed before it was asked for, so a client
//! that starts from it and then takes the output that follows misses nothing and sees
//! nothing twice.
//!
//! The emulator also answers the program's questions to the terminal (where is the
//! cursor, what colour is the background, which device is this), on behalf of whoever
//! is showing the session, or of nobody. Those answers go straight back to the program.
//! The client tells the session what its view looks like (colours, cell size, scheme,
//! focus) so that the answers match what the user sees.
//!
//! A program that redraws its whole screen may say when it begins and when it is done
//! (synchronized output, mode 2026). Whoever reads the screen as text in between is
//! given the screen as it was when the program began, not the half of the next one: the
//! text is copied at that moment and answers until the program says it is done, or for
//! a short while if it never does. Nothing is copied for a program that does not ask.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use lume_proto::{CellPx, ClientFacts, ColorScheme, HydratePayload, TermSize};
use lume_vt::{Colors, Format, Terminal, palette_256};
use tokio::sync::{mpsc, oneshot};

use crate::tuning::{
    INPUT_BACKLOG_MAX, SCREEN_QUEUE, SCROLLBACK_MAX_BYTES, SYNC_HOLD_MAX, WAIT_TEXT_CHECK,
};

/// What a program prints to say it begins a screen it does not want shown the half of.
const BEGIN_SYNC: &[u8] = b"\x1b[?2026h";

/// The number of the mode that is set while it draws.
const SYNC_MODE: u16 = 2026;

/// Where in `bytes` a program says it begins such a screen, if it does. Every byte a
/// program prints passes here, a flood's too, so only the escape bytes among them are
/// looked at closely: plain text has none.
fn begin_of_sync(bytes: &[u8]) -> Option<usize> {
    let mut from = 0;
    while let Some(found) = bytes[from..].iter().position(|byte| *byte == 0x1b) {
        let at = from + found;
        if bytes[at..].starts_with(BEGIN_SYNC) {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

/// The version of the terminal engine every session's screen runs.
pub fn vt_version() -> &'static str {
    lume_vt::library_version()
}

/// The screen in use as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScreenText {
    pub(crate) size: TermSize,
    /// Column, row, shown.
    pub(crate) cursor: (u16, u16, bool),
    pub(crate) alternate: bool,
    /// One string per row, with the blanks at its end removed.
    pub(crate) lines: Vec<String>,
}

/// Where a looked-for text was found: the first row that holds it, and that row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    pub(crate) row: u16,
    pub(crate) line: String,
}

/// How a newly attached client is to be brought up to date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HydrateWith {
    /// A snapshot, which only the same engine version can load. Falls back to a replay
    /// when the screen cannot be saved as one.
    Snapshot,
    /// Output that rebuilds the screen in any engine.
    Replay,
}

enum Op {
    Feed(Vec<u8>),
    Resize(TermSize, CellPx),
    /// What the clients say together; with none attached, the last of them, unfocused.
    Facts(ClientFacts),
    Hydrate(HydrateWith, oneshot::Sender<Option<HydratePayload>>),
    Text(oneshot::Sender<Option<ScreenText>>),
    WaitText {
        text: String,
        deadline: Instant,
        reply: oneshot::Sender<Option<Found>>,
    },
}

/// Where the screen's answers to the program go: the session's writing thread, with the
/// count of bytes it has yet to write.
pub(crate) struct Answers {
    pub(crate) input: mpsc::UnboundedSender<Vec<u8>>,
    pub(crate) queued: Arc<AtomicUsize>,
}

impl Answers {
    fn send(&self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        let queued = self.queued.load(Ordering::Relaxed);
        if queued.saturating_add(bytes.len()) > INPUT_BACKLOG_MAX {
            // A program that asks and does not read its answers is not owed them.
            return;
        }
        self.queued.fetch_add(bytes.len(), Ordering::Relaxed);
        if self.input.send(bytes).is_err() {
            self.queued.store(0, Ordering::Relaxed);
        }
    }
}

/// The way to a session's screen. Dropping it ends the thread.
pub(crate) struct Screen {
    ops: mpsc::Sender<Op>,
}

impl Screen {
    /// Starts the thread of a screen of `size` keeping about `scrollback` lines. Must be
    /// called within a Tokio runtime.
    pub(crate) fn start(
        name: &str,
        size: TermSize,
        scrollback: usize,
        answers: Answers,
    ) -> std::io::Result<Self> {
        let (ops, queued) = mpsc::channel(SCREEN_QUEUE);
        let runtime = tokio::runtime::Handle::current();
        let (made, made_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name(format!("{name}-screen"))
            .spawn(move || {
                // `Terminal` cannot move between threads, so it is made on this one.
                let terminal = Terminal::new(size.cols(), size.rows()).and_then(|mut t| {
                    t.set_scrollback_limits(scrollback, SCROLLBACK_MAX_BYTES)?;
                    Ok(t)
                });
                match terminal {
                    Ok(terminal) => {
                        let _ = made.send(Ok(()));
                        State::new(terminal, answers).run(queued, &runtime);
                    }
                    Err(error) => {
                        let _ = made.send(Err(std::io::Error::other(error.to_string())));
                    }
                }
            })?;
        made_rx
            .recv()
            .map_err(|_| std::io::Error::other("the screen's thread ended at once"))??;
        Ok(Self { ops })
    }

    /// Another way to the same screen, for a wait that outlives the call that made it.
    /// The screen's thread runs until every way to it is gone.
    pub(crate) fn waiter(&self) -> Self {
        Self {
            ops: self.ops.clone(),
        }
    }

    async fn send(&self, op: Op) {
        // A screen whose thread has stopped has nothing to say; the session goes on.
        let _ = self.ops.send(op).await;
    }

    /// Feeds output the program printed. Waits while the screen is behind, which in turn
    /// slows the session's reading.
    pub(crate) async fn feed(&self, bytes: Vec<u8>) {
        self.send(Op::Feed(bytes)).await;
    }

    pub(crate) async fn resize(&self, size: TermSize, cell_px: CellPx) {
        self.send(Op::Resize(size, cell_px)).await;
    }

    pub(crate) async fn facts(&self, facts: ClientFacts) {
        self.send(Op::Facts(facts)).await;
    }

    /// The state a new client starts from, as of everything fed so far.
    pub(crate) async fn hydrate(&self, with: HydrateWith) -> Option<HydratePayload> {
        let (reply, answer) = oneshot::channel();
        self.send(Op::Hydrate(with, reply)).await;
        answer.await.ok().flatten()
    }

    /// The screen in use as text, as of everything fed so far.
    pub(crate) async fn text(&self) -> Option<ScreenText> {
        let (reply, answer) = oneshot::channel();
        self.send(Op::Text(reply)).await;
        answer.await.ok().flatten()
    }

    /// Resolves with the first row holding `text` once there is one, or with `None` at
    /// `deadline` or if the screen goes away first.
    pub(crate) async fn wait_text(&self, text: String, deadline: Instant) -> Option<Found> {
        let (reply, answer) = oneshot::channel();
        self.send(Op::WaitText {
            text,
            deadline,
            reply,
        })
        .await;
        answer.await.ok().flatten()
    }
}

struct Wait {
    text: String,
    deadline: Instant,
    reply: oneshot::Sender<Option<Found>>,
}

/// Everything the screen's thread owns.
struct State {
    terminal: Terminal,
    answers: Answers,
    /// What the client last said about its view. `None` until one has.
    facts: Option<ClientFacts>,
    /// Whether a program asking to hear of focus was last told it has it.
    focused: bool,
    waits: Vec<Wait>,
    /// Whether output arrived since the waits last looked at the screen.
    changed: bool,
    /// The screen as it was when the program began the one it is drawing, and when
    /// that was. `None` while it draws none.
    front: Option<(ScreenText, Instant)>,
}

impl State {
    fn new(terminal: Terminal, answers: Answers) -> Self {
        Self {
            terminal,
            answers,
            facts: None,
            focused: false,
            waits: Vec::new(),
            changed: false,
            front: None,
        }
    }

    /// Takes in what the program printed. Where it says it begins a screen, the screen
    /// before it is kept as text for whoever reads meanwhile.
    fn feed(&mut self, bytes: &[u8]) {
        let begins = if self.front.is_none() {
            begin_of_sync(bytes)
        } else {
            None
        };
        match begins {
            Some(at) => {
                self.terminal.write(&bytes[..at]);
                let before = self.text();
                self.terminal.write(&bytes[at..]);
                self.front = before.map(|text| (text, Instant::now()));
            }
            None => self.terminal.write(bytes),
        }
        // Done within what was fed, or said to be done now: there is one screen again.
        if self.front.is_some() && !self.terminal.mode(SYNC_MODE, false).unwrap_or(false) {
            self.front = None;
        }
    }

    /// The screen as whoever reads it is to have it: the one the program last
    /// finished while it draws the next and may be believed, else the one there is.
    fn shown(&self) -> Option<ScreenText> {
        match &self.front {
            Some((text, since)) if since.elapsed() < SYNC_HOLD_MAX => Some(text.clone()),
            _ => self.text(),
        }
    }

    fn run(mut self, mut ops: mpsc::Receiver<Op>, runtime: &tokio::runtime::Handle) {
        loop {
            // While somebody waits for text, the screen is looked at now and then, and
            // not after every write: a flood would otherwise be read as text over and
            // over.
            let next = if self.waits.is_empty() {
                runtime.block_on(ops.recv())
            } else {
                let deadline = self.waits.iter().map(|wait| wait.deadline).min().map_or(
                    WAIT_TEXT_CHECK,
                    |at| {
                        at.saturating_duration_since(Instant::now())
                            .min(WAIT_TEXT_CHECK)
                    },
                );
                // The timer is made inside the runtime, which this thread is not.
                let next =
                    runtime.block_on(async { tokio::time::timeout(deadline, ops.recv()).await });
                match next {
                    Ok(op) => op,
                    Err(_) => {
                        self.check_waits(true);
                        continue;
                    }
                }
            };
            let Some(op) = next else {
                break;
            };
            self.on_op(op);
            // Drain what else is queued before looking at the screen.
            while let Ok(op) = ops.try_recv() {
                self.on_op(op);
            }
            self.check_waits(false);
        }
        for wait in self.waits.drain(..) {
            let _ = wait.reply.send(None);
        }
    }

    fn on_op(&mut self, op: Op) {
        match op {
            Op::Feed(bytes) => {
                self.feed(&bytes);
                self.answers.send(self.terminal.take_replies());
                self.changed = true;
            }
            Op::Resize(size, cell_px) => {
                let _ = self
                    .terminal
                    .set_cell_px(u32::from(cell_px.w), u32::from(cell_px.h));
                if let Err(error) = self.terminal.resize(size.cols(), size.rows()) {
                    tracing::debug!(%error, "the screen could not be resized");
                }
                // A program may be told the new size as the terminal changes.
                self.answers.send(self.terminal.take_replies());
                // A screen of another size is not the one that was kept.
                self.front = None;
                self.changed = true;
            }
            Op::Facts(facts) => self.on_facts(facts),
            Op::Hydrate(with, reply) => {
                let _ = reply.send(self.hydrate(with));
            }
            Op::Text(reply) => {
                let _ = reply.send(self.shown());
            }
            Op::WaitText {
                text,
                deadline,
                reply,
            } => {
                // Looked at once at once, whether or not anything changed.
                match self.find(&text) {
                    Some(found) => {
                        let _ = reply.send(Some(found));
                    }
                    None => self.waits.push(Wait {
                        text,
                        deadline,
                        reply,
                    }),
                }
            }
        }
    }

    fn on_facts(&mut self, facts: ClientFacts) {
        let before = self.facts.replace(facts.clone());
        let palette = &facts.palette;
        let ansi: [[u8; 3]; 16] = std::array::from_fn(|i| palette.ansi[i].0);
        let colors = Colors {
            foreground: palette.fg.0,
            background: palette.bg.0,
            cursor: palette.cursor.0,
            palette: palette_256(&ansi),
        };
        if before.as_ref().map(|b| &b.palette) != Some(palette)
            && let Err(error) = self.terminal.set_colors(&colors)
        {
            tracing::debug!(%error, "the screen's colours could not be set");
        }
        let _ = self
            .terminal
            .set_cell_px(u32::from(facts.cell_px.w), u32::from(facts.cell_px.h));

        let scheme = match facts.scheme {
            ColorScheme::Light => lume_vt::ColorScheme::Light,
            ColorScheme::Dark => lume_vt::ColorScheme::Dark,
        };
        self.terminal.set_color_scheme(Some(scheme));
        // A program that asked to hear of scheme changes hears of this one. The first
        // facts are not a change: the program has not been told of another scheme.
        let changed = before.is_some_and(|before| before.scheme != facts.scheme);
        if changed
            && self
                .terminal
                .mode(COLOR_SCHEME_REPORT, false)
                .unwrap_or(false)
        {
            self.answers.send(lume_vt::color_scheme_report(scheme));
        }
        self.set_focused(facts.focused);
    }

    fn set_focused(&mut self, focused: bool) {
        if focused == self.focused {
            return;
        }
        self.focused = focused;
        if self.terminal.mode(FOCUS_EVENTS, false).unwrap_or(false) {
            self.answers.send(lume_vt::focus_report(focused));
        }
    }

    fn hydrate(&self, with: HydrateWith) -> Option<HydratePayload> {
        if with == HydrateWith::Snapshot {
            match self.terminal.snapshot() {
                Ok(bytes) if bytes.len() <= HYDRATE_MAX_BYTES => {
                    return Some(HydratePayload::Snapshot {
                        vt_version: vt_version().to_owned(),
                        bytes,
                    });
                }
                Ok(_) => tracing::debug!("the screen is too large to save; replaying it"),
                Err(error) => {
                    tracing::debug!(%error, "the screen could not be saved; replaying it");
                }
            }
        }
        match self.terminal.format(Format::Replay) {
            Ok(replay) if replay.bytes.len() <= HYDRATE_MAX_BYTES => {
                Some(HydratePayload::VtReplay(replay.bytes))
            }
            Ok(_) => {
                tracing::warn!("the screen is too large to replay");
                None
            }
            Err(error) => {
                tracing::warn!(%error, "the screen could not be replayed");
                None
            }
        }
    }

    fn text(&self) -> Option<ScreenText> {
        let terminal = &self.terminal;
        let cursor = terminal.cursor().ok()?;
        Some(ScreenText {
            size: TermSize::new(terminal.cols(), terminal.rows()).ok()?,
            cursor: (cursor.x, cursor.y, cursor.visible),
            alternate: terminal.alternate_screen().ok()?,
            lines: terminal.screen_text().ok()?,
        })
    }

    fn find(&self, text: &str) -> Option<Found> {
        let lines = self.shown()?.lines;
        let row = lines.iter().position(|line| line.contains(text))?;
        Some(Found {
            row: u16::try_from(row).ok()?,
            line: lines[row].clone(),
        })
    }

    /// Answers the waits whose text is on the screen, and those whose time is up.
    fn check_waits(&mut self, timer: bool) {
        if self.waits.is_empty() {
            return;
        }
        let now = Instant::now();
        let lines = if self.changed || timer {
            self.changed = false;
            self.shown().map(|shown| shown.lines)
        } else {
            None
        };
        let mut kept = Vec::with_capacity(self.waits.len());
        for wait in self.waits.drain(..) {
            let found = lines.as_ref().and_then(|lines| {
                let row = lines.iter().position(|line| line.contains(&wait.text))?;
                Some(Found {
                    row: u16::try_from(row).ok()?,
                    line: lines[row].clone(),
                })
            });
            if found.is_some() || now >= wait.deadline {
                let _ = wait.reply.send(found);
            } else {
                kept.push(wait);
            }
        }
        self.waits = kept;
    }
}

/// The most bytes of screen a hydrate message carries, leaving room for its header and
/// the engine version within what the protocol accepts.
const HYDRATE_MAX_BYTES: usize = lume_proto::tuning::MAX_HYDRATE - 1024;

/// The DEC private mode a program sets to hear of focus changes.
const FOCUS_EVENTS: u16 = 1004;
/// The DEC private mode a program sets to hear of colour scheme changes.
const COLOR_SCHEME_REPORT: u16 = 2031;

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        let (input, _) = mpsc::unbounded_channel();
        let answers = Answers {
            input,
            queued: Arc::new(AtomicUsize::new(0)),
        };
        State::new(Terminal::new(20, 4).unwrap(), answers)
    }

    fn first(state: &State) -> String {
        state.shown().unwrap().lines[0].clone()
    }

    #[test]
    fn a_screen_being_drawn_is_read_as_the_one_before_it_until_it_is_done() {
        let mut state = state();
        state.feed(b"old screen");
        assert_eq!(first(&state), "old screen");
        assert!(state.front.is_none());

        // The program says it begins, clears, and has drawn half: what is read is
        // still the old screen, with what came before the beginning in it.
        state.feed(b"!\x1b[?2026h\x1b[2J\x1b[Hnew sc");
        assert_eq!(first(&state), "old screen!");
        assert!(state.find("new sc").is_none());
        assert_eq!(state.find("old").map(|found| found.row), Some(0));
        state.feed(b"reen");
        assert_eq!(first(&state), "old screen!");

        // Done: the new one, whole.
        state.feed(b"\x1b[?2026l");
        assert!(state.front.is_none());
        assert_eq!(first(&state), "new screen");
    }

    #[test]
    fn the_beginning_of_a_screen_is_found_among_other_escapes() {
        assert_eq!(begin_of_sync(b"plain text"), None);
        assert_eq!(begin_of_sync(b""), None);
        assert_eq!(begin_of_sync(b"\x1b[31mred\x1b[0m\x1b[?2026"), None);
        assert_eq!(begin_of_sync(b"\x1b[?2026h"), Some(0));
        assert_eq!(begin_of_sync(b"a\x1b[1m\x1b\x1b[?2026hrest"), Some(6));
    }

    #[test]
    fn a_screen_begun_and_done_in_one_piece_keeps_nothing() {
        let mut state = state();
        state.feed(b"one\x1b[?2026h\x1b[2J\x1b[Htwo\x1b[?2026l");
        assert!(state.front.is_none());
        assert_eq!(first(&state), "two");
    }

    #[test]
    fn a_program_that_never_says_it_is_done_is_read_as_it_stands_after_a_while() {
        let mut state = state();
        state.feed(b"old\x1b[?2026h\x1b[2J\x1b[Hstuck");
        assert_eq!(first(&state), "old");
        // As if the while had passed.
        if let Some((_, since)) = &mut state.front {
            *since = Instant::now() - SYNC_HOLD_MAX;
        }
        assert_eq!(first(&state), "stuck");
        // More of the same screen does not begin the while again.
        state.feed(b" still");
        assert_eq!(first(&state), "stuck still");
    }

    #[test]
    fn a_screen_of_another_size_is_read_as_it_is() {
        let mut state = state();
        state.feed(b"old\x1b[?2026h\x1b[2J\x1b[Hnew");
        assert_eq!(first(&state), "old");
        state.on_op(Op::Resize(
            TermSize::new(30, 5).unwrap(),
            CellPx { w: 8, h: 16 },
        ));
        assert_eq!(first(&state), "new");
        assert_eq!(state.shown().unwrap().size, TermSize::new(30, 5).unwrap());
    }
}
