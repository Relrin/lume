//! The application's control endpoint.
//!
//! The application serves a local endpoint that only its user can reach. The command-line
//! tool drives sessions through it, and a second copy of the application uses it to bring
//! the first one's window to the front instead of opening another.
//!
//! A connection carries requests one after another, each answered before the next is
//! read. Every answer says what the request changed. What needs a session's screen is
//! passed on to the page that shows the session; the rest is answered here.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use lume_ipc::{Endpoint, Listener};
use lume_proto::app::{
    APP_PROTO_VERSION, AppError, AppMessage, AppReply, AppReplyBody, AppRequest, AppRequestBody,
    AppResponse, AppSession, PageRequestBody, codes,
};
use lume_proto::{ClientKind, ControlDecoder, SessionId, negotiate};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::carrier::{Link, SessionKey};
use crate::state::AppState;
use crate::tuning::{PAGE_ANSWER_TIMEOUT, PAGE_OPEN_TIMEOUT, WAIT_MAX, WAIT_SLACK};

/// How many bytes are read from a client at a time.
const READ_CHUNK: usize = 16 * 1024;

/// What the endpoint can do to the application's window.
pub trait Shell: Send + Sync + 'static {
    /// Brings the window to the front.
    fn activate(&self);
    /// Closes the application as closing its window would.
    fn quit(&self);
    /// Whether the application serves its own copy of the page.
    fn bundled_ui(&self) -> bool;
}

/// What a request needs to be carried out.
pub struct Control {
    state: Arc<AppState>,
    shell: Arc<dyn Shell>,
    /// How long a request waits for a page that has not connected yet.
    page_wait: Duration,
}

/// What to do once an answer has been written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Then {
    Carry,
    /// The application was asked to close; it does once the client has its answer.
    Quit,
}

fn reply(effect: impl Into<String>, body: AppReplyBody) -> Result<AppReply, AppError> {
    Ok(AppReply {
        effect: effect.into(),
        body,
    })
}

fn no_window() -> AppError {
    AppError::new(
        codes::NO_WINDOW,
        "the application has no window with a page that could answer",
    )
}

/// The time a request may wait: what it asked for, within what any request may.
fn wait_limit(timeout_ms: u64) -> Duration {
    Duration::from_millis(timeout_ms).min(WAIT_MAX)
}

impl Control {
    /// `page_wait` is how long a request waits for a page that has not connected yet.
    pub fn new(state: Arc<AppState>, shell: Arc<dyn Shell>, page_wait: Duration) -> Arc<Self> {
        Arc::new(Self {
            state,
            shell,
            page_wait,
        })
    }

    async fn page(&self) -> Result<Arc<Link>, AppError> {
        self.state
            .page_within(self.page_wait)
            .await
            .ok_or_else(no_window)
    }

    /// The session a request means: the one it names, or the one the window shows.
    async fn session(&self, named: Option<SessionId>) -> Result<SessionKey, AppError> {
        match named {
            Some(session) => self.state.find(session).await,
            None => self
                .state
                .shown_within(self.page_wait)
                .await
                .ok_or_else(|| {
                    AppError::new(
                        codes::NO_WINDOW,
                        "no session was named and no window is showing one",
                    )
                }),
        }
    }

    async fn handle(&self, body: AppRequestBody) -> (Result<AppReply, AppError>, Then) {
        let quit = matches!(body, AppRequestBody::Quit {});
        let result = self.carry_out(body).await;
        let then = if quit && result.is_ok() {
            Then::Quit
        } else {
            Then::Carry
        };
        (result, then)
    }

