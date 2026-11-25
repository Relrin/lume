//! Programs run on a server's machine.
//!
//! A server has no request that runs a program, but the `rex` program, which is there
//! wherever a server is, runs scripts that can. A run is a terminal block in no window
//! with that program in it, and a script (`runner.lua`) that asks this client, over the
//! control connection there is, what to run and what it reads, and hands it what the
//! program wrote and how it ended. Nothing of it goes through the block's terminal but
//! one secret, typed into it: whoever asks without the secret is told nothing.
//!
//! The script's program is told which server it belongs to and never to start one: in a
//! block it would otherwise go to the server at the usual address, which may be another.
//!
//! A run ends with the connection it was asked for over, with the session its block is
//! in, when its time is up, and when nobody waits for it any more. Its block is closed
//! then, which ends the program with all it started; and a script that is refused, or
//! not answered, ends by itself.
//!
//! What a server says of a run is not trusted: it is taken within the limits of what was
//! asked, from the one that showed the secret first, and what does not fit ends the run.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::{OnceCell, OwnedSemaphorePermit, Semaphore, mpsc, watch};
use tokio::time::Instant;

use crate::api::{BlockMade, ServerStatus, Welcome};
use crate::backend::{RexBackend, Shared};
use crate::control::Control;
use crate::error::RexError;
use crate::text::{cut, plain};
use crate::tuning::{
    RUN_CAME_WAIT, RUN_GRACE, RUN_HOLD_BYTES, RUN_INPUT_MAX_BYTES, RUN_JOB_MAX_BYTES, RUN_LOOK,
    RUN_PART_BYTES, RUN_PART_SHARE, RUN_SEND_MOST, RUN_SILENT, RUN_STREAMS_AT_ONCE, RUNS_AT_ONCE,
    TEXT_MAX_BYTES,
};

/// What the script of a run is, with the place for this client's id.
const SCRIPT: &str = include_str!("runner.lua");
const CLIENT: &str = "@CLIENT@";

/// What the requests of a run's script begin with.
const ACTION: &str = "lume.run.";

/// What a run's block is called, for whoever lists a session's blocks.
const LABEL: &str = "Lume";

/// The flavor of a block that is a terminal with a program in it.
const FLAVOR: &str = "com.superlogical.terminal.shell";

/// How much room a message leaves around what it carries.
const MESSAGE_ROOM: usize = 4096;

/// How long a piece of a program's input is, in bytes: the script takes the bytes back
/// out of each piece by itself.
const PIECE: usize = 128;

/// How many blocks that closed are kept in mind, for one whose making was not answered
/// yet.
const CLOSED_KEPT: usize = 64;

/// A program to be run on a server's machine.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunAsked {
    /// Its name, which is looked up on the path over there.
    pub program: String,
    pub args: Vec<String>,
    /// The folder it runs in; the user's home folder there when `None`.
    pub folder: Option<String>,
    pub env: Vec<(String, String)>,
}

/// How a program is run to its end.
#[derive(Clone, Debug)]
pub struct RunLimits {
    /// What it reads, all of it.
    pub stdin: Vec<u8>,
    pub timeout: Duration,
    /// How many bytes of its output and of its error output are kept, each.
    pub max_output: usize,
}

/// What a program did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ran {
    /// Its exit code; `None` when it was stopped.
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Whether it ran past its time and was stopped.
    pub timed_out: bool,
    /// Whether there was more output than is kept.
    pub truncated: bool,
}

/// Why a program was not run, or not to its end.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RunError {
    #[error("the server is not connected, or no session of it is open here")]
    NotConnected,
    /// The server cannot run programs for this client, and trying again would not help.
    #[error("programs cannot be run on this server: {0}")]
    CannotRun(String),
    #[error("the folder is not there")]
    NoFolder,
    #[error("the program was not found")]
    NoProgram,
    #[error("{0}")]
    TooLarge(&'static str),
    #[error("too many programs whose output is read are running on the server")]
    Busy,
    #[error("the program could not be run: {0}")]
    Failed(String),
}

