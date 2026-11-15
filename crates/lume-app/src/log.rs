//! Where the application's log goes.
//!
//! The core says what it does through `tracing`: sessions started and ended, connections
//! made and lost, things that failed. This module decides who hears it. Events are
//! written as lines of text to standard error and to a file in the user's own log
//! directory, because an application started from a launcher has no standard error
//! anyone can read.
//!
//! The log says what happened, never what was in a terminal. Nothing that passes through
//! a session (what a program printed, what the user typed) is given to `tracing`
//! anywhere in the core, and the framework's own tracing, which records the contents of
//! requests, is left off.
//!
//! `LUME_LOG` sets how much is written: `off`, `error`, `warn`, `info` (the default),
//! `debug` or `trace`. `LUME_LOG_FILE` names the file to write instead of the default.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use tracing::Level;

use crate::tuning::LOG_FILE_MAX_BYTES;

/// The variable that sets how much is logged.
const LEVEL_VARIABLE: &str = "LUME_LOG";

/// The variable that names the log file.
const FILE_VARIABLE: &str = "LUME_LOG_FILE";

/// The log file's name in the log directory.
const FILE_NAME: &str = "lume.log";

/// How much to log, as `LUME_LOG` spells it. `None` is no logging at all.
fn level(spelling: Option<&str>) -> Option<Level> {
    match spelling.map(str::trim).unwrap_or("info") {
        "off" | "none" => None,
        "error" => Some(Level::ERROR),
        "warn" => Some(Level::WARN),
        "debug" => Some(Level::DEBUG),
        "trace" => Some(Level::TRACE),
        // Anything else, the empty string included, is the default: a misspelt level
        // should not silence the log.
        _ => Some(Level::INFO),
    }
}

/// The user's directory for an application's logs, from the variables that place it.
fn log_dir(variable: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    if cfg!(windows) {
        Some(variable("LOCALAPPDATA")?.join("Lume").join("logs"))
    } else if cfg!(target_os = "macos") {
        Some(variable("HOME")?.join("Library/Logs/Lume"))
    } else {
        let state = variable("XDG_STATE_HOME")
            .filter(|dir| dir.is_absolute())
            .or_else(|| Some(variable("HOME")?.join(".local/state")))?;
        Some(state.join("lume"))
    }
}

/// The file to log to: the one named, or the default in the user's log directory.
fn log_file() -> Option<PathBuf> {
    let variable = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    variable(FILE_VARIABLE).or_else(|| Some(log_dir(variable)?.join(FILE_NAME)))
}

/// Opens the log file for appending. A file that has grown past its limit is set aside
/// first, replacing the one set aside before, so the log is bounded at twice the limit.
fn open(path: &Path, limit: u64) -> io::Result<File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    if fs::metadata(path).is_ok_and(|meta| meta.len() > limit) {
        let mut aside = path.as_os_str().to_owned();
        aside.push(".old");
        // If it cannot be moved it is appended to after all.
        let _ = fs::rename(path, aside);
    }
    OpenOptions::new().create(true).append(true).open(path)
}

/// Writes each line to standard error and to the log file, if there is one.
#[derive(Clone)]
struct Sink {
    file: Option<Arc<Mutex<File>>>,
}

impl Write for Sink {
    fn write(&mut self, line: &[u8]) -> io::Result<usize> {
        // Neither destination may stop the other, or the application: a log that cannot
        // be written is not a reason to fail.
        let _ = io::stderr().write_all(line);
        if let Some(file) = &self.file {
            let mut file = file.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = file.write_all(line);
        }
        Ok(line.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Starts logging for the rest of the process. Call once, before anything logs.
pub fn install() {
    let Some(level) = level(std::env::var(LEVEL_VARIABLE).ok().as_deref()) else {
        return;
    };
    let path = log_file();
    let (file, problem) = match path.as_deref().map(|path| open(path, LOG_FILE_MAX_BYTES)) {
        Some(Ok(file)) => (Some(Arc::new(Mutex::new(file))), None),
        Some(Err(error)) => (None, Some(error)),
        None => (None, None),
    };
    let sink = Sink { file };
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(move || sink.clone())
        .finish();
    if tracing::subscriber::set_global_default(subscriber).is_err() {
        return;
    }
    if let (Some(path), Some(error)) = (&path, &problem) {
        tracing::warn!(path = %path.display(), %error, "the log file could not be opened");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_level_is_read_from_its_spelling() {
        assert_eq!(level(None), Some(Level::INFO));
        assert_eq!(level(Some("off")), None);
        assert_eq!(level(Some("error")), Some(Level::ERROR));
        assert_eq!(level(Some("warn")), Some(Level::WARN));
        assert_eq!(level(Some("info")), Some(Level::INFO));
        assert_eq!(level(Some(" debug ")), Some(Level::DEBUG));
        assert_eq!(level(Some("trace")), Some(Level::TRACE));
        assert_eq!(level(Some("loud")), Some(Level::INFO));
        assert_eq!(level(Some("")), Some(Level::INFO));
    }

    #[test]
    fn the_log_directory_is_the_users_own() {
        let home = |name: &str| match name {
            "LOCALAPPDATA" => Some(PathBuf::from(r"C:\Users\u\AppData\Local")),
            "HOME" => Some(PathBuf::from("/home/u")),
            _ => None,
        };
        let dir = log_dir(home).unwrap();
        if cfg!(windows) {
            assert_eq!(dir, PathBuf::from(r"C:\Users\u\AppData\Local\Lume\logs"));
        } else if cfg!(target_os = "macos") {
            assert_eq!(dir, PathBuf::from("/home/u/Library/Logs/Lume"));
        } else {
            assert_eq!(dir, PathBuf::from("/home/u/.local/state/lume"));
        }
        assert_eq!(log_dir(|_| None), None);
    }

    #[test]
    fn a_log_past_its_limit_is_set_aside_and_begun_again() {
        let dir = std::env::temp_dir().join(format!("lume-log-test-{}", std::process::id()));
        let path = dir.join("nested").join(FILE_NAME);
        let aside = dir.join("nested").join("lume.log.old");

        open(&path, 10).unwrap().write_all(b"short").unwrap();
        // Within the limit: appended to.
        open(&path, 10).unwrap().write_all(b" and long").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"short and long");
        assert!(!aside.exists());

        // Past it: set aside, and the new file starts empty.
        open(&path, 10).unwrap().write_all(b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(fs::read(&aside).unwrap(), b"short and long");

        fs::remove_dir_all(&dir).unwrap();
    }
}