    async fn carry_out(&self, body: AppRequestBody) -> Result<AppReply, AppError> {
        match body {
            AppRequestBody::Hello { .. } => Err(AppError::new(
                codes::BAD_REQUEST,
                "hello has been said on this connection",
            )),
            AppRequestBody::Activate {} => {
                self.shell.activate();
                reply(
                    "brought the window to the front",
                    AppReplyBody::Activated {},
                )
            }
            AppRequestBody::Quit {} => reply("closing the application", AppReplyBody::Quitting {}),
            AppRequestBody::Status {} => reply(
                "read the application's status",
                AppReplyBody::Status {
                    app_version: env!("CARGO_PKG_VERSION").to_owned(),
                    pid: std::process::id(),
                    bundled_ui: self.shell.bundled_ui(),
                    window: self.state.page().is_some(),
                    shown: self.state.shown().map(|key| key.session),
                    marks: self.state.marks().read(),
                },
            ),
            AppRequestBody::Spawn { spec } => {
                let page = self.page().await?;
                let opened = page
                    .ask(PageRequestBody::Open { spec }, PAGE_OPEN_TIMEOUT)
                    .await?;
                let effect = match &opened {
                    AppReplyBody::Spawned {
                        session,
                        previous: Some(previous),
                    } => format!(
                        "started session {session} and showed it in the window; \
                         session {previous} keeps running"
                    ),
                    AppReplyBody::Spawned { session, .. } => {
                        format!("started session {session} and showed it in the window")
                    }
                    _ => return Err(unexpected("spawn")),
                };
                reply(effect, opened)
            }
            AppRequestBody::List {} => {
                let shown = self.state.shown();
                let sessions: Vec<AppSession> = self
                    .state
                    .list_all()
                    .await?
                    .into_iter()
                    .map(|listed| AppSession {
                        shown: shown.as_ref().is_some_and(|key| {
                            key.backend == listed.backend && key.session == listed.info.id
                        }),
                        backend: listed.backend,
                        info: listed.info,
                    })
                    .collect();
                reply(
                    format!("listed {} sessions", sessions.len()),
                    AppReplyBody::Sessions { sessions },
                )
            }
            AppRequestBody::Kill { session } => {
                let key = self.state.find(session).await?;
                self.state.end(&key).await?;
                reply(format!("ended session {session}"), AppReplyBody::Killed {})
            }
            AppRequestBody::Send { session, text } => {
                let key = self.session(session).await?;
                let input = self.page().await?.input_of(&key).ok_or_else(|| {
                    AppError::new(
                        codes::NOT_SHOWN,
                        format!(
                            "session {} is not shown in a window, and only a shown \
                             session can be typed into",
                            key.session
                        ),
                    )
                })?;
                let bytes = text.into_bytes();
                if !bytes.is_empty() {
                    input
                        .write(&bytes)
                        .await
                        .map_err(|error| AppError::from(crate::error::CommandError::from(error)))?;
                }
                reply(
                    format!("typed {} bytes into session {}", bytes.len(), key.session),
                    AppReplyBody::Sent {
                        bytes: bytes.len() as u64,
                    },
                )
            }
            AppRequestBody::Resize { session, size } => {
                let resized = self
                    .page()
                    .await?
                    .ask(
                        PageRequestBody::Resize { session, size },
                        PAGE_ANSWER_TIMEOUT,
                    )
                    .await?;
                match &resized {
                    AppReplyBody::Resized { size } => reply(
                        format!("the terminal is now {} by {}", size.cols(), size.rows()),
                        resized,
                    ),
                    _ => Err(unexpected("resize")),
                }
            }
            AppRequestBody::WaitExit {
                session,
                timeout_ms,
            } => {
                let key = self.session(session).await?;
                let status = self.state.wait_exit(&key, wait_limit(timeout_ms)).await?;
                reply(
                    format!("session {} has ended", key.session),
                    AppReplyBody::Exited { status },
                )
            }
            AppRequestBody::Screen { session } => {
                let screen = self
                    .page()
                    .await?
                    .ask(PageRequestBody::Screen { session }, PAGE_ANSWER_TIMEOUT)
                    .await?;
                match &screen {
                    AppReplyBody::Screen { lines, .. } => {
                        reply(format!("read {} lines of the screen", lines.len()), screen)
                    }
                    _ => Err(unexpected("screen")),
                }
            }
            AppRequestBody::WaitText {
                session,
                text,
                timeout_ms,
            } => {
                // The page keeps the time. It is asked for a little longer, so that its
                // own answer that the time ran out arrives before this gives up on it.
                let limit = wait_limit(timeout_ms);
                let found = self
                    .page()
                    .await?
                    .ask(
                        PageRequestBody::WaitText {
                            session,
                            text,
                            timeout_ms: u64::try_from(limit.as_millis()).unwrap_or(u64::MAX),
                        },
                        limit + WAIT_SLACK,
                    )
                    .await?;
                match &found {
                    AppReplyBody::Matched { row, .. } => {
                        reply(format!("the text is on row {row}"), found)
                    }
                    _ => Err(unexpected("wait_text")),
                }
            }
        }
    }
}

