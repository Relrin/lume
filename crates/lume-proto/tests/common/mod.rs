//! Generators of valid messages, and a helper that decodes a byte stream in arbitrary chunks.

// Each test file compiles this module separately and uses a different part of it.
#![allow(dead_code)]

use lume_proto::tuning::MAX_PAYLOAD;
use lume_proto::{
    BackendCaps, Body, CellPx, ClientFacts, ClientKind, ColorScheme, Control, DecodeError,
    ErrorCode, ExitStatus, FrameDecoder, HydrateKinds, HydrateMode, HydratePayload, Launcher,
    Message, Notification, Palette, ProtoError, Request, RequestBody, Responder, Response,
    ResponseBody, Rgb, Role, SessionEvent, SessionId, SessionInfo, SessionStats, SpawnSpec,
    TermSize,
};
use proptest::collection::vec;
use proptest::option;
use proptest::prelude::*;
use proptest::sample::{Index, select};

pub fn term_size() -> impl Strategy<Value = TermSize> {
    (1..=TermSize::MAX, 1..=TermSize::MAX)
        .prop_map(|(cols, rows)| TermSize::new(cols, rows).unwrap())
}

pub fn cell_px() -> impl Strategy<Value = CellPx> {
    any::<(u16, u16)>().prop_map(|(w, h)| CellPx { w, h })
}

fn rgb() -> impl Strategy<Value = Rgb> {
    any::<[u8; 3]>().prop_map(Rgb)
}

fn palette() -> impl Strategy<Value = Palette> {
    (rgb(), rgb(), rgb(), any::<[[u8; 3]; 16]>()).prop_map(|(fg, bg, cursor, ansi)| Palette {
        fg,
        bg,
        cursor,
        ansi: ansi.map(Rgb),
    })
}

pub fn facts() -> impl Strategy<Value = ClientFacts> {
    (
        term_size(),
        cell_px(),
        palette(),
        any::<(bool, bool, bool)>(),
    )
        .prop_map(
            |(size, cell_px, palette, (dark, focused, visible))| ClientFacts {
                size,
                cell_px,
                palette,
                scheme: if dark {
                    ColorScheme::Dark
                } else {
                    ColorScheme::Light
                },
                focused,
                visible,
            },
        )
}

fn session_id() -> impl Strategy<Value = SessionId> {
    (1..=u64::MAX).prop_map(|raw| SessionId::new(raw).unwrap())
}

pub fn exit_status() -> impl Strategy<Value = ExitStatus> {
    prop_oneof![
        any::<u32>().prop_map(ExitStatus::Code),
        any::<u32>().prop_map(ExitStatus::Signal),
        Just(ExitStatus::Unknown),
    ]
}

/// Short text, including characters JSON must escape and characters outside ASCII.
fn text() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 \"\\\\/\u{0}-\u{1f}é漢😀]{0,24}"
}

/// Text long enough that a control message holding it needs more than one frame.
fn long_text() -> impl Strategy<Value = String> {
    (
        MAX_PAYLOAD..MAX_PAYLOAD * 2 + 100,
        select(vec!['a', 'é', '漢']),
    )
        .prop_map(|(len, c)| std::iter::repeat_n(c, len / c.len_utf8()).collect())
}

fn spawn_spec() -> impl Strategy<Value = SpawnSpec> {
    let value = prop_oneof![9 => text(), 1 => long_text()];
    (
        text(),
        vec(text(), 0..4),
        any::<bool>(),
        option::of(text()),
        vec((text(), value), 0..3),
        term_size(),
    )
        .prop_map(|(program, args, login, cwd, env, size)| SpawnSpec {
            launcher: Launcher::Shell {
                program,
                args,
                login,
            },
            cwd,
            env,
            size,
        })
}

fn stats() -> impl Strategy<Value = SessionStats> {
    any::<(u64, u64, u32, bool)>().prop_map(|(read_bytes, sent_bytes, pending_bytes, paused)| {
        SessionStats {
            read_bytes,
            sent_bytes,
            pending_bytes,
            paused,
        }
    })
}

fn session_info() -> impl Strategy<Value = SessionInfo> {
    (
        session_id(),
        text(),
        option::of(any::<u32>()),
        term_size(),
        any::<u64>(),
        any::<bool>(),
        option::of(exit_status()),
        stats(),
    )
        .prop_map(
            |(id, program, pid, size, created_unix_ms, attached, exit, stats)| SessionInfo {
                id,
                program,
                pid,
                size,
                created_unix_ms,
                attached,
                exit,
                stats,
            },
        )
}

fn caps() -> impl Strategy<Value = BackendCaps> {
    any::<(bool, bool, bool, bool, bool, bool)>().prop_map(
        |(persistent, multi_client, server_layout, backend, snapshot, vt_replay)| BackendCaps {
            persistent,
            multi_client,
            server_layout,
            responder: if backend {
                Responder::Backend
            } else {
                Responder::Client
            },
            hydrate: HydrateKinds {
                snapshot,
                vt_replay,
            },
        },
    )
}

fn error_code() -> impl Strategy<Value = ErrorCode> {
    select(ErrorCode::ALL.to_vec())
}

fn request_body() -> impl Strategy<Value = RequestBody> {
    prop_oneof![
        (
            any::<(u32, u32)>(),
            option::of(text()),
            select(vec![ClientKind::App, ClientKind::Ctl, ClientKind::Test])
        )
            .prop_map(
                |((proto, proto_min), vt_version, client_kind)| RequestBody::Hello {
                    proto,
                    proto_min,
                    vt_version,
                    client_kind,
                }
            ),
        spawn_spec().prop_map(|spec| RequestBody::Spawn { spec }),
        Just(RequestBody::List {}),
        (session_id(), 1..=u32::MAX, facts()).prop_map(|(session, stream, facts)| {
            RequestBody::Attach {
                session,
                stream,
                facts,
            }
        }),
        session_id().prop_map(|session| RequestBody::Kill { session }),
    ]
}

