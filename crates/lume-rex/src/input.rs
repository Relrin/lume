//! What a view sends to its terminal block: what is typed, the size it would like, and
//! whether it has the focus.
//!
//! All of it goes over the control connection, as method calls. A server runs calls
//! that type into a terminal side by side unless each is awaited before the next is
//! made, so one task makes them one at a time, in the order the view sent them. What
//! is typed while a call is on its way is joined and goes in the next.
//!
//! A server takes what is typed as text. Bytes that are not text cannot be typed, and
//! are dropped.
//!
//! When the views of several clients want a terminal at different sizes, it has the
//! size its owner wants. A view takes its block when it gains the focus, which makes
//! the terminal follow the view a person is looking at. Several views of one terminal
//! in this application are one client to the server, and speak with one voice.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use lume_proto::{CellPx, ClientFacts, ErrorCode, ProtoError, TermSize};
use lume_session::SessionError;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::backend::Shared;
use crate::control::Control;
use crate::error::RexError;
use crate::ids::BlockRef;
use crate::tuning::{INPUT_CALL_MAX_BYTES, INPUT_PENDING_MAX_BYTES};

/// The arguments of a method of a block. A method that takes none of its own is given
/// null for `args`, which is then left out: a server does not take null for them.
pub(crate) fn of_block(block: &BlockRef, args: Value) -> Value {
    let mut of_block = json!({ "session_id": block.session_id, "block_id": block.block_id });
    if !args.is_null() {
        of_block["args"] = args;
    }
    of_block
}

/// Types text into a terminal, as much at a time as goes in one call, each call
/// answered before the next is made.
pub(crate) async fn type_text(
    control: &Control,
    block: &BlockRef,
    text: &str,
) -> Result<(), RexError> {
    let mut rest = text;
    while !rest.is_empty() {
        let mut end = rest.len().min(INPUT_CALL_MAX_BYTES);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        let (now, later) = rest.split_at(end);
        control
            .call(
                "com.superlogical.terminal.write",
                of_block(block, json!({ "data": now })),
            )
            .await?;
        rest = later;
    }
    Ok(())
}

/// Asks for a terminal to have a size. A server gives it that size at once if this
/// client owns the block or nobody does, and when this client comes to own it
/// otherwise.
async fn ask_size(
    control: &Control,
    block: &BlockRef,
    size: TermSize,
    cell_px: CellPx,
) -> Result<(), RexError> {
    let mut args = json!({ "columns": size.cols(), "rows": size.rows() });
    // A cell's size is said whole or not at all.
    if cell_px.w > 0 && cell_px.h > 0 {
        args["cell_width_px"] = json!(cell_px.w);
        args["cell_height_px"] = json!(cell_px.h);
    }
    control
        .call("com.superlogical.terminal.resize", of_block(block, args))
        .await?;
    Ok(())
}

/// Takes a block for this client, so that the terminal has the size this client asks
/// for.
async fn take_block(control: &Control, block: &BlockRef) -> Result<(), RexError> {
    control
        .call(
            "session.own_block",
            json!({
                "session_id": block.session_id,
                "block_ids": [block.block_id],
                "mode": "take",
            }),
        )
        .await?;
    Ok(())
}

/// Says that the terminal gained or lost the focus. On gaining it the block is taken.
async fn tell_focus(control: &Control, block: &BlockRef, focused: bool) -> Result<(), RexError> {
    if focused {
        take_block(control, block).await?;
    }
    let action = if focused { "in" } else { "out" };
    control
        .call(
            "block.event",
            json!({
                "session_id": block.session_id,
                "block_id": block.block_id,
                "event": { "type": "focus", "payload": { "action": action } },
            }),
        )
        .await?;
    Ok(())
}

/// Gives up this client's claim on a block, so that the terminal follows whoever is
/// next in line for it.
pub(crate) async fn give_up(shared: &Shared, block: &BlockRef) {
    let Ok(control) = shared.control() else {
        return;
    };
    let _ = control
        .call(
            "session.disown_block",
            json!({ "session_id": block.session_id, "block_ids": [block.block_id] }),
        )
        .await;
}

