//! Bytes that are not valid frames produce an error, never a panic, and the error is final.

mod common;

use lume_proto::tuning::{MAX_CONTROL_MESSAGE, MAX_PAYLOAD};
use lume_proto::{DecodeError, FrameDecoder, Header, Kind, Message, Role, encode};
use proptest::collection::vec;
use proptest::prelude::*;
use proptest::sample::Index;

/// Builds one frame by hand, bypassing every check `encode` makes.
fn frame(stream: u32, kind_byte: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&stream.to_le_bytes());
    out.push(kind_byte);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

fn first_error(role: Role, wire: &[u8]) -> Option<DecodeError> {
    common::decode_in_chunks(role, wire, &[]).1
}

#[derive(Clone, Debug)]
enum Damage {
    Flip { at: Index, mask: u8 },
    Truncate { at: Index },
    Insert { at: Index, byte: u8 },
    Remove { at: Index },
}

fn damage() -> impl Strategy<Value = Damage> {
    prop_oneof![
        (any::<Index>(), 1..=u8::MAX).prop_map(|(at, mask)| Damage::Flip { at, mask }),
        any::<Index>().prop_map(|at| Damage::Truncate { at }),
        any::<(Index, u8)>().prop_map(|(at, byte)| Damage::Insert { at, byte }),
        any::<Index>().prop_map(|at| Damage::Remove { at }),
    ]
}

fn apply(wire: &mut Vec<u8>, damage: &Damage) {
    if wire.is_empty() {
        return;
    }
    let len = wire.len();
    match damage {
        Damage::Flip { at, mask } => wire[at.index(len)] ^= mask,
        Damage::Truncate { at } => wire.truncate(at.index(len)),
        Damage::Insert { at, byte } => wire.insert(at.index(len), *byte),
        Damage::Remove { at } => {
            wire.remove(at.index(len));
        }
    }
}

proptest! {
    #[test]
    fn arbitrary_bytes_never_panic(
        wire in vec(any::<u8>(), 0..4096),
        cuts in common::cuts(),
        host in any::<bool>(),
    ) {
        let role = if host { Role::Host } else { Role::Client };
        // Either outcome is acceptable; reaching this line is the property.
        let _ = common::decode_in_chunks(role, &wire, &cuts);
    }

    #[test]
    fn damaged_valid_streams_never_panic(
        messages in vec(common::message_from_host(), 1..8),
        damages in vec(damage(), 1..6),
        cuts in common::cuts(),
    ) {
        let mut wire = Vec::new();
        for message in &messages {
            encode(message, &mut wire).unwrap();
        }
        for damage in &damages {
            apply(&mut wire, damage);
        }
        let _ = common::decode_in_chunks(Role::Client, &wire, &cuts);
    }

    #[test]
    fn decoder_stays_failed_after_an_error(
        later in vec(common::message_from_client(), 0..4),
    ) {
        let mut decoder = FrameDecoder::new(Role::Host);
        decoder.push(&frame(1, 0x7f, b""));
        let error = decoder.next().unwrap_err();
        prop_assert_eq!(&error, &DecodeError::UnknownKind(0x7f));

        // Valid frames after the failure change nothing: the stream position is lost.
        for message in &later {
            let mut wire = Vec::new();
            encode(message, &mut wire).unwrap();
            decoder.push(&wire);
            prop_assert_eq!(decoder.next(), Err(error.clone()));
        }
    }
}

#[test]
fn payload_of_65536_is_accepted_and_65537_rejected() {
    assert_eq!(MAX_PAYLOAD, 65536);

    let wire = frame(1, Kind::Data as u8, &vec![0; MAX_PAYLOAD]);
    let (messages, error) = common::decode_in_chunks(Role::Client, &wire, &[]);
    assert_eq!(error, None);
    assert_eq!(messages.len(), 1);

    // Only the header is needed to refuse an oversized frame.
    let mut header = Vec::new();
    header.extend_from_slice(&1u32.to_le_bytes());
    header.push(Kind::Data as u8);
    header.extend_from_slice(&(MAX_PAYLOAD as u32 + 1).to_le_bytes());
    assert_eq!(
        first_error(Role::Client, &header),
        Some(DecodeError::PayloadTooLarge(MAX_PAYLOAD as u32 + 1))
    );
}

