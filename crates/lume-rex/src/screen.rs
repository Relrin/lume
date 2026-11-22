//! A terminal block's screen as text, for whoever asks without showing the terminal.
//!
//! A server writes a terminal's screen out as output that rebuilds it. That output is
//! fed to a terminal of this application's own engine, of the same size, and the text
//! and the cursor are read from there: the same way a view would come to show them.
//! A server writes the scrollback out along with the screen, so the more a terminal
//! has scrolled, the more reading its screen costs.

use lume_proto::TermSize;
use lume_proto::app::ScreenCursor;
use lume_session::{ScreenText, TextFound};
use lume_vt::Terminal;
use serde_json::Value;

use crate::api::{TerminalModes, TerminalSize};
use crate::control::Control;
use crate::error::RexError;
use crate::ids::BlockRef;
use crate::input::of_block;
use crate::terminal::read_replay;

/// Reads the screen of a terminal block.
pub(crate) async fn read(control: &Control, block: &BlockRef) -> Result<ScreenText, RexError> {
    let (size, _) = control
        .call_as::<TerminalSize>(
            "com.superlogical.terminal.size",
            of_block(block, Value::Null),
        )
        .await?;
    let (modes, _) = control
        .call_as::<TerminalModes>(
            "com.superlogical.terminal.modes",
            of_block(block, Value::Null),
        )
        .await?;
    let replay = read_replay(control, block).await?;

    let fits = |count: u32| u16::try_from(count).unwrap_or(u16::MAX).min(TermSize::MAX);
    let size = TermSize::new(fits(size.columns), fits(size.rows))
        .map_err(|_| RexError::Protocol("a terminal has no size".to_owned()))?;
    let alternate = modes.screen == "alternate";
    let failed = |error: lume_vt::Error| {
        RexError::Protocol(format!("a terminal's screen could not be rebuilt: {error}"))
    };
    // The engine's terminal stays on one thread, which must not be one that others
    // wait on.
    let rebuilding = tokio::task::spawn_blocking(move || {
        let mut terminal = Terminal::new(size.cols(), size.rows()).map_err(failed)?;
        terminal.write(&replay);
        let cursor = terminal.cursor().map_err(failed)?;
        Ok(ScreenText {
            size,
            cursor: ScreenCursor {
                col: cursor.x,
                row: cursor.y,
                visible: cursor.visible,
            },
            alternate,
            lines: terminal.screen_text().map_err(failed)?,
        })
    });
    rebuilding.await.map_err(|error| {
        RexError::Protocol(format!("a terminal's screen was not rebuilt: {error}"))
    })?
}

/// Where `text` is on a screen: the first row that holds it.
pub(crate) fn find(screen: &ScreenText, text: &str) -> Option<TextFound> {
    let row = screen.lines.iter().position(|line| line.contains(text))?;
    Some(TextFound {
        row: u16::try_from(row).ok()?,
        line: screen.lines[row].clone(),
    })
}
