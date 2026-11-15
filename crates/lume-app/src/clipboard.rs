//! The system clipboard.
//!
//! The webview reaches the clipboard through the core, not through the browser's own
//! interface, which asks the user for permission in ways a terminal cannot explain.
//!
//! One thread owns the clipboard for as long as the application runs. On some systems
//! the contents of the clipboard live in the program that put them there, so a handle
//! that was dropped after each write would take the text with it.

use std::sync::mpsc;

use tokio::sync::oneshot;

use crate::error::CommandError;

enum Request {
    Read(oneshot::Sender<Result<Option<String>, String>>),
    Write(String, oneshot::Sender<Result<(), String>>),
}

/// The way to the thread that owns the clipboard.
pub struct ClipboardThread {
    requests: mpsc::Sender<Request>,
}

impl ClipboardThread {
    /// Starts the thread. The clipboard itself is opened when it is first asked for.
    pub fn start() -> Self {
        let (requests, queued) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("clipboard".to_owned())
            .spawn(move || serve(&queued));
        if let Err(error) = spawned {
            // Requests then fail one by one, which says more than failing to start.
            tracing::error!(%error, "the clipboard thread did not start");
        }
        Self { requests }
    }

    /// The text on the clipboard, or `None` when it holds no text.
    pub async fn read(&self) -> Result<Option<String>, CommandError> {
        let (reply, answered) = oneshot::channel();
        self.ask(Request::Read(reply), answered).await
    }

    /// Puts text on the clipboard, replacing what was there.
    pub async fn write(&self, text: String) -> Result<(), CommandError> {
        let (reply, answered) = oneshot::channel();
        self.ask(Request::Write(text, reply), answered).await
    }

    async fn ask<T>(
        &self,
        request: Request,
        answered: oneshot::Receiver<Result<T, String>>,
    ) -> Result<T, CommandError> {
        let gone = || CommandError::internal("the clipboard is not available");
        self.requests.send(request).map_err(|_| gone())?;
        answered
            .await
            .map_err(|_| gone())?
            .map_err(CommandError::internal)
    }
}

fn serve(queued: &mpsc::Receiver<Request>) {
    let mut clipboard: Option<arboard::Clipboard> = None;
    while let Ok(request) = queued.recv() {
        // Opened on demand, and again after a failure: a clipboard that was not there
        // when the application started may be there now.
        let opened = match &mut clipboard {
            Some(open) => Ok(open),
            None => arboard::Clipboard::new().map(|open| clipboard.insert(open)),
        };
        match (request, opened) {
            (Request::Read(reply), Ok(open)) => {
                let text = match open.get_text() {
                    Ok(text) => Ok(Some(text)),
                    // Empty, or holding something that is not text.
                    Err(arboard::Error::ContentNotAvailable) => Ok(None),
                    Err(error) => Err(error.to_string()),
                };
                let _ = reply.send(text);
            }
            (Request::Write(text, reply), Ok(open)) => {
                let _ = reply.send(open.set_text(text).map_err(|error| error.to_string()));
            }
            (Request::Read(reply), Err(error)) => {
                let _ = reply.send(Err(error.to_string()));
            }
            (Request::Write(_, reply), Err(error)) => {
                let _ = reply.send(Err(error.to_string()));
            }
        }
    }
}
