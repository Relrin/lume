//! How far the application's start has come.
//!
//! Each point on the way from the start of the process to a usable terminal is noted the
//! first time it is reached. The control endpoint reports them, which is how a slow start
//! is told apart from a slow shell.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
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
    plugins: OnceLock<u64>,
    /// Files of plugins read before the first output: none should be, so that the
    /// number of plugins installed never slows the start.
    plugin_files_early: AtomicU32,
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
            plugins: OnceLock::new(),
            plugin_files_early: AtomicU32::new(0),
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

    /// Whether a session's output has reached a page yet.
    pub fn has_output(&self) -> bool {
        self.output.get().is_some()
    }

    /// The installed plugins were read and those that start at once were started.
    pub fn plugins(&self) {
        self.note(&self.plugins);
    }

    /// A file of a plugin was read. Counted when it is before the first output.
    pub fn plugin_file(&self) {
        if !self.has_output() {
            self.plugin_files_early.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The marks as the control endpoint reports them.
    pub fn read(&self) -> StartMarks {
        StartMarks {
            window_ms: self.window.get().copied(),
            page_ms: self.page.get().copied(),
            carrier_ms: self.carrier.get().copied(),
            attached_ms: self.attached.get().copied(),
            output_ms: self.output.get().copied(),
            plugins_ms: self.plugins.get().copied(),
            plugin_files_before_output: self.plugin_files_early.load(Ordering::Relaxed),
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
