//! What a view sends to its terminal, and what is asked of a terminal without a view,
//! against a pretend server.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{DataSide, Listener, SESSION, Served, World, client, serve, streams, until};
use lume_proto::{CellPx, ClientFacts, ColorScheme, ErrorCode, Palette, SessionId, TermSize};
use lume_rex::RexBackend;
use lume_session::{Attachment, SessionBackend, SessionError};
use serde_json::{Value, json};

fn facts(focused: bool) -> ClientFacts {
    ClientFacts {
        size: TermSize::new(120, 40).unwrap(),
        cell_px: CellPx { w: 9, h: 18 },
        palette: Palette::default(),
        scheme: ColorScheme::Dark,
        focused,
        visible: true,
    }
}

struct Setup {
    backend: Arc<RexBackend>,
    served: Served,
    listener: Listener,
    id: SessionId,
    /// The server's name for the terminal.
    block: String,
}

async fn setup_with(world: World) -> Setup {
    let (dialer, mut listener) = streams();
    let backend = RexBackend::over("test".to_owned(), dialer, client());
    let served = serve(listener.accept().await, world);
    backend.ready().await.unwrap();
    let view = backend.open_session(SESSION).await.unwrap();
    let block = view
        .blocks()
        .find(|block| block.is_terminal())
        .and_then(|block| block.block_id.clone())
        .unwrap();
    let id = backend.list().await.unwrap()[0].id;
    Setup {
        backend,
        served,
        listener,
        id,
        block,
    }
}

impl Setup {
    async fn attach(&mut self, facts: ClientFacts) -> (Attachment, DataSide) {
        let backend = Arc::clone(&self.backend);
        let id = self.id;
        let attaching = tokio::spawn(async move { backend.attach(id, facts).await });
        let mut side = DataSide::accept(self.listener.stream().await).await;
        let copy = lume_vt::Terminal::new(120, 40).unwrap().snapshot().unwrap();
        side.bytes(&copy).await;
        side.size(120, 40).await;
        (attaching.await.unwrap().unwrap(), side)
    }

    /// What was typed into the terminal so far, call by call.
    fn typed(&self) -> Vec<String> {
        self.served
            .asked("com.superlogical.terminal.write")
            .iter()
            .map(|asked| asked["args"]["data"].as_str().unwrap().to_owned())
            .collect()
    }

    /// The calls a view's coming, going and input make, in order.
    fn said(&self) -> Vec<String> {
        self.served.called_of(&[
            "session.own_block",
            "session.disown_block",
            "block.event",
            "com.superlogical.terminal.resize",
            "com.superlogical.terminal.write",
            "data.connect",
        ])
    }
}

#[tokio::test]
async fn a_view_that_has_the_focus_takes_its_block_and_gives_it_its_size() {
    let mut setup = setup_with(World::recorded()).await;
    let (attachment, _side) = setup.attach(facts(true)).await;

    // Before the stream begins, so that the copy the view starts from is of its size.
    assert_eq!(
        setup.said(),
        [
            "session.own_block",
            "block.event",
            "com.superlogical.terminal.resize",
            "data.connect"
        ]
    );
    assert_eq!(
        setup.served.asked("session.own_block"),
        [json!({ "session_id": SESSION, "block_ids": [setup.block], "mode": "take" })]
    );
    assert_eq!(
        setup.served.asked("block.event")[0]["event"],
        json!({ "type": "focus", "payload": { "action": "in" } })
    );
    assert_eq!(
        setup.served.asked("com.superlogical.terminal.resize")[0]["args"],
        json!({ "columns": 120, "rows": 40, "cell_width_px": 9, "cell_height_px": 18 })
    );

    // Gone, it gives the block up, for whoever is next in line.
    attachment.input.detach().await.unwrap();
    assert_eq!(
        setup.served.asked("session.disown_block"),
        [json!({ "session_id": SESSION, "block_ids": [setup.block] })]
    );
}