impl From<RexError> for RunError {
    fn from(error: RexError) -> Self {
        match error {
            RexError::Disconnected | RexError::Closed { .. } => RunError::NotConnected,
            error => RunError::Failed(error.to_string()),
        }
    }
}

fn failed(why: &str) -> RunError {
    RunError::Failed(why.to_owned())
}

/// What lets only so many programs run on a server at once.
pub(crate) struct Turns {
    runs: Arc<Semaphore>,
    streams: Arc<Semaphore>,
}

impl Default for Turns {
    fn default() -> Self {
        Self {
            runs: Arc::new(Semaphore::new(RUNS_AT_ONCE)),
            streams: Arc::new(Semaphore::new(RUN_STREAMS_AT_ONCE)),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Output on its way to whoever reads it.
#[derive(Default)]
struct Pipe {
    held: Mutex<Piped>,
    ready: Condvar,
}

#[derive(Default)]
struct Piped {
    bytes: VecDeque<u8>,
    /// Whether no more will come.
    ended: bool,
    /// Whether nobody reads it any more.
    left: bool,
}

impl Pipe {
    fn end(&self) {
        lock(&self.held).ended = true;
        self.ready.notify_all();
    }

    /// Whether there is more than may wait to be read.
    fn full(&self) -> bool {
        lock(&self.held).bytes.len() > RUN_HOLD_BYTES
    }
}

/// The output of a program that runs on a server's machine, as it comes. Reading it
/// waits, so it belongs on a thread that may.
pub struct RunReader(Arc<Pipe>);

impl Read for RunReader {
    fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
        if into.is_empty() {
            return Ok(0);
        }
        let mut held = lock(&self.0.held);
        while held.bytes.is_empty() && !held.ended {
            held = self
                .0
                .ready
                .wait(held)
                .unwrap_or_else(PoisonError::into_inner);
        }
        held.bytes.read(into)
    }
}

impl Drop for RunReader {
    fn drop(&mut self) {
        let mut held = lock(&self.0.held);
        held.left = true;
        held.bytes = VecDeque::new();
    }
}

/// Where a program's output goes.
enum Output {
    /// It is kept, up to so much of each and one byte that tells there was more.
    Kept {
        max: usize,
        out: Vec<u8>,
        err: Vec<u8>,
    },
    /// It is read as it comes.
    Read { out: Arc<Pipe>, err: Arc<Pipe> },
}

/// How a program ended, by its script's word.
struct End {
    code: Option<i32>,
    timed_out: bool,
    out: Vec<u8>,
    err: Vec<u8>,
}

/// What whoever waits for a run is told, if it is not over with no such word.
enum Said {
    /// The program runs.
    Began,
    /// How it ended, by its script's word.
    Ended(Result<End, RunError>),
}

/// A run that was asked for and has not ended.
struct Run {
    /// What the script is told; given once.
    job: Option<Value>,
    input: Vec<u8>,
    given: usize,
    output: Output,
    /// Who showed the secret first, as the server knows them.
    script: Option<String>,
    block: Option<String>,
    /// When the script last asked something, or the run was asked for, and when output
    /// that is kept began to arrive.
    heard: Instant,
    sending: Option<Instant>,
    said: mpsc::UnboundedSender<Result<Said, RunError>>,
}

/// How a run stands.
struct Stands {
    /// Whether its script has asked what it is to run.
    came: bool,
    heard: Instant,
    sending: Option<Instant>,
}

/// Why a request is not done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refusal {
    /// It is for something this client does not do.
    Unknown,
    /// Nobody waits for such a run, or it was asked for what it was not to be.
    NoRun,
}

impl Refusal {
    fn told(self) -> Value {
        match self {
            Refusal::Unknown => json!({ "code": "not_supported", "message": "no such action" }),
            Refusal::NoRun => json!({ "code": "refused", "message": "no such run" }),
        }
    }
}

/// The programs run over one connection.
pub(crate) struct Runs {
    /// What the server knows this client by, where it can stand in a script.
    client: Option<String>,
    max_message: usize,
    open: Mutex<Open>,
    /// Where the server's own program finds it, once that was asked.
    primary: OnceCell<Option<String>>,
}

#[derive(Default)]
struct Open {
    /// Whether the connection has ended.
    over: bool,
    /// The runs by their secrets.
    runs: HashMap<String, Run>,
    /// The blocks that closed last.
    closed: VecDeque<String>,
}

impl Runs {
    pub(crate) fn new(welcome: &Welcome) -> Self {
        let sound = |id: &String| {
            !id.is_empty()
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '_' | '-'))
        };
        Self {
            client: welcome.client_id.clone().filter(sound),
            max_message: welcome.max_message_bytes,
            open: Mutex::default(),
            primary: OnceCell::new(),
        }
    }

    /// How much of an input or an output one message holds.
    fn part(&self) -> usize {
        RUN_PART_BYTES.min(self.max_message / RUN_PART_SHARE)
    }

    /// Ends every run: the connection has ended.
    fn end_all(&self) {
        let mut open = lock(&self.open);
        open.over = true;
        for (_, run) in open.runs.drain() {
            let _ = run.said.send(Err(RunError::NotConnected));
        }
    }

    /// A block closed: if a program was run in it, that is over.
    pub(crate) fn block_closed(&self, block_id: &str) {
        let mut open = lock(&self.open);
        let of_it = open
            .runs
            .iter()
            .find(|(_, run)| run.block.as_deref() == Some(block_id))
            .map(|(secret, _)| secret.clone());
        if let Some(run) = of_it.and_then(|secret| open.runs.remove(&secret)) {
            let _ = run.said.send(Err(ended_silently()));
            return;
        }
        if open.closed.len() == CLOSED_KEPT {
            open.closed.pop_front();
        }
        open.closed.push_back(block_id.to_owned());
    }

    /// Notes the block that a run was given. Says whether the run is still wanted.
    fn made(&self, secret: &str, block_id: &str) -> bool {
        let mut open = lock(&self.open);
        let closed = open.closed.iter().any(|closed| closed == block_id);
        match open.runs.get_mut(secret) {
            Some(_) if closed => {
                if let Some(run) = open.runs.remove(secret) {
                    let _ = run.said.send(Err(ended_silently()));
                }
                true
            }
            Some(run) => {
                run.block = Some(block_id.to_owned());
                true
            }
            // It may have come and gone before its block was told of.
            None => closed,
        }
    }

    /// Ends a run that its server does not go along with.
    fn break_off(&self, secret: &str, error: RunError) {
        if let Some(run) = lock(&self.open).runs.remove(secret) {
            let _ = run.said.send(Err(error));
        }
    }

    fn stands(&self, secret: &str) -> Option<Stands> {
        let open = lock(&self.open);
        let run = open.runs.get(secret)?;
        Some(Stands {
            came: run.job.is_none(),
            heard: run.heard,
            sending: run.sending,
        })
    }

    /// Does what the server asks of this client, and gives what it is answered.
    fn perform(&self, asked: &Value) -> Result<Value, Refusal> {
        let text = |value: &Value, field: &str| {
            value
                .get(field)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty() && text.len() <= TEXT_MAX_BYTES)
                .map(str::to_owned)
        };
        let action = text(asked, "action").ok_or(Refusal::Unknown)?;
        let action = action.strip_prefix(ACTION).ok_or(Refusal::Unknown)?;
        let args = asked.get("args").ok_or(Refusal::NoRun)?;
        let secret = text(args, "secret").ok_or(Refusal::NoRun)?;
        let script = asked
            .get("requester")
            .and_then(|requester| text(requester, "client_id"))
            .ok_or(Refusal::NoRun)?;
        let mut open = lock(&self.open);
        let run = open.runs.get_mut(&secret).ok_or(Refusal::NoRun)?;
        if run.script.get_or_insert_with(|| script.clone()) != &script {
            return Err(Refusal::NoRun);
        }
        run.heard = Instant::now();
        let done = run.perform(action, args, self.part());
        match done {
            Ok(Done::Goes(answer)) => Ok(answer),
            Ok(Done::Over(end)) => {
                if let Some(run) = open.runs.remove(&secret) {
                    run.over(end);
                }
                Ok(Value::Null)
            }
            Err(Refused::Unknown) => Err(Refusal::Unknown),
            // Another than the run's own script would not get this far.
            Err(Refused::Broken(why)) => {
                if let Some(run) = open.runs.remove(&secret) {
                    let _ = run.said.send(Err(failed(why)));
                }
                Err(Refusal::NoRun)
            }
            Err(Refused::Left) => Err(Refusal::NoRun),
        }
    }
}

