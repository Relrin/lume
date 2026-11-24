//! Asking the user what the SSH client wants to know while a connection is first made:
//! a password, the passphrase of a key, whether a machine is the one they mean.
//!
//! The client runs `lume` to ask, which comes to the control endpoint with the client's
//! words and with a token that only that run of the client was given. The words are put
//! to the user in the window they connect from, and the answer goes back the same way.
//!
//! What the user types is kept for the attempt it was typed for, because reaching a
//! server may take two logins, and is wiped when the attempt ends. It is in no argument,
//! no file and no record.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use async_trait::async_trait;
use lume_layout::WindowId;
use lume_proto::Secret;
use lume_proto::app::{
    ASKPASS_ENDPOINT_VAR, ASKPASS_TOKEN_VAR, AppError, AppReplyBody, PageRequestBody, SignInKind,
    codes,
};
use lume_rex::{PromptRun, Prompter};
use serde_json::json;
use tokio::sync::watch;
use tokio::time::Instant;

use crate::carrier::Link;
use crate::state::AppState;
use crate::tuning::{
    ASKPASS_PAGE_WAIT, ASKPASS_POLL, ASKPASS_QUESTION_LIMIT, ASKPASS_QUESTIONS_MAX,
    ASKPASS_WORDS_MAX,
};
use crate::window::label_of;

/// The program the client is given to ask with.
const PROGRAM: &str = if cfg!(windows) { "lume.exe" } else { "lume" };
/// The bytes of a token.
const TOKEN_BYTES: usize = 32;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How the SSH client can ask the user, and which runs of it may.
pub struct Asking {
    /// The name of the control endpoint.
    endpoint: String,
    /// The program that asks there.
    program: PathBuf,
    /// The runs of the client that may ask, by their tokens.
    runs: Mutex<HashMap<String, Arc<Run>>>,
    asks: AtomicU64,
}

/// One run of the client.
struct Run {
    attempt: Arc<Attempt>,
    /// The questions that were answered in this run. One that comes back was not taken.
    answered: Mutex<HashSet<String>>,
    /// Whether the run may no longer ask.
    ended: watch::Sender<bool>,
}

/// Ends what a run may ask when it is dropped.
struct Ends {
    asking: Arc<Asking>,
    token: Secret,
}

impl Drop for Ends {
    fn drop(&mut self) {
        let run = lock(&self.asking.runs).remove(self.token.expose());
        if let Some(run) = run {
            run.ended.send_replace(true);
        }
    }
}

/// How long the user has been waited for. Holds nothing they said, so it may be kept by
/// whoever has to know.
#[derive(Debug, Default)]
pub struct Waited(Mutex<(Option<Instant>, Duration)>);

impl Waited {
    fn so_far(&self) -> Duration {
        let waited = lock(&self.0);
        waited.1 + waited.0.map(|since| since.elapsed()).unwrap_or_default()
    }

    fn begin(&self) {
        lock(&self.0).0 = Some(Instant::now());
    }

    fn end(&self) {
        let mut waited = lock(&self.0);
        if let Some(since) = waited.0.take() {
            waited.1 += since.elapsed();
        }
    }

    /// Waits for `work`, for `limit` at most, not counting the time the user is waited
    /// for.
    pub async fn within<T>(&self, limit: Duration, work: impl Future<Output = T>) -> Option<T> {
        let began = Instant::now();
        let before = self.so_far();
        let mut work = std::pin::pin!(work);
        loop {
            let asked = self.so_far().saturating_sub(before);
            let left = (began + limit + asked).saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            tokio::select! {
                biased;
                done = &mut work => return Some(done),
                () = tokio::time::sleep(left.max(ASKPASS_POLL)) => {}
            }
        }
    }
}

/// One attempt at connecting, from the window the user asked for it in.
pub struct Attempt {
    this: Weak<Attempt>,
    asking: Arc<Asking>,
    state: Weak<AppState>,
    window: WindowId,
    /// What the user calls the server, and its address.
    host: String,
    address: String,
    /// What was typed, by the question it answered.
    kept: Mutex<HashMap<String, Secret>>,
    waited: Arc<Waited>,
    asked: AtomicU64,
    given_up: watch::Sender<bool>,
    /// One question is put at a time.
    putting: tokio::sync::Mutex<()>,
}

