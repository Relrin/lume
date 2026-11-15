//! Why something asked of the configuration could not be done.

use std::path::PathBuf;

use crate::source::Problem;

/// Why a change to the configuration, or a read of it, failed.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A file could not be read or written.
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The file to change is not TOML, so nothing can be changed in it safely.
    #[error("{}: {message}", path.display())]
    Syntax { path: PathBuf, message: String },
    /// The path names no setting.
    #[error("there is no setting called `{path}`{}", hint(.suggestion))]
    UnknownKey {
        path: String,
        suggestion: Option<String>,
    },
    /// The value is not one the setting takes.
    #[error("`{path}` cannot be set to that: {message}")]
    InvalidValue { path: String, message: String },
    /// The path is not written the way paths are.
    #[error("{0}")]
    BadPath(String),
    /// The setting cannot be written where it was asked to be.
    #[error("{0}")]
    NotWritable(String),
    /// The name cannot be a workspace's.
    #[error("{0}")]
    BadName(String),
    /// The platform says nothing about where the user's files go.
    #[error("the directory for the configuration could not be found")]
    NoDirectory,
    /// The file was changed, but what is in it now cannot be used.
    #[error("the configuration cannot be used: {}", first(.0))]
    Rejected(Vec<Problem>),
    /// The directory could not be watched for changes.
    #[error("the configuration directory cannot be watched: {0}")]
    Watch(String),
}

fn hint(suggestion: &Option<String>) -> String {
    match suggestion {
        Some(suggestion) => format!(" (did you mean `{suggestion}`?)"),
        None => String::new(),
    }
}

fn first(problems: &[Problem]) -> String {
    problems
        .first()
        .map(ToString::to_string)
        .unwrap_or_else(|| "no reason was given".to_owned())
}

impl ConfigError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
