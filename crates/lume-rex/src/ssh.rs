//! A server on another machine, reached through the system's SSH client.
//!
//! A server listens on a socket only its own account can open. The SSH client logs in
//! as that account with the user's own keys, agent and configuration, and forwards a
//! place on this machine to that socket; every connection to the server is then a
//! stream to that place. Nothing listens on a network but the SSH server.
//!
//! The client runs out of sight. While a connection is first made it may ask the user
//! what it has to know, through the [`Prompter`] it was given: a password, the passphrase
//! of a key, whether an unknown machine is the one they mean. That is over with the first
//! stream to the server, or with the failure to open one. From then on, and without a
//! prompter from the start, the client is told never to ask anything: a machine it could
//! only be signed in to by asking fails with what the client said.
//!
//! The host and the account are words of an address, checked before they are used, and
//! the client is started from a list of arguments in which the destination follows
//! `--`: nothing in an address is ever read as an option or by a shell on this machine.

use std::io;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use lume_proto::{DisconnectReason, ExitStatus};
use lume_session::ssh::{Tail, check_word, classify, client};
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio::sync::watch;
use tokio::task::JoinSet;

use crate::error::RexError;
use crate::prompter::{Asking, Cut, PromptRun, Prompter};
use crate::transport::Dialed;
use crate::tuning::{
    SSH_ASK_TIMEOUT, SSH_CONNECT_SECONDS, SSH_PORT_TRIES, SSH_READY_POLL, SSH_REST_WAIT,
    SSH_SOCKET_MAX_BYTES, SSH_START_TIMEOUT, SSH_STATUS_MAX_BYTES,
};

/// What Rex is asked on the other machine to learn where the account's server listens.
/// It starts that server if none is running, which is what an `ssh://` address means.
const ASK: &str = "server status --json";
/// Where Rex is looked for there: on the `PATH` an SSH login gets, then where a server
/// installed for one account puts it.
const REX_PROGRAMS: [&str; 2] = ["rex", "~/.local/bin/rex"];
/// Where the server of one account listens when it was started with no runtime
/// directory to use, as the other machine's shell writes it.
const FALLBACK_SOCKET: &str = "~/.local/state/rex/run/server.sock";
/// What a shell exits with when it has no such command.
const NOT_FOUND: u32 = 127;

/// How the SSH client is run, beyond what an address says.
#[derive(Clone, Debug, Default)]
pub struct SshOptions {
    /// The client to run; the system's own when none.
    pub client: Option<PathBuf>,
    /// More arguments for the client, before the destination: the user's own, such as
    /// `-F` and a configuration file.
    pub args: Vec<String>,
    /// Whom the client may ask while the connection is first made; nobody when none.
    pub prompter: Option<Arc<dyn Prompter>>,
}

/// Where on this machine the forwarded socket is reached.
#[derive(Debug)]
enum Local {
    /// A port only this machine reaches. Every program on it does.
    #[cfg_attr(unix, allow(dead_code))]
    Port(u16),
    /// A socket in a directory only the user can enter.
    #[cfg(unix)]
    Socket(PathBuf),
}

/// How many SSH clients run.
static CLIENTS: LazyLock<watch::Sender<usize>> = LazyLock::new(|| watch::Sender::new(0));

/// Counts a client for as long as it is held. It is to be let go of after the client.
struct Counted;

impl Counted {
    fn new() -> Self {
        CLIENTS.send_modify(|clients| *clients += 1);
        Self
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        CLIENTS.send_modify(|clients| *clients -= 1);
    }
}

/// Comes back once every SSH client of this process has been ended, as they are when the
/// backends that use them are shut down.
pub async fn clients_ended() {
    let mut clients = CLIENTS.subscribe();
    let _ = clients.wait_for(|clients| *clients == 0).await;
}

/// A client that is running and forwarding.
struct Up {
    child: Child,
    local: Local,
    /// The end of what the client printed.
    said: Arc<Mutex<Tail>>,
    /// Last, so that it goes after the client.
    _counted: Counted,
}

