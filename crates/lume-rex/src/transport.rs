//! Reaching a server: the stream of bytes a connection is made over.

use std::io;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use rustls::ClientConfig;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::endpoint::{Endpoint, HTTP_PORT, HTTPS_PORT, REX_PORT, authority};
use crate::error::RexError;
use crate::ssh::{SshOptions, Tunnel};
use crate::tls::{self, Trust};
use crate::tuning::{
    CONTROL_PATH, CONTROL_SUBPROTOCOL, MESSAGE_FLOOR_BYTES, PROBE_CLOSE_TIMEOUT, PROBE_TIMEOUT,
};
use crate::ws;

/// A stream of bytes to a server and back.
pub trait Io: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Io for T {}

/// What a server is called in a request when its stream is not found by a name: a pipe
/// or a socket on this machine. A server takes any name there.
pub const LOCAL_AUTHORITY: &str = "localhost";

/// A stream to a server, with nothing said on it yet.
pub struct Dialed {
    pub io: Box<dyn Io>,
    /// The host, and the port where it is not the usual one, that the server is asked
    /// for by in requests on this stream.
    pub authority: String,
}

impl Dialed {
    /// A stream to a server on this machine.
    pub fn local(io: impl Io) -> Self {
        Self::here(Box::new(io))
    }

    fn here(io: Box<dyn Io>) -> Self {
        Self {
            io,
            authority: LOCAL_AUTHORITY.to_owned(),
        }
    }
}

/// What the user has said of how a server may be reached.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Options {
    /// The user agreed, for this connection, to reach a server with nothing encrypted
    /// over a network. Without it such a server is not dialled. It is asked of the user
    /// each time and kept nowhere.
    pub plain_http_agreed: bool,
    /// Whose word is taken that a server behind TLS is the one its address names.
    pub trust: Trust,
    /// How the SSH client is run for a server reached through it.
    pub ssh: SshOptions,
}

impl Options {
    /// What holds when the user has been asked nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same, with the user's answer about an unencrypted connection.
    #[must_use]
    pub fn plain_http_agreed(mut self, agreed: bool) -> Self {
        self.plain_http_agreed = agreed;
        self
    }

    /// The same, with whose word is taken for a server behind TLS.
    #[must_use]
    pub fn trust(mut self, trust: Trust) -> Self {
        self.trust = trust;
        self
    }

    /// The same, with how the SSH client is run.
    #[must_use]
    pub fn ssh(mut self, ssh: SshOptions) -> Self {
        self.ssh = ssh;
        self
    }
}

fn unreachable(endpoint: &Endpoint, source: io::Error) -> RexError {
    RexError::Unreachable {
        endpoint: endpoint.to_string(),
        source,
    }
}

/// Opens a stream to the server at an endpoint, with the user having been asked
/// nothing. Nothing is said on it yet.
pub async fn dial(endpoint: &Endpoint) -> Result<Box<dyn Io>, RexError> {
    Ok(dial_with(endpoint, &Options::default()).await?.io)
}

/// Whether anything listens where a server on this machine does. Nothing is started,
/// and nothing is said to what answers.
pub async fn listens(endpoint: &Endpoint) -> bool {
    endpoint.is_on_this_machine() && dial(endpoint).await.is_ok()
}

/// Opens a stream to the server at an endpoint. Nothing is said on it yet.
///
/// A server that would be reached with nothing encrypted is dialled only on this
/// machine, on a tailnet when the address says so, or when the user agreed.
///
/// A server reached through the SSH client is not dialled this way: its streams last
/// only as long as the client that carries them, which a [`Reach`] keeps.
pub async fn dial_with(endpoint: &Endpoint, options: &Options) -> Result<Dialed, RexError> {
    if let Endpoint::Ssh { .. } = endpoint {
        return Err(RexError::Unsupported {
            endpoint: endpoint.to_string(),
            why: "a stream through the SSH client is opened by what keeps the client".to_owned(),
        });
    }
    Reach::new(endpoint.clone(), options.clone()).dial().await
}

