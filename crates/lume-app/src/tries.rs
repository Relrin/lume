//! Tries of a machine that a page waits for, so that they can be ended when it does not
//! any more: what they ask the user is then not asked.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::future::Future;
use std::sync::{Mutex, MutexGuard, PoisonError};

use tokio::sync::oneshot;

use crate::error::CommandError;

/// What a page calls one of its tries: its webview's label, and a number of its own.
type Name = (String, u32);

/// The tries that go on.
#[derive(Default)]
pub struct Tries(Mutex<HashMap<Name, oneshot::Sender<()>>>);

/// A try that goes on, which is forgotten when it is over.
struct Going<'a> {
    tries: &'a Tries,
    name: Name,
}

impl Drop for Going<'_> {
    fn drop(&mut self) {
        self.tries.going().remove(&self.name);
    }
}

impl Tries {
    fn going(&self) -> MutexGuard<'_, HashMap<Name, oneshot::Sender<()>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Goes through a try until it is over or is ended.
    pub async fn run<T>(
        &self,
        label: &str,
        number: u32,
        work: impl Future<Output = Result<T, CommandError>>,
    ) -> Result<T, CommandError> {
        let name = (label.to_owned(), number);
        let (end, ended) = oneshot::channel();
        match self.going().entry(name.clone()) {
            Entry::Occupied(_) => {
                return Err(CommandError::bad_request("the try goes on already"));
            }
            Entry::Vacant(free) => free.insert(end),
        };
        let _going = Going { tries: self, name };
        tokio::select! {
            biased;
            _ = ended => Err(CommandError::new("ended", "nobody waits for it any more")),
            done = work => done,
        }
    }

    /// Ends one try of a page. One that is over is passed over.
    pub fn end(&self, label: &str, number: u32) {
        drop(self.going().remove(&(label.to_owned(), number)));
    }

    /// Ends every try of a page that has gone.
    pub fn end_of(&self, label: &str) {
        self.going().retain(|(of, _), _| of != label);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::task::JoinHandle;
    use tokio::time::timeout;

    use super::*;

    fn begin(tries: &Arc<Tries>, label: &str, number: u32) -> JoinHandle<Result<(), CommandError>> {
        let (tries, label) = (Arc::clone(tries), label.to_owned());
        tokio::spawn(async move { tries.run(&label, number, std::future::pending()).await })
    }

    async fn ended(going: JoinHandle<Result<(), CommandError>>) {
        let error = timeout(Duration::from_secs(20), going)
            .await
            .expect("the try ends")
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, "ended");
    }

    #[tokio::test]
    async fn a_try_ends_when_its_page_says_so_or_goes() {
        let tries = Arc::new(Tries::default());
        let one = begin(&tries, "main-1", 1);
        let two = begin(&tries, "main-1", 2);
        let other = begin(&tries, "main-2", 1);
        while tries.going().len() < 3 {
            tokio::task::yield_now().await;
        }
        let same = tries.run("main-1", 1, async { Ok(()) }).await;
        assert_eq!(same.unwrap_err().code, "bad_request");
        tries.end("main-1", 1);
        ended(one).await;
        tries.end("main-1", 1);
        tries.end_of("main-1");
        ended(two).await;
        assert!(!other.is_finished());
        assert_eq!(tries.run("main-1", 1, async { Ok(7) }).await.unwrap(), 7);
        tries.end_of("main-2");
        ended(other).await;
        assert!(tries.going().is_empty());
    }
}
