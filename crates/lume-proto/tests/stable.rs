//! The wire format does not drift: kind values, error codes and byte layouts are pinned.
//!
//! `tests/vectors/` holds one encoded stream per direction and a JSON description of the
//! messages in it. Decoders written in other languages are tested against the same files.
//! Run with `UPDATE_GOLDEN=1` to rewrite them after a deliberate protocol change.

use std::fs;
use std::path::PathBuf;

use lume_proto::app::ScreenCursor;
use lume_proto::{
    BackendCaps, BlockKind, Body, CellPx, ClientFacts, ClientKind, ColorScheme, CommandState,
    Control, DisconnectReason, Disconnected, ErrorCode, ExitStatus, Foreground, FrameDecoder,
    HydrateKinds, HydrateMode, HydratePayload, Kind, Launcher, Message, Notification,
    PROTO_VERSION, Palette, ProgramState, ProgramStatus, ProtoError, Request, RequestBody,
    Responder, Response, ResponseBody, Rgb, Role, SessionEvent, SessionId, SessionInfo,
    SessionState, SessionStats, SpawnSpec, TermSize, encode,
};
use serde_json::{Value, json};

#[test]
fn protocol_version_is_2() {
    assert_eq!(PROTO_VERSION, 2);
}

#[test]
fn kind_values_are_stable() {
    let expected = [
        (Kind::Control, 0x01),
        (Kind::Hydrate, 0x02),
        (Kind::Data, 0x03),
        (Kind::Resized, 0x04),
        (Kind::Event, 0x05),
        (Kind::Exit, 0x06),
        (Kind::Input, 0x10),
        (Kind::Resize, 0x11),
        (Kind::Facts, 0x12),
        (Kind::Credit, 0x13),
        (Kind::Detach, 0x14),
        (Kind::Rehydrate, 0x15),
        (Kind::Release, 0x16),
    ];
    for (kind, value) in expected {
        assert_eq!(kind as u8, value, "{kind:?}");
        assert_eq!(Kind::from_wire(value), Some(kind));
    }
    let known: Vec<u8> = expected.iter().map(|(_, value)| *value).collect();
    for byte in 0..=u8::MAX {
        if !known.contains(&byte) {
            assert_eq!(Kind::from_wire(byte), None, "{byte:#04x}");
        }
    }
    assert_eq!(Kind::MORE, 0x80);
}

#[test]
fn error_codes_are_stable() {
    let expected = [
        "bad_request",
        "hello_required",
        "version_mismatch",
        "not_found",
        "already_attached",
        "stream_in_use",
        "invalid_stream",
        "invalid_spec",
        "spawn_failed",
        "unsupported",
        "limit_exceeded",
        "session_exited",
        "protocol_error",
        "internal",
        "timeout",
    ];
    assert_eq!(ErrorCode::ALL.len(), expected.len());
    for (code, spelling) in ErrorCode::ALL.into_iter().zip(expected) {
        assert_eq!(code.as_str(), spelling);
        assert_eq!(serde_json::to_value(code).unwrap(), json!(spelling));
        assert_eq!(
            serde_json::from_value::<ErrorCode>(json!(spelling)).unwrap(),
            code
        );
    }
}

fn size(cols: u16, rows: u16) -> TermSize {
    TermSize::new(cols, rows).unwrap()
}

fn facts() -> ClientFacts {
    let mut ansi = [Rgb::default(); 16];
    for (index, colour) in ansi.iter_mut().enumerate() {
        let n = index as u8;
        *colour = Rgb([n, n * 2, n * 3]);
    }
    ClientFacts {
        size: size(120, 40),
        cell_px: CellPx { w: 9, h: 18 },
        palette: Palette {
            fg: Rgb([0xd0, 0xd1, 0xd2]),
            bg: Rgb([0x10, 0x11, 0x12]),
            cursor: Rgb([0xfe, 0xdc, 0xba]),
            ansi,
        },
        scheme: ColorScheme::Dark,
        focused: true,
        visible: false,
    }
}

