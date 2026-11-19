//! Streams a plugin holds open: a file it reads, a program's pipes, a response's body.
//!
//! Each stream belongs to the plugin it was opened for and has an id of its own; another
//! plugin asking for that id is told there is no such stream. A read waits for data, so
//! each stream is behind a lock of its own: a read that waits holds up only its own
//! stream, never another one or the table of them. A plugin holds only so many at once.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::scope::ServiceError;

/// The most one read gives, whatever is asked for.
pub const READ_MAX: usize = 1024 * 1024;

type Reader = Box<dyn Read + Send>;
type Writer = Box<dyn Write + Send>;

struct Stream {
    owner: String,
    reader: Option<Mutex<Reader>>,
    writer: Option<Mutex<Writer>>,
}

/// The streams every plugin holds open.
pub struct Streams {
    max_per_owner: usize,
    open: Mutex<HashMap<u64, Arc<Stream>>>,
    next: AtomicU64,
}

impl std::fmt::Debug for Streams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Streams")
            .field("max_per_owner", &self.max_per_owner)
            .finish_non_exhaustive()
    }
}

fn no_stream(id: u64) -> ServiceError {
    ServiceError::not_found(format!("there is no stream {id}"))
}

impl Streams {
    /// No streams yet; each owner may hold `max_per_owner` at once.
    pub fn new(max_per_owner: usize) -> Self {
        Self {
            max_per_owner,
            open: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
        }
    }

