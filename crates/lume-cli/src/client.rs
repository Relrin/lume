//! A connection to the running application's control endpoint.

use std::io;
use std::time::Duration;

use lume_ipc::{Endpoint, Stream};
use lume_proto::app::{
    APP_PROTO_VERSION, AppError, AppEvent, AppMessage, AppReply, AppRequest, AppRequestBody,
    AppResponse, ENDPOINT_NAME,
};
use lume_proto::{ClientKind, ControlDecoder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// How many bytes are read from the application at a time.
const READ_CHUNK: usize = 16 * 1024;

/// How long the application may take to answer a request that does not wait for anything.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);

/// Why a request got no answer to act on.
#[derive(Debug)]
pub enum Failure {
    /// Nothing serves the endpoint: the application is not running.
    NotRunning(io::Error),
    /// The connection failed or the application said something that is not an answer.
    Broken(io::Error),
    /// The application refused the request or could not carry it out.
    Refused(AppError),
}

/// The endpoint the application serves: the one named, or the default.
pub fn endpoint(name: Option<&str>) -> io::Result<Endpoint> {
    Endpoint::named(name.unwrap_or(ENDPOINT_NAME))
}

/// An open connection on which `hello` has been said.
pub struct Client {
    io: Stream,
    decoder: ControlDecoder,
    next_id: u64,
}

impl Client {
    /// Connects to the application serving `endpoint` and introduces itself.
    pub async fn connect(endpoint: &Endpoint) -> Result<Self, Failure> {
        let io = lume_ipc::connect(endpoint)
            .await
            .map_err(|error| match error.kind() {
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => {
                    Failure::NotRunning(error)
                }
                _ => Failure::Broken(error),
            })?;
        let mut client = Self {
            io,
            decoder: ControlDecoder::new(),
            next_id: 1,
        };
        let hello = AppRequestBody::Hello {
            proto: APP_PROTO_VERSION,
            proto_min: APP_PROTO_VERSION,
            client_kind: ClientKind::Ctl,
        };
        client.ask(hello, ANSWER_TIMEOUT).await?;
        Ok(client)
    }

    /// Sends one request and waits for its answer, for `limit` at most.
    pub async fn ask(
        &mut self,
        body: AppRequestBody,
        limit: Duration,
    ) -> Result<AppReply, Failure> {
        let id = self.next_id;
        self.next_id += 1;
        let exchange = self.exchange(id, body);
        match tokio::time::timeout(limit, exchange).await {
            Ok(Ok(result)) => result.map_err(Failure::Refused),
            Ok(Err(error)) => Err(Failure::Broken(error)),
            Err(_) => Err(Failure::Broken(io::Error::new(
                io::ErrorKind::TimedOut,
                "the application did not answer",
            ))),
        }
    }

    async fn exchange(
        &mut self,
        id: u64,
        body: AppRequestBody,
    ) -> io::Result<Result<AppReply, AppError>> {
        let mut frames = Vec::new();
        AppMessage::Request(AppRequest { id, body })
            .encode(&mut frames)
            .map_err(io::Error::other)?;
        self.io.write_all(&frames).await?;
        self.io.flush().await?;

        let mut chunk = vec![0; READ_CHUNK];
        loop {
            while let Some(json) = self.decoder.next().map_err(io::Error::other)? {
                match AppMessage::decode(&json).map_err(io::Error::other)? {
                    AppMessage::Response(AppResponse {
                        id: answered,
                        result,
                    }) if answered == id => {
                        return Ok(result);
                    }
                    // An answer to nothing this connection is waiting for.
                    AppMessage::Response(_) => {}
                    AppMessage::Request(_) | AppMessage::Event(_) => {
                        return Err(io::Error::other(
                            "the application sent something that is not an answer",
                        ));
                    }
                }
            }
            let read = self.io.read(&mut chunk).await?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the application closed the connection",
                ));
            }
            self.decoder.push(&chunk[..read]);
        }
    }

    /// The connection and the bytes read after the last answer, for a connection that
    /// goes on to carry an attachment.
    pub fn into_parts(self) -> (Stream, Vec<u8>) {
        (self.io, self.decoder.into_rest())
    }

    /// The next thing a session reported, on a connection that has asked to follow
    /// sessions. `None` once the application has closed the connection.
    pub async fn event(&mut self) -> io::Result<Option<AppEvent>> {
        let mut chunk = vec![0; READ_CHUNK];
        loop {
            while let Some(json) = self.decoder.next().map_err(io::Error::other)? {
                match AppMessage::decode(&json).map_err(io::Error::other)? {
                    AppMessage::Event(event) => return Ok(Some(event)),
                    // An answer to something this connection no longer waits for.
                    AppMessage::Response(_) => {}
                    AppMessage::Request(_) => {
                        return Err(io::Error::other("the application sent a request"));
                    }
                }
            }
            let read = self.io.read(&mut chunk).await?;
            if read == 0 {
                return Ok(None);
            }
            self.decoder.push(&chunk[..read]);
        }
    }
}
