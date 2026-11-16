//! Limits of what is read while looking for places to open a terminal in. Each value
//! is defined here and nowhere else.

use std::time::Duration;

/// How long `wsl.exe` is given to list the distributions. It answers in a fraction of a
/// second when it is well, and never when its service is stuck.
pub const WSL_LIST_TIMEOUT: Duration = Duration::from_secs(5);

/// How often a running `wsl.exe` is looked at to see whether it has finished.
pub const WSL_LIST_POLL: Duration = Duration::from_millis(20);

/// How long the listing's output is waited for once `wsl.exe` has ended. It is there
/// at once unless something else still holds the pipe it was written to.
pub const WSL_OUTPUT_GRACE: Duration = Duration::from_millis(500);

/// The most of a listing that is read. A real one is a few hundred bytes.
pub const WSL_LIST_MAX_BYTES: u64 = 1024 * 1024;

/// How deep one OpenSSH configuration file may include another. The client itself
/// stops at this depth.
pub const SSH_INCLUDE_DEPTH_MAX: usize = 16;

/// How many OpenSSH configuration files are read in all, however they include each
/// other.
pub const SSH_CONFIG_FILES_MAX: usize = 256;

/// The largest OpenSSH configuration file that is read.
pub const SSH_CONFIG_FILE_MAX_BYTES: u64 = 1024 * 1024;

/// The most hosts taken from the OpenSSH configuration. More is not a list a person
/// picks from.
pub const SSH_HOSTS_MAX: usize = 4096;

/// The longest path the folder for shared connections may make, once the client has
/// put a connection's name in it. A Unix socket's path holds 104 bytes on the systems
/// with the least room, and the client needs 18 of them for the temporary name it
/// makes the socket under.
pub const SSH_CONTROL_PATH_MAX_BYTES: usize = 86;

/// The length of the name the client gives a shared connection (its `%C`).
pub const SSH_CONTROL_NAME_BYTES: usize = 40;

/// The most of the system's list of login shells that is read.
pub const SHELLS_FILE_MAX_BYTES: u64 = 64 * 1024;
