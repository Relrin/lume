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