fn session() -> SessionId {
    SessionId::new(0x0123_4567_89ab_cdef).unwrap()
}

fn request(id: u64, body: RequestBody) -> Message {
    Message::control(Control::Request(Request { id, body }))
}

fn response(id: u64, result: Result<ResponseBody, ProtoError>) -> Message {
    Message::control(Control::Response(Response { id, result }))
}

/// One of every message a client sends.
fn client_messages() -> Vec<Message> {
    vec![
        request(
            1,
            RequestBody::Hello {
                proto: 2,
                proto_min: 2,
                vt_version: Some("1.0.1".to_owned()),
                client_kind: ClientKind::App,
            },
        ),
        request(
            2,
            RequestBody::Spawn {
                spec: SpawnSpec {
                    launcher: Launcher::Shell {
                        program: "pwsh.exe".to_owned(),
                        args: vec!["-NoLogo".to_owned()],
                        login: false,
                    },
                    cwd: Some("C:\\Users\\someone".to_owned()),
                    env: vec![("LANG".to_owned(), "en_GB.UTF-8".to_owned())],
                    size: size(80, 24),
                    shell_integration: false,
                    scrollback_lines: Some(10_000),
                },
            },
        ),
        request(3, RequestBody::List {}),
        request(
            4,
            RequestBody::Attach {
                session: session(),
                stream: 1,
                facts: facts(),
                replay: false,
            },
        ),
        Message::new(1, Body::Credit(1024 * 1024)),
        Message::new(
            1,
            Body::Resize {
                size: size(120, 40),
                cell_px: CellPx { w: 9, h: 18 },
            },
        ),
        Message::new(1, Body::Facts(facts())),
        Message::new(1, Body::Input(b"echo hello\r".to_vec())),
        Message::new(1, Body::Input(Vec::new())),
        Message::new(1, Body::Detach),
        request(5, RequestBody::Kill { session: session() }),
        Message::new(2, Body::Rehydrate),
        request(6, RequestBody::Screen { session: session() }),
        request(
            7,
            RequestBody::WaitText {
                session: session(),
                text: "$ ".to_owned(),
                timeout_ms: 5000,
            },
        ),
        request(8, RequestBody::HostStatus {}),
        request(
            9,
            RequestBody::Shutdown {
                end_sessions: false,
            },
        ),
        // Added after the messages above were pinned, and so after them.
        Message::new(2, Body::Release),
        request(
            10,
            RequestBody::Send {
                session: session(),
                text: "dir\r".to_owned(),
            },
        ),
        request(
            11,
            RequestBody::Attach {
                session: session(),
                stream: 3,
                facts: facts(),
                replay: true,
            },
        ),
    ]
}

