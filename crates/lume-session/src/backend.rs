//! The interface every session backend offers, and what attaching to a session yields.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lume_layout::LayoutAuthority;
use lume_proto::app::ScreenCursor;
use lume_proto::{
    BackendCaps, CellPx, ClientFacts, ErrorCode, ExitStatus, ProtoError, SessionEvent, SessionId,
    SessionInfo, SpawnSpec, TermSize,
};
use tokio::sync::mpsc;

use crate::{Result, SessionError};

/// The screen a session's program draws on, as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenText {
    pub size: TermSize,
    pub cursor: ScreenCursor,
    /// Whether a full-screen program has switched to the alternate screen.
    pub alternate: bool,
    /// One string for each row, with the blanks at its end removed.
    pub lines: Vec<String>,
}

/// Where text that was waited for appeared on a screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextFound {
    /// The first row that holds it.
    pub row: u16,
    /// That row's text.
    pub line: String,
}

fn unsupported(what: &str) -> SessionError {
    SessionError::Refused(ProtoError::new(
        ErrorCode::Unsupported,
        format!("this backend cannot {what}"),
    ))
}

/// Something that runs terminal sessions.
///
/// Every call is addressed by id and may cross a process or a network, so every call
/// that does work is asynchronous. Callers branch on [`caps`](Self::caps), never on
/// which backend they have.
#[async_trait]
pub trait SessionBackend: Send + Sync {
    /// What this backend can do.
    fn caps(&self) -> BackendCaps;

    async fn list(&self) -> Result<Vec<SessionInfo>>;

    async fn spawn(&self, spec: SpawnSpec) -> Result<SessionId>;

    /// Attaches to a session. `facts` describes the view the session will be shown in.
    async fn attach(&self, id: SessionId, facts: ClientFacts) -> Result<Attachment>;

    async fn kill(&self, id: SessionId) -> Result<()>;

    /// Reads a session's screen, for a backend that keeps one. Others refuse with
    /// `unsupported`, and the screen has to be read from a view showing the session.
    async fn screen(&self, _id: SessionId) -> Result<ScreenText> {
        Err(unsupported("read a screen"))
    }

    /// Waits up to `timeout` for `text` to be on a session's screen, for a backend that
    /// keeps one. Fails with `timeout` when it is not there in time.
    async fn wait_text(
        &self,
        _id: SessionId,
        _text: &str,
        _timeout: Duration,
    ) -> Result<TextFound> {
        Err(unsupported("wait for text on a screen"))
    }

    /// Who decides how this backend's sessions are arranged in panes, for a backend
    /// that decides it itself (its [`caps`](Self::caps) say `server_layout`). Any other
    /// backend has nothing to offer here: its sessions are arranged by the
    /// application's own layout.
    fn layout(&self) -> Option<Arc<dyn LayoutAuthority>> {
        None
    }
}

/// An open attachment: everything that connects a view to its session.
pub struct Attachment {
    /// The terminal's size when the attachment opened.
    pub size: TermSize,
    /// Whether the view's terminal should pull lines back out of scrollback when it
    /// grows taller. It should not when the session's own terminal keeps a copy of the
    /// screen that knows nothing of the view's scrollback.
    pub resize_pulls_scrollback: bool,
    /// The state to start from, before any output is applied.
    pub hydrate: Hydrate,
    pub output: FrameStream,
    pub input: InputSink,
}

/// How a newly attached view is brought up to date.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hydrate {
    /// Nothing: the view starts blank and follows the output from here.
    None,
    /// A serialised terminal, loadable only by an engine of the version that wrote it.
    Snapshot { vt_version: String, bytes: Vec<u8> },
    /// Terminal output that rebuilds the screen when fed to any engine.
    VtReplay(Vec<u8>),
}

/// One thing a session sends to an attached view, in the order it happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputFrame {
    /// Terminal output. Counted against the credit the view has granted.
    Data(Vec<u8>),
    /// The terminal was resized at this point of the output.
    Resized(TermSize),
    Event(SessionEvent),
    /// The session's program ended. Nothing follows.
    Exit(ExitStatus),
    /// The state to start again from, as asked for with [`InputSink::rehydrate`]: the
    /// output before it is part of it, and the output after it follows on from it.
    Hydrate(Hydrate),
}

/// The frames of one attachment.
///
/// The stream holds what has arrived and not been taken. That is bounded by the credit
/// granted through the attachment's [`InputSink`]: a session sends no more output than
/// it has credit for, so a view that stops granting stops the flow.
pub struct FrameStream {
    frames: mpsc::UnboundedReceiver<OutputFrame>,
}

impl FrameStream {
    pub(crate) fn new(frames: mpsc::UnboundedReceiver<OutputFrame>) -> Self {
        Self { frames }
    }

    /// The next frame, or `None` once the attachment has ended.
    pub async fn next(&mut self) -> Option<OutputFrame> {
        self.frames.recv().await
    }
}

/// What a backend does with the frames a view sends. One per attachment.
#[async_trait]
pub(crate) trait Sink: Send + Sync {
    async fn write(&self, bytes: &[u8]) -> Result<()>;
    async fn resize(&self, size: TermSize, cell_px: CellPx) -> Result<()>;
    async fn facts(&self, facts: &ClientFacts) -> Result<()>;
    async fn credit(&self, bytes: u32) -> Result<()>;
    async fn rehydrate(&self) -> Result<()>;
    async fn detach(&self) -> Result<()>;
}

/// The way from a view to its session. Cheap to clone.
#[derive(Clone)]
pub struct InputSink {
    sink: Arc<dyn Sink>,
}

impl InputSink {
    pub(crate) fn new(sink: Arc<dyn Sink>) -> Self {
        Self { sink }
    }

    /// Sends bytes to the session's program, as if typed.
    pub async fn write(&self, bytes: &[u8]) -> Result<()> {
        self.sink.write(bytes).await
    }

    /// Asks for the terminal to be resized. The session answers with a
    /// [`OutputFrame::Resized`] at the place in the output where it took effect.
    pub async fn resize(&self, size: TermSize, cell_px: CellPx) -> Result<()> {
        self.sink.resize(size, cell_px).await
    }

    /// Tells the session how the view has changed. This never resizes the terminal.
    pub async fn facts(&self, facts: &ClientFacts) -> Result<()> {
        self.sink.facts(facts).await
    }

    /// Lets the session send `bytes` more of output. A new attachment has no credit,
    /// so nothing arrives until this is called.
    pub async fn credit(&self, bytes: u32) -> Result<()> {
        self.sink.credit(bytes).await
    }

    /// Asks for the state to start again from, as of this point of the output. It
    /// arrives as an [`OutputFrame::Hydrate`] in its place among the frames, from a
    /// backend whose caps say it can hydrate.
    pub async fn rehydrate(&self) -> Result<()> {
        self.sink.rehydrate().await
    }

    /// Ends the attachment. The session carries on.
    pub async fn detach(&self) -> Result<()> {
        self.sink.detach().await
    }
}
