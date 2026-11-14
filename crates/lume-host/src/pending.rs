//! What a session has read from its terminal and not yet sent, in order.
//!
//! Besides output bytes the queue holds the two markers that must reach a client at an
//! exact place in the output: the terminal was resized here, and the program ended
//! here. A marker never overtakes the bytes in front of it, and bytes are only released
//! against credit, so a marker behind unsent bytes waits with them.
//!
//! The queue also decides when waiting output should be sent; see [`Pending::push_bytes`].

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use lume_proto::tuning::MAX_PAYLOAD;
use lume_proto::{ExitStatus, TermSize};

use crate::tuning::{COALESCE_MAX_BYTES, COALESCE_MAX_DELAY, ECHO_IMMEDIATE_WINDOW};

/// One thing to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Segment {
    /// Output, at most one frame's worth.
    Bytes(Vec<u8>),
    Resized(TermSize),
    Exit(ExitStatus),
}

/// When output that was just queued should be sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Flush {
    Now,
    /// No later than this.
    At(Instant),
}

#[derive(Debug)]
enum Entry {
    /// A chunk as it was read, and how much of its front has already been sent.
    Bytes {
        data: Vec<u8>,
        sent: usize,
    },
    Resized(TermSize),
    Exit(ExitStatus),
}

#[derive(Debug, Default)]
pub(crate) struct Pending {
    entries: VecDeque<Entry>,
    /// Output bytes queued and not yet popped.
    bytes: usize,
    /// When the oldest unsent output was queued. Its wait is what the delay limits.
    oldest: Option<Instant>,
}

impl Pending {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Output bytes waiting.
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Queues output read at `now` and says when to send it.
    ///
    /// Output goes at once if a frame's worth is waiting, or if the user typed something
    /// within [`ECHO_IMMEDIATE_WINDOW`], since it is then probably the echo. Otherwise it
    /// waits for more to arrive, but never longer than [`COALESCE_MAX_DELAY`] from when
    /// the oldest waiting byte was read; later output does not extend that wait.
    pub(crate) fn push_bytes(
        &mut self,
        data: Vec<u8>,
        now: Instant,
        last_input: Option<Instant>,
    ) -> Flush {
        if data.is_empty() && self.bytes == 0 {
            // Nothing is waiting, so there is no wait to bound.
            return Flush::At(now + COALESCE_MAX_DELAY);
        }
        if !data.is_empty() {
            self.bytes += data.len();
            self.entries.push_back(Entry::Bytes { data, sent: 0 });
        }
        let oldest = *self.oldest.get_or_insert(now);

        let echo = last_input.is_some_and(|at| elapsed(at, now) <= ECHO_IMMEDIATE_WINDOW);
        if self.bytes >= COALESCE_MAX_BYTES || echo {
            Flush::Now
        } else {
            Flush::At(oldest + COALESCE_MAX_DELAY)
        }
    }

    /// Marks the place where the terminal was resized. Several resizes with no output
    /// between them leave one marker, the last: only the final size matters to a client.
    pub(crate) fn push_resized(&mut self, size: TermSize) {
        if let Some(Entry::Resized(last)) = self.entries.back_mut() {
            *last = size;
        } else {
            self.entries.push_back(Entry::Resized(size));
        }
    }

    /// Marks the place where the program ended. Nothing is queued after it.
    pub(crate) fn push_exit(&mut self, status: ExitStatus) {
        self.entries.push_back(Entry::Exit(status));
    }

    /// Drops the output bytes and keeps the markers, in order.
    pub(crate) fn discard_bytes(&mut self) {
        self.entries
            .retain(|entry| !matches!(entry, Entry::Bytes { .. }));
        self.bytes = 0;
        self.oldest = None;
    }

    /// Takes the next thing to send. Output is released only against `credit`, at most
    /// one frame at a time; markers need no credit. `None` means there is nothing, or
    /// the next thing is output and there is no credit for it.
    pub(crate) fn pop(&mut self, credit: usize) -> Option<Segment> {
        let segment = match self.entries.front_mut()? {
            Entry::Bytes { data, sent } => {
                let take = (data.len() - *sent).min(credit).min(MAX_PAYLOAD);
                if take == 0 {
                    return None;
                }
                let piece = if *sent == 0 && take == data.len() {
                    // The whole chunk goes: hand over the buffer instead of copying it.
                    *sent = take;
                    std::mem::take(data)
                } else {
                    let piece = data[*sent..*sent + take].to_vec();
                    *sent += take;
                    piece
                };
                self.bytes -= take;
                let finished = data.is_empty() || *sent == data.len();
                if finished {
                    self.entries.pop_front();
                }
                Segment::Bytes(piece)
            }
            Entry::Resized(size) => {
                let size = *size;
                self.entries.pop_front();
                Segment::Resized(size)
            }
            Entry::Exit(status) => {
                let status = *status;
                self.entries.pop_front();
                Segment::Exit(status)
            }
        };
        if self.bytes == 0 {
            self.oldest = None;
        }
        Some(segment)
    }
}