/// One of every message a host sends.
fn host_messages() -> Vec<Message> {
    vec![
        response(
            1,
            Ok(ResponseBody::HelloAck {
                proto: 2,
                host_version: "0.0.0".to_owned(),
                vt_version: Some("1.0.1".to_owned()),
                caps: BackendCaps {
                    persistent: false,
                    multi_client: false,
                    server_layout: false,
                    responder: Responder::Backend,
                    hydrate: HydrateKinds {
                        snapshot: true,
                        vt_replay: true,
                    },
                },
                max_payload: 65536,
                pid: 4242,
            }),
        ),
        response(2, Ok(ResponseBody::Spawned { session: session() })),
        response(
            3,
            Ok(ResponseBody::Sessions {
                sessions: vec![SessionInfo {
                    id: session(),
                    program: "pwsh.exe".to_owned(),
                    pid: Some(1234),
                    size: size(80, 24),
                    created_unix_ms: 1_800_000_000_000,
                    attached: false,
                    exit: None,
                    stats: SessionStats {
                        read_bytes: 512,
                        sent_bytes: 0,
                        pending_bytes: 512,
                        paused: true,
                    },
                    title: None,
                    cwd: None,
                }],
            }),
        ),
        response(
            4,
            Ok(ResponseBody::Attached {
                size: size(80, 24),
                hydrate: HydrateMode::Snapshot,
                resize_pulls_scrollback: false,
            }),
        ),
        Message::new(
            1,
            Body::Hydrate(HydratePayload::Snapshot {
                vt_version: "1.0.1".to_owned(),
                bytes: vec![0xde, 0xad, 0xbe, 0xef],
            }),
        ),
        Message::new(
            2,
            Body::Hydrate(HydratePayload::VtReplay(b"\x1b[2Jreplayed".to_vec())),
        ),
        Message::new(1, Body::Data(b"hello\r\n\x1b[31mred\x1b[0m".to_vec())),
        Message::new(1, Body::Resized(size(120, 40))),
        Message::new(
            1,
            Body::Event(SessionEvent::Title {
                title: "~/src \u{2014} vim".to_owned(),
            }),
        ),
        Message::new(
            1,
            Body::Event(SessionEvent::Cwd {
                url: "file://host/home/someone".to_owned(),
            }),
        ),
        Message::new(1, Body::Event(SessionEvent::Bell)),
        Message::new(1, Body::Exit(ExitStatus::Code(3))),
        Message::new(2, Body::Exit(ExitStatus::Signal(9))),
        Message::new(3, Body::Exit(ExitStatus::Unknown)),
        Message::control(Control::Notify(Notification::SessionExited {
            session: session(),
            status: ExitStatus::Code(3),
        })),
        response(5, Ok(ResponseBody::Killed {})),
        response(
            6,
            Err(ProtoError::new(ErrorCode::NotFound, "no such session")),
        ),
        Message::control(Control::Notify(Notification::Fatal {
            code: ErrorCode::ProtocolError,
            message: "unknown frame kind 0x7f".to_owned(),
        })),
        response(
            7,
            Ok(ResponseBody::Screen {
                size: size(4, 2),
                cursor: ScreenCursor {
                    col: 2,
                    row: 1,
                    visible: true,
                },
                alternate: false,
                lines: vec!["ok".to_owned(), "$ ".to_owned()],
            }),
        ),
        response(
            8,
            Ok(ResponseBody::Matched {
                row: 1,
                line: "$ ".to_owned(),
            }),
        ),
        response(
            9,
            Err(ProtoError::new(
                ErrorCode::Timeout,
                "the text did not appear on the screen in time",
            )),
        ),
    ]
    .into_iter()
    // Added after the messages above were pinned, and so after them: what was
    // recorded before is still the beginning of what is recorded now.
    .chain(
        later_events()
            .into_iter()
            .map(|event| Message::new(4, Body::Event(event))),
    )
    .chain([
        response(
            10,
            Ok(ResponseBody::HostStatus {
                host_version: "0.0.0".to_owned(),
                pid: 4242,
                started_unix_ms: 1_763_400_000_000,
                persistent: true,
                sessions: 2,
                clients: 1,
                idle_exit_ms: Some(60_000),
            }),
        ),
        response(11, Ok(ResponseBody::Stopping { ended: 2 })),
        response(12, Ok(ResponseBody::Sent { bytes: 4 })),
    ])
    .collect()
}

