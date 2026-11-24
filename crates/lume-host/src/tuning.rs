//! Timings and sizes of the host. Each value is defined here and nowhere else.

use std::time::Duration;

/// The longest terminal output waits in the host before it is sent. Programs write a
/// screen update in many small pieces; sending each piece alone would cost a frame and
/// a wake-up apiece, so pieces that arrive within this time travel together.
pub const COALESCE_MAX_DELAY: Duration = Duration::from_millis(4);

/// Output waiting to be sent is sent at once when it reaches this size, however little
/// time has passed. Also the most the host holds for a client that has attached and is
/// not taking output: beyond it the host stops reading the terminal.
pub const COALESCE_MAX_BYTES: usize = 64 * 1024;

/// The most output the host keeps for one of several clients of a session while it takes
/// that output more slowly than another. Beyond it that client is dropped behind: its
/// waiting output is thrown away, and once it takes output again it starts afresh from a
/// copy of the screen. One slow client does not slow the others down.
pub const ATTACHMENT_QUEUE_MAX: usize = 1024 * 1024;

/// The most clients one session shows its output to at once.
pub const ATTACHMENTS_PER_SESSION_MAX: usize = 16;

/// Output read this soon after input is sent without waiting. It is most likely the
/// echo of what the user typed, and typing must not feel delayed.
pub const ECHO_IMMEDIATE_WINDOW: Duration = Duration::from_millis(20);

/// How many pieces of work may wait for a session's screen. When it is behind, the
/// session waits, and so stops reading its terminal: the program is slowed rather than
/// its output held.
pub const SCREEN_QUEUE: usize = 8;

/// How many lines of scrollback a session's screen keeps when the client that started it
/// did not say.
pub const SCROLLBACK_DEFAULT_LINES: usize = 10_000;

/// The most lines of scrollback a session's screen keeps, whatever was asked for.
pub const SCROLLBACK_MAX_LINES: usize = 100_000;

/// The most bytes of scrollback a session's screen keeps. The same bound a view keeps.
pub const SCROLLBACK_MAX_BYTES: usize = 192 * 1024 * 1024;

/// How long a screen read as text is the last whole one, for a program that said it is
/// drawing the next (synchronized output) and has not said it is done. A program that
/// redraws says so within a frame or two; one that never does is read as it stands
/// after this long.
pub const SYNC_HOLD_MAX: Duration = Duration::from_millis(150);

/// How often a screen is looked at for text somebody waits for, at most, while output
/// arrives. Looking reads the whole screen, so it is not done after every write.
pub const WAIT_TEXT_CHECK: Duration = Duration::from_millis(50);

/// The longest a client may wait for text to appear on a screen.
pub const WAIT_TEXT_MAX: Duration = Duration::from_secs(600);

/// The size of one read from a terminal.
pub const READ_CHUNK: usize = 64 * 1024;

/// How many reads may wait between the thread that reads a terminal and the session that
/// handles them. Small on purpose: when the session stops taking them, the reading
/// thread must stop almost at once, so the program is slowed down instead of buffered.
pub const READ_QUEUE_CHUNKS: usize = 2;

/// The most input held for a program that is not reading its input. Beyond this, input
/// is dropped: a program that has stopped reading will not miss it, and holding it
/// would let a client grow the host without bound.
pub const INPUT_BACKLOG_MAX: usize = 4 * 1024 * 1024;

/// How long the host keeps reading after a session's program has ended and its output
/// has gone quiet. A terminal stays open while any program holds it, so one left
/// running in the background would otherwise keep the session from ever finishing.
pub const EXIT_DRAIN_TIMEOUT: Duration = Duration::from_millis(500);

/// How long a session whose program ended while no client was attached is kept, so
/// that a client that asked for it a moment ago, or comes back for it, still gets its
/// last screen and exit status. A host whose sessions outlive their clients keeps them
/// for [`EXITED_UNSEEN_LINGER`] instead.
pub const UNATTACHED_EXIT_LINGER: Duration = Duration::from_secs(10);

/// How long a host whose sessions outlive their clients keeps a session that ended
/// while nobody was looking: long enough for the user to come back and see how it ended.
pub const EXITED_UNSEEN_LINGER: Duration = Duration::from_secs(60 * 60);

/// How long a connection that is closing waits for the sessions it started to end.
/// Serving a connection is not over until its programs are gone: a host that returned
/// earlier could exit and leave them running with nothing to end them. A program has a
/// few seconds to end by itself before it is forced, so this is a little more than that.
pub const ABANDON_WAIT: Duration = Duration::from_secs(6);