/// Turns bytes into the text in them, across the pieces they come in.
#[derive(Default)]
struct Text {
    /// The beginning of a character whose end has not come yet.
    begun: Vec<u8>,
}

impl Text {
    /// The text in the next bytes. Bytes that are no part of any text are left out,
    /// and counted in `dropped`.
    fn take(&mut self, bytes: &[u8], dropped: &mut usize) -> String {
        let mut all = std::mem::take(&mut self.begun);
        all.extend_from_slice(bytes);
        let mut text = String::with_capacity(all.len());
        let mut rest = &all[..];
        loop {
            match std::str::from_utf8(rest) {
                Ok(valid) => {
                    text.push_str(valid);
                    return text;
                }
                Err(error) => {
                    let (valid, after) = rest.split_at(error.valid_up_to());
                    text.push_str(std::str::from_utf8(valid).unwrap_or_default());
                    match error.error_len() {
                        Some(invalid) => {
                            *dropped += invalid;
                            rest = &after[invalid..];
                        }
                        // A character that is not finished yet.
                        None => {
                            self.begun = after.to_vec();
                            return text;
                        }
                    }
                }
            }
        }
    }
}

/// Something the views of a terminal send to it.
#[derive(Debug, PartialEq, Eq)]
enum Op {
    /// What one view typed.
    Write(u64, Vec<u8>),
    Resize(TermSize, CellPx),
    /// A view gained the focus while another view here had it: the block is taken, and
    /// the terminal, which has the focus already, is told nothing.
    Take,
    Focus(bool),
    /// A view is gone, and with it what it had begun to type.
    Gone(u64),
}

/// Joins what can be done in one: what one view typed in several pieces is typed at
/// once, and of several sizes asked for only the last is.
fn fold(ops: Vec<Op>) -> Vec<Op> {
    let last_size = ops.iter().rposition(|op| matches!(op, Op::Resize(..)));
    let mut folded: Vec<Op> = Vec::with_capacity(ops.len());
    for (at, op) in ops.into_iter().enumerate() {
        match (folded.last_mut(), op) {
            (Some(Op::Write(view, typed)), Op::Write(same, more)) if *view == same => {
                typed.extend(more);
            }
            (_, Op::Resize(..)) if Some(at) != last_size => {}
            (_, op) => folded.push(op),
        }
    }
    folded
}

/// What one view last said of itself.
struct Voice {
    size: (TermSize, CellPx),
    focused: bool,
}

/// The views of one terminal in this application, and which of them speaks for it.
///
/// A server knows one client here, however many views of a terminal the application
/// shows: one owner of the block, one size asked for, one focus. So the views speak
/// with one voice. The view that gained the focus last decides the size, also after it
/// lost the focus again; while no view ever had it, the view that asked last does. The
/// terminal has the focus while any view has.
#[derive(Default)]
struct Voices {
    views: HashMap<u64, Voice>,
    speaker: Option<u64>,
}

impl Voices {
    fn focused(&self) -> bool {
        self.views.values().any(|voice| voice.focused)
    }

    /// A view gained the focus: it speaks from now on.
    fn gained(&mut self, view: u64, had_focus: bool) -> Vec<Op> {
        self.speaker = Some(view);
        let mut ops = vec![if had_focus { Op::Take } else { Op::Focus(true) }];
        if let Some(voice) = self.views.get(&view) {
            ops.push(Op::Resize(voice.size.0, voice.size.1));
        }
        ops
    }

    fn join(&mut self, view: u64, facts: &ClientFacts) -> Vec<Op> {
        let had_focus = self.focused();
        self.views.insert(
            view,
            Voice {
                size: (facts.size, facts.cell_px),
                focused: facts.focused,
            },
        );
        if facts.focused {
            self.gained(view, had_focus)
        } else if self.speaker.is_none() {
            vec![Op::Resize(facts.size, facts.cell_px)]
        } else {
            Vec::new()
        }
    }