/// One of the ways a host is reached when its address does not say which.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Way {
    /// Behind TLS, on this port.
    Tls(u16),
    /// With nothing encrypted, on this port.
    Plain(u16),
}

/// The ways a host alone is tried, in order: behind TLS first, and on the port its
/// address names when it names one.
fn ways(port: Option<u16>) -> Vec<Way> {
    match port {
        Some(port) => vec![Way::Tls(port), Way::Plain(port)],
        None => vec![
            Way::Tls(HTTPS_PORT),
            Way::Tls(REX_PORT),
            Way::Plain(REX_PORT),
        ],
    }
}

/// Whether what went wrong on one way to a host says that the others are not to be
/// tried: a certificate that is not believed is never answered by doing without one,
/// and a server that was found and refuses is the server.
fn settles(error: &RexError) -> bool {
    matches!(
        error,
        RexError::Untrusted { .. }
            | RexError::Unconfirmed { .. }
            | RexError::Refused { .. }
            | RexError::Unsupported { .. }
            | RexError::NotInstalled { .. }
            | RexError::BadEndpoint(_)
    )
}

/// How one server is reached, for every stream opened to it: what the user said, what
/// encryption is set up with, and which way a host alone was found by.
pub struct Reach {
    endpoint: Endpoint,
    /// The endpoint as it is written, for what is said of failures.
    name: String,
    options: Options,
    tls: OnceLock<Result<Arc<ClientConfig>, String>>,
    /// The way a host alone answered as a Rex server, until it stops doing so.
    proved: Mutex<Option<Way>>,
    /// The SSH client that carries the streams, for an address that is reached so.
    tunnel: Option<Tunnel>,
}

impl Reach {
    pub fn new(endpoint: Endpoint, mut options: Options) -> Self {
        let name = endpoint.to_string();
        // Handed to what asks, and held nowhere beside it.
        let ssh = std::mem::take(&mut options.ssh);
        let tunnel = match &endpoint {
            Endpoint::Ssh {
                user,
                host,
                port,
                socket,
            } => Some(Tunnel::new(
                name.clone(),
                user.clone(),
                host.clone(),
                *port,
                socket.clone(),
                ssh,
            )),
            _ => None,
        };
        Self {
            tunnel,
            name,
            endpoint,
            options,
            tls: OnceLock::new(),
            proved: Mutex::new(None),
        }
    }

    /// Ends what the streams run through, if anything does. None is opened after this.
    pub fn close(&self) {
        if let Some(tunnel) = &self.tunnel {
            tunnel.close();
        }
    }

    /// Opens a stream to the server. Nothing is said on it yet.
    pub async fn dial(&self) -> Result<Dialed, RexError> {
        let endpoint = &self.endpoint;
        match endpoint {
            Endpoint::Local => platform::local(endpoint).await.map(Dialed::here),
            Endpoint::Pipe(name) => platform::pipe(endpoint, name).await.map(Dialed::here),
            Endpoint::Unix(path) => platform::unix(endpoint, path).await.map(Dialed::here),
            Endpoint::Http { host, port } => self.plain(host, *port, self.asked()).await,
            Endpoint::Tailnet { host, port } => self.plain(host, *port, tcp::Plain::Tailnet).await,
            Endpoint::Https { host, port } => self.secure(host, *port).await,
            Endpoint::Host { host, port } => self.found(host, *port).await,
            Endpoint::Ssh { .. } => match &self.tunnel {
                Some(tunnel) => tunnel.dial().await,
                None => Err(RexError::BadEndpoint(self.name.clone())),
            },
        }
    }

    /// The rule for a stream with nothing encrypted on it, as the user answered.
    fn asked(&self) -> tcp::Plain {
        tcp::Plain::Asked {
            agreed: self.options.plain_http_agreed,
        }
    }