impl Drop for Up {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        #[cfg(unix)]
        if let Local::Socket(path) = &self.local {
            let _ = std::fs::remove_file(path);
            if let Some(dir) = path.parent() {
                let _ = std::fs::remove_dir(dir);
            }
        }
    }
}

/// A way to one server through the SSH client. The client is started when a stream is
/// first asked for, serves every stream after that, and is started again when it has
/// ended. It ends with this.
pub(crate) struct Tunnel {
    /// The endpoint as it is written, for what is said of failures.
    name: String,
    user: Option<String>,
    host: String,
    port: Option<u16>,
    socket: Option<String>,
    options: SshOptions,
    /// Whom the client may ask, until the first stream was asked for. It is given up with
    /// it, so that nothing the user said is held once a connection was made or was not.
    prompter: Mutex<Option<Arc<dyn Prompter>>>,
    /// Whether no client is to be started any more.
    closed: AtomicBool,
    up: tokio::sync::Mutex<Option<Up>>,
}

/// What a server says of itself, as far as it is read here.
#[derive(Deserialize)]
struct Status {
    endpoint: Option<String>,
    listeners: Option<Vec<Listener>>,
}

#[derive(Deserialize)]
struct Listener {
    endpoint: Option<String>,
    addresses: Option<Vec<String>>,
    transport: Option<String>,
    #[serde(default)]
    primary: bool,
}

/// Whether a path can be handed to the client as the socket to forward to: a whole
/// path, with nothing the client would read as the end of it.
fn is_socket_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= SSH_SOCKET_MAX_BYTES
        && !path.contains(':')
        && !path.chars().any(char::is_control)
}

/// The Unix socket a server listens on, from what it printed when asked for its
/// status. `Err` says why there is none to forward to.
fn socket_in(printed: &[u8]) -> Result<String, String> {
    let text = String::from_utf8_lossy(printed);
    // A login may print before the command does.
    let json = match (text.find('{'), text.rfind('}')) {
        (Some(from), Some(to)) if from < to => &text[from..=to],
        _ => return Err("the server did not say where it listens".to_owned()),
    };
    let status: Status = serde_json::from_str(json)
        .map_err(|_| "what the server said of itself could not be read".to_owned())?;
    let path_of = |address: &str| {
        let path = address.strip_prefix("unix://").unwrap_or(address);
        is_socket_path(path).then(|| path.to_owned())
    };
    let mut listeners = status.listeners.unwrap_or_default();
    listeners.sort_by_key(|listener| !listener.primary);
    let listed = listeners
        .iter()
        .filter(|listener| listener.transport.as_deref() == Some("unix"))
        // A socket for a login server or for Tailscale Serve is a Unix socket too, and
        // is not where a client of the server connects.
        .filter(|listener| {
            let endpoint = listener.endpoint.as_deref().unwrap_or("unix://");
            endpoint.starts_with("unix://") || endpoint.starts_with('/')
        })
        .find_map(|listener| {
            let bound = listener.addresses.iter().flatten().map(String::as_str);
            bound.chain(listener.endpoint.as_deref()).find_map(path_of)
        });
    if let Some(path) = listed.or_else(|| status.endpoint.as_deref().and_then(path_of)) {
        return Ok(path);
    }
    if status
        .endpoint
        .is_some_and(|endpoint| endpoint.starts_with("npipe:"))
    {
        return Err(
            "the server listens on a named pipe, which the SSH client cannot forward to; \
             a server on Windows is reached over its network listener"
                .to_owned(),
        );
    }
    Err("the server listens on no socket that can be forwarded to".to_owned())
}

/// A client's exit as the application knows the end of a program.
fn exit_of(status: std::process::ExitStatus) -> ExitStatus {
    match status.code() {
        // The Windows client's own -1 is read as it is written there.
        Some(code) => ExitStatus::Code(code.cast_unsigned()),
        None => ExitStatus::Unknown,
    }
}

