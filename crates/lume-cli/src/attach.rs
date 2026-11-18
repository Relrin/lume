//! `lume ctl attach`: a session shown in the console `lume` runs in.
//!
//! `--raw` makes the console a terminal of the session: it is cleared and drawn as the
//! session's screen is, the output follows, every key goes to the session, and the
//! console gives the session its size, as a window that is focused does. Ctrl+\ twice
//! in a row detaches; the session runs on. Through the application when it runs, else
//! straight through the session host.
//!
//! `--text` prints the screen as `capture` prints it, again each time it changes, until
//! the session goes or `--timeout` has passed. It reads the screen and does not attach,
//! so it never changes the session's size or how fast its output flows.

use std::time::{Duration, Instant};

use lume_ipc::Stream;
use lume_proto::app::{AppReply, AppReplyBody, AppRequestBody, codes};
use lume_proto::tuning::CREDIT_WINDOW;
use lume_proto::{
    Body, CellPx, ClientFacts, ColorScheme, FrameDecoder, HydratePayload, Message, Palette,
    RequestBody, ResponseBody, Role, SessionId, TermSize, encode,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::args::Ctl;
use crate::client::{ANSWER_TIMEOUT, Client, Failure, endpoint};
use crate::console;
use crate::ctl::{self, EXIT_FAILED, EXIT_OK};
use crate::direct;
use crate::host::HostClient;

/// The stream the attachment is carried on.
const STREAM: u32 = 1;

/// The key that, pressed twice in a row, detaches: Ctrl+\.
const DETACH_KEY: u8 = 0x1c;

/// How often the console's size is looked at.
const CONSOLE_SIZE_POLL: Duration = Duration::from_millis(250);

/// How often `--text` reads the screen.
const TEXT_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// The cell size the console is taken to have: it cannot be asked.
const CELL_PX: CellPx = CellPx { w: 9, h: 18 };

/// The size the session is given when the console's cannot be read.
const FALLBACK_SIZE: (u16, u16) = (80, 24);

/// Written before the screen the session starts from: the console starts blank.
const CLEAR: &[u8] = b"\x1b[0m\x1b[H\x1b[2J\x1b[3J";

/// How many bytes are read at a time.
const READ_CHUNK: usize = 64 * 1024;

/// Keeps an eye on the keys for the one that detaches. A first press is held back until
/// the next key: a second one detaches, any other is sent after it.
#[derive(Default)]
struct DetachKey {
    held: bool,
}

impl DetachKey {
    /// The bytes to send for `keys`, and whether the user asked to detach.
    fn filter(&mut self, keys: &[u8]) -> (Vec<u8>, bool) {
        let mut out = Vec::with_capacity(keys.len());
        for &key in keys {
            if key == DETACH_KEY {
                if self.held {
                    self.held = false;
                    return (out, true);
                }
                self.held = true;
                continue;
            }
            if self.held {
                self.held = false;
                out.push(DETACH_KEY);
            }
            out.push(key);
        }
        (out, false)
    }
}

/// How an attachment ended.
enum Ended {
    Detached,
    Exited(lume_proto::ExitStatus),
    Lost,
}

/// A connection that carries one attachment on [`STREAM`].
struct Carried {
    io: Stream,
    frames: FrameDecoder,
    session: SessionId,
    via: &'static str,
}

impl Carried {
    async fn send(&mut self, body: Body) -> std::io::Result<()> {
        let mut out = Vec::new();
        encode(&Message::new(STREAM, body), &mut out).map_err(std::io::Error::other)?;
        self.io.write_all(&out).await?;
        self.io.flush().await
    }
}

fn console_size() -> TermSize {
    console::size()
        .or(TermSize::new(FALLBACK_SIZE.0, FALLBACK_SIZE.1).ok())
        .expect("the fallback size is a size")
}

/// Opens the attachment, through the application if it runs, else through the host.
async fn open(ctl: &Ctl, size: TermSize) -> Result<Carried, Failure> {
    let AppRequestBody::Attach { pane, session, .. } = &ctl.request else {
        return Err(Failure::Broken(std::io::Error::other(
            "attach was asked without its request",
        )));
    };
    let request = AppRequestBody::Attach {
        pane: pane.clone(),
        session: *session,
        size,
    };
    let endpoint = endpoint(ctl.endpoint.as_deref()).map_err(Failure::Broken)?;
    match Client::connect(&endpoint).await {
        Ok(mut client) => {
            let reply = client.ask(request, ANSWER_TIMEOUT).await?;
            let AppReplyBody::Attached { session, .. } = reply.body else {
                return Err(Failure::Broken(std::io::Error::other(
                    "the application answered attach with something else",
                )));
            };
            let (io, rest) = client.into_parts();
            let mut frames = FrameDecoder::new(Role::Client);
            frames.push(&rest);
            Ok(Carried {
                io,
                frames,
                session,
                via: "the application",
            })
        }
        Err(Failure::NotRunning(error)) => {
            let Some(mut host) = direct::reach(ctl.endpoint.as_deref()).await? else {
                return Err(Failure::NotRunning(error));
            };
            let (None, Some(session)) = (pane, *session) else {
                return Err(direct::needs_app(
                    "attaching to a session not named by --session",
                ));
            };
            attach_to_host(&mut host, session, size).await?;
            let (io, frames) = host.into_parts();
            Ok(Carried {
                io,
                frames,
                session,
                via: "the session host",
            })
        }
        Err(failure) => Err(failure),
    }
}

async fn attach_to_host(
    host: &mut HostClient,
    session: SessionId,
    size: TermSize,
) -> Result<(), Failure> {
    let facts = ClientFacts {
        size,
        cell_px: CELL_PX,
        palette: Palette::default(),
        scheme: ColorScheme::Dark,
        focused: true,
        visible: true,
    };
    let request = RequestBody::Attach {
        session,
        stream: STREAM,
        facts,
        replay: true,
    };
    match host.ask(request).await.map_err(Failure::Broken)? {
        Ok(ResponseBody::Attached { .. }) => Ok(()),
        Ok(_) => Err(Failure::Broken(std::io::Error::other(
            "the session host answered attach with something else",
        ))),
        Err(error) => Err(Failure::Refused(lume_proto::app::AppError::new(
            error.code.as_str(),
            error.message,
        ))),
    }
}

/// Reads standard input on a thread of its own, since a console read blocks.
fn keys() -> mpsc::UnboundedReceiver<Vec<u8>> {
    let (send, keys) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let mut input = std::io::stdin();
        let mut buf = [0u8; 1024];
        loop {
            match std::io::Read::read(&mut input, &mut buf) {
                Ok(0) | Err(_) => return,
                Ok(read) => {
                    if send.send(buf[..read].to_vec()).is_err() {
                        return;
                    }
                }
            }
        }
    });
    keys
}

