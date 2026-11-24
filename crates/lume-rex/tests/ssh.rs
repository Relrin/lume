//! A server reached through the SSH client while the client has something to ask: a
//! program that stands in for the client, and a pretend server behind it.

mod common;

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use common::{Listener, World, client, door, serve, until};
use lume_rex::{Endpoint, Link, Options, PromptRun, Prompter, RexBackend, SshOptions};
use lume_session::SessionBackend as _;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;

const PASSWORD_WORDS: &str = "me@build's password: ";
const PASSWORD: &str = "correct horse";
const DENIED: &str = "me@build: Permission denied (publickey,password).";

/// The program that stands in for the client, and for what it asks through.
fn stand_in() -> PathBuf {
    let name = format!("stand_in_ssh{}", std::env::consts::EXE_SUFFIX);
    let tests = std::env::current_exe().unwrap();
    // Examples are built beside the folder the tests are in, however deep that is.
    let found = tests
        .ancestors()
        .map(|folder| folder.join("examples").join(&name))
        .find(|program| program.is_file());
    found.unwrap_or_else(|| panic!("{name} is built with the tests of the whole crate"))
}

/// A folder of one test, gone with it.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("lume-rex-ssh-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// What happened to the client so far.
    fn log(&self) -> Vec<String> {
        std::fs::read_to_string(self.file("log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What a user answers when asked for the nth time, if they answer.
type Answers = Box<dyn Fn(usize, &str) -> Option<String> + Send + Sync>;

/// Somebody at the window.
struct User {
    at: std::net::SocketAddr,
    runs: AtomicUsize,
    open: Arc<AtomicUsize>,
    asked: Arc<Mutex<Vec<String>>>,
    waited: Arc<Mutex<Duration>>,
    gone: watch::Sender<bool>,
    /// Goes up when a run may no longer ask.
    ended: watch::Sender<usize>,
    /// Whether this was let go of.
    dropped: Arc<AtomicBool>,
}

impl std::fmt::Debug for User {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("User")
    }
}

impl Drop for User {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

/// What is left to look at of a user when the client no longer asks them.
struct Seen {
    asked: Arc<Mutex<Vec<String>>>,
    open: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
    gone: watch::Sender<bool>,
}

impl Seen {
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl User {
    /// A user who answers as `answers` says, and takes `thinks` each time.
    async fn new(thinks: Duration, answers: Answers) -> (Arc<Self>, Seen) {
        let door = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let user = Arc::new(Self {
            at: door.local_addr().unwrap(),
            runs: AtomicUsize::new(0),
            open: Arc::default(),
            asked: Arc::default(),
            waited: Arc::default(),
            gone: watch::Sender::new(false),
            ended: watch::Sender::new(0),
            dropped: Arc::default(),
        });
        let seen = Seen {
            asked: Arc::clone(&user.asked),
            open: Arc::clone(&user.open),
            dropped: Arc::clone(&user.dropped),
            gone: user.gone.clone(),
        };
        let (asked, waited) = (Arc::clone(&user.asked), Arc::clone(&user.waited));
        let ended = user.ended.clone();
        tokio::spawn(async move {
            while let Ok((mut question, _)) = door.accept().await {
                let mut words = String::new();
                let _ = question.read_to_string(&mut words).await;
                let nth = {
                    let mut asked = asked.lock().unwrap();
                    asked.push(words.clone());
                    asked.len()
                };
                // Counted as it goes by, as an open question is.
                let until = Instant::now() + thinks;
                while Instant::now() < until {
                    let step = Duration::from_millis(20);
                    tokio::time::sleep(step).await;
                    *waited.lock().unwrap() += step;
                }
                match answers(nth, &words) {
                    Some(answer) => {
                        let _ = question.write_all(format!("+{answer}").as_bytes()).await;
                    }
                    // Left open until the run may no longer ask. Who asked then learns
                    // that there is no answer.
                    None => {
                        let mut ended = ended.subscribe();
                        tokio::spawn(async move {
                            let _ = ended.changed().await;
                            drop(question);
                        });
                    }
                }
            }
        });
        (user, seen)
    }
}

/// Counts the runs that may ask.
struct Open(Arc<AtomicUsize>, watch::Sender<usize>);

impl Drop for Open {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
        self.1.send_modify(|ended| *ended += 1);
    }
}

#[async_trait]
impl Prompter for User {
    fn run(&self) -> Option<PromptRun> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        self.open.fetch_add(1, Ordering::SeqCst);
        Some(PromptRun {
            env: vec![
                ("SSH_ASKPASS".to_owned(), stand_in().into_os_string()),
                ("SSH_ASKPASS_REQUIRE".to_owned(), OsString::from("force")),
                (
                    "STAND_IN_ASKPASS".to_owned(),
                    OsString::from(self.at.to_string()),
                ),
            ],
            ends: Box::new(Open(Arc::clone(&self.open), self.ended.clone())),
        })
    }

    fn waited(&self) -> Duration {
        *self.waited.lock().unwrap()
    }

    async fn given_up(&self) {
        let mut gone = self.gone.subscribe();
        let _ = gone.wait_for(|gone| *gone).await;
    }
}

/// A machine that has a server, and lets nobody in who does not answer.
struct Machine {
    folder: Folder,
    listener: Listener,
}

impl Machine {
    async fn new(name: &str, asks: Value) -> Self {
        let folder = Folder::new(name);
        let (address, listener) = door().await;
        let script = json!({
            "asks": asks,
            "status": r#"{"endpoint":"unix:///run/rex/server.sock"}"#,
            "target": address.to_string(),
            "cut": folder.file("cut"),
            "alive": folder.file("alive"),
            "log": folder.file("log"),
        });
        std::fs::write(folder.file("script"), script.to_string()).unwrap();
        Self { folder, listener }
    }

    fn backend(&self, prompter: Option<Arc<dyn Prompter>>) -> Arc<RexBackend> {
        let ssh = SshOptions {
            client: Some(stand_in()),
            args: vec![
                "-F".to_owned(),
                self.folder.file("script").display().to_string(),
            ],
            prompter,
        };
        RexBackend::connect_with(
            common::backend_id(),
            Endpoint::parse("ssh://me@build").unwrap(),
            client(),
            Options::new().ssh(ssh),
        )
    }
}

/// The question of an account with a password.
fn password() -> Value {
    json!([{
        "words": PASSWORD_WORDS,
        "right": PASSWORD,
        "tries": 3,
        "again": "Permission denied, please try again.",
        "failure": DENIED,
    }])
}

async fn link(backend: &RexBackend, wanted: impl Fn(&Link) -> bool) -> Link {
    let mut link = backend.link();
    let reached = tokio::time::timeout(Duration::from_secs(20), link.wait_for(|link| wanted(link)))
        .await
        .expect("the backend never stood so with its server")
        .unwrap();
    reached.clone()
}

fn refused(link: &Link) -> bool {
    matches!(link, Link::Refused { .. })
}

#[tokio::test]
async fn what_the_client_asks_is_answered_and_the_server_is_reached() {
    let mut machine = Machine::new("answered", password()).await;
    let answers: Answers = Box::new(|_, _| Some(PASSWORD.to_owned()));
    let (user, seen) = User::new(Duration::ZERO, answers).await;
    let backend = machine.backend(Some(user));

    let _served = serve(machine.listener.accept().await, World::recorded());
    backend.ready().await.unwrap();
    // Where the server listens is asked in one login, and the way to it is another.
    assert_eq!(seen.asked(), [PASSWORD_WORDS, PASSWORD_WORDS]);
    assert_eq!(
        machine.folder.log(),
        [
            "may ask", "asks", "right", "told", "may ask", "asks", "right", "forwards"
        ]
    );
    // Nothing is asked once the client forwards, and whoever answered is let go of.
    assert_eq!(seen.open.load(Ordering::SeqCst), 0);
    assert!(seen.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn the_client_ends_with_the_backend_whoever_still_holds_something_of_it() {
    let mut machine = Machine::new("ended", password()).await;
    let answers: Answers = Box::new(|_, _| Some(PASSWORD.to_owned()));
    let (user, _seen) = User::new(Duration::ZERO, answers).await;
    let backend = machine.backend(Some(user));
    let _served = serve(machine.listener.accept().await, World::recorded());
    backend.ready().await.unwrap();
    let alive = machine.folder.file("alive");
    until("the client forwards", || alive.exists()).await;
    // The layout of a session on show is held by whoever shows it.
    let layout = backend.layout();
    backend.shutdown();
    let there = async {
        loop {
            let _ = std::fs::remove_file(&alive);
            tokio::time::sleep(Duration::from_millis(400)).await;
            if !alive.exists() {
                return;
            }
        }
    };
    let gone = tokio::time::timeout(Duration::from_secs(10), there).await;
    assert!(gone.is_ok(), "the client goes on");
    drop(layout);
}

#[tokio::test]
async fn the_user_may_mistake_themselves() {
    let mut machine = Machine::new("again", password()).await;
    let answers: Answers =
        Box::new(|nth, _| Some(if nth == 1 { "wrong horse" } else { PASSWORD }.to_owned()));
    let (user, seen) = User::new(Duration::ZERO, answers).await;
    let backend = machine.backend(Some(user));
    let _served = serve(machine.listener.accept().await, World::recorded());
    backend.ready().await.unwrap();
    assert_eq!(seen.asked().len(), 3);
}

#[tokio::test]
async fn a_user_who_is_not_let_in_is_told_so_at_once() {
    let machine = Machine::new("denied", password()).await;
    let answers: Answers = Box::new(|_, _| Some("wrong horse".to_owned()));
    let (user, seen) = User::new(Duration::ZERO, answers).await;
    let backend = machine.backend(Some(user));
    let Link::Refused { code, error } = link(&backend, refused).await else {
        unreachable!();
    };
    assert_eq!(code, "ssh");
    assert!(error.contains("Permission denied"), "{error}");
    assert_eq!(seen.asked().len(), 3);
    assert!(seen.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn giving_up_ends_the_client_before_it_hears_of_it() {
    let machine = Machine::new("given-up", password()).await;
    // The question is never answered.
    let answers: Answers = Box::new(|_, _| None);
    let (user, seen) = User::new(Duration::ZERO, answers).await;
    let backend = machine.backend(Some(user));
    until("the user is asked", || seen.asked().len() == 1).await;
    seen.gone.send_replace(true);
    let Link::Refused { code, .. } = link(&backend, refused).await else {
        unreachable!();
    };
    assert_eq!(code, "given_up");
    // The client was not told that there is no answer, which it would have taken for an
    // empty one and tried.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(machine.folder.log(), ["may ask", "asks"]);
    assert_eq!(seen.open.load(Ordering::SeqCst), 0);
    assert!(seen.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn giving_up_at_the_second_login_is_giving_up() {
    let machine = Machine::new("given-up-later", password()).await;
    // Where the server listens was asked; the way to it asks again.
    let answers: Answers = Box::new(|nth, _| (nth == 1).then(|| PASSWORD.to_owned()));
    let (user, seen) = User::new(Duration::ZERO, answers).await;
    let backend = machine.backend(Some(user));
    until("the user is asked again", || seen.asked().len() == 2).await;
    seen.gone.send_replace(true);
    let Link::Refused { code, .. } = link(&backend, refused).await else {
        unreachable!();
    };
    assert_eq!(code, "given_up");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let first = ["may ask", "asks", "right", "told"];
    assert_eq!(machine.folder.log()[..4], first);
    assert_eq!(machine.folder.log()[4..], ["may ask", "asks"]);
    assert_eq!(seen.open.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_backend_that_is_let_go_of_while_it_asks_leaves_no_question() {
    let machine = Machine::new("let-go", password()).await;
    let answers: Answers = Box::new(|_, _| None);
    let (user, seen) = User::new(Duration::ZERO, answers).await;
    let backend = machine.backend(Some(user));
    until("the user is asked", || seen.asked().len() == 1).await;
    backend.shutdown();
    until("the question is withdrawn", || {
        seen.open.load(Ordering::SeqCst) == 0 && seen.dropped.load(Ordering::SeqCst)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(machine.folder.log(), ["may ask", "asks"]);
}

#[tokio::test]
async fn a_connection_that_is_made_again_asks_nothing() {
    let mut machine = Machine::new("again-quiet", password()).await;
    let answers: Answers = Box::new(|_, _| Some(PASSWORD.to_owned()));
    let (user, seen) = User::new(Duration::ZERO, answers).await;
    let backend = machine.backend(Some(user));
    let served = serve(machine.listener.accept().await, World::recorded());
    backend.ready().await.unwrap();

    // The connection of the client goes, and with it everything it carries.
    std::fs::write(machine.folder.file("cut"), "").unwrap();
    drop(served);
    let Link::Refused { code, error } = link(&backend, refused).await else {
        unreachable!();
    };
    assert_eq!(code, "ssh");
    assert!(error.contains("Permission denied"), "{error}");
    assert_eq!(seen.asked().len(), 2, "nothing more was asked");
    let log = machine.folder.log();
    assert_eq!(log[log.len() - 2..], ["quiet", "refused"]);
}

#[tokio::test]
async fn a_client_nobody_stands_behind_asks_nothing() {
    let machine = Machine::new("nobody", password()).await;
    let backend = machine.backend(None);
    let Link::Refused { code, .. } = link(&backend, refused).await else {
        unreachable!();
    };
    assert_eq!(code, "ssh");
    assert_eq!(machine.folder.log(), ["quiet", "refused"]);
}

#[tokio::test]
async fn each_question_of_a_login_is_put_in_its_turn() {
    let host_key = "The authenticity of host 'build (10.0.0.7)' can't be established.\n\
                    ED25519 key fingerprint is SHA256:abc.\n\
                    Are you sure you want to continue connecting (yes/no/[fingerprint])? ";
    let asks = json!([
        {
            "words": host_key,
            "right": "yes",
            "tries": 1,
            "failure": "Host key verification failed.",
        },
        password()[0],
    ]);
    let mut machine = Machine::new("in-turn", asks).await;
    let answers: Answers = Box::new(|_, words| {
        let machine = words.starts_with("The authenticity");
        Some(if machine { "yes" } else { PASSWORD }.to_owned())
    });
    let (user, seen) = User::new(Duration::from_millis(50), answers).await;
    let backend = machine.backend(Some(user));
    let _served = serve(machine.listener.accept().await, World::recorded());
    backend.ready().await.unwrap();
    // A question of several lines is asked whole.
    assert_eq!(
        seen.asked(),
        [host_key, PASSWORD_WORDS, host_key, PASSWORD_WORDS]
    );
}

/// Names a machine with a Rex server that takes the account's password, as an address.
const LIVE_SSH: &str = "LUME_REX_LIVE_SSH";
/// The SSH client's configuration file, where it is not the user's own.
const LIVE_SSH_CONFIG: &str = "LUME_REX_LIVE_SSH_CONFIG";
/// The password.
const LIVE_SSH_PASSWORD: &str = "LUME_REX_LIVE_SSH_PASSWORD";

#[tokio::test]
#[ignore = "needs a Rex server on a machine that takes a password, named in LUME_REX_LIVE_SSH"]
async fn the_systems_own_client_asks_for_a_password_and_is_answered() {
    let (Ok(address), Ok(password)) = (std::env::var(LIVE_SSH), std::env::var(LIVE_SSH_PASSWORD))
    else {
        eprintln!("skipped: {LIVE_SSH} or {LIVE_SSH_PASSWORD} is not set");
        return;
    };
    let args = std::env::var(LIVE_SSH_CONFIG)
        .map(|config| vec!["-F".to_owned(), config])
        .unwrap_or_default();
    let connect = |prompter: Option<Arc<dyn Prompter>>| {
        let ssh = SshOptions {
            args: args.clone(),
            prompter,
            ..SshOptions::default()
        };
        RexBackend::connect_with(
            common::backend_id(),
            Endpoint::parse(&address).unwrap(),
            client(),
            Options::new().ssh(ssh),
        )
    };
    // The first answer is not right, and each takes a while to type.
    let answers: Answers = Box::new(move |nth, words| {
        Some(if words.starts_with("The authenticity") {
            "yes".to_owned()
        } else if nth <= 2 {
            "not the password".to_owned()
        } else {
            password.clone()
        })
    });
    let (user, seen) = User::new(Duration::from_secs(4), answers).await;
    let backend = connect(Some(user));
    let waiting = tokio::time::timeout(Duration::from_secs(90), backend.ready());
    let version = waiting.await.expect("no answer").expect("not reached");
    println!(
        "asked {} times, reached Rex {}",
        seen.asked().len(),
        version.rex_version
    );
    assert!(
        seen.asked()
            .iter()
            .any(|words| words.ends_with("'s password: "))
    );
    assert_eq!(seen.open.load(Ordering::SeqCst), 0);
    backend.round_trip().await.unwrap();
    backend.shutdown();

    // Giving up ends the client before it tries anything.
    let (user, seen) = User::new(Duration::ZERO, Box::new(|_, _| None)).await;
    let backend = connect(Some(user));
    until("the user is asked", || seen.asked().len() == 1).await;
    seen.gone.send_replace(true);
    let Link::Refused { code, .. } = link(&backend, refused).await else {
        unreachable!();
    };
    assert_eq!(code, "given_up");
}
