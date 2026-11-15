//! The byte carrier between the core and one webview, and the streams that travel on it.
//!
//! A webview shows sessions through attachments. Each attachment is a stream: the
//! session's output, resize marks, events and exit go to the webview as frames, and the
//! webview's input, size requests, facts, credit and detach come back as frames. The
//! framing is the session protocol's own, so both ends are checked against the same
//! recorded byte streams, and the bytes of a terminal are never re-encoded as text.
//!
//! A [`Link`] is one carrier with its streams. It keeps no credit: the webview's grants
//! go straight to the session, which sends no more than it was granted. What waits in
//! a link is therefore bounded by the credit the webview handed out.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use lume_proto::{Body, FrameDecoder, Header, HydratePayload, Kind, Message, Role, encode};
use lume_session::{
    Attachment, BackendId, ExitStatus, Hydrate, InputSink, OutputFrame, SessionError, SessionId,
};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use crate::error::CommandError;
use crate::tuning::CARRIER_BATCH_MAX;

/// The other end of the carrier is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CarrierClosed;

/// The way from the core to one webview.
pub trait Carrier: Send + Sync + 'static {
    /// Hands over whole frames. Successive calls are delivered in order.
    fn deliver(&self, frames: Vec<u8>) -> Result<(), CarrierClosed>;
}

/// A carrier over a Tauri channel, which takes raw bytes and keeps their order.
pub struct TauriChannelCarrier(pub tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>);

impl Carrier for TauriChannelCarrier {
    fn deliver(&self, frames: Vec<u8>) -> Result<(), CarrierClosed> {
        self.0
            .send(tauri::ipc::InvokeResponseBody::Raw(frames))
            .map_err(|_| CarrierClosed)
    }
}

/// Names a session wherever it runs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionKey {
    pub backend: BackendId,
    pub session: SessionId,
}

/// Called when a session's program has ended, from the task that forwards its output.
pub type OnExit = Arc<dyn Fn(&SessionKey) + Send + Sync>;

/// One carrier and the streams on it.
pub struct Link {
    id: u32,
    outgoing: mpsc::UnboundedSender<Message>,
    state: Mutex<LinkState>,
    writer: AbortHandle,
    runtime: Handle,
    on_exit: OnExit,
}

struct LinkState {
    streams: HashMap<u32, Stream>,
    /// The highest stream id the webview has used. Ids only go up, so one is never
    /// reused and a late frame cannot land on a newer attachment.
    last_stream: u32,
    retired: bool,
}

struct Stream {
    input: InputSink,
    pump: AbortHandle,
}

