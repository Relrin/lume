//! Attaching to a terminal block, against a pretend server: what a view starts from,
//! what it is sent and when, and how the stream is kept through what goes wrong.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{
    DATA_TOKEN, DataSide, Listener, NONCE, SESSION, Served, Tell, World, client, serve, streams,
};
use lume_proto::{
    CellPx, ClientFacts, ColorScheme, ErrorCode, ExitStatus, Palette, ProgramState, ProgramStatus,
    SessionEvent, SessionId, TermSize,
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
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
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

/// The next frame of an attachment, of whatever kind, or `None` once it is over.
async fn any_frame(attachment: &mut Attachment) -> Option<OutputFrame> {
    tokio::time::timeout(Duration::from_secs(20), attachment.output.next())
        .await
        .expect("no frame came")
}

/// The next frame of an attachment that is not a report of the terminal's, or `None`
/// once it is over.
async fn frame(attachment: &mut Attachment) -> Option<OutputFrame> {
    loop {
        match any_frame(attachment).await {
            Some(OutputFrame::Event(_)) => {}
            other => return other,
        }
    }
}

/// Whether nothing but reports comes of an attachment for a moment.
async fn quiet(attachment: &mut Attachment) -> bool {
    let waiting = async {
        loop {
            match attachment.output.next().await {
                Some(OutputFrame::Event(_)) => {}
                _ => return,
            }
        }
    };
    tokio::time::timeout(Duration::from_millis(150), waiting)
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
async fn the_views_of_a_terminal_share_its_one_stream() {
    let mut setup = setup().await;
    let (mut first, mut side) = setup.attach("as it stood").await;
    first.input.credit(1000).await.unwrap();
    side.bytes(b"one").await;
    assert_eq!(
        frame(&mut first).await,
        Some(OutputFrame::Data(b"one".to_vec()))
    );

    // A second view: a copy of the terminal comes only at the head of a stream, so
    // the stream is begun again, and the view that was following starts again too.
    let closed = tokio::spawn(async move { side.closed_by_client().await });
    let (mut second, mut side) = setup.attach("with one").await;
    assert_eq!(closed.await.unwrap(), Some(1000));
    assert_eq!(setup.data_connects(), 2);
    match &second.hydrate {
        Hydrate::Snapshot { bytes, .. } => assert_eq!(first_row(bytes), "with one"),
        other => panic!("{other:?}"),
    }
    match frame(&mut first).await {
        Some(OutputFrame::Hydrate(Hydrate::Snapshot { bytes, .. })) => {
            assert_eq!(first_row(&bytes), "with one");
        }
        other => panic!("{other:?}"),
    }

    // Both are sent what the terminal prints, each as it grants.
    side.bytes(b"both").await;
    side.size(90, 30).await;
    assert_eq!(
        frame(&mut first).await,
        Some(OutputFrame::Data(b"both".to_vec()))
    );
    let size = OutputFrame::Resized(TermSize::new(90, 30).unwrap());
    assert_eq!(frame(&mut first).await, Some(size.clone()));
    assert!(quiet(&mut second).await);
    second.input.credit(2).await.unwrap();
    assert_eq!(
        frame(&mut second).await,
        Some(OutputFrame::Data(b"bo".to_vec()))
    );
    second.input.credit(1000).await.unwrap();
    assert_eq!(
        frame(&mut second).await,
        Some(OutputFrame::Data(b"th".to_vec()))
    );
    assert_eq!(frame(&mut second).await, Some(size));

    // One view lets go of its screen: the other is still sent everything, and the
    // stream stays.
    second.input.release().await.unwrap();
    side.bytes(b"for one").await;
    assert_eq!(
        frame(&mut first).await,
        Some(OutputFrame::Data(b"for one".to_vec()))
    );
    assert!(quiet(&mut second).await);
    assert_eq!(setup.data_connects(), 2);

    // It asks for its screen again: a new stream, and both start from its copy.
    let closed = tokio::spawn(async move { side.closed_by_client().await });
    second.input.rehydrate().await.unwrap();
    let mut side = setup.stream("for both again").await;
    assert_eq!(closed.await.unwrap(), Some(1000));
    for view in [&mut first, &mut second] {
        // The new stream says the terminal is 80 by 24 again.
        assert!(matches!(frame(view).await, Some(OutputFrame::Resized(_))));
        match frame(view).await {
            Some(OutputFrame::Hydrate(Hydrate::Snapshot { bytes, .. })) => {
                assert_eq!(first_row(&bytes), "for both again");
            }
            other => panic!("{other:?}"),
        }
    }

    // One view goes: the other is served on, on the same stream.
    second.input.detach().await.unwrap();
    assert_eq!(frame(&mut second).await, None);
    side.bytes(b"alone").await;
    assert_eq!(
        frame(&mut first).await,
        Some(OutputFrame::Data(b"alone".to_vec()))
    );
    assert_eq!(setup.data_connects(), 3);
    assert!(setup.served.asked("session.disown_block").is_empty());

    // The last view goes: the stream is closed and the block given up.
    let closed = tokio::spawn(async move { side.closed_by_client().await });
    first.input.detach().await.unwrap();
    assert_eq!(closed.await.unwrap(), Some(1000));
    assert_eq!(setup.served.asked("session.disown_block").len(), 1);
}

#[tokio::test]
async fn a_view_that_takes_too_little_does_not_hold_the_others_back() {
    let mut setup = setup().await;
    let (mut quick, side) = setup.attach("first").await;
    let closed = tokio::spawn(async move {
        let mut side = side;
        side.closed_by_client().await
    });
    let (mut slow, mut side) = setup.attach("second").await;
    assert_eq!(closed.await.unwrap(), Some(1000));
    assert!(matches!(
        frame(&mut quick).await,
        Some(OutputFrame::Hydrate(_))
    ));

    // More output than is kept for a view. One view takes all of it, the other none.
    let piece = vec![b'x'; 65_536];
    let pieces = PENDING_MAX_BYTES / piece.len() + 2;
    let sending = tokio::spawn(async move {
        for _ in 0..pieces {
            side.bytes(&piece).await;
        }
        side
    });
    let mut taken = 0;
    while taken < pieces * 65_536 {
        quick.input.credit(65_536).await.unwrap();
        match frame(&mut quick).await {
            Some(OutputFrame::Data(bytes)) => taken += bytes.len(),
            other => panic!("{other:?}"),
        }
    }
    let mut side = sending.await.unwrap();
    assert!(quiet(&mut slow).await);
    // The stream was kept all along, for the view that followed it.
    assert_eq!(setup.data_connects(), 2);
    side.bytes(b"still here").await;
    quick.input.credit(100).await.unwrap();
    assert_eq!(
        frame(&mut quick).await,
        Some(OutputFrame::Data(b"still here".to_vec()))
    );

    // The slow view takes output again: it starts from a new copy, and so does the
    // other, which a new stream leaves no choice.
    let closed = tokio::spawn(async move { side.closed_by_client().await });
    slow.input.credit(100).await.unwrap();
    let mut side = setup.stream("third").await;
    assert_eq!(closed.await.unwrap(), Some(1000));
    for view in [&mut slow, &mut quick] {
        match frame(view).await {
            Some(OutputFrame::Hydrate(Hydrate::Snapshot { bytes, .. })) => {
                assert_eq!(first_row(&bytes), "third");
            }
            other => panic!("{other:?}"),
        }
    }
    side.bytes(b"fresh").await;
    assert_eq!(
        frame(&mut slow).await,
        Some(OutputFrame::Data(b"fresh".to_vec()))
    );
}

#[tokio::test]
async fn every_view_hears_of_the_programs_end() {
    let mut setup = setup().await;
    let (mut first, side) = setup.attach("first").await;
    let closed = tokio::spawn(async move {
        let mut side = side;
        side.closed_by_client().await
    });
    let (mut second, mut side) = setup.attach("second").await;
    closed.await.unwrap();
    assert!(matches!(
        frame(&mut first).await,
        Some(OutputFrame::Hydrate(_))
    ));
    // One of them has let go of its screen, and hears of the end all the same.
    second.input.release().await.unwrap();
    side.note(NONCE, json!({ "type": "child_exited", "exit_code": 0 }))
        .await;
    for view in [&mut first, &mut second] {
        assert_eq!(
            frame(view).await,
            Some(OutputFrame::Exit(ExitStatus::Code(0)))
        );
        assert_eq!(frame(view).await, None);
    }
    assert_eq!(side.closed_by_client().await, Some(1000));
}

#[tokio::test]
async fn a_view_that_is_gone_is_refused_everything() {
    let mut setup = setup().await;
    let (mut attachment, mut side) = setup.attach("first").await;

    let unknown = setup
        .backend
        .attach(SessionId::new(9999).unwrap(), facts())
        .await;
    assert_eq!(code_of(unknown.err().unwrap()), ErrorCode::NotFound);
    assert_eq!(setup.data_connects(), 1);

    // More credit than a stream may hold is not taken.
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
        attachment.input.facts(&facts()).await,
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

#[tokio::test]
async fn what_a_terminal_reports_of_itself_reaches_its_views() {
    let mut setup = setup().await;
    let block = setup
        .backend
        .view(SESSION)
        .unwrap()
        .blocks()
        .find(|block| block.is_terminal())
        .and_then(|block| block.block_id.clone())
        .unwrap();
    let (mut first, side) = setup.attach("first").await;

    // A view is told first what the terminal reported before it came.
    let Some(OutputFrame::Event(SessionEvent::State { state })) = any_frame(&mut first).await
    else {
        panic!("no state");
    };
    assert_eq!(state.cwd.as_deref(), Some("file://recorded/home/someone"));
    assert!(state.foreground.is_some());
    assert_eq!(state.statuses.len(), 1);
    assert_eq!(state.statuses[0].state, ProgramState::Working);
    assert_eq!(state.statuses[0].app.as_deref(), Some("cargo"));

    let closed = tokio::spawn(async move {
        let mut side = side;
        side.closed_by_client().await
    });
    let (mut second, _side) = setup.attach("second").await;
    closed.await.unwrap();
    assert!(matches!(
        any_frame(&mut second).await,
        Some(OutputFrame::Event(SessionEvent::State { .. }))
    ));
    assert!(matches!(
        any_frame(&mut first).await,
        Some(OutputFrame::Hydrate(_))
    ));

    let report = |block: &str, creator: &str, name: &str, payload: serde_json::Value| {
        Tell::SessionEvent(
            SESSION.to_owned(),
            "block_event".to_owned(),
            json!({ "block_id": block, "creator_name": creator, "name": name, "payload": payload }),
        )
    };
    let terminal = "com.superlogical.terminal";
    // Of another block, of a block that is no terminal, and of no use to anyone.
    setup
        .served
        .tell(report("block:other", terminal, "bell", json!({})));
    setup
        .served
        .tell(report(&block, "com.superlogical.tunnel", "bell", json!({})));
    setup.served.tell(report(
        &block,
        terminal,
        "clipboard_written",
        json!({ "text": "secret" }),
    ));
    setup.served.tell(report(
        &block,
        terminal,
        "title_changed",
        json!({ "title": 7 }),
    ));
    // And what the views are told, in order.
    setup.served.tell(report(
        &block,
        terminal,
        "title_changed",
        json!({ "title": "vim" }),
    ));
    setup
        .served
        .tell(report(&block, terminal, "bell", json!({})));
    setup.served.tell(report(
        &block,
        terminal,
        "program_status_removed",
        json!({ "ids": [""], "reason": "exit" }),
    ));
    for view in [&mut first, &mut second] {
        assert_eq!(
            any_frame(view).await,
            Some(OutputFrame::Event(SessionEvent::Title {
                title: "vim".to_owned()
            }))
        );
        assert_eq!(
            any_frame(view).await,
            Some(OutputFrame::Event(SessionEvent::Bell))
        );
        assert_eq!(
            any_frame(view).await,
            Some(OutputFrame::Event(SessionEvent::ProgramStatus(
                ProgramStatus::new(ProgramState::Clear, None)
            )))
        );
    }
}

#[tokio::test]
async fn the_reports_a_real_server_sent_are_read() {
    let mut setup = setup().await;
    let block = setup
        .backend
        .view(SESSION)
        .unwrap()
        .blocks()
        .find(|block| block.is_terminal())
        .and_then(|block| block.block_id.clone())
        .unwrap();
    let (mut view, _side) = setup.attach("first").await;
    assert!(matches!(
        any_frame(&mut view).await,
        Some(OutputFrame::Event(SessionEvent::State { .. }))
    ));

    // Every report of a block that was recorded, as if it were of this terminal.
    let recorded = common::Recorded::load();
    let mut sent = 0;
    for (name, payload) in recorded.events() {
        if name != "session_event" || payload["event"]["type"] != "block_event" {
            continue;
        }
        let mut report = payload["event"]["payload"].clone();
        report["block_id"] = json!(block);
        setup.served.tell(Tell::SessionEvent(
            SESSION.to_owned(),
            "block_event".to_owned(),
            report,
        ));
        sent += 1;
    }
    assert!(sent >= 3, "{sent} reports were recorded");
    setup.served.tell(Tell::SessionEvent(
        SESSION.to_owned(),
        "block_event".to_owned(),
        json!({ "block_id": block, "creator_name": "com.superlogical.terminal", "name": "bell" }),
    ));

    // A shell that ran a line said what was in front and what its title was.
    let mut heard = Vec::new();
    loop {
        match any_frame(&mut view).await {
            Some(OutputFrame::Event(SessionEvent::Bell)) => break,
            Some(OutputFrame::Event(event)) => heard.push(event),
            other => panic!("{other:?}"),
        }
    }
    assert!(
        heard.iter().any(
            |event| matches!(event, SessionEvent::Foreground(front) if front.name == "cmd.exe")
        ),
        "{heard:?}"
    );
    assert!(
        heard.iter().any(
            |event| matches!(event, SessionEvent::Title { title } if title.contains("echo  recorded"))
        ),
        "{heard:?}"
    );
}