#[test]
fn an_all_zero_header_is_refused() {
    assert_eq!(
        first_error(Role::Host, &[0; Header::LEN]),
        Some(DecodeError::UnknownKind(0))
    );
}

#[test]
fn a_kind_sent_in_the_wrong_direction_is_refused() {
    let data = frame(1, Kind::Data as u8, b"x");
    assert_eq!(
        first_error(Role::Host, &data),
        Some(DecodeError::WrongDirection(Kind::Data))
    );

    let input = frame(1, Kind::Input as u8, b"x");
    assert_eq!(
        first_error(Role::Client, &input),
        Some(DecodeError::WrongDirection(Kind::Input))
    );
}

#[test]
fn control_is_refused_off_stream_0_and_everything_else_on_it() {
    assert_eq!(
        first_error(Role::Host, &frame(1, Kind::Control as u8, b"{}")),
        Some(DecodeError::WrongPlane {
            kind: Kind::Control,
            stream: 1
        })
    );
    assert_eq!(
        first_error(Role::Host, &frame(0, Kind::Input as u8, b"x")),
        Some(DecodeError::WrongPlane {
            kind: Kind::Input,
            stream: 0
        })
    );
}

#[test]
fn only_control_and_hydrate_may_be_split() {
    let split_data = frame(1, Kind::Data as u8 | Kind::MORE, b"x");
    assert_eq!(
        first_error(Role::Client, &split_data),
        Some(DecodeError::NotFragmentable(Kind::Data))
    );
}

#[test]
fn a_fragment_without_bytes_is_refused() {
    let empty = frame(0, Kind::Control as u8 | Kind::MORE, b"");
    assert_eq!(
        first_error(Role::Host, &empty),
        Some(DecodeError::EmptyFragment)
    );
}

#[test]
fn a_different_kind_may_not_interrupt_an_unfinished_message() {
    let mut wire = frame(4, Kind::Hydrate as u8 | Kind::MORE, &[2, b'a']);
    wire.extend(frame(4, Kind::Data as u8, b"x"));
    assert_eq!(
        first_error(Role::Client, &wire),
        Some(DecodeError::Interrupted {
            stream: 4,
            unfinished: Kind::Hydrate,
            got: Kind::Data
        })
    );
}

#[test]
fn a_control_message_may_not_grow_past_its_limit() {
    let fragment = frame(
        0,
        Kind::Control as u8 | Kind::MORE,
        &vec![b' '; MAX_PAYLOAD],
    );
    let mut decoder = FrameDecoder::new(Role::Host);
    let within_limit = MAX_CONTROL_MESSAGE / MAX_PAYLOAD;
    for _ in 0..within_limit {
        decoder.push(&fragment);
        assert_eq!(decoder.next(), Ok(None));
    }
    decoder.push(&fragment);
    assert_eq!(
        decoder.next(),
        Err(DecodeError::MessageTooLarge(Kind::Control))
    );
}

#[test]
fn fixed_layout_payloads_of_the_wrong_length_are_refused() {
    for (role, kind, expected) in [
        (Role::Client, Kind::Resized, 4usize),
        (Role::Client, Kind::Exit, 5),
        (Role::Host, Kind::Resize, 8),
        (Role::Host, Kind::Facts, 67),
        (Role::Host, Kind::Credit, 4),
        (Role::Host, Kind::Detach, 0),
    ] {
        for len in [expected + 1, expected.saturating_sub(1)] {
            if len == expected {
                continue;
            }
            assert_eq!(
                first_error(role, &frame(1, kind as u8, &vec![1; len])),
                Some(DecodeError::BadLength {
                    kind,
                    len,
                    expected
                }),
                "{kind:?} with {len} bytes"
            );
        }
    }
}

