//! Shared by this crate's tests: a carrier that records what it is given, and the
//! pieces of a session against a real shell.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use lume_proto::{FrameDecoder, Header, Kind, Message, Role, encode};
use lume_session::{CellPx, ClientFacts, ColorScheme, Launcher, Palette, SpawnSpec, TermSize};

use crate::carrier::{Carrier, CarrierClosed};

/// How long any single step of a test may take before the test fails.
pub const STEP: Duration = Duration::from_secs(20);

pub const CREDIT: u32 = 1024 * 1024;

/// A carrier that keeps every message it is handed.
#[derive(Default)]
pub struct RecordingCarrier {
    batches: Mutex<Vec<Vec<u8>>>,
    closed: AtomicBool,
}

impl RecordingCarrier {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Makes every later delivery fail, as it does once a webview is gone.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
    }

    /// The messages delivered so far, one entry for each delivery.
    pub fn batches(&self) -> Vec<Vec<u8>> {
        self.batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The frames delivered so far, each as its header and its payload.
    fn frames(&self) -> Vec<(Header, Vec<u8>)> {
        let mut frames = Vec::new();
        for batch in self.batches() {
            let mut rest = batch.as_slice();
            while let Some(header) = rest.first_chunk::<{ Header::LEN }>() {
                let header = Header::parse(header).expect("the carrier was given valid frames");
                let end = Header::LEN + header.len as usize;
                frames.push((header, rest[Header::LEN..end].to_vec()));
                rest = &rest[end..];
            }
        }
        frames
    }

    /// Every frame of a stream delivered so far, decoded as a webview would decode it.
    pub fn messages(&self) -> Vec<Message> {
        let mut decoder = FrameDecoder::new(Role::Client);
        for (header, payload) in self.frames() {
            // What the core asks the page is not a message of the session protocol.
            if header.kind != Kind::Control {
                let mut frame = Vec::new();
                header.write(&mut frame);
                frame.extend_from_slice(&payload);
                decoder.push(&frame);
            }
        }
        let mut messages = Vec::new();
        while let Some(message) = decoder.next().expect("the carrier was given valid frames") {
            messages.push(message);
        }
        messages
    }

    /// Everything the core has asked the page so far, as the page receives it.
    pub fn requests(&self) -> Vec<serde_json::Value> {
        self.frames()
            .into_iter()
            .filter(|(header, _)| header.kind == Kind::Control)
            .map(|(_, payload)| serde_json::from_slice(&payload).expect("a request is JSON"))
            .collect()
    }
}

impl Carrier for RecordingCarrier {
    fn deliver(&self, frames: Vec<u8>) -> Result<(), CarrierClosed> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(CarrierClosed);
        }
        self.batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(frames);
        Ok(())
    }
}

/// The bytes a webview would send for `messages`.
pub fn client_frames(messages: &[Message]) -> Vec<u8> {
    let mut out = Vec::new();
    for message in messages {
        encode(message, &mut out).expect("a message a view may send");
    }
    out
}

pub fn size(cols: u16, rows: u16) -> TermSize {
    TermSize::new(cols, rows).expect("a valid size")
}

pub fn facts() -> ClientFacts {
    ClientFacts {
        size: size(80, 24),
        cell_px: CellPx { w: 9, h: 18 },
        palette: Palette::default(),
        scheme: ColorScheme::Dark,
        focused: true,
        visible: true,
    }
}

/// An interactive shell with `LUME_MARK` set, so a command can print something that
/// does not appear in its own echo.
pub fn shell() -> SpawnSpec {
    let (program, args) = if cfg!(windows) {
        ("cmd.exe", vec!["/d"])
    } else {
        ("/bin/sh", vec![])
    };
    SpawnSpec {
        launcher: Launcher::Shell {
            program: program.to_owned(),
            args: args.into_iter().map(str::to_owned).collect(),
            login: false,
        },
        cwd: None,
        env: vec![("LUME_MARK".to_owned(), "k7q".to_owned())],
        size: size(80, 24),
    }
}

/// A command that prints `lume-k7q-done`.
pub const PRINT_MARK: &[u8] = if cfg!(windows) {
    b"echo lume-%LUME_MARK%-done\r"
} else {
    b"echo \"lume-${LUME_MARK}-done\"\r"
};
