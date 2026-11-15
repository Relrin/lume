//! The application's tuning values. Each is defined here and nowhere else.

use std::time::Duration;

/// How long a session waits for a new page after the page showing it went away.
///
/// A reload replaces the page, and the new one asks for its sessions again. Sessions
/// nobody asks for within this time belonged to a page that is not coming back, and are
/// ended.
pub const REATTACH_GRACE: Duration = Duration::from_secs(5);

/// The most bytes handed to the webview in one message.
///
/// Frames that are ready together travel together, which saves a round trip into the
/// webview for each. The limit keeps one message from holding the webview's thread for
/// long, and bounds what is in flight beyond a stream's credit.
pub const CARRIER_BATCH_MAX: usize = 256 * 1024;

/// The longest link the core will open. Longer ones are not addresses anyone typed.
pub const LINK_MAX_BYTES: usize = 2048;

/// How long a request on the control endpoint waits for a page that has not connected
/// or has not shown its session yet. A request sent as soon as the application starts
/// arrives before the page has loaded.
pub const PAGE_START_TIMEOUT: Duration = Duration::from_secs(15);

/// How long a page may take to answer a request about what it shows.
pub const PAGE_ANSWER_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a page may take to start a session and show it. Starting a shell is the
/// slow part, and a slow one must not look like a page that has stopped answering.
pub const PAGE_OPEN_TIMEOUT: Duration = Duration::from_secs(30);

/// How much longer than a wait's own limit the core waits for the page to say the wait
/// has run out, before it says so itself.
pub const WAIT_SLACK: Duration = Duration::from_secs(2);

/// The longest a request on the control endpoint may wait, whatever it asks for.
pub const WAIT_MAX: Duration = Duration::from_secs(60 * 60);

/// How often a wait for a session to end looks at the session list, for the sessions
/// whose exit nothing reports because no page is attached to them.
pub const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// How many sessions' exit statuses are remembered after the sessions are gone.
pub const EXITS_KEPT: usize = 256;

/// How large the log file may grow before it is set aside and begun again. One file is
/// kept aside, so the log takes at most twice this.
pub const LOG_FILE_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// How long a test suite may run in the webview before the run is given up as silent.
/// The slowest suite times many thousands of frames and several floods.
pub const HARNESS_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// The largest file a test suite may write: a picture of a full window with room to spare.
pub const HARNESS_FILE_MAX_BYTES: usize = 32 * 1024 * 1024;

/// The name of the backend whose terminals run the built-in programs of a benchmark.
pub const BENCH_BACKEND: &str = "bench";

/// The longest title the window takes from a page. The page cuts titles to the same
/// length; this holds whatever the page does.
pub const TITLE_MAX_BYTES: usize = 1024;