/// Reads what a client prints to its end, keeping the end of it.
async fn keep_tail(mut from: impl AsyncRead + Unpin, said: Arc<Mutex<Tail>>) {
    let mut buffer = [0; 1024];
    while let Ok(read) = from.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        said.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(&buffer[..read]);
    }
}

/// Reads what another machine printed, as far as a server would say something, for what
/// is read to be looked at when the client has ended.
async fn keep_answer(from: impl AsyncRead + Unpin, answer: Arc<Mutex<Vec<u8>>>) {
    let mut from = from.take(SSH_STATUS_MAX_BYTES as u64 + 1);
    let mut buffer = [0; 4096];
    while let Ok(read) = from.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        answer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(&buffer[..read]);
    }
}

impl Tunnel {
    /// A way to the server an `ssh://` address names. `socket` is the server's socket
    /// on that machine, when the address names one.
    pub(crate) fn new(
        name: String,
        user: Option<String>,
        host: String,
        port: Option<u16>,
        socket: Option<String>,
        mut options: SshOptions,
    ) -> Self {
        let prompter = Mutex::new(options.prompter.take());
        Self {
            name,
            user,
            host,
            port,
            socket,
            options,
            prompter,
            closed: AtomicBool::new(false),
            up: tokio::sync::Mutex::new(None),
        }
    }

