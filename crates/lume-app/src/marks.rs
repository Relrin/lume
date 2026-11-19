//! How far the application's start has come.
//!
//! Each point on the way from the start of the process to a usable terminal is noted the
//! first time it is reached. The control endpoint reports them, which is how a slow start
//! is told apart from a slow shell.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Instant;

use lume_proto::app::StartMarks;

/// Steps of a page's start: milliseconds since the page began as the page reports them,
/// or on this process's clock once placed.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageSteps<T = f64> {
    /// The page's code was read and began to run.
    pub parsed: Option<T>,
    /// The terminal emulation was ready.
    pub vt: Option<T>,
    /// The graphics card answered: terminals may draw with it.
    pub gpu: Option<T>,
    /// The terminal's fonts were loaded.
    pub fonts: Option<T>,
    /// The panes were laid out.
    pub layout: Option<T>,
    /// The first frame of a terminal was drawn, with or without output.
    pub frame: Option<T>,
    /// The first output of a session reached the page.
    pub output: Option<T>,
    /// The first output of a session was drawn.
    pub drawn: Option<T>,
}

/// The points of the start that have been reached, and when.
pub struct Marks {
    started: Instant,
    config: OnceLock<u64>,
    host: OnceLock<u64>,
    panes: OnceLock<u64>,
    setup: OnceLock<u64>,
    window: OnceLock<u64>,
    spawned: OnceLock<u64>,
    /// What the first page said of its own start, already on this clock.
    page_steps: OnceLock<PageSteps<u64>>,
    /// When a window that was kept ready was last shown, and when its page drew its
    /// first frame after that, in milliseconds since the Unix epoch.
    shown: Mutex<(Option<u64>, Option<u64>)>,
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
            config: OnceLock::new(),
            host: OnceLock::new(),
            panes: OnceLock::new(),
            setup: OnceLock::new(),
            window: OnceLock::new(),
            spawned: OnceLock::new(),
            page_steps: OnceLock::new(),
            shown: Mutex::new((None, None)),
            page: OnceLock::new(),
            carrier: OnceLock::new(),
            attached: OnceLock::new(),
            output: OnceLock::new(),
            plugins: OnceLock::new(),
            plugin_files_early: AtomicU32::new(0),
        }
    }

    fn now(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn note(&self, mark: &OnceLock<u64>) {
        mark.get_or_init(|| self.now());
    }

    /// The user's configuration was read.
    pub fn config(&self) {
        self.note(&self.config);
    }

    /// The host of this machine's sessions was ready.
    pub fn host(&self) {
        self.note(&self.host);
    }

    /// The workspace to show was read and put in the application's charge.
    pub fn panes(&self) {
        self.note(&self.panes);
    }

    /// The application was built and began to set itself up.
    pub fn setup(&self) {
        self.note(&self.setup);
    }

    /// The first shell was started.
    pub fn spawned(&self) {
        self.note(&self.spawned);
    }

    /// A window that was kept ready was shown.
    pub fn shown(&self) {
        *self.shown.lock().unwrap_or_else(PoisonError::into_inner) =
            (Some(crate::panes::now_unix_ms()), None);
    }

    /// The page of the window last shown drew its first frame since.
    pub fn shown_drawn(&self) {
        let mut shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner);
        if shown.0.is_some() && shown.1.is_none() {
            shown.1 = Some(crate::panes::now_unix_ms());
        }
    }

    /// What the first page to tell of its start said: each step in milliseconds since
    /// the page began, with `now` the page's clock when it told. Each step is put on
    /// this process's clock by how long before `now` it was reached.
    pub fn page_steps(&self, steps: &PageSteps, now: f64) {
        let received = self.now() as f64;
        let place = |at: Option<f64>| {
            at.filter(|at| at.is_finite() && *at >= 0.0 && *at <= now)
                .map(|at| (received - (now - at)).max(0.0).round() as u64)
        };
        self.page_steps.get_or_init(|| PageSteps {
            parsed: place(steps.parsed),
            vt: place(steps.vt),
            gpu: place(steps.gpu),
            fonts: place(steps.fonts),
            layout: place(steps.layout),
            frame: place(steps.frame),
            output: place(steps.output),
            drawn: place(steps.drawn),
        });
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
        let page = self.page_steps.get().copied().unwrap_or_default();
        let (shown_unix_ms, shown_drawn_unix_ms) =
            *self.shown.lock().unwrap_or_else(PoisonError::into_inner);
        StartMarks {
            shown_unix_ms,
            shown_drawn_unix_ms,
            config_ms: self.config.get().copied(),
            host_ms: self.host.get().copied(),
            panes_ms: self.panes.get().copied(),
            setup_ms: self.setup.get().copied(),
            spawned_ms: self.spawned.get().copied(),
            parsed_ms: page.parsed,
            vt_ms: page.vt,
            gpu_ms: page.gpu,
            fonts_ms: page.fonts,
            layout_ms: page.layout,
            frame_ms: page.frame,
            page_output_ms: page.output,
            drawn_ms: page.drawn,
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

    #[test]
    fn a_pages_steps_are_put_on_the_processs_clock() {
        let marks = Marks::start();
        std::thread::sleep(std::time::Duration::from_millis(30));
        let steps = PageSteps {
            parsed: Some(5.0),
            drawn: Some(20.0),
            fonts: Some(f64::NAN),
            layout: Some(500.0),
            ..PageSteps::default()
        };
        marks.page_steps(&steps, 25.0);
        let read = marks.read();
        let drawn = read.drawn_ms.expect("drawn is placed");
        let parsed = read.parsed_ms.expect("parsed is placed");
        assert_eq!(drawn - parsed, 15);
        assert!(drawn >= 25, "{drawn}");
        assert_eq!(
            read.fonts_ms, None,
            "a step that is not a number is left out"
        );
        assert_eq!(
            read.layout_ms, None,
            "a step after the page's now is left out"
        );
        // Only the first page's account is kept.
        marks.page_steps(&PageSteps::default(), 0.0);
        assert_eq!(marks.read().drawn_ms, Some(drawn));
    }
}
