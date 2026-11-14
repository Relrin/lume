//! Credit accounting holds for any order of grants, sends and consumption.

use lume_proto::tuning::MAX_WINDOW;
use lume_proto::{Granter, SendWindow};
use proptest::collection::vec;
use proptest::prelude::*;

#[derive(Clone, Debug)]
enum SenderStep {
    Grant(u32),
    Take(usize),
}

fn sender_step() -> impl Strategy<Value = SenderStep> {
    prop_oneof![
        (0..=MAX_WINDOW / 4).prop_map(SenderStep::Grant),
        (0..=MAX_WINDOW as usize).prop_map(SenderStep::Take),
    ]
}

#[derive(Clone, Debug)]
enum StreamStep {
    /// The sender has this many bytes it would like to send.
    Send(usize),
    /// The consumer processes up to this many of the bytes it has received.
    Consume(usize),
}

fn stream_step(window: u32) -> impl Strategy<Value = StreamStep> {
    let most = window as usize * 2;
    prop_oneof![
        (0..=most).prop_map(StreamStep::Send),
        (0..=most).prop_map(StreamStep::Consume)
    ]
}

proptest! {
    #[test]
    fn sender_never_exceeds_credit(steps in vec(sender_step(), 0..200)) {
        let mut window = SendWindow::new();
        let mut granted: u64 = 0;
        let mut taken: u64 = 0;
        for step in steps {
            match step {
                SenderStep::Grant(bytes) => {
                    if window.grant(bytes).is_ok() {
                        granted += u64::from(bytes);
                    }
                }
                SenderStep::Take(want) => {
                    let got = window.take(want);
                    prop_assert!(got <= want);
                    taken += got as u64;
                }
            }
            prop_assert!(taken <= granted);
            prop_assert_eq!(u64::from(window.available()), granted - taken);
            prop_assert!(window.available() <= MAX_WINDOW);
        }
    }

    #[test]
    fn granter_never_has_more_than_a_window_outstanding(
        (window, steps) in (1..=4096u32).prop_flat_map(|window| {
            (Just(window), vec(stream_step(window), 0..200))
        }),
    ) {
        let (mut granter, first) = Granter::new(window);
        let mut sender = SendWindow::new();
        sender.grant(first).unwrap();
        // Bytes the consumer has received and not yet processed.
        let mut buffered: usize = 0;

        for step in steps {
            match step {
                StreamStep::Send(want) => {
                    let sent = sender.take(want);
                    // A sender that keeps to its window is never reported.
                    prop_assert_eq!(granter.on_received(sent), Ok(()));
                    buffered += sent;
                }
                StreamStep::Consume(want) => {
                    let consumed = want.min(buffered);
                    buffered -= consumed;
                    if let Some(grant) = granter.on_consumed(consumed) {
                        prop_assert!(grant > 0);
                        sender.grant(grant).unwrap();
                    }
                }
            }
            // What the consumer may have to hold never exceeds one window.
            prop_assert!(buffered + sender.available() as usize <= window as usize);
            prop_assert_eq!(granter.outstanding(), sender.available());
            // The stream cannot stall: once everything received is consumed, the sender
            // holds credit again.
            if buffered == 0 {
                prop_assert!(sender.available() > 0);
            }
        }
    }
}
