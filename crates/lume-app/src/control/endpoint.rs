//! The endpoint itself: binding and serving it, greeting a client, writing what it is
//! sent, relaying an attachment's bytes, and handing a launch to a copy already running.

use super::*;

/// The page answered with something that is not an answer to what it was asked.
/// Writes an attachment's frames to a client and passes on what the client sends, until
/// the client detaches or goes, or the session ends.
pub(super) async fn carry<S>(
    io: &mut S,
    rest: Vec<u8>,
    hydrate: Hydrate,
    output: &mut lume_session::FrameStream,
    input: &InputSink,
) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut out = Vec::new();
    if let Some(payload) = payload_of(hydrate) {
        encode_frame(Body::Hydrate(payload), &mut out);
        io.write_all(&out).await?;
        io.flush().await?;
    }
    let mut decoder = FrameDecoder::new(Role::Host);
    decoder.push(&rest);
    let mut chunk = vec![0; READ_CHUNK];
    loop {
        while let Some(message) = decoder.next().map_err(io::Error::other)? {
            if message.stream != ATTACHED_STREAM {
                continue;
            }
            let result = match message.body {
                Body::Input(bytes) if bytes.is_empty() => Ok(()),
                Body::Input(bytes) => input.write(&bytes).await,
                Body::Resize { size, cell_px } => input.resize(size, cell_px).await,
                Body::Facts(facts) => input.facts(&facts).await,
                Body::Credit(bytes) => input.credit(bytes).await,
                Body::Rehydrate => input.rehydrate().await,
                Body::Release => input.release().await,
                Body::Detach => return Ok(()),
                // Requests are not read once the attachment is carried.
                _ => Ok(()),
            };
            match result {
                Ok(()) | Err(SessionError::Detached) => {}
                Err(SessionError::CreditOverflow) => {
                    return Err(io::Error::other(
                        "the client granted more credit than a stream may hold",
                    ));
                }
                Err(error) => tracing::debug!(%error, "a client's frame could not be passed on"),
            }
        }
        tokio::select! {
            frame = output.next() => {
                let Some(frame) = frame else {
                    // The session went without an exit: its backend went away.
                    return Ok(());
                };
                let exit = matches!(frame, OutputFrame::Exit(_));
                out.clear();
                match frame {
                    OutputFrame::Data(bytes) => {
                        encode_bytes(ATTACHED_STREAM, BytesKind::Data, &bytes, &mut out);
                    }
                    OutputFrame::Resized(size) => encode_frame(Body::Resized(size), &mut out),
                    OutputFrame::Event(event) => encode_frame(Body::Event(event), &mut out),
                    OutputFrame::Exit(status) => encode_frame(Body::Exit(status), &mut out),
                    OutputFrame::Hydrate(hydrate) => {
                        if let Some(payload) = payload_of(hydrate) {
                            encode_frame(Body::Hydrate(payload), &mut out);
                        }
                    }
                }
                io.write_all(&out).await?;
                io.flush().await?;
                if exit {
                    return Ok(());
                }
            }
            read = io.read(&mut chunk) => {
                let read = read?;
                if read == 0 {
                    return Ok(());
                }
                decoder.push(&chunk[..read]);
            }
        }
    }
}

/// Appends one frame of the carried attachment. Every frame carried is one a session
/// sent and so one that encodes; one that did not would be left out.
pub(super) fn encode_frame(body: Body, out: &mut Vec<u8>) {
    if let Err(error) = encode(&Message::new(ATTACHED_STREAM, body), out) {
        tracing::error!(%error, "a frame of a carried attachment could not be encoded");
    }
}

pub(super) fn payload_of(hydrate: Hydrate) -> Option<HydratePayload> {
    match hydrate {
        Hydrate::None => None,
        Hydrate::Snapshot { vt_version, bytes } => {
            Some(HydratePayload::Snapshot { vt_version, bytes })
        }
        Hydrate::VtReplay(bytes) => Some(HydratePayload::VtReplay(bytes)),
    }
}

pub(super) fn session_error(error: lume_session::SessionError) -> AppError {
    AppError::from(crate::error::CommandError::from(error))
}

pub(super) fn unexpected(request: &str) -> AppError {
    AppError::new(
        codes::INTERNAL,
        format!("the page gave an answer that does not fit a {request} request"),
    )
}

