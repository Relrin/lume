//! Credit accounting for terminal output, with no I/O.
//!
//! A client grants the host credit in bytes, per stream. The host sends no more `Data`
//! than the credit it holds, and stops reading the terminal when it holds none, so a slow
//! client slows the program down instead of filling memory. Only `Data` is counted.
//!
//! [`SendWindow`] is the host's side. [`ReceiveWindow`] is what any receiver uses to check
//! the host kept to its grants. [`Granter`] adds the usual policy for the end consumer:
//! grant a window up front and top it up as bytes are consumed.

use crate::error::FlowError;
use crate::tuning::MAX_WINDOW;

/// The credit a sender holds for one stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SendWindow {
    available: u32,
}

impl SendWindow {
    /// A window with no credit: nothing may be sent until the peer grants some.
    pub const fn new() -> Self {
        Self { available: 0 }
    }

    pub fn available(&self) -> u32 {
        self.available
    }

    /// Adds credit the peer granted. Holding more than [`MAX_WINDOW`] is refused, because
    /// a peer that grants without limit has stopped providing any backpressure.
    pub fn grant(&mut self, bytes: u32) -> Result<(), FlowError> {
        match self.available.checked_add(bytes) {
            Some(total) if total <= MAX_WINDOW => {
                self.available = total;
                Ok(())
            }
            _ => Err(FlowError::WindowOverflow),
        }
    }

    /// Takes credit for up to `want` bytes and returns how many may be sent now.
    pub fn take(&mut self, want: usize) -> usize {
        let taken = want.min(self.available as usize);
        // `taken` is at most `available`, so it fits.
        self.available -= taken as u32;
        taken
    }
}

/// The credit a receiver has granted and not yet seen used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReceiveWindow {
    outstanding: u32,
}

impl ReceiveWindow {
    pub const fn new() -> Self {
        Self { outstanding: 0 }
    }

    /// Credit granted and not yet used by the sender.
    pub fn outstanding(&self) -> u32 {
        self.outstanding
    }

    /// Records a grant sent to the peer.
    pub fn granted(&mut self, bytes: u32) -> Result<(), FlowError> {
        match self.outstanding.checked_add(bytes) {
            Some(total) if total <= MAX_WINDOW => {
                self.outstanding = total;
                Ok(())
            }
            _ => Err(FlowError::WindowOverflow),
        }
    }

    /// Records `bytes` of `Data` received. An error means the sender exceeded its credit.
    pub fn received(&mut self, bytes: usize) -> Result<(), FlowError> {
        if bytes > self.outstanding as usize {
            return Err(FlowError::GrantExceeded {
                granted: self.outstanding,
                received: bytes,
            });
        }
        self.outstanding -= bytes as u32;
        Ok(())
    }
}

/// A receiver that keeps at most one window of output in flight or waiting to be consumed.
#[derive(Clone, Copy, Debug)]
pub struct Granter {
    window: u32,
    receive: ReceiveWindow,
    /// Bytes consumed since the last grant.
    consumed: u32,
}

impl Granter {
    /// Starts a stream with `window` bytes of credit. The second value is the first grant
    /// to send. `window` must be between 1 and [`MAX_WINDOW`].
    pub fn new(window: u32) -> (Self, u32) {
        assert!(
            (1..=MAX_WINDOW).contains(&window),
            "a credit window of {window} bytes"
        );
        let granter = Self {
            window,
            receive: ReceiveWindow {
                outstanding: window,
            },
            consumed: 0,
        };
        (granter, window)
    }

    /// Credit granted and not yet used by the sender.
    pub fn outstanding(&self) -> u32 {
        self.receive.outstanding()
    }

    /// Records `bytes` of `Data` received. An error means the sender exceeded its credit.
    pub fn on_received(&mut self, bytes: usize) -> Result<(), FlowError> {
        self.receive.received(bytes)
    }

    /// Records that `bytes` of received output have been processed, and returns a grant
    /// to send once half a window has been freed. Granting in halves keeps the sender
    /// supplied without a `Credit` frame for every frame of output.
    pub fn on_consumed(&mut self, bytes: usize) -> Option<u32> {
        // Only bytes that were received can be consumed, and a window of them at most.
        let bytes = u32::try_from(bytes).unwrap_or(u32::MAX);
        self.consumed = self.consumed.saturating_add(bytes).min(self.window);
        if self.consumed < self.window.div_ceil(2) {
            return None;
        }
        let grant = std::mem::take(&mut self.consumed);
        self.receive.outstanding += grant;
        Some(grant)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_send_window_lets_nothing_through() {
        let mut window = SendWindow::new();
        assert_eq!(window.take(100), 0);
        window.grant(10).unwrap();
        assert_eq!(window.take(100), 10);
        assert_eq!(window.take(100), 0);
    }

    #[test]
    fn a_send_window_refuses_credit_beyond_the_maximum() {
        let mut window = SendWindow::new();
        window.grant(MAX_WINDOW).unwrap();
        assert_eq!(window.grant(1), Err(FlowError::WindowOverflow));
        assert_eq!(window.available(), MAX_WINDOW);
        assert_eq!(
            SendWindow::new().grant(u32::MAX),
            Err(FlowError::WindowOverflow)
        );
    }

    #[test]
    fn a_receive_window_reports_a_sender_that_exceeds_its_credit() {
        let mut window = ReceiveWindow::new();
        window.granted(10).unwrap();
        window.received(4).unwrap();
        assert_eq!(
            window.received(7),
            Err(FlowError::GrantExceeded {
                granted: 6,
                received: 7
            })
        );
    }

    #[test]
    fn a_granter_tops_up_once_half_a_window_is_consumed() {
        let (mut granter, first) = Granter::new(100);
        assert_eq!(first, 100);
        granter.on_received(60).unwrap();
        assert_eq!(granter.on_consumed(49), None);
        assert_eq!(granter.on_consumed(1), Some(50));
        assert_eq!(granter.outstanding(), 90);
        assert_eq!(granter.on_consumed(10), None);
    }
}
