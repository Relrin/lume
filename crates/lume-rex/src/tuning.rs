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

/// How long to wait before connecting again after a connection was lost: this long the
/// first time, twice as long each time after, up to the most.
pub const RECONNECT_FIRST: Duration = Duration::from_millis(250);
pub const RECONNECT_MOST: Duration = Duration::from_secs(30);
/// For how long after a connection drops the server keeps its client to be continued.
pub const RESUME_WINDOW: Duration = Duration::from_secs(60);