/// The endpoint the application serves: the one named, or the default.
pub fn endpoint(name: Option<&str>) -> io::Result<Endpoint> {
    Endpoint::named(name.unwrap_or(lume_proto::app::ENDPOINT_NAME))
}

/// Starts serving `endpoint`, or reports that another process already does. Must be
/// called inside the runtime.
pub fn bind(endpoint: &Endpoint) -> io::Result<Option<Listener>> {
    match Listener::bind(endpoint) {
        Ok(listener) => Ok(Some(listener)),
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => Ok(None),
        Err(error) => Err(error),
    }
}

/// Serves clients for as long as the endpoint can accept them.
pub async fn serve(mut listener: Listener, control: Arc<Control>) {
    loop {
        match listener.accept().await {
            Ok(client) => {
                let control = Arc::clone(&control);
                tokio::spawn(async move {
                    if let Err(error) = serve_client(client, &control).await {
                        tracing::debug!(%error, "a control connection ended with an error");
                    }
                });
            }
            Err(error) => {
                tracing::error!(%error, "the control endpoint stopped accepting clients");
                return;
            }
        }
    }
}

pub(super) async fn write_event<S: AsyncWrite + Unpin>(
    io: &mut S,
    event: AppEvent,
) -> io::Result<()> {
    write_message(io, &AppMessage::Event(event)).await
}

pub(super) async fn write_response<S: AsyncWrite + Unpin>(
    io: &mut S,
    id: u64,
    result: Result<AppReply, AppError>,
) -> io::Result<()> {
    write_message(io, &AppMessage::Response(AppResponse { id, result })).await
}

/// Answers the first request, which must be `hello` and must share a version with us.
pub(super) fn greet(body: &AppRequestBody) -> Result<(AppReply, ClientKind), AppError> {
    let AppRequestBody::Hello {
        proto,
        proto_min,
        client_kind,
    } = body
    else {
        return Err(AppError::new(
            codes::HELLO_REQUIRED,
            "the first request on a connection must be hello",
        ));
    };
    let agreed = negotiate(APP_PROTO_VERSION, *proto, *proto_min).ok_or_else(|| {
        AppError::new(
            codes::VERSION_MISMATCH,
            format!(
                "the application speaks version {APP_PROTO_VERSION}, the client \
                 {proto_min} to {proto}"
            ),
        )
    })?;
    let reply = AppReply {
        effect: "connected".to_owned(),
        body: AppReplyBody::Hello {
            proto: agreed,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            pid: std::process::id(),
        },
    };
    Ok((reply, *client_kind))
}

pub(super) async fn write_message<S: AsyncWrite + Unpin>(
    io: &mut S,
    message: &AppMessage,
) -> io::Result<()> {
    let mut bytes = Vec::new();
    message.encode(&mut bytes).map_err(io::Error::other)?;
    if message.holds_secret() {
        let bytes = Zeroizing::new(bytes);
        io.write_all(&bytes).await?;
    } else {
        io.write_all(&bytes).await?;
    }
    io.flush().await
}

/// Sends what a step of a verb prints: the result first, then what is said beside it.
pub(super) async fn write_step<S: AsyncWrite + Unpin>(io: &mut S, step: &Step) -> io::Result<()> {
    for (text, error) in [(&step.out, false), (&step.err, true)] {
        for piece in pieces(text, OUTPUT_PIECE_MAX) {
            let output = AppOutput {
                text: piece.to_owned(),
                error,
            };
            write_message(io, &AppMessage::Output(output)).await?;
        }
    }
    Ok(())
}

/// Does something for a client for as long as the client is there. What it sends
/// meanwhile is kept for later.
pub(super) async fn until_gone<S, T>(
    io: &mut S,
    decoder: &mut ControlDecoder,
    work: impl Future<Output = T>,
) -> Option<T>
where
    S: AsyncRead + Unpin,
{
    let mut work = std::pin::pin!(work);
    let mut chunk = [0; 1024];
    let mut sent = 0;
    loop {
        tokio::select! {
            done = &mut work => return Some(done),
            read = io.read(&mut chunk) => match read {
                Ok(read) if read > 0 && sent < READ_CHUNK => {
                    sent += read;
                    decoder.push(&chunk[..read]);
                }
                _ => return None,
            },
        }
    }
}

