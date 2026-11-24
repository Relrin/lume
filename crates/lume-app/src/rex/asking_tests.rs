//! What the SSH client may ask, against a page that answers as the test says.

use lume_proto::app::PageResult;
use lume_session::Router;
use serde_json::Value;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::*;
use crate::marks::Marks;
use crate::testing::{RecordingCarrier, STEP};

const PASSWORD: &str = "me@build's password: ";
const PASSPHRASE: &str = "Enter passphrase for key '/home/me/.ssh/id': ";
const HOST_KEY: &str = "The authenticity of host 'build (10.0.0.4)' can't be established.\n\
     ED25519 key fingerprint is SHA256:abc.\n\
     This key is not known by any other names.\n\
     Are you sure you want to continue connecting (yes/no/[fingerprint])? ";

/// A window's page.
struct Page {
    label: String,
    id: u32,
    carrier: Arc<RecordingCarrier>,
}

struct World {
    state: Arc<AppState>,
    asking: Arc<Asking>,
    window: WindowId,
}

impl World {
    fn new() -> Self {
        Self {
            state: AppState::new(
                Handle::current(),
                Router::new(),
                Vec::new(),
                Duration::from_secs(600),
                Arc::new(Marks::start()),
            ),
            asking: Asking::new("the-endpoint", PathBuf::from("the program")),
            window: WindowId::new(1),
        }
    }

    async fn page(&self, window: u32) -> Page {
        let label = label_of(WindowId::new(window));
        let carrier = RecordingCarrier::new();
        let id = self.state.open(&label, carrier.clone()).await;
        Page { label, id, carrier }
    }

    fn attempt(&self) -> Arc<Attempt> {
        self.asking.attempt(
            &Arc::downgrade(&self.state),
            self.window,
            "Build box",
            "ssh://me@build",
        )
    }

    /// What the client asks in a run of it.
    fn ask(&self, run: &PromptRun, words: &str) -> Asked {
        self.ask_hinted(run, words, None)
    }

    fn ask_hinted(&self, run: &PromptRun, words: &str, hint: Option<&'static str>) -> Asked {
        let (asking, token, words) = (Arc::clone(&self.asking), token_of(run), words.to_owned());
        Asked(tokio::spawn(async move {
            asking.answer(&token, &words, hint).await
        }))
    }

    fn answer(&self, page: &Page, put: &Value, secret: Option<&str>) {
        let answer = PageResult::Ok(AppReplyBody::Secret {
            secret: secret.map(|secret| Secret::new(secret.to_owned())),
        });
        let id = put["id"].as_u64().unwrap();
        self.state
            .answer(&page.label, page.id, id, answer.into())
            .unwrap();
    }
}

/// A question on its way.
struct Asked(JoinHandle<Result<Option<Secret>, AppError>>);

impl Asked {
    async fn answered(self) -> Result<Option<Secret>, AppError> {
        timeout(STEP, self.0).await.expect("an answer").unwrap()
    }

    async fn text(self) -> String {
        let secret = self.answered().await.unwrap().expect("an answer");
        secret.expose().to_owned()
    }
}

impl Page {
    fn put(&self) -> Vec<Value> {
        self.carrier
            .requests()
            .into_iter()
            .filter_map(|sent| sent.get("request").cloned())
            .collect()
    }