#[tokio::test]
async fn a_view_without_the_focus_only_says_what_size_it_would_like() {
    let mut setup = setup_with(World::recorded()).await;
    let (attachment, _side) = setup.attach(facts(false)).await;
    assert_eq!(
        setup.said(),
        ["com.superlogical.terminal.resize", "data.connect"]
    );

    // The focus comes and goes: the block is taken each time it comes, and nothing is
    // said when what a view reports changes nothing.
    let input = &attachment.input;
    input.facts(&facts(false)).await.unwrap();
    input.facts(&facts(true)).await.unwrap();
    input.facts(&facts(true)).await.unwrap();
    input.facts(&facts(false)).await.unwrap();
    input
        .resize(TermSize::new(100, 30).unwrap(), CellPx::default())
        .await
        .unwrap();
    until("the size is asked for", || {
        setup.served.asked("com.superlogical.terminal.resize").len() == 2
    })
    .await;
    assert_eq!(
        setup.said()[2..],
        [
            "session.own_block",
            "block.event",
            "block.event",
            "com.superlogical.terminal.resize"
        ]
    );
    let actions: Vec<Value> = setup
        .served
        .asked("block.event")
        .iter()
        .map(|asked| asked["event"]["payload"]["action"].clone())
        .collect();
    assert_eq!(actions, [json!("in"), json!("out")]);
    // A cell whose size is not known is not said.
    assert_eq!(
        setup.served.asked("com.superlogical.terminal.resize")[1]["args"],
        json!({ "columns": 100, "rows": 30 })
    );
}

#[tokio::test]
async fn what_is_typed_reaches_the_terminal_in_order_as_text() {
    let mut setup = setup_with(World::recorded()).await;
    let (attachment, _side) = setup.attach(facts(false)).await;
    let input = &attachment.input;

    let snowman = "☃".as_bytes();
    let mut all = String::new();
    for round in 0..50 {
        let line = format!("line {round}\r");
        input.write(line.as_bytes()).await.unwrap();
        all.push_str(&line);
    }
    // A character in two pieces, a key that is not a letter, and bytes that are no
    // text at all.
    input.write(&snowman[..2]).await.unwrap();
    input.write(&snowman[2..]).await.unwrap();
    input.write(b"\x1b[A").await.unwrap();
    input.write(b"\xff\xfe").await.unwrap();
    input.write(b"end").await.unwrap();
    all.push_str("☃\u{1b}[Aend");

    until("everything is typed", || {
        setup.typed().concat().len() >= all.len()
    })
    .await;
    assert_eq!(setup.typed().concat(), all);
    // Joined while a call was on its way, not one call for each piece.
    assert!(setup.typed().len() < 55, "{}", setup.typed().len());

    // More than goes in one call is typed in several, none of them cut inside a
    // character.
    let long = "é".repeat(70_000);
    input.write(long.as_bytes()).await.unwrap();
    until("the long text is typed", || {
        setup.typed().concat().len() == all.len() + long.len()
    })
    .await;
    let calls = setup.typed();
    assert!(calls.iter().all(|call| call.len() <= 65_536));
    assert!(calls.concat().ends_with(&long));
}

#[tokio::test]
async fn a_terminal_is_typed_into_read_and_ended_without_a_view() {
    let mut world = World::recorded();
    world.screen = vec![
        "\u{1b}[2J\u{1b}[Hfirst row\r\n".to_owned(),
        "second \u{1b}[1mrow\u{1b}[0m".to_owned(),
    ];
    world.alternate = true;
    let setup = setup_with(world).await;
    let backend = &setup.backend;

    backend.send(setup.id, "make\r").await.unwrap();
    assert_eq!(setup.typed(), ["make\r"]);

    let screen = backend.screen(setup.id).await.unwrap();
    assert_eq!((screen.size.cols(), screen.size.rows()), (80, 24));
    assert_eq!(screen.lines.len(), 24);
    assert_eq!(screen.lines[0], "first row");
    assert_eq!(screen.lines[1], "second row");
    assert_eq!((screen.cursor.col, screen.cursor.row), (10, 1));
    assert!(screen.alternate);

    let found = backend
        .wait_text(setup.id, "second", Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!((found.row, found.line.as_str()), (1, "second row"));
    let never = backend
        .wait_text(setup.id, "third", Duration::from_millis(450))
        .await;
    match never {
        Err(SessionError::Refused(error)) => assert_eq!(error.code, ErrorCode::Timeout),
        other => panic!("{other:?}"),
    }
    // It looked more than once while it waited.
    assert!(setup.served.asked("com.superlogical.terminal.format").len() >= 4);

    backend.kill(setup.id).await.unwrap();
    assert_eq!(
        setup.served.asked("block.close"),
        [json!({ "session_id": SESSION, "block_id": setup.block })]
    );
}