    fn resize(&mut self, view: u64, size: TermSize, cell_px: CellPx) -> Vec<Op> {
        let Some(voice) = self.views.get_mut(&view) else {
            return Vec::new();
        };
        voice.size = (size, cell_px);
        if self.speaker.is_none_or(|speaker| speaker == view) {
            vec![Op::Resize(size, cell_px)]
        } else {
            Vec::new()
        }
    }

    fn focus(&mut self, view: u64, focused: bool) -> Vec<Op> {
        let had_focus = self.focused();
        match self.views.get_mut(&view) {
            Some(voice) if voice.focused != focused => voice.focused = focused,
            _ => return Vec::new(),
        }
        if focused {
            self.gained(view, had_focus)
        } else if !self.focused() {
            vec![Op::Focus(false)]
        } else {
            Vec::new()
        }
    }

    fn leave(&mut self, view: u64) -> Vec<Op> {
        let Some(voice) = self.views.remove(&view) else {
            return Vec::new();
        };
        if self.speaker == Some(view) {
            self.speaker = None;
        }
        let mut ops = vec![Op::Gone(view)];
        if voice.focused && !self.focused() {
            ops.push(Op::Focus(false));
        }
        ops
    }
}

/// Does what is not typing. Typing needs to know what each view has begun to type, and
/// is done by the task that keeps that.
async fn say(control: &Control, block: &BlockRef, op: &Op) -> Result<(), RexError> {
    match op {
        Op::Resize(size, cell_px) => ask_size(control, block, *size, *cell_px).await,
        Op::Take => take_block(control, block).await,
        Op::Focus(focused) => tell_focus(control, block, *focused).await,
        Op::Write(..) | Op::Gone(_) => Ok(()),
    }
}

/// The way from the views of a terminal block to the block.
pub(crate) struct Input {
    shared: Arc<Shared>,
    block: BlockRef,
    ops: mpsc::UnboundedSender<Op>,
    /// How many bytes were typed and not yet taken up.
    waiting: Arc<AtomicUsize>,
    voices: Mutex<Voices>,
}

impl Input {
    /// Starts the task that sends the views' input to `block`, and returns the way to
    /// it. The task ends when this is dropped.
    pub(crate) fn start(shared: Arc<Shared>, block: BlockRef) -> (Self, JoinHandle<()>) {
        let (ops, taken) = mpsc::unbounded_channel();
        let waiting = Arc::new(AtomicUsize::new(0));
        let task = tokio::spawn(run(
            Arc::clone(&shared),
            block.clone(),
            taken,
            Arc::clone(&waiting),
        ));
        let input = Self {
            shared,
            block,
            ops,
            waiting,
            voices: Mutex::new(Voices::default()),
        };
        (input, task)
    }

    fn voices(&self) -> MutexGuard<'_, Voices> {
        self.voices.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn send(&self, ops: Vec<Op>) {
        for op in ops {
            let _ = self.ops.send(op);
        }
    }

    /// The first view of a terminal says what it is, and this waits until the server
    /// has heard: the terminal is then already of the view's size in the copy the
    /// view starts from, where the server lets this client decide. Nothing here keeps
    /// the view from being made.
    pub(crate) async fn arrive(&self, view: u64, facts: &ClientFacts) {
        let ops = self.voices().join(view, facts);
        let Ok(control) = self.shared.control() else {
            return;
        };
        for op in &ops {
            if let Err(error) = say(&control, &self.block, op).await {
                tracing::debug!(server = %self.shared.name, %error, "a terminal was not told of its new view");
                return;
            }
        }
    }

    /// A further view of a terminal says what it is.
    pub(crate) fn join(&self, view: u64, facts: &ClientFacts) {
        let ops = self.voices().join(view, facts);
        self.send(ops);
    }

    /// A view is gone.
    pub(crate) fn leave(&self, view: u64) {
        let ops = self.voices().leave(view);
        self.send(ops);
    }

