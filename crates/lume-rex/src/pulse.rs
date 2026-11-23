//! Noticing that a server has gone without a word.
//!
//! A connection over a network can stop carrying anything without either end being
//! told. A server pings its clients to notice; a client that pings too notices when it
//! is the server that is gone. While anything arrives the server is there, so a ping is
//! sent only after a silence, and the connection is given up when that ping brings
//! nothing back either.

use tokio::time::Instant;

use crate::tuning::{PING_EVERY, PING_WAIT};

/// What a connection does when its pulse says it is time.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Beat {
    /// Send a ping.
    Ping,
    /// Nothing came back: the server is gone.
    Lost,
}

/// When a connection last heard from its server, and what it does about a silence.
pub(crate) struct Pulse {
    /// When something is to be done, if nothing arrives before.
    due: Instant,
    /// A ping was sent and nothing has arrived since.
    asked: bool,
}

impl Pulse {
    pub(crate) fn new() -> Self {
        Self {
            due: Instant::now() + PING_EVERY,
            asked: false,
        }
    }

    /// Something arrived, whatever it was.
    pub(crate) fn heard(&mut self) {
        self.due = Instant::now() + PING_EVERY;
        self.asked = false;
    }

    /// Waits until it is time to do something about a silence. May be dropped and
    /// called again at any point.
    pub(crate) async fn due(&self) {
        tokio::time::sleep_until(self.due).await;
    }

    /// What to do now that it is time.
    pub(crate) fn beat(&mut self) -> Beat {
        if self.asked {
            return Beat::Lost;
        }
        self.asked = true;
        self.due = Instant::now() + PING_WAIT;
        Beat::Ping
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_silence_is_asked_about_once_and_then_given_up_on() {
        let start = Instant::now();
        let mut pulse = Pulse::new();
        pulse.due().await;
        assert_eq!(start.elapsed(), PING_EVERY);
        assert_eq!(pulse.beat(), Beat::Ping);
        pulse.due().await;
        assert_eq!(start.elapsed(), PING_EVERY + PING_WAIT);
        assert_eq!(pulse.beat(), Beat::Lost);
    }

    #[tokio::test(start_paused = true)]
    async fn whatever_arrives_puts_the_question_off() {
        let start = Instant::now();
        let mut pulse = Pulse::new();
        pulse.due().await;
        assert_eq!(pulse.beat(), Beat::Ping);
        tokio::time::advance(PING_WAIT / 2).await;
        pulse.heard();
        pulse.due().await;
        assert_eq!(start.elapsed(), PING_EVERY + PING_WAIT / 2 + PING_EVERY);
        assert_eq!(pulse.beat(), Beat::Ping);
    }
}