fn is_bad_payload(error: Option<DecodeError>, kind: Kind) -> bool {
    matches!(error, Some(DecodeError::BadPayload { kind: got, .. }) if got == kind)
}

#[test]
fn payloads_with_invalid_contents_are_refused() {
    // A grid with no columns.
    assert!(is_bad_payload(
        first_error(Role::Client, &frame(1, Kind::Resized as u8, &[0, 0, 24, 0])),
        Kind::Resized
    ));
    assert!(is_bad_payload(
        first_error(
            Role::Host,
            &frame(1, Kind::Resize as u8, &[80, 0, 0, 0, 8, 0, 16, 0])
        ),
        Kind::Resize
    ));
    // An exit form that does not exist, and an unknown status that carries a value.
    assert!(is_bad_payload(
        first_error(Role::Client, &frame(1, Kind::Exit as u8, &[3, 0, 0, 0, 0])),
        Kind::Exit
    ));
    assert!(is_bad_payload(
        first_error(Role::Client, &frame(1, Kind::Exit as u8, &[2, 1, 0, 0, 0])),
        Kind::Exit
    ));

    // Facts with an unknown flag, and with the reserved byte set.
    let mut facts = vec![0u8; 67];
    facts[2] = 80;
    facts[4] = 24;
    assert_eq!(
        first_error(Role::Host, &frame(1, Kind::Facts as u8, &facts)),
        None
    );
    let mut unknown_flag = facts.clone();
    unknown_flag[0] = 0x08;
    assert!(is_bad_payload(
        first_error(Role::Host, &frame(1, Kind::Facts as u8, &unknown_flag)),
        Kind::Facts
    ));
    let mut reserved = facts;
    reserved[1] = 1;
    assert!(is_bad_payload(
        first_error(Role::Host, &frame(1, Kind::Facts as u8, &reserved)),
        Kind::Facts
    ));

    // Hydrate: empty, an unknown form, a snapshot whose version is cut short.
    for payload in [
        &[][..],
        &[9][..],
        &[1][..],
        &[1, 5, b'1'][..],
        &[1, 1, 0xff][..],
    ] {
        assert!(
            is_bad_payload(
                first_error(Role::Client, &frame(1, Kind::Hydrate as u8, payload)),
                Kind::Hydrate
            ),
            "{payload:?}"
        );
    }

    // Control and event payloads that are not the JSON they must be.
    assert!(is_bad_payload(
        first_error(Role::Host, &frame(0, Kind::Control as u8, b"{}")),
        Kind::Control
    ));
    assert!(is_bad_payload(
        first_error(Role::Host, &frame(0, Kind::Control as u8, b"\xff")),
        Kind::Control
    ));
    assert!(is_bad_payload(
        first_error(
            Role::Client,
            &frame(1, Kind::Event as u8, br#"{"type":"no_such_event"}"#)
        ),
        Kind::Event
    ));
}

#[test]
fn messages_that_cannot_be_sent_are_refused_when_encoding() {
    use lume_proto::{Body, Control, EncodeError, Notification};

    let mut out = Vec::new();
    assert_eq!(
        encode(&Message::new(0, Body::Detach), &mut out),
        Err(EncodeError::WrongPlane {
            kind: Kind::Detach,
            stream: 0
        })
    );
    let notice = Control::Notify(Notification::Fatal {
        code: lume_proto::ErrorCode::Internal,
        message: "x".repeat(MAX_CONTROL_MESSAGE),
    });
    assert_eq!(
        encode(&Message::new(1, Body::Control(notice.clone())), &mut out),
        Err(EncodeError::WrongPlane {
            kind: Kind::Control,
            stream: 1
        })
    );
    assert_eq!(
        encode(&Message::control(notice), &mut out),
        Err(EncodeError::MessageTooLarge(Kind::Control))
    );
    assert!(out.is_empty(), "a refused message writes nothing");
}