fn ended_silently() -> RunError {
    failed("its block closed before it had ended")
}

/// What came of a script's request.
enum Done {
    Goes(Value),
    /// The run is over.
    Over(Result<(Option<i32>, bool), RunError>),
}

enum Refused {
    /// There is no such request.
    Unknown,
    /// It is not one that a script makes, or not now: the run is over.
    Broken(&'static str),
    /// Nobody reads the output any more.
    Left,
}

/// The bytes that a text stands for, each a character of the same number.
fn bytes_of(text: &str) -> Option<Vec<u8>> {
    text.chars()
        .map(|c| u8::try_from(u32::from(c)).ok())
        .collect()
}

/// A part of a program's input as the script takes it: short pieces, each byte the
/// character of the same number.
fn pieces_of(bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks(PIECE)
        .map(|piece| piece.iter().map(|byte| char::from(*byte)).collect())
        .collect()
}

impl Run {
    fn perform(&mut self, action: &str, args: &Value, part: usize) -> Result<Done, Refused> {
        if action == "job" {
            let job = self.job.take().ok_or(Refused::Broken("it asked twice"))?;
            return Ok(Done::Goes(job));
        }
        if self.job.is_some() {
            return Err(Refused::Broken("it did not ask what it is to run"));
        }
        match action {
            "in" => {
                let from = self.given;
                let to = self.input.len().min(from + part);
                if from == to {
                    self.input = Vec::new();
                    self.given = 0;
                    return Ok(Done::Goes(Value::Null));
                }
                self.given = to;
                Ok(Done::Goes(
                    json!({ "data": pieces_of(&self.input[from..to]) }),
                ))
            }
            "began" => match self.output {
                Output::Read { .. } => {
                    let _ = self.said.send(Ok(Said::Began));
                    Ok(Done::Goes(Value::Null))
                }
                Output::Kept { .. } => Err(Refused::Broken("it was not to say that it began")),
            },
            "out" => self.take(args, part),
            "room" => {
                let full = match &self.output {
                    Output::Read { out, err } => match args["to"].as_str() {
                        Some("out") => out.full(),
                        Some("err") => err.full(),
                        _ => return Err(Refused::Broken("it named an output there is not")),
                    },
                    Output::Kept { .. } => false,
                };
                Ok(Done::Goes(json!({ "hold": full })))
            }
            "alive" => Ok(Done::Goes(Value::Null)),
            "done" => Ok(Done::Over(ended(args))),
            _ => Err(Refused::Unknown),
        }
    }

