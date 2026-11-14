//! Creating a pseudoconsole, starting a program in it, and taking it down again.
//!
//! A pseudoconsole is a console host process with two pipes: we write the user's input
//! into one and read the rendered output from the other. Three things about it shape
//! this module:
//!
//! - The output pipe reports end-of-file only when the console host exits, and the host
//!   stays alive for as long as the pseudoconsole handle is open, even after every
//!   program in it has ended. So the handle is closed as soon as the program that was
//!   started ends; that is also when a terminal application closes a tab.
//! - Closing the handle can block until the host has written its last output, and the
//!   host blocks while nobody reads that output. Closing therefore never happens on the
//!   thread that reads, or on any thread that must not block: it gets its own.
//! - Closing the input pipe tells the host its terminal has gone, which ends the session.
//!   The writer is kept for the life of the session for that reason.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    ERROR_INSUFFICIENT_BUFFER, ERROR_OPERATION_ABORTED, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Console::{COORD, HPCON};
use windows_sys::Win32::System::IO::CancelIoEx;
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    UpdateProcThreadAttribute,
};

use super::api::ConPtyApi;
use super::child::{ChildEnd, WinChild};
use super::cmdline::{command_line, plain_path, resolve_program, wide};
use super::env::Environment;
use super::{check, check_hresult};
use crate::{CommandSpec, Pty, PtyRead, PtySize, ReadStop};

/// Flags passed when a pseudoconsole is created. None are set:
///
/// - Inheriting the cursor makes the console host ask the terminal for the cursor
///   position and wait for the answer, and nothing answers before a client attaches.
/// - The Windows-specific input encoding is not requested, because the input sent here is
///   plain terminal input.
const PSEUDOCONSOLE_FLAGS: u32 = 0;

/// The size of each pipe's buffer. The console host writes output in small pieces; a
/// buffer this size lets it keep going while the reader is busy with the previous piece.
const PIPE_BUFFER_BYTES: u32 = 64 * 1024;

/// How long [`ReadStop::stop`] keeps trying to interrupt a read that is in progress.
const STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// An open pseudoconsole. Shared by the terminal (which resizes it) and the child (which
/// closes it when the program ends).
pub(crate) struct Console {
    api: &'static ConPtyApi,
    /// `None` once closed.
    handle: Mutex<Option<HPCON>>,
}

impl Console {
    fn resize(&self, size: PtySize) -> io::Result<()> {
        let coord = coord(size)?;
        let handle = self.handle.lock().unwrap_or_else(PoisonError::into_inner);
        match *handle {
            // SAFETY: the handle is open: it is only closed after being taken out of the
            // mutex, which is held here.
            Some(hpc) => check_hresult(
                unsafe { (self.api.resize)(hpc, coord) },
                "ResizePseudoConsole",
            ),
            None => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the terminal is closed",
            )),
        }
    }

    /// Closes the pseudoconsole, once. The programs still attached are told their
    /// console is going away. Returns at once: the close runs on its own thread.
    pub(crate) fn close(&self) {
        let taken = self
            .handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let Some(hpc) = taken else { return };
        let close = self.api.close;
        // SAFETY: `hpc` was taken out of the mutex, so nothing else can use or close it.
        let run = move || unsafe { close(hpc) };
        if let Err(error) = std::thread::Builder::new()
            .name("pty-close".to_owned())
            .spawn(run)
        {
            // Without a thread the close happens here. It may block until the output is
            // drained, but leaking the console host would be worse.
            tracing::warn!(%error, "closing a pseudoconsole on the calling thread");
            run();
        }
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        self.close();
    }
}

fn coord(size: PtySize) -> io::Result<COORD> {
    let dimension = |cells: u16| {
        i16::try_from(cells)
            .ok()
            .filter(|&cells| cells > 0)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("a terminal cannot be {cells} cells"),
                )
            })
    };
    Ok(COORD {
        X: dimension(size.cols)?,
        Y: dimension(size.rows)?,
    })
}

/// An anonymous pipe: the read end, then the write end. Neither is inheritable.
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let mut read: HANDLE = std::ptr::null_mut();
    let mut write: HANDLE = std::ptr::null_mut();
    // SAFETY: both out pointers are valid, and null attributes ask for the defaults.
    check(unsafe { CreatePipe(&mut read, &mut write, std::ptr::null(), PIPE_BUFFER_BYTES) })?;
    // SAFETY: on success both are open handles that nothing else owns.
    unsafe {
        Ok((
            OwnedHandle::from_raw_handle(read),
            OwnedHandle::from_raw_handle(write),
        ))
    }
}

