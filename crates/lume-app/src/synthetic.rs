//! Terminals with no program behind them, for measuring the way from a session to the
//! screen.
//!
//! A real terminal's speed is its program's and the operating system's. To time what
//! lies between the session host and the page (coalescing, credit, the carrier, the
//! page's decoder), the bytes must come from something that is never the slow part.
//! These terminals run one of two built-in programs:
//!
//! - `flood <bytes>` prints that many bytes of coloured text as fast as it is read, and
//!   exits with status 0;
//! - `echo` prints back whatever is typed, at once, until it is ended.
//!
//! They are served by a session host of their own, which treats them as it treats any
//! terminal. Only a benchmark run has that host.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use lume_pty::{
    Child, ChildKiller, CommandSpec, ExitStatus, Pty, PtyRead, PtySize, PtySystem, ReadStop,
};

/// How many bytes of text the flood repeats. Whole lines, so it can be cut anywhere a
/// read ends and still be the output of a program printing lines.
const FLOOD_BLOCK_BYTES: usize = 64 * 1024;

/// The pseudo-terminals of the two built-in programs.
pub struct SyntheticPty;

enum Program {
    /// Prints until `left` bytes have been read.
    Flood { left: u64, at: usize },
    /// Prints what was written.
    Echo { typed: VecDeque<u8> },
}

struct State {
    program: Program,
    /// Reading was stopped or the program was ended.
    closed: bool,
    exit: Option<ExitStatus>,
}

/// What the reader, the writer and the child share.
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn end(&self, status: ExitStatus) {
        let mut state = self.lock();
        state.closed = true;
        state.exit.get_or_insert(status);
        self.changed.notify_all();
    }
}

/// Text that looks like what a busy program prints: numbered lines with a colour each.
fn flood_block() -> &'static [u8] {
    static BLOCK: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    BLOCK.get_or_init(|| {
        let mut block = Vec::with_capacity(FLOOD_BLOCK_BYTES + 128);
        let mut line = 0u32;
        while block.len() < FLOOD_BLOCK_BYTES {
            let colour = 31 + line % 6;
            let text = format!(
                "\x1b[{colour}m{line:08}\x1b[0m the quick brown fox jumps over the lazy dog \
                 0123456789 abcdefghijklmnopqrstuvwxyz\r\n"
            );
            block.extend_from_slice(text.as_bytes());
            line += 1;
        }
        block
    })
}

impl PtySystem for SyntheticPty {
    fn spawn(
        &self,
        command: &CommandSpec,
        _size: PtySize,
    ) -> io::Result<(Box<dyn Pty>, Box<dyn Child>)> {
        let invalid = |reason: String| io::Error::new(io::ErrorKind::InvalidInput, reason);
        let program = match command.program.to_str() {
            Some("flood") => {
                let bytes = command
                    .args
                    .first()
                    .and_then(|arg| arg.to_str())
                    .and_then(|arg| arg.parse::<u64>().ok())
                    .ok_or_else(|| invalid("flood takes the number of bytes to print".into()))?;
                Program::Flood { left: bytes, at: 0 }
            }
            Some("echo") => Program::Echo {
                typed: VecDeque::new(),
            },
            _ => {
                return Err(invalid(format!(
                    "{} is not one of the built-in programs",
                    command.program.to_string_lossy()
                )));
            }
        };
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                program,
                closed: false,
                exit: None,
            }),
            changed: Condvar::new(),
        });
        let pty = Terminal {
            shared: Arc::clone(&shared),
            reader_taken: false,
            writer_taken: false,
        };
        Ok((Box::new(pty), Box::new(Process { shared })))
    }

    fn resize_pulls_scrollback(&self) -> bool {
        true
    }
}

struct Terminal {
    shared: Arc<Shared>,
    reader_taken: bool,
    writer_taken: bool,
}

impl Pty for Terminal {
    fn take_reader(&mut self) -> io::Result<Box<dyn PtyRead>> {
        if std::mem::replace(&mut self.reader_taken, true) {
            return Err(io::Error::other("the output stream was already taken"));
        }
        Ok(Box::new(Output {
            shared: Arc::clone(&self.shared),
        }))
    }

    fn take_writer(&mut self) -> io::Result<Box<dyn Write + Send>> {
        if std::mem::replace(&mut self.writer_taken, true) {
            return Err(io::Error::other("the input stream was already taken"));
        }
        Ok(Box::new(Input {
            shared: Arc::clone(&self.shared),
        }))
    }

    fn resize(&self, _size: PtySize) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Closing a terminal ends what runs in it.
        self.shared.end(ExitStatus::Unknown);
    }
}

struct Output {
    shared: Arc<Shared>,
}