    /// Takes in a part of the program's output.
    fn take(&mut self, args: &Value, part: usize) -> Result<Done, Refused> {
        let bytes = args["data"]
            .as_str()
            .and_then(bytes_of)
            .filter(|bytes| bytes.len() <= part)
            .ok_or(Refused::Broken("its output is not as it was asked for"))?;
        let no_such = Refused::Broken("it named an output there is not");
        match &mut self.output {
            Output::Kept { max, out, err } => {
                let kept = match args["to"].as_str() {
                    Some("out") => out,
                    Some("err") => err,
                    _ => return Err(no_such),
                };
                if bytes.is_empty() || kept.len() + bytes.len() > max.saturating_add(1) {
                    return Err(Refused::Broken("its output is not as it was asked for"));
                }
                kept.extend(bytes);
                self.sending.get_or_insert_with(Instant::now);
                Ok(Done::Goes(Value::Null))
            }
            Output::Read { out, err } => {
                let pipe = match args["to"].as_str() {
                    Some("out") => out,
                    Some("err") => err,
                    _ => return Err(no_such),
                };
                let mut held = lock(&pipe.held);
                if held.left {
                    return Err(Refused::Left);
                }
                if held.bytes.len() > RUN_HOLD_BYTES {
                    return Err(Refused::Broken("it did not wait for its output to be read"));
                }
                held.bytes.extend(bytes);
                let hold = held.bytes.len() > RUN_HOLD_BYTES;
                drop(held);
                pipe.ready.notify_all();
                Ok(Done::Goes(json!({ "hold": hold })))
            }
        }
    }