fn response_body() -> impl Strategy<Value = ResponseBody> {
    prop_oneof![
        (
            any::<u32>(),
            text(),
            option::of(text()),
            caps(),
            any::<(u32, u32)>()
        )
            .prop_map(
                |(proto, host_version, vt_version, caps, (max_payload, pid))| {
                    ResponseBody::HelloAck {
                        proto,
                        host_version,
                        vt_version,
                        caps,
                        max_payload,
                        pid,
                    }
                }
            ),
        session_id().prop_map(|session| ResponseBody::Spawned { session }),
        vec(session_info(), 0..4).prop_map(|sessions| ResponseBody::Sessions { sessions }),
        (
            term_size(),
            select(vec![
                HydrateMode::None,
                HydrateMode::Snapshot,
                HydrateMode::VtReplay
            ]),
            any::<bool>()
        )
            .prop_map(
                |(size, hydrate, resize_pulls_scrollback)| ResponseBody::Attached {
                    size,
                    hydrate,
                    resize_pulls_scrollback,
                }
            ),
        Just(ResponseBody::Killed {}),
    ]
}

fn control() -> impl Strategy<Value = Control> {
    let result = prop_oneof![
        response_body().prop_map(Ok),
        (error_code(), text()).prop_map(|(code, message)| Err(ProtoError { code, message })),
    ];
    prop_oneof![
        (any::<u64>(), request_body())
            .prop_map(|(id, body)| Control::Request(Request { id, body })),
        (any::<u64>(), result).prop_map(|(id, result)| Control::Response(Response { id, result })),
        (session_id(), exit_status()).prop_map(|(session, status)| Control::Notify(
            Notification::SessionExited { session, status }
        )),
        (error_code(), text())
            .prop_map(|(code, message)| Control::Notify(Notification::Fatal { code, message })),
    ]
}

/// Terminal bytes that fit one frame: mostly short, sometimes right at the limit.
fn terminal_bytes() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        8 => vec(any::<u8>(), 0..200),
        1 => (MAX_PAYLOAD - 2..=MAX_PAYLOAD, any::<u8>()).prop_map(|(len, byte)| vec![byte; len]),
    ]
}

fn event() -> impl Strategy<Value = SessionEvent> {
    prop_oneof![
        text().prop_map(|title| SessionEvent::Title { title }),
        text().prop_map(|url| SessionEvent::Cwd { url }),
        Just(SessionEvent::Bell),
    ]
}

fn hydrate() -> impl Strategy<Value = HydratePayload> {
    let bytes = || {
        prop_oneof![
            4 => vec(any::<u8>(), 0..200),
            1 => (MAX_PAYLOAD..MAX_PAYLOAD * 3, any::<u8>()).prop_map(|(len, byte)| vec![byte; len]),
        ]
    };
    prop_oneof![
        ("[0-9.]{0,12}", bytes())
            .prop_map(|(vt_version, bytes)| HydratePayload::Snapshot { vt_version, bytes }),
        bytes().prop_map(HydratePayload::VtReplay),
    ]
}

fn data_stream() -> impl Strategy<Value = u32> {
    prop_oneof![4 => 1..6u32, 1 => 1..=u32::MAX]
}

/// A message a host may send.
pub fn message_from_host() -> impl Strategy<Value = Message> {
    let body = prop_oneof![
        6 => terminal_bytes().prop_map(Body::Data),
        1 => hydrate().prop_map(Body::Hydrate),
        1 => term_size().prop_map(Body::Resized),
        1 => event().prop_map(Body::Event),
        1 => exit_status().prop_map(Body::Exit),
    ];
    prop_oneof![
        4 => (data_stream(), body).prop_map(|(stream, body)| Message { stream, body }),
        1 => control().prop_map(Message::control),
    ]
}

/// A message a client may send.
pub fn message_from_client() -> impl Strategy<Value = Message> {
    let body = prop_oneof![
        6 => terminal_bytes().prop_map(Body::Input),
        1 => (term_size(), cell_px()).prop_map(|(size, cell_px)| Body::Resize { size, cell_px }),
        1 => facts().prop_map(Body::Facts),
        1 => any::<u32>().prop_map(Body::Credit),
        1 => Just(Body::Detach),
    ];
    prop_oneof![
        4 => (data_stream(), body).prop_map(|(stream, body)| Message { stream, body }),
        1 => control().prop_map(Message::control),
    ]
}

/// Places to cut a byte stream, as positions relative to its length.
pub fn cuts() -> impl Strategy<Value = Vec<Index>> {
    vec(any::<Index>(), 0..24)
}

/// Feeds `wire` to a decoder in the chunks `cuts` describe and collects every message.
/// Stops at the first error and returns it with the messages decoded before it.
pub fn decode_in_chunks(
    role: Role,
    wire: &[u8],
    cuts: &[Index],
) -> (Vec<Message>, Option<DecodeError>) {
    let mut positions: Vec<usize> = cuts.iter().map(|cut| cut.index(wire.len() + 1)).collect();
    positions.push(wire.len());
    positions.sort_unstable();

    let mut decoder = FrameDecoder::new(role);
    let mut messages = Vec::new();
    let mut start = 0;
    for end in positions {
        decoder.push(&wire[start..end]);
        start = end;
        loop {
            match decoder.next() {
                Ok(Some(message)) => messages.push(message),
                Ok(None) => break,
                Err(error) => return (messages, Some(error)),
            }
        }
    }
    (messages, None)
}
