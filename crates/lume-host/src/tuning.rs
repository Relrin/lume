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

/// Output read this soon after input is sent without waiting. It is most likely the
/// echo of what the user typed, and typing must not feel delayed.
pub const ECHO_IMMEDIATE_WINDOW: Duration = Duration::from_millis(20);

/// How much output a session keeps while no client has attached yet. A program starts
/// printing as soon as it runs, before the client that asked for it can attach; this
/// holds those first bytes. When it is full the host stops reading, so nothing is lost
/// and nothing grows.
pub const PRE_ATTACH_BYTES: usize = 256 * 1024;

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

/// How long a session whose program ended before any client attached is kept, so that
/// a client that asked for it a moment ago still gets its output and exit status.
pub const UNATTACHED_EXIT_LINGER: Duration = Duration::from_secs(10);

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