/// One of every event a session reports besides a title, a directory and a bell, in
/// each of its shapes.
fn later_events() -> Vec<SessionEvent> {
    let status = ProgramStatus {
        state: ProgramState::Blocked,
        id: Some("deploy/us-east".to_owned()),
        kind: Some(BlockKind::Permission),
        progress: Some(40),
        app: Some("terraform".to_owned()),
        title: Some("Plan".to_owned()),
        message: Some("Apply 3 resources?".to_owned()),
    };
    let foreground = Foreground {
        pid: 4242,
        name: "vim".to_owned(),
    };
    let disconnected = Disconnected {
        reason: DisconnectReason::ConnectionLost,
        detail: Some("client_loop: send disconnect: Broken pipe".to_owned()),
    };
    vec![
        SessionEvent::CommandStarted {
            command: Some("make test".to_owned()),
        },
        SessionEvent::CommandStarted { command: None },
        SessionEvent::CommandFinished { exit: Some(-1) },
        SessionEvent::CommandFinished { exit: None },
        SessionEvent::ProgramStatus(status.clone()),
        SessionEvent::ProgramStatus(ProgramStatus::new(ProgramState::Clear, None)),
        SessionEvent::Foreground(foreground.clone()),
        SessionEvent::Activity,
        SessionEvent::Disconnected(disconnected.clone()),
        SessionEvent::Disconnected(Disconnected {
            reason: DisconnectReason::Exited,
            detail: None,
        }),
        SessionEvent::State {
            state: SessionState {
                title: Some("~/src".to_owned()),
                cwd: Some("file://host/home/someone/src".to_owned()),
                command: Some(CommandState::Running {
                    command: Some("vim notes.md".to_owned()),
                }),
                statuses: vec![status, ProgramStatus::new(ProgramState::Done, None)],
                foreground: Some(foreground),
                disconnected: Some(disconnected),
            },
        },
        SessionEvent::State {
            state: SessionState {
                command: Some(CommandState::Finished { exit: Some(2) }),
                ..SessionState::default()
            },
        },
        SessionEvent::State {
            state: SessionState::default(),
        },
    ]
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A message as a decoder in another language is expected to report it.
fn describe(message: &Message) -> Value {
    let value = |v: Result<Value, serde_json::Error>| v.expect("a wire type is JSON-representable");
    let (kind, fields) = match &message.body {
        Body::Control(control) => (
            "control",
            json!({ "message": value(serde_json::to_value(control)) }),
        ),
        Body::Hydrate(HydratePayload::Snapshot { vt_version, bytes }) => (
            "hydrate",
            json!({ "form": "snapshot", "vt_version": vt_version, "bytes": hex(bytes) }),
        ),
        Body::Hydrate(HydratePayload::VtReplay(bytes)) => (
            "hydrate",
            json!({ "form": "vt_replay", "bytes": hex(bytes) }),
        ),
        Body::Data(bytes) => ("data", json!({ "bytes": hex(bytes) })),
        Body::Resized(size) => (
            "resized",
            json!({ "size": value(serde_json::to_value(size)) }),
        ),
        Body::Event(event) => (
            "event",
            json!({ "event": value(serde_json::to_value(event)) }),
        ),
        Body::Exit(status) => (
            "exit",
            json!({ "status": value(serde_json::to_value(status)) }),
        ),
        Body::Input(bytes) => ("input", json!({ "bytes": hex(bytes) })),
        Body::Resize { size, cell_px } => (
            "resize",
            json!({
                "size": value(serde_json::to_value(size)),
                "cell_px": value(serde_json::to_value(cell_px)),
            }),
        ),
        Body::Facts(facts) => (
            "facts",
            json!({ "facts": value(serde_json::to_value(facts)) }),
        ),
        Body::Credit(bytes) => ("credit", json!({ "bytes": bytes })),
        Body::Detach => ("detach", json!({})),
        Body::Rehydrate => ("rehydrate", json!({})),
        Body::Release => ("release", json!({})),
    };
    let mut described = json!({ "stream": message.stream, "kind": kind });
    if let (Value::Object(into), Value::Object(from)) = (&mut described, fields) {
        into.extend(from);
    }
    described
}

fn check_vector(name: &str, role: Role, messages: &[Message]) {
    let mut wire = Vec::new();
    for message in messages {
        encode(message, &mut wire).unwrap();
    }
    let description: Vec<Value> = messages.iter().map(describe).collect();
    let mut description = serde_json::to_string_pretty(&description).unwrap();
    description.push('\n');

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("vectors");
    let bin_path = dir.join(format!("{name}.bin"));
    let json_path = dir.join(format!("{name}.json"));

    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::create_dir_all(&dir).unwrap();
        fs::write(&bin_path, &wire).unwrap();
        fs::write(&json_path, &description).unwrap();
    }

    let hint = "run with UPDATE_GOLDEN=1 if the protocol changed on purpose";
    let golden_wire =
        fs::read(&bin_path).unwrap_or_else(|e| panic!("{}: {e}; {hint}", bin_path.display()));
    assert!(
        golden_wire == wire,
        "{name}.bin no longer matches what is encoded; {hint}"
    );
    let golden_description = fs::read_to_string(&json_path)
        .unwrap_or_else(|e| panic!("{}: {e}; {hint}", json_path.display()));
    assert_eq!(
        golden_description.replace("\r\n", "\n"),
        description,
        "{name}.json; {hint}"
    );

    // The stored bytes decode to exactly the messages they were made from.
    let mut decoder = FrameDecoder::new(role);
    decoder.push(&golden_wire);
    let mut decoded = Vec::new();
    while let Some(message) = decoder.next().unwrap() {
        decoded.push(message);
    }
    assert_eq!(decoded, messages);
}