    fn open(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Arc<Stream>>> {
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The stream `id`, if `owner` holds it.
    fn get(&self, owner: &str, id: u64) -> Result<Arc<Stream>, ServiceError> {
        self.open()
            .get(&id)
            .filter(|stream| stream.owner == owner)
            .cloned()
            .ok_or_else(|| no_stream(id))
    }

    /// Opens a stream for `owner` to read, write or both, and gives its id. Refused as
    /// too large when the owner holds as many as it may.
    pub fn add(
        &self,
        owner: &str,
        reader: Option<Box<dyn Read + Send>>,
        writer: Option<Box<dyn Write + Send>>,
    ) -> Result<u64, ServiceError> {
        if reader.is_none() && writer.is_none() {
            return Err(ServiceError::invalid(
                "a stream is read, written or both; this one would be neither",
            ));
        }
        let mut open = self.open();
        let held = open.values().filter(|stream| stream.owner == owner).count();
        if held >= self.max_per_owner {
            return Err(ServiceError::too_large(format!(
                "{owner} already holds {held} streams open, as many as a plugin may"
            )));
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        open.insert(
            id,
            Arc::new(Stream {
                owner: owner.to_owned(),
                reader: reader.map(Mutex::new),
                writer: writer.map(Mutex::new),
            }),
        );
        Ok(id)
    }

    /// Reads at most `max` bytes (and never more than [`READ_MAX`]) from a stream,
    /// waiting for at least one. `None` at its end.
    pub fn read(&self, owner: &str, id: u64, max: usize) -> Result<Option<Vec<u8>>, ServiceError> {
        let stream = self.get(owner, id)?;
        let reader = stream
            .reader
            .as_ref()
            .ok_or_else(|| ServiceError::invalid(format!("stream {id} cannot be read")))?;
        if max == 0 {
            return Err(ServiceError::invalid("a read asks for at least one byte"));
        }
        let mut buffer = vec![0u8; max.min(READ_MAX)];
        let mut reader = reader.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => return Ok(None),
                Ok(read) => {
                    buffer.truncate(read);
                    return Ok(Some(buffer));
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => {
                    return Err(stream_error(id, "read", &error));
                }
            }
        }
    }

    /// Writes all of `bytes` to a stream, waiting while it is full.
    pub fn write(&self, owner: &str, id: u64, bytes: &[u8]) -> Result<(), ServiceError> {
        let stream = self.get(owner, id)?;
        let writer = stream
            .writer
            .as_ref()
            .ok_or_else(|| ServiceError::invalid(format!("stream {id} cannot be written")))?;
        let mut writer = writer.lock().unwrap_or_else(PoisonError::into_inner);
        writer
            .write_all(bytes)
            .and_then(|()| writer.flush())
            .map_err(|error| stream_error(id, "written", &error))
    }

    /// Closes a stream. A read waiting on it at that moment still finishes; what it read
    /// from is let go of once it has.
    pub fn close(&self, owner: &str, id: u64) -> Result<(), ServiceError> {
        let mut open = self.open();
        match open.get(&id) {
            Some(stream) if stream.owner == owner => {
                open.remove(&id);
                Ok(())
            }
            _ => Err(no_stream(id)),
        }
    }

    /// Closes every stream `owner` holds, as when the plugin stops.
    pub fn close_all(&self, owner: &str) {
        self.open().retain(|_, stream| stream.owner != owner);
    }

    /// How many streams `owner` holds open.
    pub fn count(&self, owner: &str) -> usize {
        self.open()
            .values()
            .filter(|stream| stream.owner == owner)
            .count()
    }
}

fn stream_error(id: u64, done: &str, error: &std::io::Error) -> ServiceError {
    match error.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
            ServiceError::timed_out(format!("stream {id} could not be {done} in time"))
        }
        std::io::ErrorKind::BrokenPipe => {
            ServiceError::failed(format!("stream {id} was closed at its other end"))
        }
        // A response's body past its limit says so this way.
        std::io::ErrorKind::FileTooLarge => {
            ServiceError::too_large(format!("stream {id}: {error}"))
        }
        _ => ServiceError::failed(format!("stream {id} could not be {done}: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;
    use crate::ErrorKind;

    fn reader(bytes: &[u8]) -> Option<Box<dyn Read + Send>> {
        Some(Box::new(Cursor::new(bytes.to_vec())))
    }

    #[test]
    fn a_stream_is_read_to_its_end() {
        let streams = Streams::new(4);
        let id = streams.add("a.x", reader(b"hello"), None).unwrap();
        assert_eq!(
            streams.read("a.x", id, 3).unwrap().as_deref(),
            Some(&b"hel"[..])
        );
        assert_eq!(
            streams.read("a.x", id, 10).unwrap().as_deref(),
            Some(&b"lo"[..])
        );
        assert_eq!(streams.read("a.x", id, 10).unwrap(), None);
        streams.close("a.x", id).unwrap();
        assert_eq!(
            streams.read("a.x", id, 1).unwrap_err().kind,
            ErrorKind::NotFound
        );
    }

    #[test]
    fn another_plugins_stream_is_not_there() {
        let streams = Streams::new(4);
        let id = streams.add("a.x", reader(b"secret"), None).unwrap();
        for error in [
            streams.read("b.y", id, 10).unwrap_err(),
            streams.write("b.y", id, b"x").unwrap_err(),
            streams.close("b.y", id).unwrap_err(),
        ] {
            assert_eq!(error.kind, ErrorKind::NotFound);
        }
        streams.close_all("b.y");
        assert_eq!(streams.count("a.x"), 1);
    }

    #[test]
    fn a_plugin_holds_only_so_many_streams() {
        let streams = Streams::new(2);
        streams.add("a.x", reader(b""), None).unwrap();
        streams.add("a.x", reader(b""), None).unwrap();
        let third = streams.add("a.x", reader(b""), None).unwrap_err();
        assert_eq!(third.kind, ErrorKind::TooLarge);
        streams.add("b.y", reader(b""), None).unwrap();
        streams.close_all("a.x");
        assert_eq!(streams.count("a.x"), 0);
        assert_eq!(streams.count("b.y"), 1);
        streams.add("a.x", reader(b""), None).unwrap();
    }

    #[test]
    fn a_stream_is_written_and_one_way_streams_refuse_the_other() {
        #[derive(Clone)]
        struct Shared(Arc<Mutex<Vec<u8>>>);
        impl Write for Shared {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let streams = Streams::new(4);
        let sink = Shared(Arc::new(Mutex::new(Vec::new())));
        let id = streams
            .add("a.x", None, Some(Box::new(sink.clone())))
            .unwrap();
        streams.write("a.x", id, b"abc").unwrap();
        assert_eq!(*sink.0.lock().unwrap(), b"abc");
        assert_eq!(
            streams.read("a.x", id, 1).unwrap_err().kind,
            ErrorKind::Invalid
        );
        let read_only = streams.add("a.x", reader(b"x"), None).unwrap();
        assert_eq!(
            streams.write("a.x", read_only, b"x").unwrap_err().kind,
            ErrorKind::Invalid
        );
        assert_eq!(
            streams.add("a.x", None, None).unwrap_err().kind,
            ErrorKind::Invalid
        );
    }

    #[test]
    fn a_read_that_waits_holds_up_no_other_stream() {
        /// A reader that waits until it is told to go on.
        struct Waiting(mpsc::Receiver<()>);
        impl Read for Waiting {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                let _ = self.0.recv();
                buffer[0] = b'!';
                Ok(1)
            }
        }
        let streams = Arc::new(Streams::new(4));
        let (go, wait) = mpsc::channel();
        let slow = streams
            .add("a.x", Some(Box::new(Waiting(wait))), None)
            .unwrap();
        let quick = streams.add("a.x", reader(b"quick"), None).unwrap();
        let waiting = {
            let streams = Arc::clone(&streams);
            std::thread::spawn(move || streams.read("a.x", slow, 1))
        };
        std::thread::sleep(Duration::from_millis(50));
        // The slow read is waiting; another stream, the count and a new stream are not.
        assert_eq!(
            streams.read("a.x", quick, 10).unwrap().as_deref(),
            Some(&b"quick"[..])
        );
        assert_eq!(streams.count("a.x"), 2);
        streams.add("b.y", reader(b""), None).unwrap();
        streams.close("a.x", slow).unwrap();
        go.send(()).unwrap();
        assert_eq!(waiting.join().unwrap().unwrap().as_deref(), Some(&b"!"[..]));
    }

    #[test]
    fn a_body_past_its_limit_is_too_large() {
        struct Capped;
        impl Read for Capped {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::FileTooLarge,
                    "the response is larger than 10 bytes",
                ))
            }
        }
        let streams = Streams::new(4);
        let id = streams.add("a.x", Some(Box::new(Capped)), None).unwrap();
        let error = streams.read("a.x", id, 10).unwrap_err();
        assert_eq!(error.kind, ErrorKind::TooLarge);
        assert!(error.message.contains("10 bytes"), "{}", error.message);
    }

    #[test]
    fn the_streams_can_be_shared_between_threads() {
        fn shared<T: Send + Sync>() {}
        shared::<Streams>();
    }
}
