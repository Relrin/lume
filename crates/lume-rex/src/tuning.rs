//! The numbers this client is built around.

use std::time::Duration;

/// The revisions of the control API this client was written for and tested against.
/// A server that cannot speak one of them is refused.
pub const API_REVISION: u32 = 19;
/// The oldest revision this client can use.
pub const API_MIN_REVISION: u32 = 19;

/// The subprotocol of the control connection.
pub const CONTROL_SUBPROTOCOL: &str = "rex.control.v1";
/// The path of the control connection.
pub const CONTROL_PATH: &str = "/api/control";

/// A hello must stay under this many bytes, and is refused by the server otherwise.
pub const HELLO_MAX_BYTES: usize = 512;
/// The longest a client's name, version or label is sent.
pub const CLIENT_TEXT_MAX_BYTES: usize = 64;

/// How long the server is given to answer the hello.
pub const WELCOME_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a request waits for its response.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a connection hears nothing from its server before it sends a ping. A
/// server pings every two minutes itself; a client that waited as long would show a
/// terminal as live for minutes after its server had gone.
pub const PING_EVERY: Duration = Duration::from_secs(30);
/// How long after its ping a connection that still hears nothing is given up.
pub const PING_WAIT: Duration = Duration::from_secs(20);
/// How long the SSH client may take to reach another machine, in seconds, as the
/// client is told.
pub const SSH_CONNECT_SECONDS: u32 = 10;
/// How long asking another machine where its server listens may take. A server that is
/// not running is started by the asking.
pub const SSH_ASK_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the SSH client may take from its start to forwarding.
pub const SSH_START_TIMEOUT: Duration = Duration::from_secs(30);
/// How often a client that has been started is looked at until it forwards.
pub const SSH_READY_POLL: Duration = Duration::from_millis(50);
/// How long what the client printed is waited for once it has ended: a program it started
/// may still hold its output.
pub const SSH_REST_WAIT: Duration = Duration::from_secs(1);
/// How soon a wait that is held up by a question to the user looks again whether it is.
pub const PROMPT_POLL: Duration = Duration::from_millis(100);
/// How much of what another machine prints about its server is read.
pub const SSH_STATUS_MAX_BYTES: usize = 256 * 1024;
/// The longest path of a socket on another machine that is forwarded to.
pub const SSH_SOCKET_MAX_BYTES: usize = 1024;
/// How many places on this machine are tried to forward from.
pub const SSH_PORT_TRIES: u32 = 3;
/// How long finding a host and opening a stream to it may take.
pub const DIAL_TIMEOUT: Duration = Duration::from_secs(10);
/// How long one way to a host may take to show whether a Rex server answers on it,
/// when the host's address does not say how it is reached.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the connection that showed it is given to close.
pub const PROBE_CLOSE_TIMEOUT: Duration = Duration::from_millis(500);
/// How long the WebSocket handshake of a connection may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The largest message taken from a server, whatever limit it names. A server names
/// 1 MiB today.
pub const MESSAGE_MAX_BYTES: usize = 4 * 1024 * 1024;
/// The smallest message limit a server may name.
pub const MESSAGE_FLOOR_BYTES: usize = 65_536;

/// Events waiting to be taken. A follower that leaves more than this untaken is too far
/// behind to be worth keeping, and the connection is closed to start again.
pub const EVENT_BACKLOG: usize = 4096;
/// Requests waiting to be written.
pub const REQUEST_BACKLOG: usize = 256;

/// The most sessions taken from one server, and the most windows, layers and blocks
/// from one session's view. A server holds at most 64 sessions, 64 windows in each and
/// 1024 blocks in all, unless it was started with other limits.
pub const SESSIONS_MAX: usize = 1024;
pub const WINDOWS_MAX: usize = 256;
pub const LAYERS_MAX: usize = 256;
pub const BLOCKS_MAX: usize = 4096;
/// How long a change asked of a server's layout waits to be told of the view it made,
/// before the view is read without being told.
pub const APPLY_SETTLE: Duration = Duration::from_secs(1);
/// How deep a layout's splits may nest.
pub const SPLIT_DEPTH_MAX: usize = 64;
/// The longest id or label taken from a server, in bytes. Longer ones are cut.
pub const TEXT_MAX_BYTES: usize = 1024;

/// How much a program may print of the folder it is in: the longest path there may be,
/// each of its bytes written as three, and a machine's name.
pub const FOLDER_PRINTED_MAX_BYTES: usize = 3 * TEXT_MAX_BYTES + 256;
/// The most marks and joiners shown on one character of a text from a server.
pub const MARKS_MAX: usize = 4;

/// How many terminals are asked at once what they are when a server's are listed, and
/// how long all of them together are waited for.
pub const DESCRIBE_AT_ONCE: usize = 16;
pub const DESCRIBE_WAIT: Duration = Duration::from_secs(5);
/// The size a terminal is listed at that does not say its own in time.
pub const DESCRIBE_COLS: u16 = 80;
pub const DESCRIBE_ROWS: u16 = 24;

/// The most cells of a terminal's screen. A larger one is shown as large as fits: each
/// cell costs the page about a hundred bytes.
pub const CELLS_MAX: u32 = 512 * 1024;
/// The most memory a copy of a terminal may ask for when it is loaded. A copy can ask
/// for far more than its own length.
pub const COPY_WEIGHT_MAX: u64 = 256 * 1024 * 1024;