/// The most sessions one host runs.
pub const MAX_SESSIONS: usize = 256;

/// The most bytes handed to the connection in one piece. Output is written to the
/// connection in pieces so that other streams get their turn in between.
pub const SEND_BATCH_BYTES: usize = 256 * 1024;

/// How many pieces may wait to be written to a connection. When it is full, sessions
/// wait, which in turn stops them reading their terminals.
pub const OUTBOX_PIECES: usize = 32;

/// How many frames from a client may wait for one session to handle them.
pub const COMMAND_QUEUE: usize = 256;

/// The most of one control string the host keeps while it looks for its end. A longer
/// one is passed over whole: no report is that long, and a program that never ends a
/// string must not make the host keep what it prints.
pub const OSC_MAX_BYTES: usize = 8 * 1024;

/// The most characters of a title that leave the host.
pub const TITLE_MAX_CHARS: usize = 256;

/// The longest working directory, as a URL, that is taken as one. A longer one is
/// dropped, not cut: a path cut short is another path.
pub const CWD_MAX_BYTES: usize = 2048;

/// The most characters of a command line that leave the host.
pub const COMMAND_MAX_CHARS: usize = 1024;

/// The limits of a program's status report. A report beyond one of them is dropped
/// whole.
pub const STATUS_SEQUENCE_MAX_BYTES: usize = 4096;
pub const STATUS_KEY_MAX_BYTES: usize = 16;
/// A program's name.
pub const STATUS_APP_MAX_BYTES: usize = 32;
/// A record's path: all of it, one step of it, and how many steps.
pub const STATUS_ID_MAX_BYTES: usize = 128;
pub const STATUS_SEGMENT_MAX_BYTES: usize = 32;
pub const STATUS_ID_DEPTH_MAX: usize = 8;
/// A record's label, as it is sent and as it reads.
pub const STATUS_TITLE_MAX_ENCODED: usize = 256;
pub const STATUS_TITLE_MAX_BYTES: usize = 192;
/// A record's line of text, as it is sent and as it reads.
pub const STATUS_MESSAGE_MAX_ENCODED: usize = 2732;
pub const STATUS_MESSAGE_MAX_BYTES: usize = 2048;

/// The most reports a session keeps for a client that is not taking them. Beyond it
/// the oldest goes: what a session reports is its latest state, and an old report is
/// worth least.
pub const EVENTS_PENDING_MAX: usize = 256;

/// The most reports a session passes on in [`EVENT_WINDOW`]. A program can print
/// reports far faster than anyone can read them, and each costs more to pass on than
/// the bytes that made it. Beyond this they are left out, and when the window ends the
/// client is told the state they added up to.
pub const EVENTS_PER_WINDOW_MAX: u32 = 200;
pub const EVENT_WINDOW: Duration = Duration::from_secs(1);

/// How often a session that prints while its view does not have the keyboard says so.
pub const ACTIVITY_INTERVAL: Duration = Duration::from_secs(1);

/// How often the program that has a session's terminal is looked up while a client
/// shows the session.
pub const FOREGROUND_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// How long after a shell says a command started or ended the program that has the
/// terminal is looked up. The shell says it a moment before the command is running,
/// and a reading of the system's processes may be reused for a tenth of a second, so
/// this is a little longer than that.
pub const FOREGROUND_SETTLE: Duration = Duration::from_millis(120);

/// How much of the end of an SSH client's output is kept, to tell from its last words
/// how the connection ended.
pub const SSH_TAIL_BYTES: usize = 1024;

/// How many of the client's last lines are looked at for that.
pub const SSH_TAIL_LINES: usize = 3;

/// The most characters of the client's last line that are passed on.
pub const SSH_DETAIL_MAX_CHARS: usize = 200;

/// How long a host that runs on its own waits, once asked to stop, for its connections
/// to close before it exits regardless.
pub const STOP_WAIT: Duration = Duration::from_secs(2);

/// How soon a host that runs on its own and has had nothing to do for its idle time looks
/// again, when a connection is open whose client has not said yet what it is.
pub const IDLE_RECHECK: Duration = Duration::from_millis(200);

/// How long a host that runs on its own stays with no running session and no client,
/// when it is not told.
pub const IDLE_EXIT_DEFAULT: Duration = Duration::from_secs(60);

/// How large the host's log file may grow before it is set aside and begun again.
pub const LOG_FILE_MAX_BYTES: u64 = 4 * 1024 * 1024;