    /// What TLS is set up with, made once.
    fn tls(&self) -> Result<&Arc<ClientConfig>, RexError> {
        let made = self
            .tls
            .get_or_init(|| tls::config(&self.options.trust).map_err(|error| error.to_string()));
        made.as_ref().map_err(|error| RexError::Unsupported {
            endpoint: self.name.clone(),
            why: format!("encryption could not be set up: {error}"),
        })
    }

    async fn plain(&self, host: &str, port: u16, rule: tcp::Plain) -> Result<Dialed, RexError> {
        let io = tcp::plain(&self.endpoint, host, port, rule).await?;
        Ok(Dialed {
            io: Box::new(io),
            authority: authority(host, port, HTTP_PORT),
        })
    }

    async fn secure(&self, host: &str, port: u16) -> Result<Dialed, RexError> {
        let config = self.tls()?;
        let addresses = tcp::resolve(&self.endpoint, host, port).await?;
        let stream = tcp::connect(&self.endpoint, &addresses).await?;
        let io = tls::begin(&self.name, config, host, stream).await?;
        Ok(Dialed {
            io: Box::new(io),
            authority: authority(host, port, HTTPS_PORT),
        })
    }

    async fn by(&self, host: &str, way: Way) -> Result<Dialed, RexError> {
        match way {
            Way::Tls(port) => self.secure(host, port).await,
            Way::Plain(port) => self.plain(host, port, self.asked()).await,
        }
    }

    fn proved(&self) -> MutexGuard<'_, Option<Way>> {
        self.proved.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A stream to a host whose address does not say how it is reached. The first time,
    /// and again after the way that was found fails, each way is tried in turn until a
    /// Rex server answers on one.
    async fn found(&self, host: &str, port: Option<u16>) -> Result<Dialed, RexError> {
        let known = *self.proved();
        if let Some(way) = known {
            let dialed = self.by(host, way).await;
            if dialed.is_err() {
                *self.proved() = None;
            }
            return dialed;
        }
        let mut last = None;
        for way in ways(port) {
            let proof = tokio::time::timeout(PROBE_TIMEOUT, self.prove(host, way)).await;
            match proof {
                Ok(Ok(())) => {
                    tracing::debug!(server = %self.name, ?way, "a host was found to be a Rex server");
                    *self.proved() = Some(way);
                    return self.by(host, way).await;
                }
                Ok(Err(error)) if settles(&error) => return Err(error),
                Ok(Err(error)) => last = Some(error),
                Err(_) => last = Some(unreachable(&self.endpoint, io::ErrorKind::TimedOut.into())),
            }
        }
        Err(last.unwrap_or_else(|| RexError::BadEndpoint(self.name.clone())))
    }

    /// Whether a Rex server answers on one way to a host: it does when it agrees to a
    /// control connection there. Nothing more is said on that connection.
    async fn prove(&self, host: &str, way: Way) -> Result<(), RexError> {
        let dialed = self.by(host, way).await?;
        let mut socket = ws::open(
            dialed.io,
            &self.name,
            &dialed.authority,
            CONTROL_PATH,
            CONTROL_SUBPROTOCOL,
            MESSAGE_FLOOR_BYTES,
        )
        .await?;
        let _ = tokio::time::timeout(PROBE_CLOSE_TIMEOUT, socket.close(None)).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_alone_is_tried_behind_tls_first() {
        assert_eq!(
            ways(None),
            [Way::Tls(443), Way::Tls(7297), Way::Plain(7297)]
        );
        assert_eq!(ways(Some(8443)), [Way::Tls(8443), Way::Plain(8443)]);
    }

    #[test]
    fn what_settles_a_host_is_not_passed_over() {
        let endpoint = || "rex.example".to_owned();
        let reason = || "why".to_owned();
        for settled in [
            RexError::Untrusted {
                endpoint: endpoint(),
                reason: reason(),
            },
            RexError::Unconfirmed {
                endpoint: endpoint(),
            },
            RexError::Refused {
                endpoint: endpoint(),
                reason: reason(),
            },
        ] {
            assert!(settles(&settled), "{settled}");
        }
        for passed in [
            unreachable(&Endpoint::Local, io::ErrorKind::ConnectionRefused.into()),
            RexError::Protocol(reason()),
            RexError::Timeout(reason()),
        ] {
            assert!(!settles(&passed), "{passed}");
        }
    }
}

/// Streams over a network.
mod tcp {
    use std::io;
    use std::net::{IpAddr, SocketAddr};