/// The path of a data connection.
pub const DATA_PATH: &str = "/api/data";
/// The subprotocol of a data connection that carries a terminal's stream.
pub const TERMINAL_SUBPROTOCOL: &str = "rex.data.terminal.v1";
/// The name of a terminal block's stream among the data the block offers.
pub const TERMINAL_DATA_REF: &str = "terminal";

/// The most bytes of a stream a server puts in one message.
pub const DATA_MESSAGE_MAX_BYTES: usize = 65_536;
/// How long the server is given to answer the first message of a data connection.
pub const DATA_ANSWER_TIMEOUT: Duration = Duration::from_secs(10);
/// How long closing a data connection properly may take before it is simply dropped.
pub const DATA_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

/// The largest copy of a terminal taken from the start of a stream. A larger one is
/// passed over, and the terminal is shown from what the server says is on its screen.
pub const SNAPSHOT_MAX_BYTES: usize = 64 * 1024 * 1024 - 1024;
/// The longest message of the server's own taken out of a terminal's stream. What looks
/// like a longer one is left in the output as it is.
pub const STREAM_NOTE_MAX_BYTES: usize = 65_536;

/// How long a stream may take to give the copy of its terminal and the terminal's size.
pub const STREAM_OPEN_TIMEOUT: Duration = Duration::from_secs(60);
/// The most output kept for a view that is not taking it. Past this the stream is let
/// go, and begun again from a new copy of the terminal once the view takes output again.
pub const PENDING_MAX_BYTES: usize = 8 * 1024 * 1024;
/// How much of a terminal's screen is asked for in one call when it is read as output
/// that rebuilds it. A server gives at most 960 KiB at once.
pub const REPLAY_PART_BYTES: usize = 900_000;
/// How many times in a row beginning a stream again may fail before its terminal is
/// given up.
pub const RESTART_ATTEMPTS_MAX: u32 = 8;
/// How long a stream has to go on for its server not to be one that keeps ending it.
/// One that does is gone along with a little later each time, up to the most.
pub const STREAM_SETTLED: Duration = Duration::from_secs(10);
pub const STREAM_PACE_MOST: Duration = Duration::from_secs(8);

/// The most reports, and sizes, of a terminal passed on in [`REPORT_WINDOW`]. Beyond
/// it they are left out, and when the window ends the views are told what they add up
/// to.
pub const REPORTS_PER_WINDOW_MAX: u32 = 200;
pub const REPORT_WINDOW: Duration = Duration::from_secs(1);

/// How long attaching waits for a terminal whose last view is just going, and how
/// often it looks whether the way is free.
pub const ATTACH_TIMEOUT: Duration = Duration::from_secs(10);
pub const ATTACH_RETRY: Duration = Duration::from_millis(20);

/// The most text typed into a terminal in one call.
pub const INPUT_CALL_MAX_BYTES: usize = 65_536;
/// The most a view may have typed that the server has not taken yet. A view that types
/// more is refused until some has gone.
pub const INPUT_PENDING_MAX_BYTES: usize = 1024 * 1024;
/// How often a terminal's screen is read while text is waited for on it.
pub const SCREEN_POLL: Duration = Duration::from_millis(200);

/// What a server asks of this client and is not answered yet. A server lets no more wait.
pub const PERFORMS_WAITING: usize = 16;
/// How many programs run on a server at once, and how many more there are whose output
/// is read as it comes. Each has at most one request to this client waiting.
pub const RUNS_AT_ONCE: usize = 8;
pub const RUN_STREAMS_AT_ONCE: usize = 4;
/// How long what runs a program may take to ask what it is to run, once it was started.
pub const RUN_CAME_WAIT: Duration = Duration::from_secs(15);
/// How long it may be silent after that. It says every few seconds that it is there.
pub const RUN_SILENT: Duration = Duration::from_secs(30);
/// How long after its time a program is ended from here, if the server has not.
pub const RUN_GRACE: Duration = Duration::from_secs(10);
/// How long after that its output may take to arrive.
pub const RUN_SEND_MOST: Duration = Duration::from_secs(5 * 60);
/// How often a run is looked at for that.
pub const RUN_LOOK: Duration = Duration::from_millis(500);
/// How much of a program's input or output one message holds. Written as JSON a byte
/// takes up to six, and a message holds a mebibyte; where a server takes less, so does
/// a part.
pub const RUN_PART_BYTES: usize = 128 * 1024;
pub const RUN_PART_SHARE: usize = 8;
/// The most a program's name, arguments and variables take together, and its input.
pub const RUN_JOB_MAX_BYTES: usize = 512 * 1024;
pub const RUN_INPUT_MAX_BYTES: usize = 16 * 1024 * 1024;
/// The most blocks that were made to run programs in and are to be closed once there is
/// a connection again.
pub const RUN_BLOCKS_MAX: usize = 256;
/// How much output that nobody has read yet has the program wait.
pub const RUN_HOLD_BYTES: usize = 1024 * 1024;

/// How long to wait before connecting again after a connection was lost: this long the
/// first time, twice as long each time after, up to the most.
pub const RECONNECT_FIRST: Duration = Duration::from_millis(250);
pub const RECONNECT_MOST: Duration = Duration::from_secs(30);
/// For how long after a connection drops the server keeps its client to be continued.
pub const RESUME_WINDOW: Duration = Duration::from_secs(60);
