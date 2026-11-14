//! Whatever is encoded decodes to the same messages, however the bytes are chunked.

mod common;

use lume_proto::tuning::MAX_PAYLOAD;
use lume_proto::{
    Body, BytesKind, Control, FrameDecoder, Header, Kind, Message, Notification, Role, encode,
    encode_bytes,
};
use proptest::collection::vec;
use proptest::prelude::*;

fn encode_all(messages: &[Message]) -> Vec<u8> {
    let mut wire = Vec::new();
    for message in messages {
        encode(message, &mut wire).expect("a generated message is encodable");
    }
    wire
}

proptest! {
    #[test]
    fn host_frames_round_trip_under_any_chunking(
        messages in vec(common::message_from_host(), 0..12),
        cuts in common::cuts(),
    ) {
        let wire = encode_all(&messages);
        let (decoded, error) = common::decode_in_chunks(Role::Client, &wire, &cuts);
        prop_assert_eq!(error, None);
        prop_assert_eq!(decoded, messages);
    }

    #[test]
    fn client_frames_round_trip_under_any_chunking(
        messages in vec(common::message_from_client(), 0..12),
        cuts in common::cuts(),
    ) {
        let wire = encode_all(&messages);
        let (decoded, error) = common::decode_in_chunks(Role::Host, &wire, &cuts);
        prop_assert_eq!(error, None);
        prop_assert_eq!(decoded, messages);
    }

    #[test]
    fn long_terminal_output_arrives_as_frames_that_join_to_the_original(
        len in 0..MAX_PAYLOAD * 3 + 7,
        seed in any::<u8>(),
        cuts in common::cuts(),
    ) {
        let bytes: Vec<u8> = (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect();
        let mut wire = Vec::new();
        encode_bytes(5, BytesKind::Data, &bytes, &mut wire);

        let (decoded, error) = common::decode_in_chunks(Role::Client, &wire, &cuts);
        prop_assert_eq!(error, None);
        prop_assert_eq!(decoded.len(), len.div_ceil(MAX_PAYLOAD).max(1));
        let mut joined = Vec::new();
        for message in decoded {
            prop_assert_eq!(message.stream, 5);
            match message.body {
                Body::Data(part) => {
                    prop_assert!(part.len() <= MAX_PAYLOAD);
                    joined.extend_from_slice(&part);
                }
                other => prop_assert!(false, "unexpected {:?}", other.kind()),
            }
        }
        prop_assert_eq!(joined, bytes);
    }
}

#[test]
fn encoding_a_message_and_encoding_its_bytes_agree() {
    let bytes = vec![7; MAX_PAYLOAD + 10];
    let mut from_message = Vec::new();
    encode(
        &Message::new(3, Body::Input(bytes.clone())),
        &mut from_message,
    )
    .unwrap();
    let mut from_bytes = Vec::new();
    encode_bytes(3, BytesKind::Input, &bytes, &mut from_bytes);
    assert_eq!(from_message, from_bytes);
}

/// Builds one frame by hand, so tests can order frames in ways `encode` never does.
fn frame(stream: u32, kind: Kind, more: bool, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    Header {
        stream,
        kind,
        more,
        len: payload.len() as u32,
    }
    .write(&mut out);
    out.extend_from_slice(payload);
    out
}

#[test]
fn fragments_reassemble_across_interleaved_streams() {
    let notice = Control::Notify(Notification::Fatal {
        code: lume_proto::ErrorCode::Internal,
        message: "split across three frames".to_owned(),
    });
    let json = serde_json::to_vec(&notice).unwrap();
    let (first, rest) = json.split_at(10);
    let (second, third) = rest.split_at(10);

    // Stream 4 carries a replay in two frames; stream 0 a control message in three. Frames
    // of each, and plain output on stream 9, arrive between frames of the other.
    let mut wire = Vec::new();
    wire.extend(frame(4, Kind::Hydrate, true, &[2, b'a', b'b']));
    wire.extend(frame(0, Kind::Control, true, first));
    wire.extend(frame(9, Kind::Data, false, b"x"));
    wire.extend(frame(0, Kind::Control, true, second));
    wire.extend(frame(4, Kind::Hydrate, false, b"cd"));
    wire.extend(frame(0, Kind::Control, false, third));

    let mut decoder = FrameDecoder::new(Role::Client);
    decoder.push(&wire);
    let mut decoded = Vec::new();
    while let Some(message) = decoder.next().unwrap() {
        decoded.push(message);
    }
    assert_eq!(
        decoded,
        vec![
            Message::new(9, Body::Data(b"x".to_vec())),
            Message::new(
                4,
                Body::Hydrate(lume_proto::HydratePayload::VtReplay(b"abcd".to_vec()))
            ),
            Message::control(notice),
        ]
    );
}
