//! Asking the user what the SSH client wants to know: a password, the passphrase of a
//! key, whether a machine is the one they mean.
//!
//! The client has no terminal to ask in. It runs a program that asks for it instead,
//! which its environment names. What that program is and how it reaches the user is
//! the application's; here is only when the client may ask, and that time spent asking
//! is not time the machine took.

use std::any::Any;
use std::ffi::OsString;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::time::Instant;

use crate::tuning::PROMPT_POLL;

/// Answers the SSH client for the user, while one connection is first made.
#[async_trait]
pub trait Prompter: fmt::Debug + Send + Sync + 'static {
    /// A run of the client begins. Gives what it needs to ask; nothing when this run
    /// cannot ask, and is told not to.
    fn run(&self) -> Option<PromptRun>;

    /// How long the user has been waited for in all, with a question that is still open.
    fn waited(&self) -> Duration;

    /// Comes back once the user has said that they do not sign in after all, and not
    /// before.
    async fn given_up(&self);
}

/// What one run of the client asks through.
pub struct PromptRun {
    /// What is set in the client's environment, and in nothing else's.
    pub env: Vec<(String, OsString)>,
    /// Kept for as long as the run may ask. A question that is open when it is dropped
    /// is not answered.
    pub ends: Box<dyn Any + Send>,
}

/// Why something was not waited for to its end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cut {
    /// It took longer than it may.
    Late,
    /// The user gave up signing in.
    GivenUp,
}

/// Whether the client may ask while one connection is made, and whom.
#[derive(Clone, Debug, Default)]
pub(crate) struct Asking(Option<Arc<dyn Prompter>>);

impl Asking {
    pub(crate) fn of(prompter: Option<Arc<dyn Prompter>>) -> Self {
        Self(prompter)
    }

    pub(crate) fn run(&self) -> Option<PromptRun> {
        self.0.as_ref()?.run()
    }

    /// Waits for `work`, for `limit` at most. The time the user was waited for does not
    /// count, and the wait ends at once when they give up.
    pub(crate) async fn within<T>(
        &self,
        limit: Duration,
        work: impl Future<Output = T>,
    ) -> Result<T, Cut> {
        let Some(prompter) = &self.0 else {
            return tokio::time::timeout(limit, work)
                .await
                .map_err(|_| Cut::Late);
        };
        let began = Instant::now();
        let before = prompter.waited();
        let mut work = std::pin::pin!(work);
        let mut given_up = std::pin::pin!(prompter.given_up());
        loop {
            let asked = prompter.waited().saturating_sub(before);
            let left = (began + limit + asked).saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Cut::Late);
            }
            tokio::select! {
                biased;
                () = &mut given_up => return Err(Cut::GivenUp),
                done = &mut work => return Ok(done),
                // An open question keeps what is left as it is, so this comes back
                // until it is answered.
                () = tokio::time::sleep(left.max(PROMPT_POLL)) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tokio::sync::watch;

    use super::*;

    /// A user who is asked between two moments, and may give up.
    #[derive(Debug)]
    struct User {
        asked: Mutex<(Option<Instant>, Duration)>,
        gone: watch::Sender<bool>,
    }

    impl User {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                asked: Mutex::new((None, Duration::ZERO)),
                gone: watch::Sender::new(false),
            })
        }

        fn ask(&self) {
            self.asked.lock().unwrap().0 = Some(Instant::now());
        }

        fn answer(&self) {
            let mut asked = self.asked.lock().unwrap();
            let since = asked.0.take().unwrap();
            asked.1 += since.elapsed();
        }
    }

    #[async_trait]
    impl Prompter for User {
        fn run(&self) -> Option<PromptRun> {
            None
        }

        fn waited(&self) -> Duration {
            let asked = self.asked.lock().unwrap();
            asked.1 + asked.0.map(|since| since.elapsed()).unwrap_or_default()
        }

        async fn given_up(&self) {
            let mut gone = self.gone.subscribe();
            let _ = gone.wait_for(|gone| *gone).await;
        }
    }

    const LIMIT: Duration = Duration::from_secs(30);

    async fn after(seconds: u64) -> u64 {
        tokio::time::sleep(Duration::from_secs(seconds)).await;
        seconds
    }

    #[tokio::test(start_paused = true)]
    async fn what_nobody_is_asked_about_has_its_time_and_no_more() {
        let asking = Asking::default();
        assert_eq!(asking.within(LIMIT, after(29)).await, Ok(29));
        assert_eq!(asking.within(LIMIT, after(31)).await, Err(Cut::Late));
        assert!(asking.run().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn the_time_the_user_takes_is_not_time_the_machine_took() {
        let user = User::new();
        let asking = Asking::of(Some(user.clone()));
        // Ten seconds of the machine's, a hundred of the user's, ten of the machine's.
        let work = async {
            after(10).await;
            user.ask();
            after(100).await;
            user.answer();
            after(10).await
        };
        assert_eq!(asking.within(LIMIT, work).await, Ok(10));
        // What an earlier wait spent asking is not given to the next.
        let began = Instant::now();
        assert_eq!(asking.within(LIMIT, after(500)).await, Err(Cut::Late));
        assert!(began.elapsed() < LIMIT + Duration::from_secs(1));
        // Once the answer is there, the machine has what was left of its time.
        let work = async {
            user.ask();
            after(100).await;
            user.answer();
            after(31).await
        };
        assert_eq!(asking.within(LIMIT, work).await, Err(Cut::Late));
    }

    #[tokio::test(start_paused = true)]
    async fn the_wait_ends_when_the_user_gives_up() {
        let user = User::new();
        let asking = Asking::of(Some(user.clone()));
        let work = async {
            user.ask();
            after(5).await;
            user.gone.send_replace(true);
            std::future::pending::<()>().await;
        };
        assert_eq!(asking.within(LIMIT, work).await, Err(Cut::GivenUp));
    }
}
