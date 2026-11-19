//! What every service shares: the folders a plugin's grants are read against, the
//! errors a service gives, and what it gives back with the grant that allowed it.

use std::fmt;
use std::path::PathBuf;

/// The folders a plugin's path patterns are read against, and what no grant reaches.
#[derive(Clone, Debug)]
pub struct Places {
    /// The workspace's root folder, set by the user. `None` makes `${workspace}` match
    /// nothing.
    pub workspace: Option<PathBuf>,
    /// The user's home folder, for `${home}`.
    pub home: PathBuf,
    /// The plugin's own data folder, for `${plugin-data}`. It lies inside Lume's state
    /// folder and is the one place there a plugin reaches.
    pub plugin_data: PathBuf,
    /// Lume's own folders (configuration, state, the plugins store, the install folder).
    /// Nothing under them is reachable, whatever is granted, except `plugin_data`.
    pub forbidden: Vec<PathBuf>,
    /// Lume's own programs by absolute path, which no plugin may run.
    pub own_programs: Vec<PathBuf>,
}

/// Why a service did not do what it was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceError {
    pub kind: ErrorKind,
    /// What went wrong, in words. It may name the path, program or address asked for,
    /// never what a file or a message held.
    pub message: String,
}

/// The kinds of failure, for the plugin's code to tell apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// No grant allows it, or nothing may.
    Denied,
    /// What was asked for is not there.
    NotFound,
    /// The request itself is wrong: a path, a name or an argument that cannot be used.
    Invalid,
    /// It is larger than the limit asked for or allowed.
    TooLarge,
    /// What the service needs is not there on this system, such as a keychain.
    Unavailable,
    /// It took longer than allowed.
    TimedOut,
    /// The system refused or something broke along the way.
    Failed,
}

impl ErrorKind {
    /// The kind's name, for the plugin's code.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Denied => "denied",
            Self::NotFound => "not-found",
            Self::Invalid => "invalid",
            Self::TooLarge => "too-large",
            Self::Unavailable => "unavailable",
            Self::TimedOut => "timed-out",
            Self::Failed => "failed",
        }
    }
}

impl ServiceError {
    /// An error of a kind, saying what went wrong.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// No grant allows it, or nothing may.
    pub fn denied(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Denied, message)
    }

    /// What was asked for is not there.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, message)
    }

    /// The request itself is wrong.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Invalid, message)
    }

    /// It is larger than allowed.
    pub fn too_large(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::TooLarge, message)
    }

    /// What the service needs is not on this system.
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unavailable, message)
    }

    /// It took longer than allowed.
    pub fn timed_out(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::TimedOut, message)
    }

    /// The system refused, or something broke.
    pub fn failed(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Failed, message)
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ServiceError {}

/// Something a grant allowed, with the grant that allowed it, as granted (for the audit
/// log).
#[derive(Debug)]
pub struct Allowed<T> {
    pub value: T,
    pub grant: String,
}