/// The attribute list that tells process creation to attach the child to a pseudoconsole.
struct AttributeList {
    /// Machine words rather than bytes, so the list is aligned for the pointers in it.
    buffer: Vec<usize>,
}

impl AttributeList {
    fn with_pseudoconsole(hpc: HPCON) -> io::Result<Self> {
        let mut bytes = 0;
        // SAFETY: a null list with a size out pointer asks how large the list must be.
        // That call fails by design with ERROR_INSUFFICIENT_BUFFER.
        let ok =
            unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut bytes) };
        let error = io::Error::last_os_error();
        if ok != 0 || error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
            return Err(error);
        }

        let mut list = Self {
            buffer: vec![0; bytes.div_ceil(size_of::<usize>())],
        };
        // SAFETY: the buffer holds at least `bytes` bytes.
        check(unsafe { InitializeProcThreadAttributeList(list.as_ptr(), 1, 0, &mut bytes) })?;
        // SAFETY: the list is initialised with room for one attribute. For this attribute
        // the value is the pseudoconsole handle itself, not a pointer to it.
        check(unsafe {
            UpdateProcThreadAttribute(
                list.as_ptr(),
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                hpc as *const core::ffi::c_void,
                size_of::<HPCON>(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        })?;
        Ok(list)
    }

    fn as_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.buffer.as_mut_ptr().cast()
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        // SAFETY: the list was initialised in `with_pseudoconsole`. If initialising it
        // failed, the buffer is still zeroed, which deleting tolerates.
        unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
    }
}

/// Starts `command` in a new pseudoconsole.
pub(crate) fn spawn(
    api: &'static ConPtyApi,
    command: &CommandSpec,
    size: PtySize,
) -> io::Result<(WinPty, WinChild)> {
    // Everything that can fail on the caller's input is done before anything is created.
    let environment = Environment::for_command(std::env::vars_os(), command);
    let environment_block = environment.block()?;
    let cwd = match &command.cwd {
        Some(dir) if dir.is_dir() => plain_path(&std::path::absolute(dir)?),
        Some(dir) => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("the working directory `{}` does not exist", dir.display()),
            ));
        }
        None => std::env::current_dir()?,
    };
    let program = resolve_program(&command.program, &cwd, environment.get("PATH"))?;
    let program_wide = wide(program.as_os_str())?;
    let cwd_wide = wide(cwd.as_os_str())?;
    let mut line = command_line(&command.program, &command.args)?;
    let coord = coord(size)?;

    let (input_read, input_write) = pipe()?;
    let (output_read, output_write) = pipe()?;

    let mut hpc: HPCON = 0;
    // SAFETY: both handles are open pipe ends and `hpc` is a valid out pointer.
    let created = unsafe {
        (api.create)(
            coord,
            input_read.as_raw_handle(),
            output_write.as_raw_handle(),
            PSEUDOCONSOLE_FLAGS,
            &mut hpc,
        )
    };
    check_hresult(created, "CreatePseudoConsole")?;
    let console = Arc::new(Console {
        api,
        handle: Mutex::new(Some(hpc)),
    });
    // The console host has its own copies of these two ends. Ours must go, or the pipes
    // would never report that the host has closed them.
    drop(input_read);
    drop(output_write);

    let mut attributes = AttributeList::with_pseudoconsole(hpc)?;
    // SAFETY: the structure is plain data for which all-zero is a valid value.
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    // Explicitly no standard handles. Without this, a caller whose own output is
    // redirected to a file or pipe passes that redirection on, and the child writes
    // there instead of to the pseudoconsole.
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
    startup.lpAttributeList = attributes.as_ptr();

    // SAFETY: plain data for which all-zero is a valid value.
    let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: every string is NUL-terminated and outlives the call; the command line is
    // mutable as the function requires; the environment block is UTF-16 as the flag
    // says; `startup` is an extended structure as the other flag says, and its attribute
    // list outlives the call. No handles are inherited: the child gets its console
    // through the attribute.
    let started = unsafe {
        CreateProcessW(
            program_wide.as_ptr(),
            line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            environment_block.as_ptr().cast(),
            cwd_wide.as_ptr(),
            &startup.StartupInfo,
            &mut process,
        )
    };
    if started == 0 {
        let error = io::Error::last_os_error();
        return Err(io::Error::new(
            error.kind(),
            format!("starting `{}`: {error}", program.display()),
        ));
    }
    // SAFETY: on success both are open handles that nothing else owns. The thread handle
    // is not needed and is closed by dropping it.
    let process_handle = unsafe {
        drop(OwnedHandle::from_raw_handle(process.hThread));
        OwnedHandle::from_raw_handle(process.hProcess)
    };

    let end = ChildEnd::new(process_handle, Arc::clone(&console));
    let pty = WinPty {
        console,
        end: end.clone(),
        reader: Some(File::from(output_read)),
        writer: Some(File::from(input_write)),
    };
    let child = WinChild::new(end, process.dwProcessId, program);
    Ok((pty, child))
}