/// The page answered with something that is not an answer to what it was asked.
fn unexpected(request: &str) -> AppError {
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

async fn write_response<S: AsyncWrite + Unpin>(
    io: &mut S,
    id: u64,
    result: Result<AppReply, AppError>,
) -> io::Result<()> {
    let mut frames = Vec::new();
    AppMessage::Response(AppResponse { id, result })
        .encode(&mut frames)
        .map_err(io::Error::other)?;
    io.write_all(&frames).await?;
    io.flush().await
}

/// Answers the first request, which must be `hello` and must share a version with us.
fn greet(body: &AppRequestBody) -> Result<(AppReply, ClientKind), AppError> {
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
            AppMessage::Response(_) => {
                return Err(io::Error::other("a client sent a response"));
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

        let (result, then) = control.handle(body).await;
        write_response(&mut io, id, result).await?;
        if then == Then::Quit {
            control.shell.quit();
            return Ok(());
        }
    }
}

/// Asks the application already serving `endpoint` to bring its window to the front.
pub async fn activate_running(endpoint: &Endpoint) -> io::Result<()> {
    let mut io = lume_ipc::connect(endpoint).await?;
    let requests = [
        AppRequestBody::Hello {
            proto: APP_PROTO_VERSION,
            proto_min: APP_PROTO_VERSION,
            client_kind: ClientKind::App,
        },
        AppRequestBody::Activate {},
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
            AppMessage::Request(_) => {
                return Err(io::Error::other("the application sent a request"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use lume_proto::app::OpenSpec;
    use lume_session::{ExitStatus, HostConfig, LocalBackend, Router};
    use tokio::io::DuplexStream;
    use tokio::runtime::Handle;
    use tokio::time::timeout;

    use super::*;
    use crate::marks::Marks;
    use crate::testing::{CREDIT, RecordingCarrier, STEP, client_frames, facts, shell};

    const WINDOW: &str = "main";

    #[derive(Default)]
    struct FakeShell {
        activated: AtomicUsize,
        quit: AtomicUsize,
    }

    impl Shell for FakeShell {
        fn activate(&self) {
            self.activated.fetch_add(1, Ordering::Relaxed);
        }

        fn quit(&self) {
            self.quit.fetch_add(1, Ordering::Relaxed);
        }

        fn bundled_ui(&self) -> bool {
            true
        }
    }

    struct Running {
        state: Arc<AppState>,
        shell: Arc<FakeShell>,
        client: Client,
    }

    /// A client of the endpoint, speaking over an in-memory connection.
    struct Client {
        io: DuplexStream,
        decoder: ControlDecoder,
        next_id: u64,
    }

    impl Client {
        async fn ask(&mut self, body: AppRequestBody) -> Result<AppReply, AppError> {
            let id = self.next_id;
            self.next_id += 1;
            let mut frames = Vec::new();
            AppMessage::Request(AppRequest { id, body })
                .encode(&mut frames)
                .unwrap();
            self.io.write_all(&frames).await.unwrap();
            let mut chunk = [0; 4096];
            let json = loop {
                if let Some(json) = self.decoder.next().unwrap() {
                    break json;
                }
                let read = timeout(STEP, self.io.read(&mut chunk))
                    .await
                    .expect("the endpoint answers")
                    .unwrap();
                assert_ne!(read, 0, "the endpoint closed the connection");
                self.decoder.push(&chunk[..read]);
            };
            match AppMessage::decode(&json).unwrap() {
                AppMessage::Response(response) => {
                    assert_eq!(response.id, id);
                    response.result
                }
                AppMessage::Request(_) => panic!("the endpoint sent a request"),
            }
        }

        async fn hello(&mut self) {
            self.ask(AppRequestBody::Hello {
                proto: APP_PROTO_VERSION,
                proto_min: APP_PROTO_VERSION,
                client_kind: ClientKind::Test,
            })
            .await
            .unwrap();
        }
    }

    async fn running() -> Running {
        let local = Arc::new(
            LocalBackend::in_process(HostConfig::default())
                .await
                .unwrap(),
        );
        let router = Router::new();
        router.insert(Router::local_id(), local.clone());
        let state = AppState::new(
            Handle::current(),
            router,
            vec![local],
            Duration::from_secs(600),
            Arc::new(Marks::start()),
        );
        let shell = Arc::new(FakeShell::default());
        let control = Control::new(Arc::clone(&state), shell.clone(), Duration::from_millis(50));
        let (ours, theirs) = tokio::io::duplex(64 * 1024);
        tokio::spawn(async move {
            let _ = serve_client(theirs, &control).await;
        });
        Running {
            state,
            shell,
            client: Client {
                io: ours,
                decoder: ControlDecoder::new(),
                next_id: 1,
            },
        }
    }

    /// Opens a carrier as a page would, starts a shell and attaches to it.
    async fn shown(state: &Arc<AppState>) -> (Arc<RecordingCarrier>, u32, SessionId) {
        let carrier = RecordingCarrier::new();
        let id = state.open(WINDOW, carrier.clone()).await;
        let session = state
            .spawn(WINDOW, id, &Router::local_id(), shell())
            .await
            .unwrap();
        state
            .attach(WINDOW, id, &Router::local_id(), session, 1, facts())
            .await
            .unwrap();
        let credit = client_frames(&[lume_proto::Message::new(
            1,
            lume_proto::Body::Credit(CREDIT),
        )]);
        state.send(WINDOW, id, &credit).await.unwrap();
        (carrier, id, session)
    }

    fn text_of(carrier: &RecordingCarrier) -> String {
        let mut text = String::new();
        for message in carrier.messages() {
            if let lume_proto::Body::Data(bytes) = &message.body {
                text.push_str(&String::from_utf8_lossy(bytes));
            }
        }
        text
    }

    async fn until(what: &str, mut done: impl FnMut() -> bool) {
        let waiting = async {
            while !done() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        if timeout(STEP, waiting).await.is_err() {
            panic!("{what} never happened");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_client_must_say_hello_first() {
        let mut app = running().await;
        let refused = app.client.ask(AppRequestBody::List {}).await.unwrap_err();
        assert_eq!(refused.code, codes::HELLO_REQUIRED);
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_client_of_another_version_is_refused() {
        let mut app = running().await;
        let refused = app
            .client
            .ask(AppRequestBody::Hello {
                proto: APP_PROTO_VERSION + 2,
                proto_min: APP_PROTO_VERSION + 1,
                client_kind: ClientKind::Test,
            })
            .await
            .unwrap_err();
        assert_eq!(refused.code, codes::VERSION_MISMATCH);
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn activate_and_quit_reach_the_window() {
        let mut app = running().await;
        app.client.hello().await;

        let activated = app.client.ask(AppRequestBody::Activate {}).await.unwrap();
        assert_eq!(activated.body, AppReplyBody::Activated {});
        assert_eq!(app.shell.activated.load(Ordering::Relaxed), 1);
        assert_eq!(app.shell.quit.load(Ordering::Relaxed), 0);

        let quitting = app.client.ask(AppRequestBody::Quit {}).await.unwrap();
        assert_eq!(quitting.body, AppReplyBody::Quitting {});
        // The window is told once the answer is on its way.
        let shell = app.shell.clone();
        until("quit", || shell.quit.load(Ordering::Relaxed) == 1).await;
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sessions_are_listed_typed_into_and_ended_without_a_page_answering() {
        let mut app = running().await;
        app.client.hello().await;
        let (carrier, _, session) = shown(&app.state).await;

        let listed = app.client.ask(AppRequestBody::List {}).await.unwrap();
        let AppReplyBody::Sessions { sessions } = listed.body else {
            panic!("not a session list");
        };
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].info.id, session);
        assert!(sessions[0].shown);
        assert_eq!(listed.effect, "listed 1 sessions");

        // No session named: the one the window shows.
        let command = String::from_utf8(crate::testing::PRINT_MARK.to_vec()).unwrap();
        let sent = app
            .client
            .ask(AppRequestBody::Send {
                session: None,
                text: command.clone(),
            })
            .await
            .unwrap();
        assert_eq!(
            sent.body,
            AppReplyBody::Sent {
                bytes: command.len() as u64
            }
        );
        until("the command's output", || {
            text_of(&carrier).contains("lume-k7q-done")
        })
        .await;

        let status = app.client.ask(AppRequestBody::Status {}).await.unwrap();
        let AppReplyBody::Status {
            window,
            shown,
            marks,
            ..
        } = status.body
        else {
            panic!("not a status");
        };
        assert!(window);
        assert_eq!(shown, Some(session));
        assert!(marks.carrier_ms.is_some() && marks.attached_ms.is_some());
        assert!(marks.output_ms.is_some());

        let waiting = app
            .client
            .ask(AppRequestBody::WaitExit {
                session: Some(session),
                timeout_ms: 50,
            })
            .await
            .unwrap_err();
        assert_eq!(waiting.code, codes::TIMEOUT);

        app.client
            .ask(AppRequestBody::Kill { session })
            .await
            .unwrap();
        let exited = app
            .client
            .ask(AppRequestBody::WaitExit {
                session: Some(session),
                timeout_ms: 20_000,
            })
            .await
            .unwrap();
        assert!(matches!(exited.body, AppReplyBody::Exited { .. }));
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_exit_code_is_reported_to_whoever_waits() {
        let mut app = running().await;
        app.client.hello().await;
        let (_carrier, _, session) = shown(&app.state).await;
        app.client
            .ask(AppRequestBody::Send {
                session: Some(session),
                text: "exit 7\r".to_owned(),
            })
            .await
            .unwrap();
        let exited = app
            .client
            .ask(AppRequestBody::WaitExit {
                session: Some(session),
                timeout_ms: 20_000,
            })
            .await
            .unwrap();
        assert_eq!(
            exited.body,
            AppReplyBody::Exited {
                status: ExitStatus::Code(7)
            }
        );
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn what_needs_a_page_is_refused_without_one_and_passed_on_with_one() {
        let mut app = running().await;
        app.client.hello().await;

        let refused = app
            .client
            .ask(AppRequestBody::Screen { session: None })
            .await
            .unwrap_err();
        assert_eq!(refused.code, codes::NO_WINDOW);
        let refused = app
            .client
            .ask(AppRequestBody::Spawn {
                spec: OpenSpec::default(),
            })
            .await
            .unwrap_err();
        assert_eq!(refused.code, codes::NO_WINDOW);

        let (carrier, id, session) = shown(&app.state).await;
        // The page answers as soon as it is asked.
        let state = Arc::clone(&app.state);
        let page = tokio::spawn(async move {
            until("the request to the page", || !carrier.requests().is_empty()).await;
            let request = carrier.requests().remove(0);
            assert_eq!(request["request"]["body"]["type"], "screen");
            let asked = request["request"]["id"].as_u64().unwrap();
            let answer = AppReplyBody::Screen {
                session,
                size: crate::testing::size(4, 1),
                cursor: lume_proto::app::ScreenCursor {
                    col: 2,
                    row: 0,
                    visible: true,
                },
                alternate: false,
                lines: vec!["$ ".to_owned()],
            };
            state.answer(WINDOW, id, asked, Ok(answer)).unwrap();
        });
        let screen = app
            .client
            .ask(AppRequestBody::Screen { session: None })
            .await
            .unwrap();
        page.await.unwrap();
        assert_eq!(screen.effect, "read 1 lines of the screen");
        assert!(matches!(screen.body, AppReplyBody::Screen { .. }));
        app.state.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_session_that_is_not_shown_cannot_be_typed_into() {
        let mut app = running().await;
        app.client.hello().await;
        let (_carrier, id, _) = shown(&app.state).await;
        let hidden = app
            .state
            .spawn(WINDOW, id, &Router::local_id(), shell())
            .await
            .unwrap();
        let refused = app
            .client
            .ask(AppRequestBody::Send {
                session: Some(hidden),
                text: "x".to_owned(),
            })
            .await
            .unwrap_err();
        assert_eq!(refused.code, codes::NOT_SHOWN);

        let unknown = app
            .client
            .ask(AppRequestBody::Kill {
                session: SessionId::new(1).unwrap(),
            })
            .await
            .unwrap_err();
        assert_eq!(unknown.code, codes::NOT_FOUND);
        app.state.shutdown().await;
    }
}