    /// Hands what the run came to to whoever waits for it.
    fn over(self, end: Result<(Option<i32>, bool), RunError>) {
        let (out, err) = match self.output {
            Output::Kept { out, err, .. } => (out, err),
            Output::Read { .. } => (Vec::new(), Vec::new()),
        };
        let end = end.map(|(code, timed_out)| End {
            code,
            timed_out,
            out,
            err,
        });
        let _ = self.said.send(Ok(Said::Ended(end)));
    }
}

/// How a program ended, by its script's word: its exit code, and whether its time was up.
fn ended(args: &Value) -> Result<(Option<i32>, bool), RunError> {
    if let Some(why) = args.get("failed").filter(|why| !why.is_null()) {
        let why = plain(why.as_str().unwrap_or_default());
        if args["folder"] == true {
            return Err(RunError::NoFolder);
        }
        let lower = why.to_lowercase();
        let missing = ["not found", "no such file", "cannot find"];
        return Err(if missing.iter().any(|words| lower.contains(words)) {
            RunError::NoProgram
        } else {
            RunError::Failed(why)
        });
    }
    let timed_out = args["timed_out"] == true;
    let stopped = timed_out || args.get("signal").is_some_and(|signal| !signal.is_null());
    let code = args["code"]
        .as_i64()
        // Windows' own codes have no sign, and are known here as the same 32 bits.
        .and_then(|code| {
            i32::try_from(code)
                .ok()
                .or_else(|| u32::try_from(code).ok().map(u32::cast_signed))
        })
        .filter(|_| !stopped);
    Ok((code, timed_out))
}

/// Answers what a server asks of this client, until the connection ends. It is not what
/// keeps the connection.
pub(crate) async fn serve(
    control: Weak<Control>,
    runs: Arc<Runs>,
    mut performs: mpsc::Receiver<Value>,
) {
    while let Some(asked) = performs.recv().await {
        let Some(call_id) = asked["call_id"]
            .as_str()
            .filter(|id| id.len() <= TEXT_MAX_BYTES)
        else {
            continue;
        };
        let reply = match runs.perform(&asked) {
            Ok(result) => json!({ "call_id": call_id, "result": result }),
            Err(refusal) => json!({ "call_id": call_id, "error": refusal.told() }),
        };
        let Some(control) = control.upgrade() else {
            break;
        };
        control.post("client.reply", reply).await;
    }
    runs.end_all();
}

/// Closes blocks that were made to run programs in.
pub(crate) async fn close_blocks(control: Arc<Control>, blocks: HashMap<String, String>) {
    for (block_id, session_id) in blocks {
        let block = json!({ "session_id": session_id, "block_id": block_id });
        // Most are gone, with their sessions or by themselves.
        let _ = control.call("block.close", block).await;
    }
}

/// The block of a run, once it was made, with its session.
type Made = Arc<OnceLock<(String, String)>>;

/// A run as whoever waits for it holds it. Letting go of it ends the run.
struct Held {
    control: Arc<Control>,
    runs: Arc<Runs>,
    secret: String,
    said: mpsc::UnboundedReceiver<Result<Said, RunError>>,
    made: Made,
    /// Whether the script said how it ended. Its block goes by itself then.
    over: bool,
    /// When the block was asked for, and how long the program may run after it came.
    asked: Instant,
    timeout: Option<Duration>,
}

impl Drop for Held {
    fn drop(&mut self) {
        lock(&self.runs.open).runs.remove(&self.secret);
        if self.over {
            return;
        }
        // One that is not made yet is closed by whoever makes it.
        if let (Some(block), Ok(runtime)) = (
            self.made.get().cloned(),
            tokio::runtime::Handle::try_current(),
        ) {
            let control = Arc::clone(&self.control);
            runtime.spawn(close_blocks(control, HashMap::from([block])));
        }
    }
}

impl Held {
    /// What is said of the run next, if it is said in time. A script that does not come,
    /// or falls silent, and a program that runs on past its time, end the run.
    async fn next(&mut self) -> Result<Said, RunError> {
        loop {
            match tokio::time::timeout(RUN_LOOK, self.said.recv()).await {
                Ok(Some(Err(error))) => return Err(error),
                Ok(Some(Ok(said))) => {
                    self.over |= matches!(said, Said::Ended(_));
                    return Ok(said);
                }
                Ok(None) => return Err(RunError::NotConnected),
                Err(_) => {}
            }
            let Some(Stands {
                came,
                heard,
                sending,
            }) = self.runs.stands(&self.secret)
            else {
                // It ended just now, and that is said.
                continue;
            };
            if !came {
                if heard.elapsed() > RUN_CAME_WAIT {
                    return Err(failed("the server did not run what it was given"));
                }
                continue;
            }
            if heard.elapsed() > RUN_SILENT {
                return Err(failed("the server does not say what became of it"));
            }
            let Some(timeout) = self.timeout else {
                continue;
            };
            let late = match sending {
                Some(since) => since.elapsed() > RUN_SEND_MOST,
                None => self.asked.elapsed() > RUN_CAME_WAIT + timeout + RUN_GRACE,
            };
            if late {
                return Err(failed("the server did not end it when its time was up"));
            }
        }
    }
}

/// A program that was started on a server's machine.
pub struct RunStarted {
    pub stdout: RunReader,
    pub stderr: RunReader,
    pub run: StartedRun,
}

/// A started program as it is waited for and ended from here. Letting go of it leaves
/// the program running.
pub struct StartedRun {
    /// Once it has ended, its exit code if it ended by itself.
    ended: watch::Receiver<Option<Option<i32>>>,
    kill: watch::Sender<bool>,
}

impl StartedRun {
    /// `None` while the program runs; once it has ended, its exit code, which is `None`
    /// when it was stopped or nothing is known of it any more.
    pub fn try_wait(&self) -> Option<Option<i32>> {
        *self.ended.borrow()
    }