    use tokio::net::TcpStream;

    use super::unreachable;
    use crate::endpoint::Endpoint;
    use crate::error::RexError;
    use crate::tuning::DIAL_TIMEOUT;

    /// When a stream with nothing encrypted on it may be opened.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Plain {
        /// On this machine always; elsewhere only if the user agreed.
        Asked { agreed: bool },
        /// Only to an address of a tailnet, which encrypts what passes.
        Tailnet,
    }

    /// Whether an address is this machine's own.
    fn is_loopback(address: IpAddr) -> bool {
        address.to_canonical().is_loopback()
    }

    /// Whether an address is one Tailscale gives the machines of a tailnet:
    /// `100.64.0.0/10` or `fd7a:115c:a1e0::/48`.
    fn is_tailnet(address: IpAddr) -> bool {
        match address.to_canonical() {
            IpAddr::V4(address) => {
                let [first, second, ..] = address.octets();
                first == 100 && second & 0xc0 == 64
            }
            IpAddr::V6(address) => address.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
        }
    }

    /// Whether a stream with nothing encrypted on it may be opened to these addresses.
    /// Every one of them has to pass: which is used is not this client's to choose.
    pub(super) fn check(
        endpoint: &Endpoint,
        rule: Plain,
        addresses: &[SocketAddr],
    ) -> Result<(), RexError> {
        let all = |passes: fn(IpAddr) -> bool| addresses.iter().all(|at| passes(at.ip()));
        match rule {
            Plain::Asked { agreed } if agreed || all(is_loopback) => Ok(()),
            Plain::Asked { .. } => Err(RexError::Unconfirmed {
                endpoint: endpoint.to_string(),
            }),
            Plain::Tailnet if all(is_tailnet) => Ok(()),
            Plain::Tailnet => Err(RexError::Unsupported {
                endpoint: endpoint.to_string(),
                why: "the host is not at an address of a tailnet, so nothing would encrypt the \
                      connection"
                    .to_owned(),
            }),
        }
    }

    /// The addresses of a host, looked up once: what is checked is what is dialled.
    pub(super) async fn resolve(
        endpoint: &Endpoint,
        host: &str,
        port: u16,
    ) -> Result<Vec<SocketAddr>, RexError> {
        if let Ok(address) = host.parse::<IpAddr>() {
            return Ok(vec![SocketAddr::new(address, port)]);
        }
        let found = tokio::time::timeout(DIAL_TIMEOUT, tokio::net::lookup_host((host, port)))
            .await
            .map_err(|_| unreachable(endpoint, io::ErrorKind::TimedOut.into()))?
            .map_err(|error| unreachable(endpoint, error))?;
        let addresses: Vec<SocketAddr> = found.collect();
        if addresses.is_empty() {
            return Err(unreachable(
                endpoint,
                io::Error::new(io::ErrorKind::NotFound, "the host has no address"),
            ));
        }
        Ok(addresses)
    }

    /// A stream to the first of the addresses that takes one.
    pub(super) async fn connect(
        endpoint: &Endpoint,
        addresses: &[SocketAddr],
    ) -> Result<TcpStream, RexError> {
        let each = async {
            let mut last = io::Error::from(io::ErrorKind::NotFound);
            for address in addresses {
                match TcpStream::connect(address).await {
                    Ok(stream) => return Ok(stream),
                    Err(error) => last = error,
                }
            }
            Err(last)
        };
        let stream = tokio::time::timeout(DIAL_TIMEOUT, each)
            .await
            .map_err(|_| unreachable(endpoint, io::ErrorKind::TimedOut.into()))?
            .map_err(|error| unreachable(endpoint, error))?;
        // What is typed is sent as it is typed, not held back for more.
        let _ = stream.set_nodelay(true);
        Ok(stream)
    }

