//! The control endpoint's messages survive the wire however the bytes are cut up, and
//! what is not a control message is refused.

use lume_proto::app::{
    AppMessage, AppReply, AppReplyBody, AppRequest, AppRequestBody, AppResponse,
};
use lume_proto::tuning::{MAX_CONTROL_MESSAGE, MAX_PAYLOAD};
use lume_proto::{
    Body, ControlDecoder, DecodeError, EncodeError, FrameDecoder, Header, Kind, Message, Role,
    encode, encode_control_json,
};
use proptest::prelude::*;

fn request(id: u64, text: String) -> AppMessage {
    AppMessage::Request(AppRequest {
        id,
        body: AppRequestBody::Send {
            pane: None,
            session: None,
            text,
        },
    })
}

fn decode_all(decoder: &mut ControlDecoder) -> Vec<AppMessage> {
    let mut messages = Vec::new();
    while let Some(json) = decoder.next().unwrap() {
        messages.push(AppMessage::decode(&json).unwrap());
    }
    messages
}

proptest! {
    #[test]
    fn messages_survive_any_chunking(
        texts in prop::collection::vec(".{0,200}", 1..6),
        cuts in prop::collection::vec(1usize..64, 1..40),
    ) {
        let sent: Vec<AppMessage> = texts
            .into_iter()
            .enumerate()
            .map(|(id, text)| request(id as u64, text))
            .collect();
        let mut wire = Vec::new();
        for message in &sent {
            message.encode(&mut wire).unwrap();
        }

        let mut decoder = ControlDecoder::new();
        let mut received = Vec::new();
        let mut rest = wire.as_slice();
        let mut cuts = cuts.into_iter().cycle();
        while !rest.is_empty() {
            let take = cuts.next().unwrap().min(rest.len());
            let (chunk, tail) = rest.split_at(take);
            decoder.push(chunk);
            received.extend(decode_all(&mut decoder));
            rest = tail;
        }
        prop_assert_eq!(received, sent);
    }
}

#[test]
fn a_message_longer_than_one_frame_arrives_whole() {
    let long = request(1, "x".repeat(MAX_PAYLOAD * 2 + 17));
    let mut wire = Vec::new();
    long.encode(&mut wire).unwrap();
    assert!(wire.len() > MAX_PAYLOAD * 2 + Header::LEN * 3);

    let mut decoder = ControlDecoder::new();
    decoder.push(&wire);
    assert_eq!(decode_all(&mut decoder), vec![long]);
}

#[test]
fn a_response_survives_the_wire() {
    let response = AppMessage::Response(AppResponse {
        id: 4,
        result: Ok(AppReply {
            effect: "started a session".to_owned(),
            body: AppReplyBody::Spawned {
                session: "00000000000000ab".parse().unwrap(),
                pane: "p4".to_owned(),
                tab: Some("t3".to_owned()),
            },
        }),
    });
    let mut wire = Vec::new();
    response.encode(&mut wire).unwrap();
    let mut decoder = ControlDecoder::new();
    decoder.push(&wire);
    assert_eq!(decode_all(&mut decoder), vec![response]);
}

#[test]
fn a_message_over_the_limit_is_not_encoded() {
    let json = vec![b' '; MAX_CONTROL_MESSAGE + 1];
    let mut wire = Vec::new();
    assert_eq!(
        encode_control_json(&json, &mut wire),
        Err(EncodeError::MessageTooLarge(Kind::Control))
    );
    assert!(wire.is_empty());
}

#[test]
fn frames_that_are_not_control_end_the_connection() {
    let mut wire = Vec::new();
    encode(&Message::new(1, Body::Input(b"ls".to_vec())), &mut wire).unwrap();
    let mut decoder = ControlDecoder::new();
    decoder.push(&wire);
    assert_eq!(decoder.next(), Err(DecodeError::NotControl(Kind::Input)));

    // The first error is final, whatever arrives afterwards.
    let mut good = Vec::new();
    request(1, String::new()).encode(&mut good).unwrap();
    decoder.push(&good);
    assert_eq!(decoder.next(), Err(DecodeError::NotControl(Kind::Input)));
}

#[test]
fn a_joined_message_over_the_limit_ends_the_connection() {
    // Frames that each pass, announcing more than a message may hold in all.
    let frames = MAX_CONTROL_MESSAGE / MAX_PAYLOAD + 1;
    let mut wire = Vec::new();
    for _ in 0..frames {
        Header {
            stream: 0,
            kind: Kind::Control,
            more: true,
            len: MAX_PAYLOAD as u32,
        }
        .write(&mut wire);
        wire.extend(std::iter::repeat_n(b' ', MAX_PAYLOAD));
    }
    let mut decoder = ControlDecoder::new();
    decoder.push(&wire);
    assert_eq!(
        decoder.next(),
        Err(DecodeError::MessageTooLarge(Kind::Control))
    );
}

#[test]
fn json_that_is_not_a_message_is_reported() {
    assert!(matches!(
        AppMessage::decode(br#"{"request":{"id":1,"body":{"type":"dance"}}}"#),
        Err(DecodeError::BadPayload {
            kind: Kind::Control,
            ..
        })
    ));
}

#[test]
fn what_follows_the_last_message_is_left_for_a_frame_decoder() {
    let mut wire = Vec::new();
    request(1, "first".to_owned()).encode(&mut wire).unwrap();
    encode(&Message::new(1, Body::Data(b"output".to_vec())), &mut wire).unwrap();
    let mut control = ControlDecoder::new();
    control.push(&wire);
    assert!(control.next().unwrap().is_some());
    let mut frames = FrameDecoder::new(Role::Client);
    frames.push(&control.into_rest());
    assert_eq!(
        frames.next().unwrap(),
        Some(Message::new(1, Body::Data(b"output".to_vec())))
    );
    assert_eq!(frames.next().unwrap(), None);
}
