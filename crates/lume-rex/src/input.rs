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
//! the terminal follow the view a person is looking at.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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

/// Says that a view gained or lost the focus. On gaining it the block is taken, so
/// that the terminal has the size this view asked for.
async fn tell_focus(control: &Control, block: &BlockRef, focused: bool) -> Result<(), RexError> {
    if focused {
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

/// What a view says of itself when it attaches, before its stream begins: the terminal
/// is then already of the view's size in the copy the view starts from, where the
/// server lets this client decide. Nothing here keeps the attachment from being made.
pub(crate) async fn arrive(shared: &Shared, block: &BlockRef, facts: &ClientFacts) {
    let Ok(control) = shared.control() else {
        return;
    };
    let said = async {
        if facts.focused {
            tell_focus(&control, block, true).await?;
        }
        ask_size(&control, block, facts.size, facts.cell_px).await
    };
    if let Err(error) = said.await {
        tracing::debug!(server = %shared.name, %error, "a terminal was not told of its new view");
    }
}

/// Gives up this client's claim on a block, so that the terminal follows whoever is
/// next in line for it.
pub(crate) async fn leave(shared: &Shared, block: &BlockRef) {
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

/// Something a view sends to its terminal.
enum Op {
    Write(Vec<u8>),
    Resize(TermSize, CellPx),
    Focus(bool),
}

/// Joins what can be done in one: what was typed in several pieces is typed at once,
/// and of several sizes asked for only the last is.
fn fold(ops: Vec<Op>) -> Vec<Op> {
    let last_size = ops.iter().rposition(|op| matches!(op, Op::Resize(..)));
    let mut folded: Vec<Op> = Vec::with_capacity(ops.len());
    for (at, op) in ops.into_iter().enumerate() {
        match (folded.last_mut(), op) {
            (Some(Op::Write(typed)), Op::Write(more)) => typed.extend(more),
            (_, Op::Resize(..)) if Some(at) != last_size => {}
            (_, op) => folded.push(op),
        }
    }
    folded
}

/// The way from a view to its terminal block.
pub(crate) struct Input {
    ops: mpsc::UnboundedSender<Op>,
    /// How many bytes were typed and not yet taken up.
    waiting: Arc<AtomicUsize>,
}

fn gone() -> SessionError {
    SessionError::Detached
}

impl Input {
    /// Starts the task that sends a view's input to `block`, and returns the way to
    /// it. The task ends when this is dropped.
    pub(crate) fn start(shared: Arc<Shared>, block: BlockRef) -> (Self, JoinHandle<()>) {
        let (ops, taken) = mpsc::unbounded_channel();
        let waiting = Arc::new(AtomicUsize::new(0));
        let task = tokio::spawn(run(shared, block, taken, Arc::clone(&waiting)));
        (Self { ops, waiting }, task)
    }

    /// Types bytes. Refused when more is waiting to be typed than is kept.
    pub(crate) fn write(&self, bytes: &[u8]) -> Result<(), SessionError> {
        let waiting = self.waiting.fetch_add(bytes.len(), Ordering::AcqRel);
        if waiting.saturating_add(bytes.len()) > INPUT_PENDING_MAX_BYTES {
            self.waiting.fetch_sub(bytes.len(), Ordering::AcqRel);
            return Err(SessionError::Refused(ProtoError::new(
                ErrorCode::LimitExceeded,
                "more was typed than the Rex server has taken yet",
            )));
        }
        self.ops.send(Op::Write(bytes.to_vec())).map_err(|_| gone())
    }

    pub(crate) fn resize(&self, size: TermSize, cell_px: CellPx) -> Result<(), SessionError> {
        self.ops.send(Op::Resize(size, cell_px)).map_err(|_| gone())
    }

    pub(crate) fn focus(&self, focused: bool) -> Result<(), SessionError> {
        self.ops.send(Op::Focus(focused)).map_err(|_| gone())
    }
}

async fn run(
    shared: Arc<Shared>,
    block: BlockRef,
    mut ops: mpsc::UnboundedReceiver<Op>,
    waiting: Arc<AtomicUsize>,
) {
    let mut text = Text::default();
    while let Some(first) = ops.recv().await {
        let mut batch = vec![first];
        while let Ok(more) = ops.try_recv() {
            batch.push(more);
        }
        for op in fold(batch) {
            // With no connection there is nowhere to send it, and what was typed then
            // is not typed later, when nobody expects it any more.
            let control = shared.control();
            let done = match op {
                Op::Write(bytes) => {
                    waiting.fetch_sub(bytes.len(), Ordering::AcqRel);
                    let mut dropped = 0;
                    let typed = text.take(&bytes, &mut dropped);
                    if dropped > 0 {
                        tracing::debug!(server = %shared.name, dropped, "bytes that are not text cannot be typed into a Rex terminal");
                    }
                    match &control {
                        Ok(control) => type_text(control, &block, &typed).await,
                        Err(_) => {
                            text = Text::default();
                            Err(RexError::Disconnected)
                        }
                    }
                }
                Op::Resize(size, cell_px) => match &control {
                    Ok(control) => ask_size(control, &block, size, cell_px).await,
                    Err(_) => Err(RexError::Disconnected),
                },
                Op::Focus(focused) => match &control {
                    Ok(control) => tell_focus(control, &block, focused).await,
                    Err(_) => Err(RexError::Disconnected),
                },
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

    #[test]
    fn what_was_typed_in_pieces_is_joined_and_only_the_last_size_asked() {
        let size = |cols| Op::Resize(TermSize::new(cols, 24).unwrap(), CellPx::default());
        let folded = fold(vec![
            Op::Write(b"a".to_vec()),
            Op::Write(b"b".to_vec()),
            size(80),
            Op::Write(b"c".to_vec()),
            Op::Focus(true),
            size(90),
            Op::Write(b"d".to_vec()),
            Op::Write(b"e".to_vec()),
            Op::Focus(false),
        ]);
        let shown: Vec<String> = folded
            .iter()
            .map(|op| match op {
                Op::Write(bytes) => String::from_utf8_lossy(bytes).into_owned(),
                Op::Resize(size, _) => format!("size {}", size.cols()),
                Op::Focus(focused) => format!("focus {focused}"),
            })
            .collect();
        // The order of what was typed is kept, across the size that was let go.
        assert_eq!(shown, ["abc", "focus true", "size 90", "de", "focus false"]);
    }
}