    /// Ends the client, if one runs. One that is being started is ended by whoever gives
    /// up waiting for it.
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Ok(mut up) = self.up.try_lock() {
            *up = None;
        }
    }

    fn unsupported(&self, why: impl Into<String>) -> RexError {
        RexError::Unsupported {
            endpoint: self.name.clone(),
            why: why.into(),
        }
    }

    fn unreachable(&self, why: impl Into<String>) -> RexError {
        RexError::Unreachable {
            endpoint: self.name.clone(),
            source: io::Error::other(why.into()),
        }
    }

    /// Why something of the client's was not waited for to its end.
    fn cut(&self, cut: Cut) -> RexError {
        match cut {
            Cut::Late => RexError::Unreachable {
                endpoint: self.name.clone(),
                source: io::ErrorKind::TimedOut.into(),
            },
            Cut::GivenUp => RexError::GivenUp {
                endpoint: self.name.clone(),
            },
        }
    }

    /// The arguments every run of the client begins with. The destination is not among
    /// them: it comes last, after `--`. A run that `asks` is not told to ask nothing.
    fn common(&self, asks: bool) -> Result<Vec<String>, RexError> {
        check_word("host", &self.host).map_err(|error| self.unsupported(error.message))?;
        if let Some(user) = &self.user {
            check_word("user's name", user).map_err(|error| self.unsupported(error.message))?;
        }
        let mut args = Vec::new();
        if !asks {
            // It runs out of sight, with nobody to answer.
            args.extend(["-o".to_owned(), "BatchMode=yes".to_owned()]);
        }
        for option in [
            format!("ConnectTimeout={SSH_CONNECT_SECONDS}"),
            // A connection of the user's own that others share is not this one's to
            // add a forward to, or to end.
            "ControlPath=none".to_owned(),
            "ServerAliveInterval=15".to_owned(),
            "ServerAliveCountMax=3".to_owned(),
        ] {
            args.extend(["-o".to_owned(), option]);
        }
        args.extend(["-T".to_owned(), "-n".to_owned()]);
        // The client keeps the first value an option is given, so nothing of the user's
        // own that follows undoes what is set above.
        args.extend(self.options.args.iter().cloned());
        if let Some(port) = self.port {
            args.extend(["-p".to_owned(), port.to_string()]);
        }
        Ok(args)
    }

    /// The account and machine, as the client takes them.
    fn destination(&self) -> String {
        match &self.user {
            Some(user) => format!("{user}@{}", self.host),
            None => self.host.clone(),
        }
    }

    /// The arguments that ask the other machine one thing and end.
    fn asking(&self, what: &str, asks: bool) -> Result<Vec<String>, RexError> {
        let mut args = self.common(asks)?;
        args.extend(["--".to_owned(), self.destination(), what.to_owned()]);
        Ok(args)
    }

    /// The arguments that forward `local` to the server's socket and do nothing else.
    fn forwarding(&self, local: &Local, socket: &str, asks: bool) -> Result<Vec<String>, RexError> {
        let mut args = self.common(asks)?;
        let forward = match local {
            Local::Port(port) => format!("127.0.0.1:{port}:{socket}"),
            #[cfg(unix)]
            Local::Socket(path) => format!("{}:{socket}", path.display()),
        };
        for option in ["ExitOnForwardFailure=yes", "StreamLocalBindUnlink=yes"] {
            args.extend(["-o".to_owned(), option.to_owned()]);
        }
        args.extend(["-N".to_owned(), "-L".to_owned(), forward]);
        args.extend(["--".to_owned(), self.destination()]);
        Ok(args)
    }

    /// The client, ready to be started with `args`, and to ask through `run` if it may.
    fn command(&self, args: Vec<String>, run: Option<&PromptRun>) -> Result<Command, RexError> {
        let program = self.options.client.clone().or_else(client).ok_or_else(|| {
            self.unsupported("this machine has no OpenSSH client to reach the server with")
        })?;
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(run) = run {
            command.envs(run.env.iter().map(|(name, value)| (name, value)));
        }
        #[cfg(windows)]
        {
            // No console window flashes up for a program that is one.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        Ok(command)
    }

    /// What a client that ended says went wrong. An account that was not let in and a
    /// machine whose key is not known stay as they are until the user does something;
    /// anything else may pass.
    fn failure(&self, status: std::process::ExitStatus, said: &Tail) -> RexError {
        let ended = classify(exit_of(status), said);
        let detail = ended
            .detail
            .unwrap_or_else(|| format!("the SSH client ended with {status}"));
        match ended.reason {
            DisconnectReason::AuthFailed | DisconnectReason::HostKey => RexError::Ssh {
                endpoint: self.name.clone(),
                reason: detail.trim_end_matches('.').to_owned(),
            },
            _ => self.unreachable(detail),
        }
    }

    /// Runs the client to ask the other machine one thing. Gives how it ended, what it
    /// printed, and the end of what it said beside that.
    async fn ask(
        &self,
        what: &str,
        asking: &Asking,
    ) -> Result<(std::process::ExitStatus, Vec<u8>, Tail), RexError> {
        // Before the client, and so let go of after it: a client that is told that nobody
        // answers tries an empty password, which the other machine takes for a failure to
        // sign in. It is ended first.
        let run = asking.run();
        let _counted = Counted::new();
        let mut child = self
            .command(self.asking(what, run.is_some())?, run.as_ref())?
            .spawn()
            .map_err(|error| self.unreachable(format!("the SSH client did not start: {error}")))?;
        let said = Arc::new(Mutex::new(Tail::new()));
        let answer = Arc::new(Mutex::new(Vec::new()));
        // Read beside the client, not by it: a program it started may hold on to what it
        // printed to after it has ended.
        let mut readers = JoinSet::new();
        if let Some(printed) = child.stdout.take() {
            readers.spawn(keep_answer(printed, Arc::clone(&answer)));
        }
        if let Some(errors) = child.stderr.take() {
            readers.spawn(keep_tail(errors, Arc::clone(&said)));
        }
        let status = match asking.within(SSH_ASK_TIMEOUT, child.wait()).await {
            Ok(status) => status.map_err(|error| self.unreachable(error.to_string()))?,
            Err(cut) => {
                let _ = child.start_kill();
                return Err(self.cut(cut));
            }
        };
        let rest = async { while readers.join_next().await.is_some() {} };
        let _ = tokio::time::timeout(SSH_REST_WAIT, rest).await;
        let answer = std::mem::take(&mut *answer.lock().unwrap_or_else(PoisonError::into_inner));
        if answer.len() > SSH_STATUS_MAX_BYTES {
            return Err(self.unreachable("the other machine printed more than a server says"));
        }
        let said = std::mem::take(&mut *said.lock().unwrap_or_else(PoisonError::into_inner));
        Ok((status, answer, said))
    }

    /// The socket to forward to: the one the address names, or the one the account's
    /// own server says it listens on.
    async fn socket(&self, asking: &Asking) -> Result<String, RexError> {
        if let Some(socket) = &self.socket {
            return if is_socket_path(socket) {
                Ok(socket.clone())
            } else {
                Err(self.unsupported(
                    "the socket in the address is not a path that can be forwarded to",
                ))
            };
        }
        // Where `rex` is: on the `PATH` an SSH login gets, or where a server installed for
        // one account puts it.
        let mut asked = None;
        for rex in REX_PROGRAMS {
            let (status, answer, said) = self.ask(&format!("{rex} {ASK}"), asking).await?;
            if exit_of(status) != ExitStatus::Code(NOT_FOUND) {
                asked = Some((rex, status, answer, said));
                break;
            }
        }
        let Some((rex, status, answer, said)) = asked else {
            return Err(RexError::NotInstalled {
                endpoint: self.name.clone(),
            });
        };
        if status.success() {
            return socket_in(&answer).map_err(|why| self.unsupported(why));
        }
        if classify(exit_of(status), &said).reason != DisconnectReason::Exited {
            return Err(self.failure(status, &said));
        }
        // Rex ran and found no server where a login over SSH has it look. A server that
        // was started from a shell with no runtime directory listens in the other place
        // a server of one account has; that one is asked, and nothing is started there.
        let elsewhere = format!("{rex} --autostart=false -S {FALLBACK_SOCKET} {ASK}");
        let (again, answer, _) = self.ask(&elsewhere, asking).await?;
        if again.success() {
            return socket_in(&answer).map_err(|why| self.unsupported(why));
        }
        let why = classify(ExitStatus::Code(255), &said).detail;
        Err(self.unreachable(format!(
            "`{rex} {ASK}` failed on the other machine: {}",
            why.unwrap_or_else(|| status.to_string())
        )))
    }

    /// A place on this machine nothing listens on yet.
    async fn place(&self) -> Result<Local, RexError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            use std::sync::atomic::{AtomicU32, Ordering};

            static SERIAL: AtomicU32 = AtomicU32::new(0);
            let base = std::env::var_os("XDG_RUNTIME_DIR")
                .filter(|dir| !dir.is_empty())
                .map_or_else(std::env::temp_dir, PathBuf::from);
            let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
            let dir = base.join(format!("lume-rex-{}-{serial}", std::process::id()));
            // Nobody but the user enters the directory, so nobody else reaches the
            // server through the socket in it.
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&dir)
                .map_err(|error| self.unreachable(error.to_string()))?;
            Ok(Local::Socket(dir.join("s")))
        }
        #[cfg(not(unix))]
        {
            // The port is free now and is taken by the client a moment later. If
            // something else takes it between, the client says so and another is tried.
            let held = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .and_then(|held| held.local_addr())
                .map_err(|error| self.unreachable(error.to_string()))?;
            Ok(Local::Port(held.port()))
        }
    }

    /// A stream to the place the client forwards.
    async fn connect(local: &Local) -> io::Result<Dialed> {
        match local {
            Local::Port(port) => {
                let stream = tokio::net::TcpStream::connect(("127.0.0.1", *port)).await?;
                let _ = stream.set_nodelay(true);
                Ok(Dialed::local(stream))
            }
            #[cfg(unix)]
            Local::Socket(path) => {
                let stream = tokio::net::UnixStream::connect(path).await?;
                Ok(Dialed::local(stream))
            }
        }
    }

    /// Starts the client forwarding to `socket`, and waits until a stream through it
    /// can be opened. `Ok(None)` when the place on this machine was taken meanwhile.
    async fn start(&self, socket: &str, asking: &Asking) -> Result<Option<(Up, Dialed)>, RexError> {
        let local = self.place().await?;
        // Let go of when the client forwards: nothing is asked of the user after that. If
        // it never does, the client is ended first, as above.
        let run = asking.run();
        let mut child = self
            .command(
                self.forwarding(&local, socket, run.is_some())?,
                run.as_ref(),
            )?
            .spawn()
            .map_err(|error| self.unreachable(format!("the SSH client did not start: {error}")))?;
        let said = Arc::new(Mutex::new(Tail::new()));
        if let Some(errors) = child.stderr.take() {
            tokio::spawn(keep_tail(errors, Arc::clone(&said)));
        }
        let mut up = Up {
            child,
            local,
            said,
            _counted: Counted::new(),
        };
        let ready = async {
            loop {
                if let Ok(dialed) = Self::connect(&up.local).await {
                    return Ok(dialed);
                }
                if let Ok(Some(status)) = up.child.try_wait() {
                    return Err(status);
                }
                tokio::time::sleep(SSH_READY_POLL).await;
            }
        };
        match asking.within(SSH_START_TIMEOUT, ready).await {
            Ok(Ok(dialed)) => Ok(Some((up, dialed))),
            Ok(Err(status)) => {
                // What the client said last is read once it has all been taken.
                tokio::time::sleep(SSH_READY_POLL).await;
                let said = up.said.lock().unwrap_or_else(PoisonError::into_inner);
                let taken = classify(ExitStatus::Code(255), &said)
                    .detail
                    .is_some_and(|line| line.contains("in use") || line.contains("cannot listen"));
                if taken {
                    return Ok(None);
                }
                Err(self.failure(status, &said))
            }
            Err(cut) => Err(self.cut(cut)),
        }
    }

    /// A stream to the server. The client is started first if it is not running.
    pub(crate) async fn dial(&self) -> Result<Dialed, RexError> {
        let mut up = self.up.lock().await;
        if self.closed.load(Ordering::SeqCst) {
            return Err(RexError::Disconnected);
        }
        if let Some(running) = up.as_mut() {
            if matches!(running.child.try_wait(), Ok(None))
                && let Ok(dialed) = Self::connect(&running.local).await
            {
                return Ok(dialed);
            }
            tracing::debug!(server = %self.name, "the SSH client that reached a server has ended");
            *up = None;
        }
        let prompter = {
            let mut prompter = self.prompter.lock().unwrap_or_else(PoisonError::into_inner);
            Asking::of(prompter.take())
        };
        let socket = self.socket(&prompter).await?;
        for _ in 0..SSH_PORT_TRIES {
            if let Some((started, dialed)) = self.start(&socket, &prompter).await? {
                tracing::debug!(server = %self.name, "an SSH client reaches a server");
                if self.closed.load(Ordering::SeqCst) {
                    return Err(RexError::Disconnected);
                }
                *up = Some(started);
                return Ok(dialed);
            }
        }
        Err(self.unreachable("no place on this machine could be forwarded from"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tunnel(address: &str) -> Tunnel {
        let crate::endpoint::Endpoint::Ssh {
            user,
            host,
            port,
            socket,
        } = crate::endpoint::Endpoint::parse(address).unwrap()
        else {
            panic!("{address} is no ssh address");
        };
        let options = SshOptions {
            client: None,
            args: vec!["-F".to_owned(), "its.conf".to_owned()],
            prompter: None,
        };
        Tunnel::new(address.to_owned(), user, host, port, socket, options)
    }

    #[test]
    fn the_client_asks_nothing_and_the_destination_is_never_an_option() {
        let asking = tunnel("ssh://alice@build.example.com:2222")
            .asking("rex server status --json", false)
            .unwrap();
        // What makes the client ask nothing comes before the user's own arguments: the
        // client keeps the first value an option is given.
        assert_eq!(asking[..2], ["-o", "BatchMode=yes"]);
        let own = asking.iter().position(|arg| arg == "-F").unwrap();
        assert_eq!(asking[own..own + 2], ["-F", "its.conf"]);
        assert!(own > asking.iter().rposition(|arg| arg == "-o").unwrap());
        let options: Vec<&str> = asking
            .windows(2)
            .filter(|pair| pair[0] == "-o")
            .map(|pair| pair[1].as_str())
            .collect();
        assert_eq!(
            options,
            [
                "BatchMode=yes",
                "ConnectTimeout=10",
                "ControlPath=none",
                "ServerAliveInterval=15",
                "ServerAliveCountMax=3"
            ]
        );
        let end = &asking[asking.len() - 5..];
        assert_eq!(
            end,
            [
                "-p",
                "2222",
                "--",
                "alice@build.example.com",
                "rex server status --json"
            ]
        );
    }

    #[test]
    fn a_client_that_may_ask_is_told_everything_else_the_same() {
        let tunnel = tunnel("ssh://alice@build");
        let quiet = tunnel.asking("rex", false).unwrap();
        let asks = tunnel.asking("rex", true).unwrap();
        assert_eq!(quiet[..2], ["-o", "BatchMode=yes"]);
        assert_eq!(quiet[2..], asks[..]);
        assert!(!asks.iter().any(|arg| arg.contains("BatchMode")));
    }

    #[test]
    fn the_client_forwards_and_does_nothing_else() {
        let tunnel = tunnel("ssh://build/run/rex/server.sock");
        let forwarding = tunnel
            .forwarding(&Local::Port(50123), "/run/rex/server.sock", false)
            .unwrap();
        let end = &forwarding[forwarding.len() - 5..];
        assert_eq!(
            end,
            [
                "-N",
                "-L",
                "127.0.0.1:50123:/run/rex/server.sock",
                "--",
                "build"
            ]
        );
        assert!(forwarding.contains(&"ExitOnForwardFailure=yes".to_owned()));
        assert!(forwarding.contains(&"-n".to_owned()));
    }

    #[test]
    fn a_word_the_client_would_read_as_an_option_never_reaches_it() {
        // An address is read so that no such word is in it; a way made from words that
        // were not read from an address refuses them all the same.
        let options = SshOptions::default();
        let named = |user: Option<&str>, host: &str| {
            Tunnel::new(
                "test".to_owned(),
                user.map(str::to_owned),
                host.to_owned(),
                None,
                None,
                options.clone(),
            )
        };
        for tunnel in [
            named(None, "-oProxyCommand=calc"),
            named(None, ""),
            named(Some("-l"), "host"),
            named(Some("a\nb"), "host"),
        ] {
            let refused = tunnel.asking("rex", true).unwrap_err();
            assert_eq!(refused.code(), "unsupported");
        }
    }

    #[test]
    fn a_socket_is_only_a_path_the_client_takes_whole() {
        assert!(is_socket_path("/run/user/1000/rex/server.sock"));
        assert!(is_socket_path("/home/a b/.local/state/rex/run/server.sock"));
        for not_one in [
            "",
            "run/rex.sock",
            "/run/rex.sock:22",
            "/run/\u{1b}rex.sock",
            "~/rex.sock",
        ] {
            assert!(!is_socket_path(not_one), "{not_one:?}");
        }
        assert!(!is_socket_path(&format!("/{}", "a".repeat(2000))));
    }

    #[test]
    fn the_socket_is_read_from_what_a_server_says_of_itself() {
        let unix = br#"Last login: today
{"endpoint":"unix:///run/user/1000/rex/server.sock","pid":7,
 "listeners":[
  {"endpoint":"tailnet://:7297","addresses":["100.64.0.1:7297"],"transport":"tailscale"},
  {"endpoint":"unix:///run/user/1000/rex/extra.sock","addresses":["/run/user/1000/rex/extra.sock"],"transport":"unix"},
  {"endpoint":"unix:///run/user/1000/rex/server.sock","addresses":["/run/user/1000/rex/server.sock"],"transport":"unix","primary":true}
 ]}
"#;
        assert_eq!(socket_in(unix).unwrap(), "/run/user/1000/rex/server.sock");
        // A server too old to list its listeners says its endpoint.
        assert_eq!(
            socket_in(br#"{"endpoint":"unix:///tmp/rex.sock"}"#).unwrap(),
            "/tmp/rex.sock"
        );
        // A socket that is a login server's is not the server's own, wherever it is listed.
        let two = br#"{"listeners":[
            {"endpoint":"login-server:///root/.local/state/rex/run/login-server.sock","addresses":["/root/.local/state/rex/run/login-server.sock"],"transport":"unix"},
            {"endpoint":"unix:///root/.local/state/rex/run/server.sock","addresses":["/root/.local/state/rex/run/server.sock"],"transport":"unix"}]}"#;
        assert_eq!(
            socket_in(two).unwrap(),
            "/root/.local/state/rex/run/server.sock"
        );
        // A listener that does not say where it is bound says what it was told.
        assert_eq!(
            socket_in(br#"{"listeners":[{"endpoint":"unix:///tmp/rex.sock","transport":"unix"}]}"#)
                .unwrap(),
            "/tmp/rex.sock"
        );
    }

    #[test]
    fn what_is_no_socket_to_forward_to_is_said() {
        let pipe = br#"{"endpoint":"npipe:////./pipe/rex-S-1-5-21-1","listeners":[
            {"endpoint":"npipe:////./pipe/rex-S-1-5-21-1","addresses":["\\\\.\\pipe\\rex-S-1-5-21-1"],"transport":"named_pipe","primary":true}]}"#;
        assert!(socket_in(pipe).unwrap_err().contains("named pipe"));
        for (printed, said) in [
            (&b""[..], "did not say"),
            (b"rex: command not found", "did not say"),
            (b"{not json}", "could not be read"),
            (b"{\"endpoint\":7}", "could not be read"),
            (br#"{"endpoint":"127.0.0.1:7297"}"#, "no socket"),
            (br#"{"endpoint":"unix://relative.sock"}"#, "no socket"),
            (br#"{"endpoint":"unix:///run/a:b.sock"}"#, "no socket"),
            (
                br#"{"listeners":[{"endpoint":"unix:///x","addresses":["/x"],"transport":"tcp"}]}"#,
                "no socket",
            ),
        ] {
            let why = socket_in(printed).unwrap_err();
            assert!(why.contains(said), "{why}");
        }
    }

    #[test]
    fn what_the_client_said_decides_whether_it_is_tried_again() {
        let tunnel = tunnel("ssh://build");
        let failed = |printed: &str| {
            let mut said = Tail::new();
            said.push(printed.as_bytes());
            // The status every failure of the client's own has.
            let status = exit_status(255);
            tunnel.failure(status, &said)
        };
        for stays in [
            "alice@build: Permission denied (publickey,password).\r\n",
            "Host key verification failed.\r\n",
            "@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @@@\r\n",
        ] {
            let error = failed(stays);
            assert_eq!(error.code(), "ssh", "{stays}");
            assert!(!error.is_transient(), "{stays}");
        }
        for passes in [
            "ssh: connect to host build port 22: Connection refused\r\n",
            "ssh: Could not resolve hostname build: Name or service not known\r\n",
            "ssh: connect to host build port 22: Connection timed out\r\n",
            "",
        ] {
            let error = failed(passes);
            assert_eq!(error.code(), "unreachable", "{passes}");
            assert!(error.is_transient(), "{passes}");
        }
    }

    #[cfg(windows)]
    fn exit_status(code: u32) -> std::process::ExitStatus {
        use std::os::windows::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code)
    }

    #[cfg(unix)]
    fn exit_status(code: u32) -> std::process::ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw((code as i32) << 8)
    }
}
