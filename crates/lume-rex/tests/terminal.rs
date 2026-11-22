//! Attaching to a terminal block, against a pretend server: what a view starts from,
//! what it is sent and when, and how the stream is kept through what goes wrong.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{
    DATA_TOKEN, DataSide, Listener, NONCE, SESSION, Served, Tell, World, client, serve, streams,
};
use lume_proto::{
    CellPx, ClientFacts, ColorScheme, ErrorCode, ExitStatus, Palette, SessionId, TermSize,
};
use lume_rex::RexBackend;
use lume_rex::tuning::PENDING_MAX_BYTES;
use lume_session::{Attachment, Hydrate, OutputFrame, SessionBackend, SessionError};
use serde_json::json;

fn facts() -> ClientFacts {
    ClientFacts {
        size: TermSize::new(80, 24).unwrap(),
        cell_px: CellPx { w: 8, h: 16 },
        palette: Palette::default(),
        scheme: ColorScheme::Dark,
        focused: true,
        visible: true,
    }
}

/// A copy of a terminal that shows `text`, as this application's own engine saves one.
fn copy_of(text: &str) -> Vec<u8> {
    let mut terminal = lume_vt::Terminal::new(80, 24).unwrap();
    terminal.write(text.as_bytes());
    terminal.snapshot().unwrap()
}

/// What a copy of a terminal shows on its first row.
fn first_row(copy: &[u8]) -> String {
    let terminal = lume_vt::Terminal::from_snapshot(copy).unwrap();
    terminal.screen_text().unwrap().remove(0)
}

struct Setup {
    backend: Arc<RexBackend>,
    served: Served,
    listener: Listener,
    /// The first terminal of the session that was opened.
    id: SessionId,
}

async fn setup_with(world: World) -> Setup {
    let (dialer, mut listener) = streams();
    let backend = RexBackend::over("test".to_owned(), dialer, client());
    let served = serve(listener.accept().await, world);
    backend.ready().await.unwrap();
    backend.open_session(SESSION).await.unwrap();
    let id = backend.list().await.unwrap()[0].id;
    Setup {
        backend,
        served,
        listener,
        id,
    }
}

async fn setup() -> Setup {
    setup_with(World::recorded()).await
}

impl Setup {
    /// The next data connection, welcomed, with a stream begun on it: the copy of a
    /// terminal that shows `text`, and its size.
    async fn stream(&mut self, text: &str) -> DataSide {
        let mut side = DataSide::accept(self.listener.stream().await).await;
        side.bytes(&copy_of(text)).await;
        side.size(80, 24).await;
        side
    }

    /// Attaches to the terminal, with a stream begun as [`stream`](Self::stream) does.
    async fn attach(&mut self, text: &str) -> (Attachment, DataSide) {
        let backend = Arc::clone(&self.backend);
        let id = self.id;
        let attaching = tokio::spawn(async move { backend.attach(id, facts()).await });
        let side = self.stream(text).await;
        (attaching.await.unwrap().unwrap(), side)
    }

    fn data_connects(&self) -> usize {
        self.served
            .called()
            .iter()
            .filter(|method| *method == "data.connect")
            .count()
    }
}

/// The next frame of an attachment, or `None` once it is over.
async fn frame(attachment: &mut Attachment) -> Option<OutputFrame> {
    tokio::time::timeout(Duration::from_secs(20), attachment.output.next())
        .await
        .expect("no frame came")
}

/// Whether nothing more comes of an attachment for a moment.
async fn quiet(attachment: &mut Attachment) -> bool {
    tokio::time::timeout(Duration::from_millis(150), attachment.output.next())
        .await
        .is_err()
}