/// The time from `earlier` to `later`, or zero if they are the other way round.
fn elapsed(earlier: Instant, later: Instant) -> Duration {
    later.checked_duration_since(earlier).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(cols: u16, rows: u16) -> TermSize {
        TermSize::new(cols, rows).unwrap()
    }

    const PLENTY: usize = usize::MAX;

    #[test]
    fn output_waits_for_more_but_not_past_the_delay_from_its_oldest_byte() {
        let mut pending = Pending::new();
        let start = Instant::now();
        assert_eq!(
            pending.push_bytes(vec![1; 10], start, None),
            Flush::At(start + COALESCE_MAX_DELAY)
        );

        // More output a moment later does not move the deadline.
        let later = start + Duration::from_millis(3);
        assert_eq!(
            pending.push_bytes(vec![2; 10], later, None),
            Flush::At(start + COALESCE_MAX_DELAY)
        );

        // Once everything has gone, the next output starts a new wait.
        while pending.pop(PLENTY).is_some() {}
        let next = start + Duration::from_millis(50);
        assert_eq!(
            pending.push_bytes(vec![3; 10], next, None),
            Flush::At(next + COALESCE_MAX_DELAY)
        );
    }

    #[test]
    fn a_frames_worth_of_output_goes_at_once() {
        let mut pending = Pending::new();
        let now = Instant::now();
        assert!(matches!(
            pending.push_bytes(vec![0; COALESCE_MAX_BYTES - 1], now, None),
            Flush::At(_)
        ));
        assert_eq!(pending.push_bytes(vec![0; 1], now, None), Flush::Now);
    }

    #[test]
    fn output_just_after_input_goes_at_once() {
        let now = Instant::now();
        let typed = |ago: Duration| now.checked_sub(ago);

        let mut pending = Pending::new();
        assert_eq!(
            pending.push_bytes(vec![b'a'], now, typed(Duration::from_millis(5))),
            Flush::Now
        );

        let mut pending = Pending::new();
        assert_eq!(
            pending.push_bytes(vec![b'a'], now, typed(ECHO_IMMEDIATE_WINDOW)),
            Flush::Now
        );

        let mut pending = Pending::new();
        let long_ago = typed(ECHO_IMMEDIATE_WINDOW + Duration::from_millis(1));
        assert!(matches!(
            pending.push_bytes(vec![b'a'], now, long_ago),
            Flush::At(_)
        ));
    }

    #[test]
    fn output_is_released_only_against_credit_and_a_frame_at_a_time() {
        let mut pending = Pending::new();
        let now = Instant::now();
        pending.push_bytes(vec![7; MAX_PAYLOAD + 100], now, None);
        assert_eq!(pending.bytes(), MAX_PAYLOAD + 100);

        assert_eq!(pending.pop(0), None);
        assert_eq!(pending.pop(10), Some(Segment::Bytes(vec![7; 10])));
        assert_eq!(
            pending.pop(PLENTY),
            Some(Segment::Bytes(vec![7; MAX_PAYLOAD]))
        );
        assert_eq!(pending.pop(PLENTY), Some(Segment::Bytes(vec![7; 90])));
        assert_eq!(pending.pop(PLENTY), None);
        assert_eq!(pending.bytes(), 0);
        assert!(pending.is_empty());
    }

    #[test]
    fn markers_keep_their_place_and_need_no_credit() {
        let mut pending = Pending::new();
        let now = Instant::now();
        pending.push_bytes(b"before".to_vec(), now, None);
        pending.push_resized(size(100, 30));
        pending.push_bytes(b"after".to_vec(), now, None);
        pending.push_exit(ExitStatus::Code(0));

        // Without credit the marker cannot overtake the output in front of it.
        assert_eq!(pending.pop(0), None);
        assert_eq!(
            pending.pop(PLENTY),
            Some(Segment::Bytes(b"before".to_vec()))
        );
        assert_eq!(pending.pop(0), Some(Segment::Resized(size(100, 30))));
        assert_eq!(pending.pop(0), None);
        assert_eq!(pending.pop(PLENTY), Some(Segment::Bytes(b"after".to_vec())));
        assert_eq!(pending.pop(0), Some(Segment::Exit(ExitStatus::Code(0))));
        assert_eq!(pending.pop(PLENTY), None);
    }

    #[test]
    fn resizes_with_no_output_between_them_leave_one_marker() {
        let mut pending = Pending::new();
        pending.push_resized(size(90, 30));
        pending.push_resized(size(100, 40));
        pending.push_bytes(b"x".to_vec(), Instant::now(), None);
        pending.push_resized(size(110, 50));

        assert_eq!(pending.pop(PLENTY), Some(Segment::Resized(size(100, 40))));
        assert_eq!(pending.pop(PLENTY), Some(Segment::Bytes(b"x".to_vec())));
        assert_eq!(pending.pop(PLENTY), Some(Segment::Resized(size(110, 50))));
        assert_eq!(pending.pop(PLENTY), None);
    }

    #[test]
    fn discarding_output_keeps_the_markers() {
        let mut pending = Pending::new();
        let now = Instant::now();
        pending.push_bytes(b"lost".to_vec(), now, None);
        pending.push_resized(size(100, 30));
        pending.push_bytes(b"lost too".to_vec(), now, None);
        pending.push_exit(ExitStatus::Signal(9));

        pending.discard_bytes();
        assert_eq!(pending.bytes(), 0);
        assert_eq!(pending.pop(0), Some(Segment::Resized(size(100, 30))));
        assert_eq!(pending.pop(0), Some(Segment::Exit(ExitStatus::Signal(9))));
        assert!(pending.is_empty());
    }
}