#[test]
fn client_to_host_wire_layout_is_stable() {
    check_vector("client-to-host", Role::Host, &client_messages());
}

#[test]
fn host_to_client_wire_layout_is_stable() {
    check_vector("host-to-client", Role::Client, &host_messages());
}

#[test]
fn fixed_layouts_are_spelt_out_byte_for_byte() {
    let encoded = |message: Message| {
        let mut wire = Vec::new();
        encode(&message, &mut wire).unwrap();
        wire
    };

    // Header: stream 0x01020304, kind, length; then the payload. All little-endian.
    assert_eq!(
        encoded(Message::new(
            0x0102_0304,
            Body::Resized(size(0x0150, 0x0018))
        )),
        [
            0x04, 0x03, 0x02, 0x01, 0x04, 4, 0, 0, 0, 0x50, 0x01, 0x18, 0x00
        ]
    );
    assert_eq!(
        encoded(Message::new(1, Body::Exit(ExitStatus::Signal(0x0a0b_0c0d)))),
        [1, 0, 0, 0, 0x06, 5, 0, 0, 0, 1, 0x0d, 0x0c, 0x0b, 0x0a]
    );
    assert_eq!(
        encoded(Message::new(
            1,
            Body::Resize {
                size: size(80, 24),
                cell_px: CellPx { w: 9, h: 0x0112 }
            }
        )),
        [1, 0, 0, 0, 0x11, 8, 0, 0, 0, 80, 0, 24, 0, 9, 0, 0x12, 0x01]
    );
    assert_eq!(
        encoded(Message::new(1, Body::Credit(0x0010_0000))),
        [1, 0, 0, 0, 0x13, 4, 0, 0, 0, 0x00, 0x00, 0x10, 0x00]
    );
    assert_eq!(
        encoded(Message::new(1, Body::Detach)),
        [1, 0, 0, 0, 0x14, 0, 0, 0, 0]
    );
    assert_eq!(
        encoded(Message::new(1, Body::Rehydrate)),
        [1, 0, 0, 0, 0x15, 0, 0, 0, 0]
    );
    assert_eq!(
        encoded(Message::new(1, Body::Release)),
        [1, 0, 0, 0, 0x16, 0, 0, 0, 0]
    );

    let facts_frame = encoded(Message::new(1, Body::Facts(facts())));
    assert_eq!(facts_frame.len(), 9 + 67);
    let payload = &facts_frame[9..];
    // Flags: focused and dark, not visible. Then the reserved byte.
    assert_eq!(payload[..2], [0b101, 0]);
    assert_eq!(payload[2..10], [120, 0, 40, 0, 9, 0, 18, 0]);
    assert_eq!(
        payload[10..19],
        [0xd0, 0xd1, 0xd2, 0x10, 0x11, 0x12, 0xfe, 0xdc, 0xba]
    );
    // ANSI colour 5 follows the three named colours and five earlier ANSI colours.
    assert_eq!(payload[19 + 5 * 3..19 + 6 * 3], [5, 10, 15]);
}