impl fmt::Debug for Attempt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Attempt")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Prompter for Attempt {
    fn run(&self) -> Option<PromptRun> {
        let attempt = self.this.upgrade()?;
        let token = new_token()?;
        let env: Vec<(String, OsString)> = vec![
            (
                "SSH_ASKPASS".to_owned(),
                self.asking.program.clone().into_os_string(),
            ),
            // Asked through, whether or not there is a terminal or a display.
            ("SSH_ASKPASS_REQUIRE".to_owned(), "force".into()),
            (
                ASKPASS_ENDPOINT_VAR.to_owned(),
                self.asking.endpoint.clone().into(),
            ),
            (ASKPASS_TOKEN_VAR.to_owned(), token.clone().into()),
        ];
        let run = Arc::new(Run {
            attempt,
            answered: Mutex::default(),
            ended: watch::Sender::new(false),
        });
        lock(&self.asking.runs).insert(token.clone(), run);
        Some(PromptRun {
            env,
            ends: Box::new(Ends {
                asking: Arc::clone(&self.asking),
                token: Secret::new(token),
            }),
        })
    }

    fn waited(&self) -> Duration {
        self.waited.so_far()
    }

    async fn given_up(&self) {
        let mut given_up = self.given_up.subscribe();
        let _ = given_up.wait_for(|given_up| *given_up).await;
    }
}

/// A token nobody can guess, if the system gives one.
fn new_token() -> Option<String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    if let Err(error) = getrandom::fill(&mut bytes) {
        tracing::error!(%error, "the SSH client cannot ask anything");
        return None;
    }
    Some(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// What a question is about, by the client's hint and words. What the client does not
/// write itself, or not in these words, is a code: it is asked for each time.
fn kind_of(words: &str, hint: Option<&str>) -> SignInKind {
    let ends = |ends: &[&str]| ends.iter().any(|end| words.ends_with(end));
    if hint == Some("confirm") {
        SignInKind::Confirm
    } else if words.starts_with("The authenticity of host '")
        && ends(&["(yes/no/[fingerprint])? ", "(yes/no)? "])
    {
        SignInKind::HostKey
    } else if words.starts_with("Enter passphrase for key '") {
        SignInKind::Passphrase
    } else if words.ends_with("'s password: ") && !words.starts_with('(') {
        SignInKind::Password
    } else if ends(&["(yes/no): ", "(yes/no)? "]) && !words.starts_with('(') {
        SignInKind::Confirm
    } else {
        SignInKind::Code
    }
}

/// The client's words as they can be shown: cut to size, in one direction, and with
/// nothing in them that is not text.
fn shown(words: &str) -> String {
    let mut shown = String::new();
    for char in words.chars() {
        let char = match char {
            '\n' => '\n',
            '\t' => ' ',
            // Marks that change the direction of text, or are not seen at all.
            '\u{61c}'
            | '\u{200b}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{feff}' => continue,
            char if char.is_control() => continue,
            char => char,
        };
        if shown.len() + char.len_utf8() > ASKPASS_WORDS_MAX {
            break;
        }
        shown.push(char);
    }
    shown.trim().to_owned()
}

/// A question on a page. It is taken off again when nobody waits for the answer.
struct Put {
    link: Arc<Link>,
    ask: u64,
    open: bool,
}

impl Drop for Put {
    fn drop(&mut self) {
        if self.open {
            self.link
                .notify(&json!({ "type": "sign_in_withdrawn", "ask": self.ask }));
        }
    }
}

/// Counts the time a question is open.
struct Open<'a>(&'a Waited);

impl<'a> Open<'a> {
    fn new(waited: &'a Waited) -> Self {
        waited.begin();
        Self(waited)
    }
}

impl Drop for Open<'_> {
    fn drop(&mut self) {
        self.0.end();
    }
}

/// What a run of the client is given to ask with. It may ask until this is dropped.
pub struct MayAsk {
    pub env: Vec<(String, OsString)>,
    _ends: Box<dyn Any + Send>,
}

impl Attempt {
    /// Lets one run of the client ask.
    pub fn lets_ask(&self) -> Option<MayAsk> {
        self.run().map(|run| MayAsk {
            env: run.env,
            _ends: run.ends,
        })
    }

    /// Comes back once the user has said that they do not sign in after all.
    pub async fn ended(&self) {
        self.given_up().await;
    }

    /// How long the user was waited for. It may be kept when the attempt is over.
    pub fn clock(&self) -> Arc<Waited> {
        Arc::clone(&self.waited)
    }

    fn is_given_up(&self) -> bool {
        *self.given_up.borrow()
    }

    /// The page the question is put in: that of the window the user connects from, or of
    /// another that shows terminals if it is gone.
    async fn page(&self) -> Option<Arc<Link>> {
        let state = self.state.upgrade()?;
        let own = label_of(self.window);
        if let Some(link) = state.page_of(&own) {
            return Some(link);
        }
        // A page that is loaded again is back in a moment.
        let loaded = state
            .soon(ASKPASS_PAGE_WAIT, |state| state.page_of(&own))
            .await;
        loaded.or_else(|| state.links().into_iter().next().map(|(_, link)| link))
    }

    fn tell(&self, link: &Link, message: &str) {
        link.notify(&json!({
            "type": "notice",
            "level": "info",
            "message": format!("{}: {message}", self.host),
        }));
    }

