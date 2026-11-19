//! Writing lines to logs away from whoever writes them.
//!
//! Plugins' logs and the audit log are written as plugins' calls are carried out, and a
//! busy plugin writes many lines. Opening, measuring and closing a file for each would
//! keep the caller waiting on the disk. Lines are handed to one thread instead, which
//! keeps the files open and writes the lines in the order they were handed over.

use std::collections::HashMap;
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};
use std::sync::{Mutex, OnceLock, PoisonError};

/// What the writing thread is given to do.
enum Job {
    /// Append a line to a file, setting the file aside as `<name>.1` first when it has
    /// grown past `max`.
    Line {
        file: PathBuf,
        line: String,
        max: u64,
    },
    /// Say so once every line handed over before has been written.
    Flush(SyncSender<()>),
}

fn writer() -> &'static Mutex<Sender<Job>> {
    static WRITER: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    WRITER.get_or_init(|| {
        let (send, receive) = channel();
        // Without the thread nothing receives, and each line is written by its caller.
        let _ = std::thread::Builder::new()
            .name("lume-log-writer".to_owned())
            .spawn(move || write_lines(&receive));
        Mutex::new(send)
    })
}

/// Appends `line` to `file` soon, in the order lines are handed over, setting the file
/// aside as `<name>.1` first when it has grown past `max`.
pub fn append_soon(file: &Path, line: String, max: u64) {
    let job = Job::Line {
        file: file.to_owned(),
        line,
        max,
    };
    let sent = writer()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .send(job);
    // Without the thread the line is written here.
    if let Err(error) = sent
        && let Job::Line { file, line, max } = error.0
    {
        crate::broker::append(&file, &line, max);
    }
}

/// Waits until every line handed over before has been written.
pub fn flush() {
    let (done, wait) = sync_channel(1);
    let sent = writer()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .send(Job::Flush(done));
    if sent.is_ok() {
        let _ = wait.recv();
    }
}

/// An open file and how long it is.
struct Open {
    file: File,
    length: u64,
}

fn write_lines(jobs: &Receiver<Job>) {
    let mut open: HashMap<PathBuf, Open> = HashMap::new();
    while let Ok(job) = jobs.recv() {
        match job {
            Job::Line { file, line, max } => {
                if let Some(held) = open.get(&file)
                    && held.length > max
                {
                    open.remove(&file);
                    let mut aside = file.as_os_str().to_owned();
                    aside.push(".1");
                    let _ = std::fs::rename(&file, aside);
                }
                if !open.contains_key(&file) {
                    let Some(held) = open_for_appending(&file, max) else {
                        continue;
                    };
                    open.insert(file.clone(), held);
                }
                if let Some(held) = open.get_mut(&file) {
                    let mut bytes = line.into_bytes();
                    bytes.push(b'\n');
                    match held.file.write_all(&bytes) {
                        Ok(()) => held.length += bytes.len() as u64,
                        // Opened afresh next time: the file may have been taken away.
                        Err(_) => {
                            open.remove(&file);
                        }
                    }
                }
            }
            Job::Flush(done) => {
                let _ = done.send(());
            }
        }
    }
}

/// Opens a file to append to, setting it aside first when it is past `max` already.
fn open_for_appending(file: &Path, max: u64) -> Option<Open> {
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(file).is_ok_and(|metadata| metadata.len() > max) {
        let mut aside = file.as_os_str().to_owned();
        aside.push(".1");
        let _ = std::fs::rename(file, aside);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .ok()?;
    let length = file.metadata().map_or(0, |metadata| metadata.len());
    Some(Open { file, length })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_written_in_order_and_a_full_file_is_set_aside() {
        let dir = crate::store::testing::TempDir::new("appender");
        let file = dir.path().join("logs").join("a.log");
        for number in 0..5 {
            append_soon(&file, format!("line {number}"), 1024);
        }
        flush();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "line 0\nline 1\nline 2\nline 3\nline 4\n"
        );

        // Past its size the file is set aside and a new one begun.
        append_soon(&file, "x".repeat(40), 30);
        append_soon(&file, "after".to_owned(), 30);
        flush();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "after\n");
        let aside = dir.path().join("logs").join("a.log.1");
        assert_eq!(
            std::fs::read_to_string(aside).unwrap().trim_end(),
            "x".repeat(40)
        );
    }
}