    /// Types what a view typed. Refused when more is waiting to be typed than is kept.
    pub(crate) fn write(&self, view: u64, bytes: &[u8]) -> Result<(), SessionError> {
        let waiting = self.waiting.fetch_add(bytes.len(), Ordering::AcqRel);
        if waiting.saturating_add(bytes.len()) > INPUT_PENDING_MAX_BYTES {
            self.waiting.fetch_sub(bytes.len(), Ordering::AcqRel);
            return Err(SessionError::Refused(ProtoError::new(
                ErrorCode::LimitExceeded,
                "more was typed than the Rex server has taken yet",
            )));
        }
        self.ops
            .send(Op::Write(view, bytes.to_vec()))
            .map_err(|_| SessionError::Detached)
    }

    /// A view would like the terminal to have a size.
    pub(crate) fn resize(&self, view: u64, size: TermSize, cell_px: CellPx) {
        let ops = self.voices().resize(view, size, cell_px);
        self.send(ops);
    }

    /// A view says whether it has the focus.
    pub(crate) fn focus(&self, view: u64, focused: bool) {
        let ops = self.voices().focus(view, focused);
        self.send(ops);
    }
}

async fn run(
    shared: Arc<Shared>,
    block: BlockRef,
    mut ops: mpsc::UnboundedReceiver<Op>,
    waiting: Arc<AtomicUsize>,
) {
    // What each view has begun to type.
    let mut texts: HashMap<u64, Text> = HashMap::new();
    while let Some(first) = ops.recv().await {
        let mut batch = vec![first];
        while let Ok(more) = ops.try_recv() {
            batch.push(more);
        }
        for op in fold(batch) {
            // With no connection there is nowhere to send it, and what was typed then
            // is not typed later, when nobody expects it any more.
            let control = shared.control();
            let done = match (&op, &control) {
                (Op::Gone(view), _) => {
                    texts.remove(view);
                    Ok(())
                }
                (Op::Write(view, bytes), control) => {
                    waiting.fetch_sub(bytes.len(), Ordering::AcqRel);
                    let mut dropped = 0;
                    let typed = texts.entry(*view).or_default().take(bytes, &mut dropped);
                    if dropped > 0 {
                        tracing::debug!(server = %shared.name, dropped, "bytes that are not text cannot be typed into a Rex terminal");
                    }
                    match control {
                        Ok(control) => type_text(control, &block, &typed).await,
                        Err(_) => {
                            texts.remove(view);
                            Err(RexError::Disconnected)
                        }
                    }
                }
                (op, Ok(control)) => say(control, &block, op).await,
                (_, Err(_)) => Err(RexError::Disconnected),
            };
            if let Err(error) = done {
                tracing::debug!(server = %shared.name, %error, "a view's input did not reach its terminal");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(pieces: &[&[u8]]) -> (String, usize) {
        let mut text = Text::default();
        let mut dropped = 0;
        let all = pieces
            .iter()
            .map(|piece| text.take(piece, &mut dropped))
            .collect();
        (all, dropped)
    }

    #[test]
    fn a_character_that_comes_in_pieces_is_typed_whole() {
        let snowman = "☃".as_bytes();
        assert_eq!(typed(&[b"a", snowman, b"b"]), ("a☃b".to_owned(), 0));
        assert_eq!(
            typed(&[b"a", &snowman[..1], &snowman[1..2], &snowman[2..], b"b"]),
            ("a☃b".to_owned(), 0)
        );
        // Keys that are not letters are text all the same.
        assert_eq!(typed(&[b"\x1b[A\x03\r\x7f"]).0, "\u{1b}[A\u{3}\r\u{7f}");
    }

    #[test]
    fn bytes_that_are_not_text_are_left_out() {
        // A mouse report of the old kind, with a coordinate past 127.
        assert_eq!(typed(&[b"\x1b[M \xf0\x21"]), ("\u{1b}[M !".to_owned(), 1));
        assert_eq!(typed(&[b"a\xff\xfeb"]), ("ab".to_owned(), 2));
        // A character begun and never finished is dropped when the next thing comes.
        let snowman = "☃".as_bytes();
        assert_eq!(typed(&[&snowman[..2], b"x"]), ("x".to_owned(), 2));
        // And waits for as long as nothing does.
        assert_eq!(typed(&[b"a", &snowman[..2]]), ("a".to_owned(), 0));
    }

    fn size(cols: u16) -> Op {
        Op::Resize(TermSize::new(cols, 24).unwrap(), CellPx::default())
    }

    fn write(view: u64, text: &str) -> Op {
        Op::Write(view, text.as_bytes().to_vec())
    }

    #[test]
    fn what_a_view_typed_in_pieces_is_joined_and_only_the_last_size_asked() {
        let folded = fold(vec![
            write(1, "a"),
            write(1, "b"),
            size(80),
            write(1, "c"),
            Op::Focus(true),
            size(90),
            write(1, "d"),
            write(2, "x"),
            write(2, "y"),
            write(1, "e"),
            Op::Focus(false),
        ]);
        // The order of what was typed is kept, across the size that was let go, and
        // what two views typed is not run together.
        assert_eq!(
            folded,
            [
                write(1, "abc"),
                Op::Focus(true),
                size(90),
                write(1, "d"),
                write(2, "xy"),
                write(1, "e"),
                Op::Focus(false),
            ]
        );
    }

    fn facts(cols: u16, focused: bool) -> ClientFacts {
        ClientFacts {
            size: TermSize::new(cols, 24).unwrap(),
            cell_px: CellPx::default(),
            palette: lume_proto::Palette::default(),
            scheme: lume_proto::ColorScheme::Dark,
            focused,
            visible: true,
        }
    }

    #[test]
    fn the_views_of_a_terminal_speak_with_one_voice() {
        let mut voices = Voices::default();
        let new_size = |cols| (TermSize::new(cols, 24).unwrap(), CellPx::default());

        // While no view ever had the focus, whoever asks last is heard.
        assert_eq!(voices.join(1, &facts(80, false)), [size(80)]);
        let (cols, cell) = new_size(81);
        assert_eq!(voices.resize(1, cols, cell), [size(81)]);

        // A view with the focus takes the block, and speaks from then on.
        assert_eq!(
            voices.join(2, &facts(100, true)),
            [Op::Focus(true), size(100)]
        );
        let (cols, cell) = new_size(82);
        assert_eq!(voices.resize(1, cols, cell), []);
        let (cols, cell) = new_size(101);
        assert_eq!(voices.resize(2, cols, cell), [size(101)]);

        // Saying the same again says nothing.
        assert_eq!(voices.focus(2, true), []);
        assert_eq!(voices.focus(1, false), []);

        // The other view gains the focus while the first still has it: the terminal
        // has the focus already, and takes the size the new speaker last asked for.
        assert_eq!(voices.focus(1, true), [Op::Take, size(82)]);
        assert_eq!(voices.focus(2, false), []);
        // The last view to lose the focus takes it from the terminal, and still
        // speaks.
        assert_eq!(voices.focus(1, false), [Op::Focus(false)]);
        let (cols, cell) = new_size(83);
        assert_eq!(voices.resize(1, cols, cell), [size(83)]);
        assert_eq!(voices.resize(2, cols, cell), []);

        // The speaker goes: whoever asks next is heard again.
        assert_eq!(voices.leave(1), [Op::Gone(1)]);
        let (cols, cell) = new_size(102);
        assert_eq!(voices.resize(2, cols, cell), [size(102)]);
        // A view that goes with the focus takes it along.
        assert_eq!(voices.focus(2, true), [Op::Focus(true), size(102)]);
        assert_eq!(voices.leave(2), [Op::Gone(2), Op::Focus(false)]);
        assert_eq!(voices.leave(2), []);
        assert_eq!(voices.focus(7, true), []);
    }
}