fn code_of(error: SessionError) -> ErrorCode {
    match error {
        SessionError::Refused(error) => error.code,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_view_starts_from_the_servers_copy_and_is_sent_what_it_grants() {
    let mut setup = setup().await;
    let backend = Arc::clone(&setup.backend);
    let id = setup.id;
    let attaching = tokio::spawn(async move { backend.attach(id, facts()).await });

    // The copy, some output hard on its heels, the size, and more output.
    let mut side = DataSide::accept(setup.listener.stream().await).await;
    let copy = copy_of("as it stood");
    let mut head = copy.clone();
    head.extend(b"early ");
    side.bytes(&head).await;
    side.size(100, 30).await;
    side.bytes(b"hello world").await;
    let mut attachment = attaching.await.unwrap().unwrap();

    assert_eq!(side.first[0], 0x01);
    assert_eq!((attachment.size.cols(), attachment.size.rows()), (100, 30));
    match &attachment.hydrate {
        Hydrate::Snapshot { vt_version, bytes } => {
            assert_eq!(vt_version, lume_vt::library_version());
            // The copy ends where the engine says it does, not where the size begins.
            assert_eq!(bytes, &copy);
            assert_eq!(first_row(bytes), "as it stood");
        }
        other => panic!("{other:?}"),
    }

    // Nothing is sent that was not granted.
    assert!(quiet(&mut attachment).await);
    attachment.input.credit(8).await.unwrap();
    assert_eq!(
        frame(&mut attachment).await,
        Some(OutputFrame::Data(b"early ".to_vec()))
    );
    assert_eq!(
        frame(&mut attachment).await,
        Some(OutputFrame::Data(b"he".to_vec()))
    );
    assert!(quiet(&mut attachment).await);
    attachment.input.credit(1000).await.unwrap();
    assert_eq!(
        frame(&mut attachment).await,
        Some(OutputFrame::Data(b"llo world".to_vec()))
    );

    // A size is told in its place among the output, and only when it is a new one.
    side.bytes(b"before").await;
    side.size(100, 30).await;
    side.size(120, 40).await;
    side.bytes(b"after").await;
    assert_eq!(
        frame(&mut attachment).await,
        Some(OutputFrame::Data(b"before".to_vec()))
    );
    assert_eq!(
        frame(&mut attachment).await,
        Some(OutputFrame::Resized(TermSize::new(120, 40).unwrap()))
    );
    assert_eq!(
        frame(&mut attachment).await,
        Some(OutputFrame::Data(b"after".to_vec()))
    );

    // The program's end is the last thing a view hears, and the stream is closed.
    side.note(NONCE, json!({ "type": "child_exited", "exit_code": 3 }))
        .await;
    assert_eq!(
        frame(&mut attachment).await,
        Some(OutputFrame::Exit(ExitStatus::Code(3)))
    );
    assert_eq!(frame(&mut attachment).await, None);
    assert_eq!(side.closed_by_client().await, Some(1000));
    assert!(matches!(
        attachment.input.credit(1).await,
        Err(SessionError::Detached)
    ));
}

#[tokio::test]
async fn a_view_that_takes_too_little_is_started_again_when_it_takes_more() {
    let mut setup = setup().await;
    let (mut attachment, mut side) = setup.attach("first").await;
    attachment.input.credit(10).await.unwrap();

    // More output than is kept for a view, which takes ten bytes of it.
    let piece = vec![b'x'; 65_536];
    let sending = tokio::spawn(async move {
        for _ in 0..=PENDING_MAX_BYTES / piece.len() + 1 {
            side.bytes(&piece).await;
        }
        // The stream is let go.
        side.closed_by_client().await
    });
    assert_eq!(
        frame(&mut attachment).await,
        Some(OutputFrame::Data(vec![b'x'; 10]))
    );
    assert_eq!(sending.await.unwrap(), Some(1000));
    assert!(quiet(&mut attachment).await);
    assert_eq!(setup.data_connects(), 1);

    // Credit says the view is taking output again: a new stream, from a new copy.
    attachment.input.credit(100).await.unwrap();
    let mut side = setup.stream("second").await;
    side.bytes(b"fresh").await;
    match frame(&mut attachment).await {
        Some(OutputFrame::Hydrate(Hydrate::Snapshot { bytes, .. })) => {
            assert_eq!(first_row(&bytes), "second");
        }
        other => panic!("{other:?}"),
    }
    // None of what waited is sent after the copy that already holds it.
    assert_eq!(
        frame(&mut attachment).await,
        Some(OutputFrame::Data(b"fresh".to_vec()))
    );
    assert_eq!(setup.data_connects(), 2);
}

#[tokio::test]
async fn a_view_lets_go_of_its_screen_and_asks_for_it_again() {
    let mut setup = setup().await;
    let (mut attachment, mut side) = setup.attach("first").await;
    attachment.input.credit(1000).await.unwrap();
    side.bytes(b"shown").await;
    assert_eq!(
        frame(&mut attachment).await,
        Some(OutputFrame::Data(b"shown".to_vec()))
    );

    attachment.input.release().await.unwrap();
    assert_eq!(side.closed_by_client().await, Some(1000));
    assert!(quiet(&mut attachment).await);
    assert_eq!(setup.data_connects(), 1);

    // The terminal changed size meanwhile: the view hears of that before the copy.
    attachment.input.rehydrate().await.unwrap();
    let mut side = DataSide::accept(setup.listener.stream().await).await;
    side.bytes(&copy_of("second")).await;
    side.size(132, 50).await;
    assert_eq!(
        frame(&mut attachment).await,
        Some(OutputFrame::Resized(TermSize::new(132, 50).unwrap()))
    );
    assert!(matches!(
        frame(&mut attachment).await,
        Some(OutputFrame::Hydrate(Hydrate::Snapshot { .. }))
    ));

    // Asked for while the stream is followed, it is a new stream all the same.
    attachment.input.rehydrate().await.unwrap();
    assert_eq!(side.closed_by_client().await, Some(1000));
    let mut side = DataSide::accept(setup.listener.stream().await).await;
    side.bytes(&copy_of("third")).await;
    side.size(132, 50).await;
    match frame(&mut attachment).await {
        Some(OutputFrame::Hydrate(Hydrate::Snapshot { bytes, .. })) => {
            assert_eq!(first_row(&bytes), "third");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(setup.data_connects(), 3);
}

#[tokio::test]
async fn a_connection_that_is_lost_is_continued_where_it_stopped() {
    let mut setup = setup().await;
    let (mut attachment, mut side) = setup.attach("first").await;
    attachment.input.credit(1000).await.unwrap();
    // Half a message of the server's own, and the connection drops.
    let size = common::note_bytes(NONCE, &json!({ "type": "resize", "cols": 90, "rows": 20 }));
    side.bytes(b"one ").await;
    side.bytes(&size[..20]).await;
    let sent = side.sent;
    assert_eq!(
        frame(&mut attachment).await,
        Some(OutputFrame::Data(b"one ".to_vec()))
    );
    drop(side);

    let mut side = DataSide::shake(setup.listener.stream().await).await;
    let mut resume = vec![0x02];
    resume.extend(DATA_TOKEN);
    resume.extend(sent.to_le_bytes());
    assert_eq!(side.first, resume);
    side.welcome([0x11; 16], 60_000, NONCE).await;
    side.bytes(&size[20..]).await;
    side.bytes(b"two").await;
    assert_eq!(
        frame(&mut attachment).await,
        Some(OutputFrame::Resized(TermSize::new(90, 20).unwrap()))
    );
    assert_eq!(
        frame(&mut attachment).await,
        Some(OutputFrame::Data(b"two".to_vec()))
    );
    // No new ticket was needed.
    assert_eq!(setup.data_connects(), 1);

    // Lost again, and this time the server no longer has the stream: a new one.
    let sent = sent + side.sent;
    drop(side);
    let side = DataSide::shake(setup.listener.stream().await).await;
    assert_eq!(side.first[17..], sent.to_le_bytes());
    assert_eq!(side.first[1..17], [0x11; 16]);
    side.reject(0x03).await;
    let _side = setup.stream("anew").await;
    match frame(&mut attachment).await {
        Some(OutputFrame::Resized(size)) => assert_eq!((size.cols(), size.rows()), (80, 24)),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        frame(&mut attachment).await,
        Some(OutputFrame::Hydrate(Hydrate::Snapshot { .. }))
    ));
    assert_eq!(setup.data_connects(), 2);
}

#[tokio::test]
async fn a_stream_the_server_lets_go_of_is_begun_again() {
    let mut setup = setup().await;
    let (mut attachment, side) = setup.attach("first").await;
    tokio::spawn(side.close(1013, "data reconnect required"));
    let side = setup.stream("second").await;
    assert!(matches!(
        frame(&mut attachment).await,
        Some(OutputFrame::Hydrate(Hydrate::Snapshot { .. }))
    ));

    // A stream that fails is begun again too.
    tokio::spawn(side.close(1011, "data provider failed"));
    let _side = setup.stream("third").await;
    match frame(&mut attachment).await {
        Some(OutputFrame::Hydrate(Hydrate::Snapshot { bytes, .. })) => {
            assert_eq!(first_row(&bytes), "third");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(setup.data_connects(), 3);
}

#[tokio::test]
async fn a_block_that_closes_ends_its_view() {
    let mut setup = setup().await;
    let (mut attachment, mut side) = setup.attach("first").await;
    attachment.input.credit(1000).await.unwrap();
    side.bytes(b"last words").await;
    tokio::spawn(side.close(4000, "block closed"));
    assert_eq!(
        frame(&mut attachment).await,
        Some(OutputFrame::Data(b"last words".to_vec()))
    );
    assert_eq!(
        frame(&mut attachment).await,
        Some(OutputFrame::Exit(ExitStatus::Unknown))
    );
    assert_eq!(frame(&mut attachment).await, None);
    assert_eq!(setup.data_connects(), 1);

    // And the terminal can be attached to again, were it still there.
    let backend = Arc::clone(&setup.backend);
    let id = setup.id;
    let attaching = tokio::spawn(async move { backend.attach(id, facts()).await });
    DataSide::shake(setup.listener.stream().await)
        .await
        .reject(0x03)
        .await;
    assert_eq!(
        code_of(attaching.await.unwrap().err().unwrap()),
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn a_copy_this_engine_cannot_load_gives_way_to_the_screen_written_out() {
    let mut world = World::recorded();
    world.screen = vec!["\u{1b}[2Jwritten ".to_owned(), "out".to_owned()];
    let mut setup = setup_with(world).await;
    let backend = Arc::clone(&setup.backend);
    let id = setup.id;
    let attaching = tokio::spawn(async move { backend.attach(id, facts()).await });
    let mut side = DataSide::accept(setup.listener.stream().await).await;
    side.bytes(b"GHOSTSNP\x09\x09\x09\x09 a copy of some later engine")
        .await;
    side.size(80, 24).await;
    side.bytes(b"and on").await;
    let mut attachment = attaching.await.unwrap().unwrap();

    assert_eq!(
        attachment.hydrate,
        Hydrate::VtReplay(b"\x1b[2Jwritten out".to_vec())
    );
    let called = setup.served.called();
    assert!(called.contains(&"com.superlogical.terminal.format".to_owned()));
    assert!(called.contains(&"com.superlogical.terminal.blob_read".to_owned()));
    attachment.input.credit(100).await.unwrap();
    assert_eq!(
        frame(&mut attachment).await,
        Some(OutputFrame::Data(b"and on".to_vec()))
    );

    // When the screen cannot be read either, the view starts blank and still follows.
    attachment.input.detach().await.unwrap();
    let mut gone = World::recorded();
    gone.screen = vec!["part".to_owned(), String::new(), "never ends".to_owned()];
    setup.served.tell(Tell::World(gone));
    let backend = Arc::clone(&setup.backend);
    let attaching = tokio::spawn(async move { backend.attach(id, facts()).await });
    let mut side = DataSide::accept(setup.listener.stream().await).await;
    side.bytes(b"not a copy at all").await;
    side.size(80, 24).await;
    let attachment = attaching.await.unwrap().unwrap();
    assert_eq!(attachment.hydrate, Hydrate::None);
}

#[tokio::test]
async fn a_terminal_takes_one_attachment_at_a_time() {
    let mut setup = setup().await;
    let (mut attachment, mut side) = setup.attach("first").await;

    let again = setup.backend.attach(setup.id, facts()).await;
    assert_eq!(code_of(again.err().unwrap()), ErrorCode::AlreadyAttached);
    let unknown = setup
        .backend
        .attach(SessionId::new(9999).unwrap(), facts())
        .await;
    assert_eq!(code_of(unknown.err().unwrap()), ErrorCode::NotFound);
    assert_eq!(setup.data_connects(), 1);

    // More credit than a stream may hold is not taken.
    attachment.input.credit(u32::MAX).await.unwrap_err();
    assert!(matches!(
        attachment.input.credit(u32::MAX).await,
        Err(SessionError::CreditOverflow)
    ));

    // Detached, its stream is closed before the call returns, and the way is free.
    let closed = tokio::spawn(async move { side.closed_by_client().await });
    attachment.input.detach().await.unwrap();
    assert_eq!(closed.await.unwrap(), Some(1000));
    assert_eq!(frame(&mut attachment).await, None);
    for refused in [
        attachment.input.detach().await,
        attachment.input.rehydrate().await,
        attachment.input.release().await,
        attachment.input.credit(1).await,
        attachment.input.write(b"x").await,
    ] {
        assert!(
            matches!(refused, Err(SessionError::Detached)),
            "{refused:?}"
        );
    }
    let (_attachment, _side) = setup.attach("second").await;
    assert_eq!(setup.data_connects(), 2);
}

#[tokio::test]
async fn a_stream_that_cannot_be_begun_is_an_attach_that_fails() {
    let mut setup = setup().await;
    let id = setup.id;

    // Refused at the handshake.
    let backend = Arc::clone(&setup.backend);
    let attaching = tokio::spawn(async move { backend.attach(id, facts()).await });
    DataSide::shake(setup.listener.stream().await)
        .await
        .reject(0x02)
        .await;
    assert_eq!(
        code_of(attaching.await.unwrap().err().unwrap()),
        ErrorCode::AlreadyAttached
    );

    // A stream that says no size after its copy, however much it sends.
    let backend = Arc::clone(&setup.backend);
    let attaching = tokio::spawn(async move { backend.attach(id, facts()).await });
    let mut side = DataSide::accept(setup.listener.stream().await).await;
    side.bytes(&copy_of("a copy")).await;
    side.note(NONCE, json!({ "type": "theme", "scheme": "dark" }))
        .await;
    for _ in 0..4 {
        side.bytes(&vec![b'x'; 65_536]).await;
    }
    assert!(matches!(
        attaching.await.unwrap(),
        Err(SessionError::Protocol(_))
    ));

    // A stream that closes before it has begun.
    let backend = Arc::clone(&setup.backend);
    let attaching = tokio::spawn(async move { backend.attach(id, facts()).await });
    let side = DataSide::accept(setup.listener.stream().await).await;
    tokio::spawn(side.close(4000, "block closed"));
    assert_eq!(
        code_of(attaching.await.unwrap().err().unwrap()),
        ErrorCode::NotFound
    );

    // Each failure leaves the terminal free to be attached to.
    let (_attachment, _side) = setup.attach("at last").await;
}

#[tokio::test]
async fn a_backend_that_is_shut_down_ends_its_attachments() {
    let mut setup = setup().await;
    let (mut attachment, mut side) = setup.attach("first").await;
    setup.backend.shutdown();
    assert_eq!(frame(&mut attachment).await, None);
    assert_eq!(side.closed_by_client().await, Some(1000));
}