    /// Ends the program, with everything it started.
    pub fn kill(&self) {
        self.kill.send_replace(true);
    }
}

/// Follows a started program to its end.
async fn follow(
    mut held: Held,
    pipes: [Arc<Pipe>; 2],
    mut killed: watch::Receiver<bool>,
    ended: watch::Sender<Option<Option<i32>>>,
    turn: OwnedSemaphorePermit,
) {
    let code = tokio::select! {
        said = async {
            loop {
                match held.next().await {
                    Ok(Said::Ended(end)) => return end,
                    Ok(Said::Began) => {}
                    Err(error) => return Err(error),
                }
            }
        } => said.ok().and_then(|end| end.code),
        // Nobody who could ask for that is not somebody who did.
        () = async {
            if killed.wait_for(|killed| *killed).await.is_err() {
                std::future::pending::<()>().await;
            }
        } => None,
    };
    drop(held);
    for pipe in pipes {
        pipe.end();
    }
    ended.send_replace(Some(code));
    drop(turn);
}

impl RexBackend {
    /// The session a run's block is made in: the one on show, else one that is open.
    fn run_session(&self) -> Result<String, RunError> {
        let opened = self.shared().opened();
        self.shown_session()
            .filter(|shown| opened.contains(shown))
            .or_else(|| opened.into_iter().next())
            .ok_or(RunError::NotConnected)
    }