/// Serves one client until it disconnects or breaks the rules.
pub async fn serve_client<S>(mut io: S, control: &Control) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut decoder = ControlDecoder::new();
    let mut chunk = vec![0; READ_CHUNK];
    let mut greeted = false;
    loop {
        let json = loop {
            if let Some(json) = decoder.next().map_err(io::Error::other)? {
                break json;
            }
            let read = io.read(&mut chunk).await?;
            if read == 0 {
                return Ok(());
            }
            decoder.push(&chunk[..read]);
        };
        let AppRequest { id, body } = match AppMessage::decode(&json).map_err(io::Error::other)? {
            AppMessage::Request(request) => request,
            AppMessage::Response(_)
            | AppMessage::Event(_)
            | AppMessage::Output(_)
            | AppMessage::Done(_) => {
                return Err(io::Error::other(
                    "a client sent what only the application sends",
                ));
            }
        };

        if !greeted {
            match greet(&body) {
                Ok((reply, client_kind)) => {
                    tracing::debug!(?client_kind, "a control client connected");
                    greeted = true;
                    write_response(&mut io, id, Ok(reply)).await?;
                }
                Err(error) => {
                    // Nothing else is said to a client that did not start properly.
                    write_response(&mut io, id, Err(error)).await?;
                    return Ok(());
                }
            }
            continue;
        }

        let (result, then) = match body {
            AppRequestBody::Askpass {
                token,
                prompt,
                hint,
            } => {
                let asked = control.askpass(&token, &prompt, hint.as_deref());
                match until_gone(&mut io, &mut decoder, asked).await {
                    Some(result) => (result, Then::Carry),
                    // The client left, and the question with it.
                    None => return Ok(()),
                }
            }
            body => control.handle(body).await,
        };
        write_response(&mut io, id, result).await?;
        match then {
            Then::Carry => {}
            Then::Quit => {
                control.services.shell.quit();
                return Ok(());
            }
            Then::Follow(following) => return control.follow(&mut io, *following).await,
            Then::Verb(run) => return control.run_verb(&mut io, *run).await,
            Then::Attach(attachment) => {
                let rest = std::mem::take(&mut decoder).into_rest();
                return control.carry_attachment(&mut io, rest, *attachment).await;
            }
        }
    }
}

/// Hands what this copy of the application was started for to the copy already
/// serving `endpoint`: it brings its window to the front, shows the workspace named and
/// runs the program named, or shows a session of the Rex server named.
pub async fn launch_running(
    endpoint: &Endpoint,
    workspace: Option<String>,
    exec: Vec<String>,
    cwd: Option<String>,
    rex: Option<String>,
) -> io::Result<()> {
    let mut io = lume_ipc::connect(endpoint).await?;
    let requests = [
        AppRequestBody::Hello {
            proto: APP_PROTO_VERSION,
            proto_min: APP_PROTO_VERSION,
            client_kind: ClientKind::App,
        },
        AppRequestBody::Launch {
            workspace,
            exec,
            cwd,
            rex,
        },
    ];
    let mut decoder = ControlDecoder::new();
    let mut chunk = vec![0; READ_CHUNK];
    for (id, body) in requests.into_iter().enumerate() {
        let mut frames = Vec::new();
        AppMessage::Request(AppRequest {
            id: id as u64,
            body,
        })
        .encode(&mut frames)
        .map_err(io::Error::other)?;
        io.write_all(&frames).await?;
        io.flush().await?;

        let json = loop {
            if let Some(json) = decoder.next().map_err(io::Error::other)? {
                break json;
            }
            let read = io.read(&mut chunk).await?;
            if read == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            decoder.push(&chunk[..read]);
        };
        match AppMessage::decode(&json).map_err(io::Error::other)? {
            AppMessage::Response(AppResponse { result: Ok(_), .. }) => {}
            AppMessage::Response(AppResponse {
                result: Err(error), ..
            }) => return Err(io::Error::other(error)),
            AppMessage::Request(_)
            | AppMessage::Event(_)
            | AppMessage::Output(_)
            | AppMessage::Done(_) => {
                return Err(io::Error::other(
                    "the application sent something that is not an answer",
                ));
            }
        }
    }
    Ok(())
}