impl Link {
    /// Starts a link that delivers to `carrier`. `id` is how the webview names it.
    pub fn open(
        id: u32,
        carrier: Arc<dyn Carrier>,
        runtime: &Handle,
        on_exit: OnExit,
    ) -> Arc<Self> {
        let (outgoing, queued) = mpsc::unbounded_channel();
        let writer = runtime.spawn(write_loop(queued, carrier)).abort_handle();
        Arc::new(Self {
            id,
            outgoing,
            state: Mutex::new(LinkState {
                streams: HashMap::new(),
                last_stream: 0,
                retired: false,
            }),
            writer,
            runtime: runtime.clone(),
            on_exit,
        })
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    fn state(&self) -> MutexGuard<'_, LinkState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Claims a stream id for an attachment that is about to be made. The webview
    /// chooses the id, so that it can be ready for frames before the attach returns;
    /// the id must be above every one it has used on this link.
    pub fn reserve(&self, stream: u32) -> Result<(), CommandError> {
        let mut state = self.state();
        if state.retired {
            return Err(CommandError::stale_carrier());
        }
        if stream <= state.last_stream {
            return Err(CommandError::invalid_stream(stream));
        }
        state.last_stream = stream;
        Ok(())
    }

    /// Puts an attachment on a reserved stream and starts forwarding its output.
    ///
    /// If the link was retired while the attachment was being made, the attachment is
    /// detached again, so the session is free for whoever asks next.
    pub async fn bind(
        &self,
        stream: u32,
        session: SessionKey,
        attachment: Attachment,
    ) -> Result<(), CommandError> {
        let Attachment {
            hydrate,
            output,
            input,
            ..
        } = attachment;
        {
            let mut state = self.state();
            if !state.retired {
                // The state to start from goes first, ahead of anything the pump sends.
                let start = match hydrate {
                    Hydrate::None => None,
                    Hydrate::Snapshot { vt_version, bytes } => {
                        Some(HydratePayload::Snapshot { vt_version, bytes })
                    }
                    Hydrate::VtReplay(bytes) => Some(HydratePayload::VtReplay(bytes)),
                };
                if let Some(start) = start {
                    let _ = self
                        .outgoing
                        .send(Message::new(stream, Body::Hydrate(start)));
                }
                let forwarding = pump(
                    stream,
                    session,
                    output,
                    self.outgoing.clone(),
                    Arc::clone(&self.on_exit),
                );
                let pump = self.runtime.spawn(forwarding).abort_handle();
                state.streams.insert(stream, Stream { input, pump });
                return Ok(());
            }
        }
        let _ = input.detach().await;
        Err(CommandError::stale_carrier())
    }

    /// Acts on the frames the webview sent, in order.
    ///
    /// `frames` must be whole frames of the kinds a view sends on a stream. Frames for
    /// a stream this link does not have are dropped: the webview may send one last
    /// frame to a stream that has just ended.
    pub async fn apply(&self, frames: &[u8]) -> Result<(), CommandError> {
        for message in decode_client_frames(frames)? {
            let Message { stream, body } = message;
            let Some(input) = self.state().streams.get(&stream).map(|s| s.input.clone()) else {
                continue;
            };
            let result = match body {
                Body::Input(bytes) if bytes.is_empty() => Ok(()),
                Body::Input(bytes) => input.write(&bytes).await,
                Body::Resize { size, cell_px } => input.resize(size, cell_px).await,
                Body::Facts(facts) => input.facts(&facts).await,
                Body::Credit(bytes) => input.credit(bytes).await,
                Body::Detach => {
                    self.close(stream).await;
                    Ok(())
                }
                // The decoder accepts nothing else from a view.
                _ => Ok(()),
            };
            match result {
                // Detached under us: the stream ended while this frame was on its way.
                Ok(()) | Err(SessionError::Detached) => {}
                Err(SessionError::CreditOverflow) => {
                    // A view that grants without bound has lost count; nothing it sends
                    // on this stream can be trusted to keep the flow bounded.
                    self.close(stream).await;
                    return Err(SessionError::CreditOverflow.into());
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    /// Ends one stream: its output stops and its session is free for another attachment.
    async fn close(&self, stream: u32) {
        let closed = self.state().streams.remove(&stream);
        if let Some(closed) = closed {
            closed.pump.abort();
            let _ = closed.input.detach().await;
        }
    }

    /// Ends the link: nothing more is delivered and every stream is detached.
    ///
    /// When this returns, each detach has been handed to its backend, so an attach
    /// made afterwards finds the sessions free.
    pub async fn retire(&self) {
        let streams = {
            let mut state = self.state();
            state.retired = true;
            std::mem::take(&mut state.streams)
        };
        self.writer.abort();
        for stream in streams.values() {
            stream.pump.abort();
        }
        for stream in streams.into_values() {
            let _ = stream.input.detach().await;
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.writer.abort();
        let state = self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        for stream in state.streams.values() {
            stream.pump.abort();
        }
    }
}

/// Reads `frames` as the whole frames of a view's streams.
fn decode_client_frames(frames: &[u8]) -> Result<Vec<Message>, CommandError> {
    // The decoder waits for more when a frame is cut short and for the rest of a split
    // message, and there is no more to come here. So the frames are measured first.
    let mut rest = frames;
    while !rest.is_empty() {
        let header = rest
            .first_chunk::<{ Header::LEN }>()
            .ok_or_else(|| CommandError::bad_request("a frame header is cut short"))
            .and_then(|bytes| {
                Header::parse(bytes).map_err(|error| CommandError::bad_request(error.to_string()))
            })?;
        if header.kind == Kind::Control {
            return Err(CommandError::bad_request(
                "control messages do not travel on the carrier",
            ));
        }
        rest = rest
            .get(Header::LEN + header.len as usize..)
            .ok_or_else(|| CommandError::bad_request("a frame is cut short"))?;
    }

    let mut decoder = FrameDecoder::new(Role::Host);
    decoder.push(frames);
    let mut messages = Vec::new();
    loop {
        match decoder.next() {
            Ok(Some(message)) => messages.push(message),
            Ok(None) => return Ok(messages),
            Err(error) => return Err(CommandError::bad_request(error.to_string())),
        }
    }
}

/// Forwards one attachment's frames to the link's writer under the webview's stream id.
async fn pump(
    stream: u32,
    session: SessionKey,
    mut output: lume_session::FrameStream,
    outgoing: mpsc::UnboundedSender<Message>,
    on_exit: OnExit,
) {
    let mut exited = false;
    while let Some(frame) = output.next().await {
        let body = match frame {
            OutputFrame::Data(bytes) => Body::Data(bytes),
            OutputFrame::Resized(size) => Body::Resized(size),
            OutputFrame::Event(event) => Body::Event(event),
            OutputFrame::Exit(status) => {
                exited = true;
                Body::Exit(status)
            }
        };
        if outgoing.send(Message::new(stream, body)).is_err() {
            return;
        }
    }
    // The attachment ended without the program's exit: the backend went away. The view
    // is told the session is over rather than left waiting for output for ever.
    if !exited {
        let _ = outgoing.send(Message::new(stream, Body::Exit(ExitStatus::Unknown)));
    }
    on_exit(&session);
}

/// Sends queued frames to the carrier, as many as are ready in each message.
///
/// This is the only sender on a carrier, which is what keeps frames in order.
async fn write_loop(mut queued: mpsc::UnboundedReceiver<Message>, carrier: Arc<dyn Carrier>) {
    let mut batch = Vec::new();
    let mut piece = Vec::new();
    while let Some(mut message) = queued.recv().await {
        loop {
            piece.clear();
            if let Err(error) = encode(&message, &mut piece) {
                tracing::error!(%error, stream = message.stream, "a frame could not be encoded");
            } else if !batch.is_empty() && batch.len() + piece.len() > CARRIER_BATCH_MAX {
                // A frame is never split across messages: this one starts the next.
                if carrier.deliver(std::mem::take(&mut batch)).is_err() {
                    return;
                }
                batch.extend_from_slice(&piece);
            } else {
                batch.extend_from_slice(&piece);
            }
            if batch.len() >= CARRIER_BATCH_MAX {
                break;
            }
            match queued.try_recv() {
                Ok(next) => message = next,
                Err(_) => break,
            }
        }
        if !batch.is_empty() && carrier.deliver(std::mem::take(&mut batch)).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use lume_proto::{CellPx, TermSize};

    use super::*;
    use crate::testing::RecordingCarrier;

    fn data(stream: u32, len: usize) -> Message {
        Message::new(stream, Body::Data(vec![b'x'; len]))
    }

    async fn written(messages: Vec<Message>) -> Arc<RecordingCarrier> {
        let carrier = RecordingCarrier::new();
        let (outgoing, queued) = mpsc::unbounded_channel();
        for message in messages {
            outgoing.send(message).unwrap();
        }
        drop(outgoing);
        write_loop(queued, carrier.clone()).await;
        carrier
    }

    #[tokio::test]
    async fn frames_that_are_ready_together_travel_in_one_message() {
        let size = TermSize::new(100, 30).unwrap();
        let messages = vec![
            data(1, 10),
            Message::new(1, Body::Resized(size)),
            data(2, 2000),
            Message::new(1, Body::Exit(ExitStatus::Code(0))),
        ];
        let carrier = written(messages.clone()).await;
        assert_eq!(carrier.batches().len(), 1);
        assert_eq!(carrier.messages(), messages);
    }

    #[tokio::test]
    async fn a_message_stays_within_the_limit_and_never_splits_a_frame() {
        let messages: Vec<Message> = (0..20).map(|_| data(1, 60_000)).collect();
        let carrier = written(messages.clone()).await;
        let batches = carrier.batches();
        assert!(batches.len() > 1);
        for batch in &batches {
            assert!(batch.len() <= CARRIER_BATCH_MAX, "{} bytes", batch.len());
            // Each message holds whole frames, so each decodes on its own.
            let mut decoder = FrameDecoder::new(Role::Client);
            decoder.push(batch);
            while decoder.next().unwrap().is_some() {}
        }
        assert_eq!(carrier.messages(), messages);
    }

    #[tokio::test]
    async fn writing_stops_when_the_carrier_is_closed() {
        let carrier = RecordingCarrier::new();
        carrier.close();
        let (outgoing, queued) = mpsc::unbounded_channel();
        outgoing.send(data(1, 1)).unwrap();
        // The sender is kept, so only the closed carrier can end the loop.
        write_loop(queued, carrier.clone()).await;
        assert!(carrier.batches().is_empty());
        drop(outgoing);
    }

    fn client_frames(messages: &[Message]) -> Vec<u8> {
        let mut out = Vec::new();
        for message in messages {
            encode(message, &mut out).unwrap();
        }
        out
    }

    #[test]
    fn whole_frames_from_a_view_are_read_in_order() {
        let size = TermSize::new(80, 24).unwrap();
        let messages = vec![
            Message::new(3, Body::Credit(1000)),
            Message::new(3, Body::Input(b"ls\r".to_vec())),
            Message::new(
                3,
                Body::Resize {
                    size,
                    cell_px: CellPx { w: 8, h: 16 },
                },
            ),
            Message::new(3, Body::Detach),
        ];
        assert_eq!(
            decode_client_frames(&client_frames(&messages)).unwrap(),
            messages
        );
        assert_eq!(decode_client_frames(&[]).unwrap(), vec![]);
    }

    #[test]
    fn a_frame_cut_short_is_refused() {
        let frames = client_frames(&[Message::new(3, Body::Input(b"hello".to_vec()))]);
        for cut in 1..frames.len() {
            let error = decode_client_frames(&frames[..cut]).unwrap_err();
            assert_eq!(error.code, "bad_request", "cut at {cut}");
        }
    }

    #[test]
    fn frames_a_view_may_not_send_are_refused() {
        // Output travels the other way.
        let mut from_host = Vec::new();
        encode(&data(3, 1), &mut from_host).unwrap();
        assert_eq!(
            decode_client_frames(&from_host).unwrap_err().code,
            "bad_request"
        );

        // Control messages belong to commands, whole or split.
        for kind in [Kind::Control as u8, Kind::Control as u8 | Kind::MORE] {
            let mut control = Vec::new();
            control.extend_from_slice(&0u32.to_le_bytes());
            control.push(kind);
            control.extend_from_slice(&2u32.to_le_bytes());
            control.extend_from_slice(b"{}");
            assert_eq!(
                decode_client_frames(&control).unwrap_err().code,
                "bad_request"
            );
        }
    }
}