    /// A stream with nothing encrypted on it, where the rule lets one be opened.
    pub(super) async fn plain(
        endpoint: &Endpoint,
        host: &str,
        port: u16,
        rule: Plain,
    ) -> Result<TcpStream, RexError> {
        let addresses = resolve(endpoint, host, port).await?;
        check(endpoint, rule, &addresses)?;
        connect(endpoint, &addresses).await
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn at(addresses: &[&str]) -> Vec<SocketAddr> {
            addresses
                .iter()
                .map(|address| SocketAddr::new(address.parse().unwrap(), 7297))
                .collect()
        }

        fn code(rule: Plain, addresses: &[&str]) -> String {
            let endpoint = Endpoint::parse("http://rex.example:7297").unwrap();
            match check(&endpoint, rule, &at(addresses)) {
                Ok(()) => "ok".to_owned(),
                Err(error) => error.code().to_owned(),
            }
        }

        #[test]
        fn nothing_unencrypted_leaves_this_machine_unless_the_user_agreed() {
            let asked = Plain::Asked { agreed: false };
            assert_eq!(code(asked, &["127.0.0.1"]), "ok");
            assert_eq!(code(asked, &["127.8.9.10", "::1"]), "ok");
            assert_eq!(code(asked, &["::ffff:127.0.0.1"]), "ok");
            for elsewhere in [
                "10.0.0.5",
                "192.168.1.20",
                "100.100.1.2",
                "8.8.8.8",
                "0.0.0.0",
                "::",
                "fe80::1",
                "fd7a:115c:a1e0::1",
                "::ffff:10.0.0.5",
            ] {
                assert_eq!(
                    code(asked, &[elsewhere]),
                    "confirmation_required",
                    "{elsewhere}"
                );
            }
            // One address elsewhere among this machine's own is enough to ask.
            assert_eq!(
                code(asked, &["127.0.0.1", "10.0.0.5"]),
                "confirmation_required"
            );

            let agreed = Plain::Asked { agreed: true };
            assert_eq!(code(agreed, &["10.0.0.5"]), "ok");
        }

        #[test]
        fn a_tailnet_address_is_only_one_a_tailnet_gives() {
            for inside in [
                "100.64.0.1",
                "100.100.1.2",
                "100.127.255.254",
                "fd7a:115c:a1e0::1",
                "fd7a:115c:a1e0:ab12:4843:cd96:6263:1",
                "::ffff:100.64.0.1",
            ] {
                assert_eq!(code(Plain::Tailnet, &[inside]), "ok", "{inside}");
            }
            for outside in [
                "100.63.255.255",
                "100.128.0.1",
                "127.0.0.1",
                "10.0.0.5",
                "8.8.8.8",
                "fd7a:115c:a1e1::1",
                "fd00::1",
                "::1",
            ] {
                assert_eq!(code(Plain::Tailnet, &[outside]), "unsupported", "{outside}");
            }
            assert_eq!(
                code(Plain::Tailnet, &["100.64.0.1", "10.0.0.5"]),
                "unsupported"
            );
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::io;
    use std::path::Path;
    use std::time::Duration;

    use tokio::net::windows::named_pipe::ClientOptions;

    use super::{Io, unreachable};
    use crate::endpoint::Endpoint;
    use crate::error::RexError;

    /// Every instance of the pipe is taken by another client for the moment.
    const ERROR_PIPE_BUSY: i32 = 231;
    /// How often a busy pipe is tried again, and how long between tries.
    const BUSY_TRIES: u32 = 20;
    const BUSY_WAIT: Duration = Duration::from_millis(25);

    /// The pipe of the user's own server: `rex-` and the account's security identifier.
    pub(super) async fn local(endpoint: &Endpoint) -> Result<Box<dyn Io>, RexError> {
        let sid = lume_ipc::user_sid().map_err(|error| unreachable(endpoint, error))?;
        pipe(endpoint, &format!(r"\\.\pipe\rex-{sid}")).await
    }

    pub(super) async fn pipe(endpoint: &Endpoint, name: &str) -> Result<Box<dyn Io>, RexError> {
        let mut tries = 0;
        loop {
            // The pipe is opened so that the server can tell who is calling and no more
            // than that, which is what a newly made `ClientOptions` asks for. A server
            // that cannot tell refuses every request on the connection.
            match ClientOptions::new().open(name) {
                Ok(client) => return Ok(Box::new(client)),
                Err(error)
                    if error.raw_os_error() == Some(ERROR_PIPE_BUSY) && tries < BUSY_TRIES =>
                {
                    tries += 1;
                    tokio::time::sleep(BUSY_WAIT).await;
                }
                Err(error) => return Err(unreachable(endpoint, error)),
            }
        }
    }

    pub(super) async fn unix(endpoint: &Endpoint, _path: &Path) -> Result<Box<dyn Io>, RexError> {
        Err(unreachable(
            endpoint,
            io::Error::new(
                io::ErrorKind::Unsupported,
                "a Unix socket is not reached from Windows",
            ),
        ))
    }
}

#[cfg(unix)]
mod platform {
    use std::io;
    use std::os::unix::fs::FileTypeExt;
    use std::path::{Path, PathBuf};

    use tokio::net::UnixStream;

    use super::{Io, unreachable};
    use crate::endpoint::Endpoint;
    use crate::error::RexError;

    /// The directories a user's own server keeps its socket in, the likeliest first:
    /// the one the user named for it, the user's runtime directory, and the place a
    /// server falls back to where there is none.
    fn runtime_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Some(dir) = std::env::var_os("REX_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
            dirs.push(PathBuf::from(dir));
        }
        if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
            dirs.push(PathBuf::from(dir).join("rex"));
        }
        if let Some(home) = std::env::home_dir() {
            dirs.push(home.join(".local/state/rex/run"));
        }
        dirs
    }