/// Shows what arrives and passes on what is typed, until the user detaches, the
/// session's program ends or the connection goes.
async fn carry(carried: &mut Carried, mut size: TermSize) -> std::io::Result<Ended> {
    carried
        .send(Body::Resize {
            size,
            cell_px: CELL_PX,
        })
        .await?;
    carried.send(Body::Credit(CREDIT_WINDOW)).await?;
    let mut keys = keys();
    let mut detach = DetachKey::default();
    let mut look = tokio::time::interval(CONSOLE_SIZE_POLL);
    let mut chunk = vec![0; READ_CHUNK];
    loop {
        while let Some(message) = carried.frames.next().map_err(std::io::Error::other)? {
            if message.stream != STREAM {
                continue;
            }
            match message.body {
                Body::Hydrate(HydratePayload::VtReplay(bytes)) => {
                    console::write(CLEAR)?;
                    console::write(&bytes)?;
                }
                Body::Data(bytes) => {
                    console::write(&bytes)?;
                    let taken = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
                    carried.send(Body::Credit(taken)).await?;
                }
                Body::Exit(status) => return Ok(Ended::Exited(status)),
                // The console has the size it has; reports are for windows.
                _ => {}
            }
        }
        tokio::select! {
            read = carried.io.read(&mut chunk) => {
                let read = read?;
                if read == 0 {
                    return Ok(Ended::Lost);
                }
                carried.frames.push(&chunk[..read]);
            }
            typed = keys.recv() => {
                let Some(typed) = typed else {
                    carried.send(Body::Detach).await?;
                    return Ok(Ended::Detached);
                };
                let (bytes, leave) = detach.filter(&typed);
                if !bytes.is_empty() {
                    carried.send(Body::Input(bytes)).await?;
                }
                if leave {
                    carried.send(Body::Detach).await?;
                    return Ok(Ended::Detached);
                }
            }
            _ = look.tick() => {
                let now = console_size();
                if now != size {
                    size = now;
                    carried.send(Body::Resize { size, cell_px: CELL_PX }).await?;
                }
            }
        }
    }
}