    /// Asks the server for a run. `timeout` is how long the program may run, where it has
    /// an end.
    async fn begin(
        &self,
        asked: &RunAsked,
        input: Vec<u8>,
        timeout: Option<Duration>,
        output: Output,
    ) -> Result<Held, RunError> {
        let (control, runs) = self.shared().line()?;
        let session_id = self.run_session()?;
        let client = runs
            .client
            .as_deref()
            .ok_or_else(|| RunError::CannotRun("it gave this client no id".to_owned()))?;
        if input.len() > RUN_INPUT_MAX_BYTES {
            return Err(RunError::TooLarge(
                "what the program is to read is longer than can be sent to a server",
            ));
        }
        let (stream, max) = match &output {
            Output::Kept { max, .. } => (false, Some(max.saturating_add(1))),
            Output::Read { .. } => (true, None),
        };
        let mut argv = vec![asked.program.as_str()];
        argv.extend(asked.args.iter().map(String::as_str));
        let env: HashMap<&str, &str> = asked
            .env
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        let job = json!({
            "argv": argv,
            "folder": asked.folder,
            "env": env,
            "timeout": timeout.map(|timeout| timeout.as_secs_f64()),
            "input": !input.is_empty(),
            "stream": stream,
            "max": max,
            "part": runs.part(),
        });
        let most = RUN_JOB_MAX_BYTES.min(runs.max_message.saturating_sub(MESSAGE_ROOM));
        if job.to_string().len() > most {
            return Err(RunError::TooLarge(
                "the arguments are longer than can be sent to a server",
            ));
        }
        let primary = runs.primary(&control).await?;
        let mut secret = [0u8; 16];
        getrandom::fill(&mut secret).map_err(|error| RunError::Failed(error.to_string()))?;
        let secret = data_encoding::HEXLOWER.encode(&secret);
        let end = if self.shared().on_windows() {
            "\r"
        } else {
            "\n"
        };
        let block = json!({
            "session_id": session_id,
            "flavor": FLAVOR,
            "label": LABEL,
            "options": {
                "shell": "none",
                "command": [
                    "rex", "--autostart=false", "-S", primary,
                    "do", "-e", SCRIPT.replace(CLIENT, client),
                ],
                "initial_input": format!("{secret}{end}"),
                // It goes with its program, however that ends.
                "exit": { "on_completion": true, "quick_exit_threshold_ms": 0 },
            },
        });
        let (tell, said) = mpsc::unbounded_channel();
        let now = Instant::now();
        {
            let mut open = lock(&runs.open);
            if open.over {
                return Err(RunError::NotConnected);
            }
            open.runs.insert(
                secret.clone(),
                Run {
                    job: Some(job),
                    input,
                    given: 0,
                    output,
                    script: None,
                    block: None,
                    heard: now,
                    sending: None,
                    said: tell,
                },
            );
        }
        let held = Held {
            control: Arc::clone(&control),
            runs: Arc::clone(&runs),
            secret: secret.clone(),
            said,
            made: Made::default(),
            over: false,
            asked: now,
            timeout,
        };
        // Beside whoever waits: a block that was made is known even when they went.
        tokio::spawn(make(
            Arc::clone(self.shared()),
            control,
            runs,
            secret,
            (session_id, Arc::clone(&held.made)),
            block,
        ));
        Ok(held)
    }

