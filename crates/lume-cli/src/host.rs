//! `lume host`: the session host that runs on its own.
//!
//! The application starts the host when it needs it, and the host exits by itself once
//! it has had nothing to do for a while, so these are seldom needed. `status` says
//! whether it runs and what it is doing, `start` starts it without the application,
//! and `stop` stops it: only while no program runs in it, unless told to end them.
//!
//! The commands speak the session protocol to the host directly, on its own endpoint,
//! which only the user who runs it can reach.

use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use lume_ipc::{Endpoint, Stream};
use lume_proto::{
    ClientKind, Control, ErrorCode, FrameDecoder, Message, PROTO_VERSION, PROTO_VERSION_MIN,
    ProtoError, Request, RequestBody, Response, ResponseBody, Role, encode,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::args::{HostAction, HostCommand};
use crate::ctl::{EXIT_FAILED, EXIT_NOT_RUNNING, EXIT_OK, failure};

/// The host program's file name.
const HOST_PROGRAM: &str = if cfg!(windows) {
    "lume-host.exe"
} else {
    "lume-host"
};

/// The status a host exits with when another one already serves its endpoint.
const EXIT_IN_USE: i32 = 3;

/// How long a host that was just started may take to serve its endpoint.
const START_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the host may take to answer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a host that was asked to stop may take to let go of its endpoint.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// How many bytes are read from the host at a time.
const READ_CHUNK: usize = 16 * 1024;

/// An open connection on which hello has been said.
struct HostClient {
    io: Stream,
    decoder: FrameDecoder,
    next_id: u64,
    /// What the host said about itself.
    hello: ResponseBody,
}

impl HostClient {
    async fn hello(io: Stream) -> io::Result<Self> {
        let mut client = Self {
            io,
            decoder: FrameDecoder::new(Role::Client),
            next_id: 1,
            hello: ResponseBody::Killed {},
        };
        let hello = RequestBody::Hello {
            proto: PROTO_VERSION,
            proto_min: PROTO_VERSION_MIN,
            vt_version: None,
            client_kind: ClientKind::Ctl,
        };
        client.hello = client.ask(hello).await?.map_err(refused)?;
        Ok(client)
    }

    fn proto(&self) -> u32 {
        match &self.hello {
            ResponseBody::HelloAck { proto, .. } => *proto,
            _ => 0,
        }
    }

    async fn ask(&mut self, body: RequestBody) -> io::Result<Result<ResponseBody, ProtoError>> {
        let id = self.next_id;
        self.next_id += 1;
        let mut frames = Vec::new();
        encode(
            &Message::control(Control::Request(Request { id, body })),
            &mut frames,
        )
        .map_err(io::Error::other)?;
        let exchange = async {
            self.io.write_all(&frames).await?;
            self.io.flush().await?;
            let mut chunk = vec![0; READ_CHUNK];
            loop {
                while let Some(message) = self.decoder.next().map_err(io::Error::other)? {
                    if let lume_proto::Body::Control(Control::Response(Response {
                        id: answered,
                        result,
                    })) = message.body
                        && answered == id
                    {
                        return Ok(result);
                    }
                }
                let read = self.io.read(&mut chunk).await?;
                if read == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "the session host closed the connection",
                    ));
                }
                self.decoder.push(&chunk[..read]);
            }
        };
        tokio::time::timeout(ANSWER_TIMEOUT, exchange)
            .await
            .map_err(|_| {
                io::Error::new(io::ErrorKind::TimedOut, "the session host did not answer")
            })?
    }

    /// What the host says it is doing, as printed.
    async fn status(&mut self, endpoint: &Endpoint) -> io::Result<Value> {
        let (version, pid, proto) = match &self.hello {
            ResponseBody::HelloAck {
                host_version,
                pid,
                proto,
                ..
            } => (host_version.clone(), *pid, *proto),
            _ => (String::new(), 0, 0),
        };
        let mut status = json!({
            "ok": true,
            "running": true,
            "endpoint": endpoint.name(),
            "version": version,
            "pid": pid,
            "proto": proto,
        });
        // An older host cannot say more.
        if proto >= PROTO_VERSION
            && let Ok(ResponseBody::HostStatus {
                started_unix_ms,
                persistent,
                sessions,
                clients,
                idle_exit_ms,
                ..
            }) = self.ask(RequestBody::HostStatus {}).await?
        {
            status["started_unix_ms"] = json!(started_unix_ms);
            status["persistent"] = json!(persistent);
            status["sessions"] = json!(sessions);
            // Windows and other clients; this connection is not one of them.
            status["clients"] = json!(clients);
            status["idle_exit_s"] = json!(idle_exit_ms.map(|ms| ms / 1000));
        }
        Ok(status)
    }
}

fn refused(error: ProtoError) -> io::Error {
    io::Error::other(error.to_string())
}