    /// The nth question put on the page.
    async fn nth(&self, nth: usize) -> Value {
        let waiting = async {
            loop {
                if let Some(put) = self.put().get(nth) {
                    return put.clone();
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        };
        timeout(STEP, waiting).await.expect("the page is asked")
    }

    async fn withdrawn(&self, ask: &Value) {
        let waiting = async {
            while !self.carrier.requests().iter().any(|sent| {
                sent["notify"]["type"] == "sign_in_withdrawn" && sent["notify"]["ask"] == *ask
            }) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        };
        timeout(STEP, waiting)
            .await
            .expect("the question is withdrawn");
    }
}

fn token_of(run: &PromptRun) -> Secret {
    let (_, token) = run
        .env
        .iter()
        .find(|(name, _)| name == ASKPASS_TOKEN_VAR)
        .unwrap();
    Secret::new(token.to_str().unwrap().to_owned())
}

#[test]
fn what_is_asked_is_known_by_the_clients_own_words() {
    use SignInKind::{Code, Confirm, HostKey, Passphrase, Password};
    let cases = [
        (HOST_KEY, None, HostKey),
        (PASSWORD, None, Password),
        (PASSPHRASE, None, Passphrase),
        ("Allow the use of the key?", Some("confirm"), Confirm),
        ("Accept updated hostkeys? (yes/no): ", None, Confirm),
        ("(me@build) Verification code: ", None, Code),
        // What the other machine writes is not taken for the client's own.
        ("(me@build) bob@bank's password: ", None, Code),
        ("(me@build) Trust me (yes/no)? ", None, Code),
        ("me@build's password: \n", None, Code),
        ("Is this the machine (yes/no/[fingerprint])? ", None, Code),
        ("", None, Code),
    ];
    for (words, hint, kind) in cases {
        assert_eq!(kind_of(words, hint), kind, "{words}");
    }
}

#[test]
fn words_are_shown_as_text_and_no_longer_than_they_may_be() {
    assert_eq!(
        shown("a\x1b[31mb\tc\r\nd\u{202e}e\u{200b}f"),
        "a[31mb c\ndef"
    );
    let long = shown(&"é".repeat(ASKPASS_WORDS_MAX));
    assert_eq!(long.len(), ASKPASS_WORDS_MAX);
}

#[tokio::test]
async fn a_run_of_the_client_is_told_how_to_ask() {
    let world = World::new();
    let attempt = world.attempt();
    let (one, two) = (attempt.run().unwrap(), attempt.run().unwrap());
    let var = |name: &str| {
        let (_, value) = one.env.iter().find(|(var, _)| var == name).unwrap();
        value.to_str().unwrap().to_owned()
    };
    assert_eq!(var("SSH_ASKPASS"), "the program");
    assert_eq!(var("SSH_ASKPASS_REQUIRE"), "force");
    assert_eq!(var(ASKPASS_ENDPOINT_VAR), "the-endpoint");
    assert_eq!(var(ASKPASS_TOKEN_VAR).len(), 2 * TOKEN_BYTES);
    assert_ne!(token_of(&one), token_of(&two));
}

#[tokio::test]
async fn the_question_is_put_and_its_answer_comes_back() {
    let world = World::new();
    let page = world.page(1).await;
    let attempt = world.attempt();
    let run = attempt.run().unwrap();
    let asked = world.ask(&run, HOST_KEY);
    let put = page.nth(0).await;
    assert_eq!(put["body"]["type"], "sign_in");
    assert_eq!(put["body"]["kind"], "host_key");
    assert_eq!(put["body"]["host"], "Build box");
    assert_eq!(put["body"]["address"], "ssh://me@build");
    assert_eq!(put["body"]["words"], HOST_KEY.trim());
    assert_eq!(put["body"]["again"], false);
    world.answer(&page, &put, Some("yes"));
    assert_eq!(asked.text().await, "yes");
    assert!(attempt.waited() > Duration::ZERO);
}

#[tokio::test]
async fn what_was_typed_serves_the_second_login_and_a_code_does_not() {
    let world = World::new();
    let page = world.page(1).await;
    let attempt = world.attempt();
    let code = "(me@build) Verification code: ";

    let first = attempt.run().unwrap();
    for (nth, (words, typed)) in [(PASSWORD, "hunter2"), (code, "111")]
        .into_iter()
        .enumerate()
    {
        let asked = world.ask(&first, words);
        world.answer(&page, &page.nth(nth).await, Some(typed));
        assert_eq!(asked.text().await, typed);
    }
    drop(first);

    let second = attempt.run().unwrap();
    assert_eq!(world.ask(&second, PASSWORD).text().await, "hunter2");
    assert_eq!(page.put().len(), 2);
    let asked = world.ask(&second, code);
    world.answer(&page, &page.nth(2).await, Some("222"));
    assert_eq!(asked.text().await, "222");
    // It is asked again for a code, but not because the last one was wrong.
    assert_eq!(page.nth(2).await["body"]["again"], false);
}

#[tokio::test]
async fn the_same_question_in_the_same_run_is_put_again() {
    let world = World::new();
    let page = world.page(1).await;
    let attempt = world.attempt();
    let run = attempt.run().unwrap();
    for (nth, typed) in ["wrong", "right"].into_iter().enumerate() {
        let asked = world.ask(&run, PASSWORD);
        let put = page.nth(nth).await;
        assert_eq!(put["body"]["again"], nth > 0);
        world.answer(&page, &put, Some(typed));
        assert_eq!(asked.text().await, typed);
    }
    // The next login is told the last one, and if that is wrong there the user is asked.
    let run = attempt.run().unwrap();
    assert_eq!(world.ask(&run, PASSWORD).text().await, "right");
    let asked = world.ask(&run, PASSWORD);
    let put = page.nth(2).await;
    assert_eq!(put["body"]["again"], true);
    world.answer(&page, &put, Some("newer"));
    assert_eq!(asked.text().await, "newer");
}

#[tokio::test]
async fn a_key_that_is_skipped_is_skipped_at_the_second_login_too() {
    let world = World::new();
    let page = world.page(1).await;
    let attempt = world.attempt();
    let run = attempt.run().unwrap();
    let asked = world.ask(&run, PASSPHRASE);
    world.answer(&page, &page.nth(0).await, Some(""));
    assert_eq!(asked.text().await, "");
    let run = attempt.run().unwrap();
    assert_eq!(world.ask(&run, PASSPHRASE).text().await, "");
    assert_eq!(page.put().len(), 1);
}

#[tokio::test]
async fn a_user_who_gives_up_is_not_told_to_the_client_until_it_is_ended() {
    let world = World::new();
    let page = world.page(1).await;
    let attempt = world.attempt();
    let run = attempt.run().unwrap();
    let mut asked = world.ask(&run, PASSWORD);
    world.answer(&page, &page.nth(0).await, None);
    timeout(STEP, attempt.given_up()).await.unwrap();
    let early = timeout(Duration::from_millis(200), &mut asked.0).await;
    assert!(early.is_err(), "the client is told {early:?}");
    // Nothing more is put to a user who gave up.
    let more = world.ask(&run, "(me@build) Verification code: ");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(page.put().len(), 1);
    drop(run);
    assert_eq!(asked.answered().await, Ok(None));
    assert_eq!(more.answered().await, Ok(None));
}

#[tokio::test]
async fn a_run_that_ends_takes_its_question_with_it() {
    let world = World::new();
    let page = world.page(1).await;
    let attempt = world.attempt();
    let run = attempt.run().unwrap();
    let token = token_of(&run);
    let asked = world.ask(&run, PASSWORD);
    let put = page.nth(0).await;
    drop(run);
    assert_eq!(asked.answered().await, Ok(None));
    page.withdrawn(&put["body"]["ask"]).await;
    // The user did not give up, and the token opens nothing any more.
    assert!(!attempt.is_given_up());
    let late = world.asking.answer(&token, PASSWORD, None).await;
    assert_eq!(late.unwrap_err().code, codes::NOT_FOUND);
    let guessed = Secret::new("0".repeat(64));
    let guessed = world.asking.answer(&guessed, PASSWORD, None).await;
    assert_eq!(guessed.unwrap_err().code, codes::NOT_FOUND);
}

#[tokio::test]
async fn a_client_that_goes_takes_its_question_with_it() {
    let world = World::new();
    let page = world.page(1).await;
    let attempt = world.attempt();
    let run = attempt.run().unwrap();
    let asked = world.ask(&run, PASSWORD);
    let put = page.nth(0).await;
    asked.0.abort();
    page.withdrawn(&put["body"]["ask"]).await;
    assert!(!attempt.is_given_up());
    // The time it was open counts, and no longer does.
    let waited = attempt.waited();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(attempt.waited(), waited);
}

#[tokio::test]
async fn a_page_that_is_loaded_again_is_asked_again() {
    let world = World::new();
    let first = world.page(1).await;
    let attempt = world.attempt();
    let run = attempt.run().unwrap();
    let asked = world.ask(&run, PASSWORD);
    first.nth(0).await;
    let second = world.page(1).await;
    let put = second.nth(0).await;
    world.answer(&second, &put, Some("hunter2"));
    assert_eq!(asked.text().await, "hunter2");
}

#[tokio::test]
async fn a_window_that_closes_leaves_the_question_to_another_or_to_nobody() {
    let world = World::new();
    let own = world.page(1).await;
    let other = world.page(2).await;
    let attempt = world.attempt();
    let run = attempt.run().unwrap();
    let asked = world.ask(&run, PASSWORD);
    own.nth(0).await;
    world.state.retire(&own.label);
    other.nth(0).await;
    world.state.retire(&other.label);
    timeout(STEP, attempt.given_up()).await.unwrap();
    drop(run);
    assert_eq!(asked.answered().await, Ok(None));
}

#[tokio::test(start_paused = true)]
async fn a_question_nobody_answers_is_given_up() {
    let world = World::new();
    let page = world.page(1).await;
    let attempt = world.attempt();
    let run = attempt.run().unwrap();
    let began = Instant::now();
    let _asked = world.ask(&run, PASSWORD);
    attempt.given_up().await;
    assert_eq!(began.elapsed().as_secs(), ASKPASS_QUESTION_LIMIT.as_secs());
    let put = page.nth(0).await;
    page.withdrawn(&put["body"]["ask"]).await;
    assert!(page.carrier.requests().iter().any(|sent| {
        sent["notify"]["type"] == "notice"
            && sent["notify"]["message"]
                .as_str()
                .is_some_and(|said| said.starts_with("Build box: "))
    }));
}

#[tokio::test]
async fn a_client_that_asks_without_end_is_not_answered_without_end() {
    let world = World::new();
    let page = world.page(1).await;
    let attempt = world.attempt();
    let run = attempt.run().unwrap();
    let most = usize::try_from(ASKPASS_QUESTIONS_MAX).unwrap();
    for nth in 0..most {
        let asked = world.ask(&run, "(me@build) Code: ");
        world.answer(&page, &page.nth(nth).await, Some("1"));
        assert_eq!(asked.text().await, "1");
    }
    let _asked = world.ask(&run, "(me@build) Code: ");
    timeout(STEP, attempt.given_up()).await.unwrap();
    assert_eq!(page.put().len(), most);
}

#[tokio::test]
async fn what_is_only_said_is_shown_and_not_waited_for() {
    let world = World::new();
    let page = world.page(1).await;
    let attempt = world.attempt();
    let run = attempt.run().unwrap();
    let touch = "Confirm user presence for key ED25519-SK";
    let asked = world.ask_hinted(&run, touch, Some("none"));
    assert_eq!(asked.text().await, "");
    assert_eq!(page.put().len(), 0);
    let said = page.carrier.requests().pop().unwrap();
    assert_eq!(said["notify"]["message"], format!("Build box: {touch}"));
}

#[tokio::test]
async fn nothing_typed_is_in_a_record_of_the_attempt_or_held_by_its_clock() {
    let world = World::new();
    let page = world.page(1).await;
    let attempt = world.attempt();
    let run = attempt.run().unwrap();
    let asked = world.ask(&run, PASSWORD);
    world.answer(&page, &page.nth(0).await, Some("hunter2"));
    asked.text().await;
    assert!(!format!("{attempt:?}").contains("hunter2"));
    let (clock, held) = (attempt.clock(), Arc::downgrade(&attempt));
    drop((run, attempt));
    // The task that answered may still be ending.
    let gone = async {
        while held.upgrade().is_some() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    timeout(STEP, gone)
        .await
        .expect("what was typed is let go of");
    assert!(clock.so_far() > Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn the_time_the_user_takes_is_not_time_the_server_took() {
    let waited = Waited::default();
    let limit = Duration::from_secs(90);
    let work = async {
        waited.begin();
        tokio::time::sleep(Duration::from_secs(300)).await;
        waited.end();
        tokio::time::sleep(Duration::from_secs(80)).await;
    };
    assert_eq!(waited.within(limit, work).await, Some(()));
    let slow = tokio::time::sleep(Duration::from_secs(91));
    assert_eq!(waited.within(limit, slow).await, None);
}
