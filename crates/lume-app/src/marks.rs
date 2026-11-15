//! How far the application's start has come.
//!
//! Each point on the way from the start of the process to a usable terminal is noted the
//! first time it is reached. The control endpoint reports them, which is how a slow start
//! is told apart from a slow shell.

use std::sync::OnceLock;
use std::time::Instant;

use lume_proto::app::StartMarks;

/// The points of the start that have been reached, and when.
pub struct Marks {
    started: Instant,
    window: OnceLock<u64>,
    page: OnceLock<u64>,
    carrier: OnceLock<u64>,
    attached: OnceLock<u64>,
    output: OnceLock<u64>,
}

impl Marks {
    /// Starts counting. Called as early in the process as possible.
    pub fn start() -> Self {
        Self {
            started: Instant::now(),
            window: OnceLock::new(),
            page: OnceLock::new(),
            carrier: OnceLock::new(),
            attached: OnceLock::new(),
            output: OnceLock::new(),
        }
    }

    fn note(&self, mark: &OnceLock<u64>) {
        mark.get_or_init(|| u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX));
    }

    /// The window was opened.
    pub fn window(&self) {
        self.note(&self.window);
    }

    /// A page finished loading.
    pub fn page(&self) {
        self.note(&self.page);
    }

    /// A page opened its carrier.
    pub fn carrier(&self) {
        self.note(&self.carrier);
    }

    /// A page attached to a session.
    pub fn attached(&self) {
        self.note(&self.attached);
    }

    /// Output of a session was handed to a page. Called for every piece of output, so it
    /// costs one load once the mark is set.
    pub fn output(&self) {
        self.note(&self.output);
    }

    /// The marks as the control endpoint reports them.
    pub fn read(&self) -> StartMarks {
        StartMarks {
            window_ms: self.window.get().copied(),
            page_ms: self.page.get().copied(),
            carrier_ms: self.carrier.get().copied(),
            attached_ms: self.attached.get().copied(),
            output_ms: self.output.get().copied(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mark_keeps_the_first_time_it_was_reached() {
        let marks = Marks::start();
        assert_eq!(marks.read(), StartMarks::default());

        marks.page();
        let first = marks.read().page_ms.expect("the page is marked");
        std::thread::sleep(std::time::Duration::from_millis(5));
        marks.page();
        assert_eq!(marks.read().page_ms, Some(first));
        assert_eq!(marks.read().output_ms, None);
    }
}