/// Connects to the host, if one serves the endpoint.
async fn reach(endpoint: &Endpoint) -> io::Result<Option<HostClient>> {
    match lume_ipc::connect(endpoint).await {
        Ok(stream) => HostClient::hello(stream).await.map(Some),
        Err(error) if lume_ipc::not_served(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

/// The host program beside this one.
fn host_program() -> io::Result<PathBuf> {
    let beside = std::env::current_exe()?.with_file_name(HOST_PROGRAM);
    if beside.is_file() {
        Ok(beside)
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is not there", beside.display()),
        ))
    }
}

/// How long the host started here stays idle: as the user's settings say.
fn idle_exit_seconds() -> u32 {
    match lume_config::Dirs::resolve() {
        Some(dirs) => {
            lume_config::ConfigStore::load(dirs, None)
                .current()
                .config
                .host
                .idle_exit
        }
        None => lume_config::schema::HostSettings::default().idle_exit,
    }
}

async fn start(endpoint: &Endpoint) -> io::Result<Value> {
    if let Some(mut client) = reach(endpoint).await? {
        let mut status = client.status(endpoint).await?;
        status["effect"] = json!("the session host was running already");
        return Ok(status);
    }
    let mut command = Command::new(host_program()?);
    command.args([
        "serve",
        "--endpoint",
        endpoint.name(),
        "--idle-exit",
        &idle_exit_seconds().to_string(),
    ]);
    if let Some(log) = lume_config::host_log_file() {
        command.arg("--log").arg(log);
    }
    if let Some(dirs) = lume_config::Dirs::resolve() {
        if dirs.state.is_dir() {
            command.current_dir(&dirs.state);
        }
        command.arg("--state").arg(&dirs.state);
    }
    let stream =
        lume_ipc::start_and_connect(endpoint, &mut command, EXIT_IN_USE, START_TIMEOUT).await?;
    let mut client = HostClient::hello(stream).await?;
    let mut status = client.status(endpoint).await?;
    status["effect"] = json!("the session host was started");
    Ok(status)
}

async fn stop(endpoint: &Endpoint, end_sessions: bool) -> io::Result<Result<Value, ProtoError>> {
    let Some(mut client) = reach(endpoint).await? else {
        return Ok(Ok(json!({
            "ok": true,
            "running": false,
            "endpoint": endpoint.name(),
            "effect": "the session host was not running",
        })));
    };
    if client.proto() < PROTO_VERSION {
        return Ok(Err(ProtoError::new(
            ErrorCode::Unsupported,
            "this session host is too old to be stopped; it stops by itself once nothing runs in it",
        )));
    }
    let ended = match client.ask(RequestBody::Shutdown { end_sessions }).await? {
        Ok(ResponseBody::Stopping { ended }) => ended,
        Ok(_) => {
            return Err(io::Error::other(
                "the session host answered with something else",
            ));
        }
        Err(refusal) => return Ok(Err(refusal)),
    };
    drop(client);
    // Gone once nothing serves its endpoint any more.
    let deadline = std::time::Instant::now() + STOP_TIMEOUT;
    while std::time::Instant::now() < deadline {
        match lume_ipc::connect(endpoint).await {
            Err(error) if lume_ipc::not_served(&error) => {
                let effect = match ended {
                    0 => "the session host stopped".to_owned(),
                    1 => "the session host ended its session and stopped".to_owned(),
                    ended => format!("the session host ended its {ended} sessions and stopped"),
                };
                return Ok(Ok(json!({
                    "ok": true,
                    "running": false,
                    "ended": ended,
                    "endpoint": endpoint.name(),
                    "effect": effect,
                })));
            }
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "the session host was asked to stop and is still there",
    ))
}

/// Carries out a `lume host` command and prints its result. Returns the exit status.
pub async fn run(command: HostCommand) -> u8 {
    let name = lume_proto::host_endpoint_name(command.endpoint.as_deref());
    let endpoint = match Endpoint::named(&name) {
        Ok(endpoint) => endpoint,
        Err(error) => {
            eprintln!("lume: {error}");
            return EXIT_FAILED;
        }
    };
    let result = match command.action {
        HostAction::Status => match reach(&endpoint).await {
            Ok(Some(mut client)) => client.status(&endpoint).await.map(|mut status| {
                status["effect"] = json!("none");
                Ok(status)
            }),
            Ok(None) => {
                let mut said =
                    failure("not_running", format!("nothing serves {}", endpoint.name()));
                said["running"] = json!(false);
                println!("{said}");
                return EXIT_NOT_RUNNING;
            }
            Err(error) => Err(error),
        },
        HostAction::Start => start(&endpoint).await.map(Ok),
        HostAction::Stop { end_sessions } => stop(&endpoint, end_sessions).await,
    };
    match result {
        Ok(Ok(value)) => {
            println!("{value}");
            EXIT_OK
        }
        Ok(Err(refusal)) => {
            println!("{}", failure(refusal.code.as_str(), refusal.message));
            EXIT_FAILED
        }
        Err(error) => {
            println!("{}", failure("disconnected", error));
            EXIT_FAILED
        }
    }
}