impl Read for Output {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut state = self.shared.lock();
        loop {
            if state.closed {
                return Ok(0);
            }
            match &mut state.program {
                Program::Flood { left, at } => {
                    if *left == 0 {
                        // Everything is printed: the program is done.
                        state.closed = true;
                        state.exit.get_or_insert(ExitStatus::Code(0));
                        self.shared.changed.notify_all();
                        return Ok(0);
                    }
                    let block = flood_block();
                    let rest = &block[*at..];
                    let take = buf
                        .len()
                        .min(rest.len())
                        .min(usize::try_from(*left).unwrap_or(usize::MAX));
                    buf[..take].copy_from_slice(&rest[..take]);
                    *at = (*at + take) % block.len();
                    *left -= take as u64;
                    return Ok(take);
                }
                Program::Echo { typed } => {
                    if !typed.is_empty() {
                        let take = buf.len().min(typed.len());
                        for (slot, byte) in buf.iter_mut().zip(typed.drain(..take)) {
                            *slot = byte;
                        }
                        return Ok(take);
                    }
                }
            }
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl PtyRead for Output {
    fn stopper(&self) -> Box<dyn ReadStop> {
        Box::new(Stop {
            shared: Arc::clone(&self.shared),
        })
    }
}

struct Stop {
    shared: Arc<Shared>,
}

impl ReadStop for Stop {
    fn stop(&self) {
        self.shared.end(ExitStatus::Unknown);
    }
}

struct Input {
    shared: Arc<Shared>,
}

impl Write for Input {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut state = self.shared.lock();
        if let Program::Echo { typed } = &mut state.program {
            typed.extend(bytes);
            self.shared.changed.notify_all();
        }
        // The flood reads nothing; what is typed at it is lost, as at any program that
        // does not read its input.
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Process {
    shared: Arc<Shared>,
}

impl Child for Process {
    fn pid(&self) -> u32 {
        // No process stands behind a built-in program.
        0
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        let mut state = self.shared.lock();
        loop {
            if let Some(status) = state.exit {
                return Ok(status);
            }
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn killer(&self) -> Box<dyn ChildKiller> {
        Box::new(Kill {
            shared: Arc::clone(&self.shared),
        })
    }
}

struct Kill {
    shared: Arc<Shared>,
}

impl ChildKiller for Kill {
    fn kill(&self) -> io::Result<()> {
        self.shared.end(ExitStatus::Unknown);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn(program: &str, args: &[&str]) -> (Box<dyn Pty>, Box<dyn Child>) {
        let command = CommandSpec::new(program).args(args.iter().copied());
        SyntheticPty
            .spawn(&command, PtySize::cells(80, 24))
            .unwrap()
    }

    #[test]
    fn the_flood_prints_exactly_what_it_was_asked_for_and_exits() {
        let (mut pty, mut child) = spawn("flood", &["200000"]);
        let mut reader = pty.take_reader().unwrap();
        let mut total = 0;
        let mut buf = vec![0; 7001];
        let mut first = Vec::new();
        loop {
            let read = reader.read(&mut buf).unwrap();
            if read == 0 {
                break;
            }
            if first.is_empty() {
                first.extend_from_slice(&buf[..read]);
            }
            total += read;
        }
        assert_eq!(total, 200_000);
        assert!(first.starts_with(b"\x1b[31m00000000\x1b[0m the quick"));
        assert_eq!(child.wait().unwrap(), ExitStatus::Code(0));
        // Once it has ended there is nothing more to read.
        assert_eq!(reader.read(&mut buf).unwrap(), 0);
    }

    #[test]
    fn the_flood_is_whole_lines_wherever_the_block_repeats() {
        let block = flood_block();
        assert!(block.len() >= FLOOD_BLOCK_BYTES);
        assert!(block.ends_with(b"\r\n"));
    }

    #[test]
    fn the_echo_returns_what_is_typed_until_it_is_ended() {
        let (mut pty, mut child) = spawn("echo", &[]);
        let mut reader = pty.take_reader().unwrap();
        let mut writer = pty.take_writer().unwrap();
        let killer = child.killer();

        writer.write_all(b"hello").unwrap();
        let mut buf = [0; 16];
        let read = reader.read(&mut buf).unwrap();
        assert_eq!(&buf[..read], b"hello");

        // A read with nothing typed waits; ending the program releases it.
        let waiting = std::thread::spawn(move || {
            let mut buf = [0; 16];
            reader.read(&mut buf).unwrap()
        });
        std::thread::sleep(std::time::Duration::from_millis(30));
        killer.kill().unwrap();
        assert_eq!(waiting.join().unwrap(), 0);
        assert_eq!(child.wait().unwrap(), ExitStatus::Unknown);
    }

    #[test]
    fn stopping_the_reader_ends_a_read_that_is_waiting() {
        let (mut pty, _child) = spawn("echo", &[]);
        let mut reader = pty.take_reader().unwrap();
        let stop = reader.stopper();
        let waiting = std::thread::spawn(move || {
            let mut buf = [0; 16];
            reader.read(&mut buf).unwrap()
        });
        std::thread::sleep(std::time::Duration::from_millis(30));
        stop.stop();
        assert_eq!(waiting.join().unwrap(), 0);
    }

    #[test]
    fn what_is_not_a_built_in_program_is_refused() {
        let unknown = SyntheticPty.spawn(&CommandSpec::new("sh"), PtySize::cells(80, 24));
        assert_eq!(unknown.err().unwrap().kind(), io::ErrorKind::InvalidInput);
        let no_count = SyntheticPty.spawn(&CommandSpec::new("flood"), PtySize::cells(80, 24));
        assert_eq!(no_count.err().unwrap().kind(), io::ErrorKind::InvalidInput);
        let (mut pty, _child) = spawn("echo", &[]);
        pty.take_reader().unwrap();
        assert!(pty.take_reader().is_err());
    }
}