pub(crate) struct WinPty {
    console: Arc<Console>,
    /// How to end the program when the terminal is dropped.
    end: ChildEnd,
    reader: Option<File>,
    writer: Option<File>,
}

fn taken(what: &str) -> io::Error {
    io::Error::other(format!("the terminal's {what} was already taken"))
}

impl Pty for WinPty {
    fn take_reader(&mut self) -> io::Result<Box<dyn PtyRead>> {
        let file = self.reader.take().ok_or_else(|| taken("reader"))?;
        Ok(Box::new(WinReader {
            shared: Arc::new(ReadShared {
                file,
                stopped: AtomicBool::new(false),
                reading: AtomicBool::new(false),
            }),
        }))
    }

    fn take_writer(&mut self) -> io::Result<Box<dyn Write + Send>> {
        let file = self.writer.take().ok_or_else(|| taken("writer"))?;
        Ok(Box::new(file))
    }

    fn resize(&self, size: PtySize) -> io::Result<()> {
        self.console.resize(size)
    }
}

impl Drop for WinPty {
    fn drop(&mut self) {
        // The child holds the console too, so dropping our reference would not close
        // it. And closing it is not always enough: a program can miss the close, so
        // this also arranges for the program to be ended if it is still there later.
        if let Err(error) = self.end.end() {
            tracing::warn!(%error, "a dropped terminal's program may still be running");
        }
    }
}

/// The output pipe and the state a stopper shares with its reader. The file lives here
/// so the handle stays open for as long as anything might still cancel a read on it.
struct ReadShared {
    file: File,
    stopped: AtomicBool,
    /// Set while the reader is, or is about to be, inside the system's read call.
    reading: AtomicBool,
}

pub(crate) struct WinReader {
    shared: Arc<ReadShared>,
}

impl Read for WinReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let shared = &*self.shared;
        loop {
            // Announce the read, then look for a stop again. A stopper that set its flag
            // before the announcement is seen here; one that sets it after sees the
            // announcement and cancels. Either way a stop is never missed.
            shared.reading.store(true, Ordering::SeqCst);
            if shared.stopped.load(Ordering::SeqCst) {
                shared.reading.store(false, Ordering::SeqCst);
                return Ok(0);
            }
            let result = (&shared.file).read(buf);
            shared.reading.store(false, Ordering::SeqCst);

            match result {
                // The read was cancelled. If that was a stop, the next turn returns.
                Err(error) if error.raw_os_error() == Some(ERROR_OPERATION_ABORTED as i32) => {}
                // The console host has gone and everything it wrote has been read.
                Err(error) if error.kind() == io::ErrorKind::BrokenPipe => return Ok(0),
                other => return other,
            }
        }
    }
}

impl PtyRead for WinReader {
    fn stopper(&self) -> Box<dyn ReadStop> {
        Box::new(WinReadStop {
            shared: Arc::clone(&self.shared),
        })
    }
}

struct WinReadStop {
    shared: Arc<ReadShared>,
}

impl ReadStop for WinReadStop {
    fn stop(&self) {
        let shared = &*self.shared;
        shared.stopped.store(true, Ordering::SeqCst);
        // Cancelling does nothing if the reader has announced a read but not yet entered
        // it, so keep cancelling until the reader is out.
        let deadline = Instant::now() + STOP_TIMEOUT;
        while shared.reading.load(Ordering::SeqCst) && Instant::now() < deadline {
            // SAFETY: the handle is open for as long as `shared` is alive. A null
            // overlapped pointer cancels every request on the handle, from any thread.
            unsafe { CancelIoEx(shared.file.as_raw_handle(), std::ptr::null()) };
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