/// `lume ctl attach --raw`. Returns the exit status.
pub async fn raw(ctl: &Ctl) -> u8 {
    let size = console_size();
    let mut carried = match open(ctl, size).await {
        Ok(carried) => carried,
        Err(failure) => return ctl::report(Err(failure)),
    };
    let ended = {
        let raw = match console::Raw::enter() {
            Ok(raw) => raw,
            Err(error) => {
                let _ = carried.send(Body::Detach).await;
                println!("{}", ctl::failure(codes::BAD_REQUEST, error));
                return EXIT_FAILED;
            }
        };
        let ended = carry(&mut carried, size).await;
        drop(raw);
        ended
    };
    // Past whatever the session drew last.
    let _ = console::write(b"\x1b[0m\r\n");
    let session = carried.session;
    let via = carried.via;
    let said: Value = match ended {
        Ok(Ended::Detached) => json!({
            "ok": true,
            "effect": format!("detached from session {session}, which runs on; shown through {via}"),
        }),
        Ok(Ended::Exited(status)) => json!({
            "ok": true,
            "effect": format!("the program of session {session} ended"),
            "type": "exited",
            "status": status,
        }),
        Ok(Ended::Lost) => ctl::failure("disconnected", format!("{via} closed the connection")),
        Err(error) => ctl::failure("disconnected", error),
    };
    let failed = said["ok"] == json!(false);
    println!("{said}");
    if failed { EXIT_FAILED } else { EXIT_OK }
}

/// Reads the screen, from the application if it runs, else from the host.
enum Reader {
    App(Client),
    Host(HostClient, SessionId),
}

impl Reader {
    async fn open(ctl: &Ctl) -> Result<Self, Failure> {
        let endpoint = endpoint(ctl.endpoint.as_deref()).map_err(Failure::Broken)?;
        match Client::connect(&endpoint).await {
            Ok(client) => Ok(Self::App(client)),
            Err(Failure::NotRunning(error)) => {
                let Some(host) = direct::reach(ctl.endpoint.as_deref()).await? else {
                    return Err(Failure::NotRunning(error));
                };
                let AppRequestBody::Screen {
                    pane: None,
                    session: Some(session),
                } = ctl.request
                else {
                    return Err(direct::needs_app(
                        "watching a session not named by --session",
                    ));
                };
                Ok(Self::Host(host, session))
            }
            Err(failure) => Err(failure),
        }
    }

    async fn screen(&mut self, ctl: &Ctl) -> Result<AppReply, Failure> {
        match self {
            Self::App(client) => client.ask(ctl.request.clone(), ANSWER_TIMEOUT).await,
            Self::Host(host, session) => direct::screen(host, *session).await,
        }
    }
}

/// `lume ctl attach --text`. Returns the exit status.
pub async fn text(ctl: &Ctl) -> u8 {
    let mut reader = match Reader::open(ctl).await {
        Ok(reader) => reader,
        Err(failure) => return ctl::report(Err(failure)),
    };
    let deadline = ctl.follow_for.map(|limit| Instant::now() + limit);
    let mut last: Option<AppReplyBody> = None;
    loop {
        match reader.screen(ctl).await {
            Ok(reply) => {
                if last.as_ref() != Some(&reply.body) {
                    if ctl.plain {
                        if let AppReplyBody::Screen { lines, .. } = &reply.body {
                            println!("{}", lines.join("\n"));
                        }
                    } else {
                        println!("{}", ctl::success(&reply));
                    }
                    last = Some(reply.body);
                }
            }
            Err(Failure::Refused(error)) if error.code == codes::NOT_FOUND && last.is_some() => {
                println!(
                    "{}",
                    json!({ "ok": true, "effect": "the session has gone" })
                );
                return EXIT_OK;
            }
            Err(failure) => return ctl::report(Err(failure)),
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return EXIT_OK;
        }
        tokio::time::sleep(TEXT_POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_detach_key_twice_detaches_and_once_is_typed() {
        let mut key = DetachKey::default();
        assert_eq!(key.filter(b"ls\r"), (b"ls\r".to_vec(), false));
        // Once, then another key: both are typed.
        assert_eq!(key.filter(&[DETACH_KEY]), (Vec::new(), false));
        assert_eq!(key.filter(b"x"), (vec![DETACH_KEY, b'x'], false));
        // Twice in a row, even across reads: detach, and what came before goes.
        assert_eq!(key.filter(&[b'a', DETACH_KEY]), (b"a".to_vec(), false));
        assert_eq!(key.filter(&[DETACH_KEY, b'b']), (Vec::new(), true));
    }
}
