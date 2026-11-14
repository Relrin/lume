//! The endpoint on Windows: a named pipe that only the current user may open.
//!
//! The pipe's name ends in the user's security identifier, so each user has their own.
//! Its access list has one entry, for that identifier. The first instance is created
//! with the flag that makes creation fail if a pipe of that name already exists: without
//! it, a process that created the name earlier would own the access list and receive
//! the clients meant for us.

use std::fmt;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_INSUFFICIENT_BUFFER, ERROR_PIPE_BUSY, HANDLE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// The revision of the textual security-descriptor format used below.
const SDDL_REVISION: u32 = 1;

/// How long a client keeps trying while every instance of the pipe is taken. The server
/// makes a new instance as soon as it accepts a client, so this is a moment's wait.
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) type Address = String;

pub(crate) fn address(name: &str) -> io::Result<Address> {
    Ok(format!(r"\\.\pipe\lume-{name}-{}", user_sid()?))
}

pub(crate) fn display(address: &Address, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(address)
}

fn check(ok: windows_sys::core::BOOL) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// The current user's security identifier in its textual form, `S-1-5-21-…`.
fn user_sid() -> io::Result<&'static str> {
    static SID: OnceLock<String> = OnceLock::new();
    if let Some(sid) = SID.get() {
        return Ok(sid);
    }
    let sid = read_user_sid()?;
    Ok(SID.get_or_init(|| sid))
}

fn read_user_sid() -> io::Result<String> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the pseudo-handle for this process is always valid and `token` is a valid
    // out pointer.
    check(unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) })?;
    // SAFETY: on success `token` is an open handle that nothing else owns.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };

    let mut needed = 0;
    // SAFETY: a null buffer of length zero asks how large the buffer must be. That call
    // fails by design with ERROR_INSUFFICIENT_BUFFER.
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            std::ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    let error = io::Error::last_os_error();
    if ok != 0 || error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
        return Err(error);
    }

    // Eight-byte words, so the buffer is aligned for the pointer the structure holds.
    let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: the buffer holds at least `needed` bytes.
    check(unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    })?;
    // SAFETY: on success the buffer starts with a `TOKEN_USER`, for which it is aligned,
    // and the identifier it points to lies within the buffer.
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };

    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: the identifier is valid while `buffer` lives, and `text` is a valid out
    // pointer.
    check(unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) })?;
    // SAFETY: on success `text` is a NUL-terminated string allocated by the system. It
    // is read up to its terminator and then released exactly once.
    let sid = unsafe {
        let len = (0..).take_while(|&at| *text.add(at) != 0).count();
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
        LocalFree(text.cast());
        sid
    };
    Ok(sid)
}

/// A security descriptor that grants the current user everything and nobody else
/// anything.
struct OwnerOnly {
    descriptor: PSECURITY_DESCRIPTOR,
}

// SAFETY: the descriptor is a heap block that is never written after it is created, and
// the functions that read and free it may be called from any thread.
unsafe impl Send for OwnerOnly {}
// SAFETY: as above; shared access only ever reads it.
unsafe impl Sync for OwnerOnly {}

impl OwnerOnly {
    fn new() -> io::Result<Self> {
        // A protected access list (it inherits nothing) with a single entry: allow,
        // generic all, to this user.
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;{})\0", user_sid()?)
            .encode_utf16()
            .collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated and `descriptor` is a valid out pointer. The
        // size out pointer is optional.
        check(unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        })?;
        Ok(Self { descriptor })
    }
}

impl Drop for OwnerOnly {
    fn drop(&mut self) {
        // SAFETY: the descriptor was allocated by the system in `new` and is released
        // exactly once.
        unsafe { LocalFree(self.descriptor) };
    }
}

pub(crate) struct Listener {
    address: Address,
    security: OwnerOnly,
    /// The instance the next client will connect to. One always exists, so a client
    /// never finds the name missing between two accepts.
    next: NamedPipeServer,
}

impl Listener {
    pub(crate) fn bind(address: &Address) -> io::Result<Self> {
        let security = OwnerOnly::new()?;
        let next = create_instance(address, &security, true).map_err(|error| {
            if error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) {
                // Creating the first instance of a name that exists is refused this way.
                io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("the endpoint {address} is already being served"),
                )
            } else {
                error
            }
        })?;
        Ok(Self {
            address: address.clone(),
            security,
            next,
        })
    }

    pub(crate) async fn accept(&mut self) -> io::Result<Stream> {
        self.next.connect().await?;
        let following = create_instance(&self.address, &self.security, false)?;
        Ok(Stream::Server(std::mem::replace(&mut self.next, following)))
    }
}

fn create_instance(
    address: &str,
    security: &OwnerOnly,
    first: bool,
) -> io::Result<NamedPipeServer> {
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security.descriptor,
        bInheritHandle: 0,
    };
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first)
        .reject_remote_clients(true);
    // SAFETY: `attributes` is a valid `SECURITY_ATTRIBUTES` whose descriptor outlives
    // the call; the system copies what it needs.
    unsafe { options.create_with_security_attributes_raw(address, (&raw mut attributes).cast()) }
}

pub(crate) async fn connect(address: &Address) -> io::Result<Stream> {
    let deadline = Instant::now() + BUSY_TIMEOUT;
    loop {
        match ClientOptions::new().open(address) {
            Ok(client) => return Ok(Stream::Client(client)),
            // Every instance is taken by a client that connected a moment ago.
            Err(error)
                if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32)
                    && Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

pub(crate) enum Stream {
    Server(NamedPipeServer),
    Client(NamedPipeClient),
}

impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Server(pipe) => Pin::new(pipe).poll_read(cx, buf),
            Self::Client(pipe) => Pin::new(pipe).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Server(pipe) => Pin::new(pipe).poll_write(cx, buf),
            Self::Client(pipe) => Pin::new(pipe).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Server(pipe) => Pin::new(pipe).poll_flush(cx),
            Self::Client(pipe) => Pin::new(pipe).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Server(pipe) => Pin::new(pipe).poll_shutdown(cx),
            Self::Client(pipe) => Pin::new(pipe).poll_shutdown(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pipe_name_ends_in_the_users_security_identifier() {
        let sid = user_sid().unwrap();
        assert!(sid.starts_with("S-1-"), "{sid}");
        assert_eq!(address("app").unwrap(), format!(r"\\.\pipe\lume-app-{sid}"));
    }
}
