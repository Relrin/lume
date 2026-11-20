//! How long a call into a plugin's code may take.
//!
//! A command, a view's call and a service call each have a limit. The time a question
//! is on show to the user is not counted against it: how long somebody reads a prompt
//! is not the code's doing, and a call that confirms something first would otherwise
//! fail whenever the user took their time.
//!
//! Any question stops every call's clock, whichever plugin asked it. A call often waits
//! on another plugin's question (a service that has a use approved), and a question is
//! itself held to a limit, so nothing waits for ever because of this.

use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::oneshot;
use tokio::time::Instant;

/// How often a call whose time has run out looks whether the user is still being asked.
const LOOK: Duration = Duration::from_millis(250);

/// The questions on show to the user, and how long questions have been on show in all.
#[derive(Default)]
pub(super) struct Asked {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// How many questions are waiting for an answer.
    open: usize,
    /// When the first of them was put.
    since: Option<Instant>,
    /// How long questions answered since were on show.
    total: Duration,
}

/// A question on show, for as long as this is held.
pub(super) struct Asking<'a>(&'a Asked);

impl Drop for Asking<'_> {
    fn drop(&mut self) {
        let mut inner = self.0.inner.lock().expect("questions lock");
        inner.open = inner.open.saturating_sub(1);
        if inner.open == 0
            && let Some(since) = inner.since.take()
        {
            inner.total += since.elapsed();
        }
    }
}

impl Asked {
    /// A question is put to the user now, and is on show until what this gives is
    /// dropped.
    pub(super) fn begin(&self) -> Asking<'_> {
        let mut inner = self.inner.lock().expect("questions lock");
        inner.open += 1;
        if inner.since.is_none() {
            inner.since = Some(Instant::now());
        }
        Asking(self)
    }

    /// How long questions have been on show so far, and whether one is now.
    fn spent(&self) -> (Duration, bool) {
        let inner = self.inner.lock().expect("questions lock");
        let running = inner.since.map_or(Duration::ZERO, |since| since.elapsed());
        (inner.total + running, inner.open > 0)
    }

    /// Waits for an answer for `limit`, not counting the time the user is being asked
    /// something. Nothing comes back when the limit has passed; an error, when whoever
    /// was to answer is gone.
    pub(super) async fn within<T>(
        &self,
        limit: Duration,
        mut receiver: oneshot::Receiver<T>,
    ) -> Option<Result<T, oneshot::error::RecvError>> {
        let began = Instant::now();
        let (asked_before, _) = self.spent();
        loop {
            let (asked, asking) = self.spent();
            let counted = began
                .elapsed()
                .saturating_sub(asked.saturating_sub(asked_before));
            let left = limit.saturating_sub(counted);
            if left.is_zero() && !asking {
                return None;
            }
            // While a question is on show the time left does not shrink, so the next
            // look comes after a while rather than after what is left.
            let wait = if asking { left.max(LOOK) } else { left };
            if let Ok(answer) = tokio::time::timeout(wait, &mut receiver).await {
                return Some(answer);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: Duration = Duration::from_millis(300);

    #[tokio::test]
    async fn an_answer_in_time_comes_back_and_one_that_is_late_does_not() {
        let asked = Asked::default();
        let (sender, receiver) = oneshot::channel();
        sender.send(7).unwrap();
        assert_eq!(asked.within(LIMIT, receiver).await, Some(Ok(7)));

        let (_sender, receiver) = oneshot::channel::<u32>();
        let began = Instant::now();
        assert_eq!(asked.within(LIMIT, receiver).await, None);
        assert!(began.elapsed() >= LIMIT);
        assert!(began.elapsed() < LIMIT * 3, "{:?}", began.elapsed());

        let (sender, receiver) = oneshot::channel::<u32>();
        drop(sender);
        assert!(matches!(asked.within(LIMIT, receiver).await, Some(Err(_))));
    }

    #[tokio::test]
    async fn the_time_a_question_is_on_show_is_not_counted() {
        let asked = std::sync::Arc::new(Asked::default());
        let (sender, receiver) = oneshot::channel();
        let asking = std::sync::Arc::clone(&asked);
        // The code asks at once, the user takes three times the limit to answer, and
        // the code then answers the call.
        tokio::spawn(async move {
            {
                let _question = asking.begin();
                tokio::time::sleep(LIMIT * 3).await;
            }
            tokio::time::sleep(LIMIT / 3).await;
            sender.send("done").unwrap();
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(asked.within(LIMIT, receiver).await, Some(Ok("done")));
    }

    #[tokio::test]
    async fn once_the_question_is_answered_the_rest_of_the_limit_is_all_there_is() {
        let asked = std::sync::Arc::new(Asked::default());
        let (_sender, receiver) = oneshot::channel::<u32>();
        let asking = std::sync::Arc::clone(&asked);
        tokio::spawn(async move {
            let _question = asking.begin();
            tokio::time::sleep(LIMIT * 2).await;
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        let began = Instant::now();
        assert_eq!(asked.within(LIMIT, receiver).await, None);
        let took = began.elapsed();
        assert!(took >= LIMIT * 3 - Duration::from_millis(40), "{took:?}");
        assert!(took < LIMIT * 5, "{took:?}");
    }

    #[tokio::test]
    async fn two_questions_at_once_are_counted_once() {
        let asked = Asked::default();
        let first = asked.begin();
        let second = asked.begin();
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(first);
        assert!(asked.spent().1);
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(second);
        let (spent, asking) = asked.spent();
        assert!(!asking);
        assert!(spent >= Duration::from_millis(200), "{spent:?}");
        assert!(spent < Duration::from_millis(390), "{spent:?}");
    }
}
