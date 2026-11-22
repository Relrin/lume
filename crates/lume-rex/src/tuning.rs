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
/// How long opening a connection may take, the WebSocket handshake included.
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

/// How long to wait before connecting again after a connection was lost: this long the
/// first time, twice as long each time after, up to the most.
pub const RECONNECT_FIRST: Duration = Duration::from_millis(250);
pub const RECONNECT_MOST: Duration = Duration::from_secs(30);
/// For how long after a connection drops the server keeps its client to be continued.
pub const RESUME_WINDOW: Duration = Duration::from_secs(60);