    /// Puts a question to the user. No answer is their giving up.
    async fn put(&self, kind: SignInKind, words: &str, again: bool) -> Option<Secret> {
        if self.asked.fetch_add(1, Ordering::Relaxed) >= ASKPASS_QUESTIONS_MAX {
            tracing::warn!(address = %self.address, "the SSH client asks without end");
            return None;
        }
        let _open = Open::new(&self.waited);
        let until = Instant::now() + ASKPASS_QUESTION_LIMIT;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            let link = self.page().await.filter(|_| !left.is_zero())?;
            let ask = self.asking.asks.fetch_add(1, Ordering::Relaxed) + 1;
            let mut put = Put {
                link: Arc::clone(&link),
                ask,
                open: true,
            };
            let request = PageRequestBody::SignIn {
                ask,
                kind,
                host: self.host.clone(),
                address: self.address.clone(),
                words: shown(words),
                again,
            };
            let error = match link.ask(request, left).await {
                Ok(AppReplyBody::Secret { secret }) => {
                    put.open = false;
                    return secret;
                }
                Ok(_) => return None,
                Err(error) => error,
            };
            match error.code.as_str() {
                // The page went with its window, or is being loaded again.
                codes::NO_WINDOW => {
                    put.open = false;
                    tokio::time::sleep(ASKPASS_POLL).await;
                }
                codes::TIMEOUT => {
                    self.tell(
                        &link,
                        "the sign-in was given up, because nothing was answered.",
                    );
                    return None;
                }
                code => {
                    tracing::warn!(code, "a question of the SSH client's could not be put");
                    return None;
                }
            }
        }
    }

    /// The answer to a question, as it was given before or is given now.
    async fn answer(&self, run: &Run, words: &str, hint: Option<&str>) -> Option<Secret> {
        if hint == Some("none") {
            // Nothing to answer: something is to be done, such as touching a key.
            if let Some(link) = self.page().await {
                self.tell(&link, &shown(words));
            }
            return Some(Secret::default());
        }
        let _one_at_a_time = self.putting.lock().await;
        if self.is_given_up() {
            return None;
        }
        let kind = kind_of(words, hint);
        let again = !lock(&run.answered).insert(words.to_owned());
        let keeps = matches!(kind, SignInKind::Password | SignInKind::Passphrase);
        if keeps {
            let mut kept = lock(&self.kept);
            if again {
                kept.remove(words);
            } else if let Some(secret) = kept.get(words) {
                return Some(secret.clone());
            }
        }
        let Some(secret) = self.put(kind, words, again).await else {
            self.given_up.send_replace(true);
            return None;
        };
        if keeps {
            lock(&self.kept).insert(words.to_owned(), secret.clone());
        }
        Some(secret)
    }
}

impl Asking {
    /// The way to ask through the control endpoint called `endpoint`, if the program that
    /// does it is beside this one.
    pub fn beside_this_program(endpoint: &str) -> Option<Arc<Self>> {
        let program = std::env::current_exe().ok()?.with_file_name(PROGRAM);
        if !program.is_file() {
            tracing::warn!(
                program = %program.display(),
                "the program the SSH client asks with is missing; it asks nothing"
            );
            return None;
        }
        Some(Self::new(endpoint, program))
    }

    pub(super) fn new(endpoint: &str, program: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            endpoint: endpoint.to_owned(),
            program,
            runs: Mutex::default(),
            asks: AtomicU64::new(0),
        })
    }

    /// An attempt at connecting to a server, in which the client may ask the user in a
    /// window.
    pub fn attempt(
        self: &Arc<Self>,
        state: &Weak<AppState>,
        window: WindowId,
        host: &str,
        address: &str,
    ) -> Arc<Attempt> {
        Arc::new_cyclic(|this| Attempt {
            this: this.clone(),
            asking: Arc::clone(self),
            state: state.clone(),
            window,
            host: host.to_owned(),
            address: address.to_owned(),
            kept: Mutex::default(),
            waited: Arc::default(),
            asked: AtomicU64::new(0),
            given_up: watch::Sender::new(false),
            putting: tokio::sync::Mutex::new(()),
        })
    }

    /// Answers what a run of the client asks. Nothing is the answer once the run may no
    /// longer ask.
    ///
    /// The client is not told of a user who gives up: it would take that for an empty
    /// password and try it. The client is ended, and with it what it may ask.
    pub async fn answer(
        &self,
        token: &Secret,
        words: &str,
        hint: Option<&str>,
    ) -> Result<Option<Secret>, AppError> {
        let run = lock(&self.runs)
            .get(token.expose())
            .cloned()
            .ok_or_else(|| AppError::new(codes::NOT_FOUND, "nothing is being signed in to"))?;
        let mut ended = run.ended.subscribe();
        let answering = async {
            match run.attempt.answer(&run, words, hint).await {
                Some(secret) => secret,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            biased;
            _ = ended.wait_for(|ended| *ended) => Ok(None),
            secret = answering => Ok(Some(secret)),
        }
    }
}

#[cfg(test)]
#[path = "asking_tests.rs"]
mod tests;
