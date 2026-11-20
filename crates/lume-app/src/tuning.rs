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

/// The most one read of a plugin's stream gives a view: what one frame carries.
pub const PIPE_READ_MAX: usize = 64 * 1024;

/// How much a view may have written to a plugin's stream that the stream has not
/// taken yet. A view that writes past it loses the stream.
pub const PIPE_WRITE_QUEUE_MAX: usize = 4 * 1024 * 1024;

/// The longest link the core will open. Longer ones are not addresses anyone typed.
pub const LINK_MAX_BYTES: usize = 2048;

/// How long a request on the control endpoint waits for a page that has not connected
/// or has not shown its session yet. A request sent as soon as the application starts
/// arrives before the page has loaded.
pub const PAGE_START_TIMEOUT: Duration = Duration::from_secs(15);

/// How long a pane that names a profile waits for the machine to have been looked at,
/// when the profile is not one the user wrote out. Looking asks another program which
/// Linux distributions there are, and that program has a time limit of its own.
pub const PROFILE_DETECT_WAIT: Duration = Duration::from_secs(6);

/// How long the OpenSSH configuration must stay quiet after a change before its hosts
/// are read again. An editor saves a file in several steps.
pub const PROFILE_REDETECT_DELAY: Duration = Duration::from_millis(300);

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

/// How long a workspace waits after a change before it is written to its file. Changes
/// come in bursts: a divider being dragged, a pane closing and the focus moving.
pub const SAVE_DEBOUNCE: Duration = Duration::from_secs(1);

/// The longest a changed workspace waits to be written, however many more changes
/// follow.
pub const SAVE_MAX_DELAY: Duration = Duration::from_secs(5);

/// How long a page may take to say how large a pane that is about to be made will be.
/// It measures nothing new to answer; a page that takes longer is busy, and the session
/// is started at a size that is put right when the pane is drawn.
pub const ESTIMATE_TIMEOUT: Duration = Duration::from_millis(500);

/// The cell size a terminal outside the application is taken to have, when the
/// command-line tool shows a session in one: it cannot tell, and programs seldom ask.
pub const ATTACH_CELL_PX: lume_proto::CellPx = lume_proto::CellPx { w: 9, h: 18 };

/// The grid a session is started with when nobody could say how large its pane is.
pub const DEFAULT_SPAWN_COLS: u16 = 80;
pub const DEFAULT_SPAWN_ROWS: u16 = 24;

/// The workspace the application opens.
pub const WORKSPACE_NAME: &str = "default";

/// How many notices wait for a page that has not connected yet. More than a start can
/// have reason to say.
pub const PENDING_NOTICES_MAX: usize = 16;

/// How long a window that is being moved or resized is left alone before where it is
/// now is kept with the layout.
pub const BOUNDS_DEBOUNCE: Duration = Duration::from_millis(500);

/// How much of a window must be on a screen, along each side, for the window to be
/// left where it was kept: enough to see it and take hold of it. In the pixels a
/// window's size is counted in.
pub const BOUNDS_MIN_VISIBLE: u32 = 64;

/// The smallest a window may be. The same as the window's description says, which
/// holds while the window is open; this holds for a size read from a file.
pub const WINDOW_MIN_WIDTH: u32 = 480;
pub const WINDOW_MIN_HEIGHT: u32 = 320;

/// The file, in the folder of the application's state, that says which workspace was
/// last on show.
pub const APP_FILE: &str = "app.json";

/// The id of the panel that lists a window's tabs.
pub const TAB_LIST_PANEL: &str = "lume.tabs";

/// How large the settings window opens, and the smallest it may be made: width and
/// height, in the pixels a window's size is counted in.
pub const SETTINGS_WINDOW_SIZE: (u32, u32) = (980, 680);
pub const SETTINGS_WINDOW_MIN_SIZE: (u32, u32) = (720, 480);

/// The longest name a section of the settings may have, in bytes: room for a plugin's
/// page, `addon:` and its id.
pub const SETTINGS_SECTION_MAX_BYTES: usize = 96;

/// The largest picture shown behind the panes. A larger file is refused, not read: the
/// whole of it is handed to the page.
pub const BACKGROUND_IMAGE_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// How many reports of sessions may wait for a client that follows them and is slow to
/// take them. One that falls further behind is told the sessions' whole state instead
/// of what it missed.
pub const SESSION_NEWS_BACKLOG: usize = 1024;

/// The folder, in the folder of the application's state, that the scripts are written
/// to with which shells are asked to report their directory and their commands.
pub const SHELL_INTEGRATION_DIR: &str = "shell-integration";

/// How long a window that asked for attention because of a bell waits before a bell
/// has it ask again. A program that rings on and on does not keep the taskbar busy.
pub const BELL_ATTENTION_INTERVAL: Duration = Duration::from_secs(5);

/// How long after a notification that a command ended the next may be shown. Commands
/// that end together, in several panes, are not a stream of notifications.
pub const NOTIFY_INTERVAL: Duration = Duration::from_secs(5);

/// How many processes under a shell's own name are taken for the shell itself at its
/// prompt: the one the session started, and a child it starts as the real shell.
pub const SHELL_ALIASES_MAX: usize = 2;

/// The size, in logical pixels, of a window that has never been opened, as the
/// application's configuration gives it: what a shell started before its window's page
/// is there is sized for.
pub const PLANNED_WINDOW_WIDTH_PX: f64 = 960.0;
pub const PLANNED_WINDOW_HEIGHT_PX: f64 = 640.0;

/// What of a window's width and height is not its panes' terminals: the bars, the
/// panes' frames and their padding, as the standard look has them. Only an estimate:
/// a terminal is resized to its real grid when its page attaches.
pub const PLANNED_CHROME_WIDTH_PX: f64 = 24.0;
pub const PLANNED_CHROME_HEIGHT_PX: f64 = 72.0;

/// The width of a cell of a monospaced font as a share of its size: what the bundled
/// terminal font and most others have.
pub const PLANNED_CELL_WIDTH_EM: f64 = 0.6;
