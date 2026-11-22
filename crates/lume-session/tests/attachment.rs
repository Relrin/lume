//! An attachment made outside this crate: what a backend other than the local one needs
//! to hand a view its session.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use lume_proto::{CellPx, ClientFacts, TermSize};
use lume_session::{Attachment, FrameStream, Hydrate, InputSink, OutputFrame, Result, Sink};
use tokio::sync::mpsc;

/// A sink that notes what it is asked and answers a resize as a session would.
struct Noting {
    asked: Mutex<Vec<String>>,
    frames: mpsc::UnboundedSender<OutputFrame>,
}

impl Noting {
    fn note(&self, what: impl Into<String>) {
        self.asked.lock().unwrap().push(what.into());
    }
}

#[async_trait]
impl Sink for Noting {
    async fn write(&self, bytes: &[u8]) -> Result<()> {
        self.note(format!("write {}", String::from_utf8_lossy(bytes)));
        Ok(())
    }

    async fn resize(&self, size: TermSize, _cell_px: CellPx) -> Result<()> {
        self.note("resize");
        let _ = self.frames.send(OutputFrame::Resized(size));
        Ok(())
    }

    async fn facts(&self, _facts: &ClientFacts) -> Result<()> {
        self.note("facts");
        Ok(())
    }

    async fn credit(&self, bytes: u32) -> Result<()> {
        self.note(format!("credit {bytes}"));
        Ok(())
    }

    async fn rehydrate(&self) -> Result<()> {
        self.note("rehydrate");
        Ok(())
    }

    async fn release(&self) -> Result<()> {
        self.note("release");
        Ok(())
    }

    async fn detach(&self) -> Result<()> {
        self.note("detach");
        Ok(())
    }
}

#[tokio::test]
async fn a_backend_of_another_crate_can_make_an_attachment() {
    let (frames, received) = mpsc::unbounded_channel();
    let sink = Arc::new(Noting {
        asked: Mutex::new(Vec::new()),
        frames: frames.clone(),
    });
    let size = TermSize::new(80, 24).unwrap();
    let mut attachment = Attachment {
        size,
        resize_pulls_scrollback: false,
        hydrate: Hydrate::VtReplay(b"hello".to_vec()),
        output: FrameStream::new(received),
        input: InputSink::new(sink.clone()),
    };

    attachment.input.credit(4096).await.unwrap();
    attachment.input.write(b"ls").await.unwrap();
    let larger = TermSize::new(100, 30).unwrap();
    attachment
        .input
        .resize(larger, CellPx { w: 9, h: 18 })
        .await
        .unwrap();
    frames.send(OutputFrame::Data(b"out".to_vec())).unwrap();
    attachment.input.detach().await.unwrap();

    assert_eq!(
        attachment.output.next().await,
        Some(OutputFrame::Resized(larger))
    );
    assert_eq!(
        attachment.output.next().await,
        Some(OutputFrame::Data(b"out".to_vec()))
    );
    assert_eq!(
        *sink.asked.lock().unwrap(),
        ["credit 4096", "write ls", "resize", "detach"]
    );

    // Once the backend lets go of its end, the attachment has ended.
    drop(frames);
    drop(sink);
    drop(attachment.input);
    assert_eq!(attachment.output.next().await, None);
}