    /// Runs a program on the server's machine to its end, or until its time is up, and
    /// gives what it wrote. Whoever stops waiting for it ends it.
    pub async fn run(&self, asked: &RunAsked, limits: RunLimits) -> Result<Ran, RunError> {
        let _turn = Arc::clone(&self.turns.runs)
            .acquire_owned()
            .await
            .map_err(|_| RunError::NotConnected)?;
        let max = limits.max_output;
        let output = Output::Kept {
            max,
            out: Vec::new(),
            err: Vec::new(),
        };
        let mut held = self
            .begin(asked, limits.stdin, Some(limits.timeout), output)
            .await?;
        loop {
            if let Said::Ended(end) = held.next().await? {
                let mut end = end?;
                let truncated = end.out.len() > max || end.err.len() > max;
                end.out.truncate(max);
                end.err.truncate(max);
                return Ok(Ran {
                    code: end.code,
                    stdout: end.out,
                    stderr: end.err,
                    timed_out: end.timed_out,
                    truncated,
                });
            }
        }
    }

    /// Starts a program on the server's machine and gives its output as it comes. It
    /// reads nothing.
    pub async fn start(&self, asked: &RunAsked) -> Result<RunStarted, RunError> {
        let turn = Arc::clone(&self.turns.streams)
            .try_acquire_owned()
            .map_err(|_| RunError::Busy)?;
        let (out, err) = (Arc::new(Pipe::default()), Arc::new(Pipe::default()));
        let output = Output::Read {
            out: Arc::clone(&out),
            err: Arc::clone(&err),
        };
        // Made first: whoever gives up on the start leaves nobody to read.
        let (stdout, stderr) = (RunReader(Arc::clone(&out)), RunReader(Arc::clone(&err)));
        let mut held = self.begin(asked, Vec::new(), None, output).await?;
        if let Said::Ended(end) = held.next().await? {
            end?;
            return Err(failed("it ended before it began"));
        }
        let (kill, killed) = watch::channel(false);
        let (tell, ended) = watch::channel(None);
        tokio::spawn(follow(held, [out, err], killed, tell, turn));
        Ok(RunStarted {
            stdout,
            stderr,
            run: StartedRun { ended, kill },
        })
    }
}

impl Runs {
    /// Where the server's own program finds it.
    async fn primary(&self, control: &Control) -> Result<String, RunError> {
        let found = self
            .primary
            .get_or_try_init(|| async {
                let (status, _) = control
                    .call_as::<ServerStatus>("server.status", Value::Null)
                    .await?;
                let primary = status
                    .listeners
                    .into_iter()
                    .find(|listens| listens.primary)
                    .map(|listens| listens.endpoint);
                Ok::<_, RexError>(
                    primary.filter(|at| !at.is_empty() && !at.starts_with('-') && plain(at) == *at),
                )
            })
            .await?;
        found.clone().ok_or_else(|| {
            RunError::CannotRun("it does not say where its own program finds it".to_owned())
        })
    }
}

/// Makes the block of a run. One that nobody wants any more is closed again.
async fn make(
    shared: Arc<Shared>,
    control: Arc<Control>,
    runs: Arc<Runs>,
    secret: String,
    (session_id, known): (String, Made),
    block: Value,
) {
    match control
        .call_as::<BlockMade>("session.new_block", block)
        .await
    {
        Ok((made, _)) => {
            let mut block_id = made.block_id;
            cut(&mut block_id, TEXT_MAX_BYTES);
            shared.note_made(&block_id, &session_id);
            let _ = known.set((block_id.clone(), session_id.clone()));
            if !runs.made(&secret, &block_id) {
                close_blocks(control, HashMap::from([(block_id, session_id)])).await;
            }
        }
        Err(error) => runs.break_off(&secret, error.into()),
    }
}