    /// What a server calls its own socket. Others beside it are for other things, such
    /// as a login server.
    const SERVER_SOCKET: &str = "server.sock";

    /// The sockets in a directory: the one a server calls its own first, then the rest
    /// in the order of their names.
    fn sockets_in(dir: &Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut found: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_socket()))
            .map(|entry| entry.path())
            .collect();
        found.sort_by_key(|socket| {
            let own = socket.file_name().is_some_and(|name| name == SERVER_SOCKET);
            (!own, socket.clone())
        });
        found
    }

    /// The socket of the user's own server, in the first of its directories that has
    /// one a server answers on.
    pub(super) async fn local(endpoint: &Endpoint) -> Result<Box<dyn Io>, RexError> {
        let mut last = io::Error::new(
            io::ErrorKind::NotFound,
            "no socket of a Rex server was found in the user's runtime directory",
        );
        for socket in runtime_dirs().iter().flat_map(|dir| sockets_in(dir)) {
            match UnixStream::connect(&socket).await {
                Ok(stream) => return Ok(Box::new(stream)),
                Err(error) => last = error,
            }
        }
        Err(unreachable(endpoint, last))
    }

    pub(super) async fn unix(endpoint: &Endpoint, path: &Path) -> Result<Box<dyn Io>, RexError> {
        match UnixStream::connect(path).await {
            Ok(stream) => Ok(Box::new(stream)),
            Err(error) => Err(unreachable(endpoint, error)),
        }
    }

    pub(super) async fn pipe(endpoint: &Endpoint, _name: &str) -> Result<Box<dyn Io>, RexError> {
        Err(unreachable(
            endpoint,
            io::Error::new(
                io::ErrorKind::Unsupported,
                "a named pipe is only reached from Windows",
            ),
        ))
    }
}
