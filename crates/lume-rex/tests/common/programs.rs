//! The programs a pretend server runs: the part of its script in a run, played as the
//! script plays it, and the part of the server's that goes with it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::{Asked, Conn};

/// The client that a pretend server's script is.
pub const SCRIPT: &str = "client:script";

/// How much of an output is in one part.
const PART: usize = 32 * 1024;

/// What a program does, which is made of what it was asked to be.
pub type Play = Arc<dyn Fn(&Job) -> Played + Send + Sync>;

/// What a script was told to run.
#[derive(Clone, Debug)]
pub struct Job {
    /// As the client wrote it.
    pub told: Value,
    pub input: Vec<u8>,
}

impl Job {
    pub fn words(&self) -> Vec<String> {
        serde_json::from_value(self.told["argv"].clone()).unwrap()
    }
}

/// What a program did.
#[derive(Clone, Debug, Default)]
pub struct Played {
    pub out: Vec<u8>,
    pub err: Vec<u8>,
    /// What its end says, or that it does not end.
    pub end: Option<Value>,
}

impl Played {
    pub fn code(code: i64) -> Self {
        Self {
            end: Some(json!({ "code": code, "timed_out": false })),
            ..Self::default()
        }
    }

    pub fn says(text: impl Into<Vec<u8>>) -> Self {
        Self {
            out: text.into(),
            ..Self::code(0)
        }
    }

    /// It could not be run, for such a reason.
    pub fn failed(why: &str, folder: bool) -> Self {
        Self {
            end: Some(json!({ "failed": why, "folder": folder })),
            ..Self::default()
        }
    }

    /// It runs on and says no more.
    pub fn stays(text: impl Into<Vec<u8>>) -> Self {
        Self {
            out: text.into(),
            ..Self::default()
        }
    }
}

/// What a script asked last.
#[derive(Clone, Debug)]
enum Step {
    Job,
    In,
    Began,
    /// A part of an output.
    Out(&'static str),
    /// Whether there is room again.
    Room(&'static str),
    /// Whether the client is there, which is only asked again when it says so.
    Alive,
    Done,
}

/// A run as its script has it.
struct Run {
    secret: String,
    session: String,
    block: String,
    job: Option<Job>,
    played: Played,
    /// How much of each output was handed over.
    sent: (usize, usize),
    step: Step,
}

/// The runs there are and were, for the test to look at.
#[derive(Default)]
pub struct Seen {
    /// The blocks made, in order, as the client asked for them.
    pub blocks: Vec<(String, Value)>,
    /// What was run.
    pub jobs: Vec<Job>,
    /// The blocks that were closed by the client.
    pub closed: Vec<String>,
    /// How often a script was told to wait.
    pub held: usize,
    /// The scripts that were refused something, and ended by it.
    pub refused: Vec<Value>,
}

impl Seen {
    /// The secret typed into a block.
    pub fn secret(&self, nth: usize) -> String {
        let typed = &self.blocks[nth].1["options"]["initial_input"];
        typed.as_str().unwrap().trim_end().to_owned()
    }
}

/// The programs of a pretend server. A copy is of the same ones.
#[derive(Clone)]
pub struct Programs {
    play: Play,
    runs: Arc<Mutex<HashMap<String, Run>>>,
    pub seen: Arc<Mutex<Seen>>,
    /// Where the server says its own program finds it.
    pub primary: Option<String>,
    /// Whether scripts ask anything at all.
    pub comes: bool,
    /// Whether a block closes before its making is answered.
    pub closes_early: bool,
    /// What a method is answered instead, and nothing done for it.
    pub answers: Option<(String, Value)>,
}

impl Programs {
    pub fn new(play: impl Fn(&Job) -> Played + Send + Sync + 'static) -> Self {
        Self {
            play: Arc::new(play),
            runs: Arc::default(),
            seen: Arc::default(),
            primary: Some("npipe:////./pipe/pretend".to_owned()),
            comes: true,
            closes_early: false,
            answers: None,
        }
    }

    pub fn seen<T>(&self, look: impl FnOnce(&Seen) -> T) -> T {
        look(&self.seen.lock().unwrap())
    }

    /// Answers a request that is about programs. Says whether it was one.
    pub async fn answer(&self, conn: &mut Conn, asked: &Asked) -> bool {
        if let Some((method, answer)) = &self.answers
            && *method == asked.method
        {
            conn.respond(&asked.id, answer.clone()).await;
            return true;
        }
        match asked.method.as_str() {
            "server.status" => {
                let listener = |endpoint: &str, primary: bool| {
                    let mut listener = json!({ "endpoint": endpoint, "transport": "tcp" });
                    if primary {
                        listener["primary"] = json!(true);
                    }
                    listener
                };
                let mut listeners: Vec<Value> = Vec::new();
                listeners.extend(self.primary.as_deref().map(|at| listener(at, true)));
                listeners.push(listener("127.0.0.1:7297", false));
                conn.respond(&asked.id, json!({ "pid": 1, "listeners": listeners }))
                    .await;
                true
            }
            "session.new_block" => {
                self.make(conn, asked).await;
                true
            }
            "block.close" => self.close(conn, asked).await,
            "client.reply" => {
                conn.respond(&asked.id, json!({})).await;
                self.replied(conn, &asked.payload).await;
                true
            }
            _ => false,
        }
    }

    async fn make(&self, conn: &mut Conn, asked: &Asked) {
        let session = asked.payload["session_id"].as_str().unwrap().to_owned();
        let block = {
            let mut seen = self.seen.lock().unwrap();
            let block = format!("block:run-{}", seen.blocks.len());
            seen.blocks.push((block.clone(), asked.payload.clone()));
            block
        };
        conn.session_event(&session, "block_created", json!({ "block_id": block }))
            .await;
        if self.closes_early {
            conn.session_event(&session, "block_closed", json!({ "block_id": block }))
                .await;
        }
        conn.respond(&asked.id, json!({ "block_id": block, "revision": 7 }))
            .await;
        if self.closes_early || !self.comes {
            return;
        }
        let typed = &asked.payload["options"]["initial_input"];
        let run = Run {
            secret: typed.as_str().unwrap().trim_end().to_owned(),
            session,
            block: block.clone(),
            job: None,
            played: Played::default(),
            sent: (0, 0),
            step: Step::Job,
        };
        self.ask(conn, run, "job", json!({})).await;
    }

    /// Has the client close the block of a run. Says whether it was one of a run.
    async fn close(&self, conn: &mut Conn, asked: &Asked) -> bool {
        let block = asked.payload["block_id"].as_str().unwrap_or_default();
        let known = self.seen(|seen| seen.blocks.iter().any(|(made, _)| made == block));
        if !known {
            return false;
        }
        self.seen.lock().unwrap().closed.push(block.to_owned());
        self.runs
            .lock()
            .unwrap()
            .retain(|_, run| run.block != block);
        let session = asked.payload["session_id"].as_str().unwrap().to_owned();
        conn.session_event(&session, "block_closed", json!({ "block_id": block }))
            .await;
        conn.respond(&asked.id, json!({ "block_id": block, "revision": 7 }))
            .await;
        true
    }

    /// Has a script ask the client something. What it is answered goes to the same run.
    async fn ask(&self, conn: &mut Conn, run: Run, action: &str, mut args: Value) {
        args["secret"] = json!(run.secret);
        let call_id = format!("call-{}", conn.seq + 1);
        self.runs.lock().unwrap().insert(call_id.clone(), run);
        conn.event(
            "client_perform_requested",
            request(&call_id, SCRIPT, &format!("lume.run.{action}"), args),
        )
        .await;
    }

    /// The client answered a script, which goes on with that.
    async fn replied(&self, conn: &mut Conn, reply: &Value) {
        let call_id = reply["call_id"].as_str().unwrap_or_default();
        let Some(mut run) = self.runs.lock().unwrap().remove(call_id) else {
            return;
        };
        if let Some(error) = reply.get("error").filter(|error| !error.is_null()) {
            self.seen.lock().unwrap().refused.push(error.clone());
            self.gone(conn, &run).await;
            return;
        }
        let result = &reply["result"];
        match run.step.clone() {
            Step::Job => {
                run.job = Some(Job {
                    told: result.clone(),
                    input: Vec::new(),
                });
                if result["input"] == true {
                    run.step = Step::In;
                    self.ask(conn, run, "in", json!({})).await;
                } else {
                    self.begin(conn, run).await;
                }
            }
            Step::In => match result["data"].as_array() {
                Some(pieces) => {
                    let job = run.job.as_mut().unwrap();
                    for piece in pieces {
                        job.input.extend(bytes_of(piece.as_str().unwrap()));
                    }
                    self.ask(conn, run, "in", json!({})).await;
                }
                None => self.begin(conn, run).await,
            },
            Step::Out(to) | Step::Room(to) if result["hold"] == true => {
                self.seen.lock().unwrap().held += 1;
                tokio::time::sleep(Duration::from_millis(5)).await;
                run.step = Step::Room(to);
                self.ask(conn, run, "room", json!({ "to": to })).await;
            }
            Step::Began | Step::Out(_) | Step::Room(_) => self.go_on(conn, run).await,
            Step::Alive => {}
            Step::Done => self.gone(conn, &run).await,
        }
    }

    /// The program runs, or could not be.
    async fn begin(&self, conn: &mut Conn, mut run: Run) {
        let job = run.job.clone().unwrap();
        self.seen.lock().unwrap().jobs.push(job.clone());
        run.played = (self.play)(&job);
        let failed = run
            .played
            .end
            .as_ref()
            .is_some_and(|end| end.get("failed").is_some());
        if job.told["stream"] == true && !failed {
            run.step = Step::Began;
            self.ask(conn, run, "began", json!({})).await;
        } else {
            self.go_on(conn, run).await;
        }
    }

    /// What a script does after what it has done.
    async fn go_on(&self, conn: &mut Conn, mut run: Run) {
        let max = run.job.as_ref().unwrap().told["max"]
            .as_u64()
            .and_then(|max| usize::try_from(max).ok())
            .unwrap_or(usize::MAX);
        let part = |bytes: &[u8], sent: usize| {
            let to = bytes.len().min(max).min(sent + PART);
            (sent < to).then(|| text_of(&bytes[sent..to]))
        };
        if let Some(data) = part(&run.played.out, run.sent.0) {
            run.sent.0 += PART;
            run.step = Step::Out("out");
            self.ask(conn, run, "out", json!({ "to": "out", "data": data }))
                .await;
        } else if let Some(data) = part(&run.played.err, run.sent.1) {
            run.sent.1 += PART;
            run.step = Step::Out("err");
            self.ask(conn, run, "out", json!({ "to": "err", "data": data }))
                .await;
        } else if let Some(end) = run.played.end.clone() {
            run.step = Step::Done;
            self.ask(conn, run, "done", end).await;
        } else {
            run.step = Step::Alive;
            self.ask(conn, run, "alive", json!({})).await;
        }
    }

    /// A script ended, and its block with it.
    async fn gone(&self, conn: &mut Conn, run: &Run) {
        conn.session_event(
            &run.session,
            "block_closed",
            json!({ "block_id": run.block }),
        )
        .await;
    }
}

/// What a client is sent when another asks it for something.
pub fn request(call_id: &str, client: &str, action: &str, args: Value) -> Value {
    json!({
        "call_id": call_id, "action": action, "args": args, "origin": "api",
        "requester": {
            "client_id": client,
            "info": { "kind": "cli", "platform": "windows", "name": "rex", "version": "0.1.0" },
            "principal": { "kind": "windows_peer", "transport": "named_pipe", "display": "S-1-5" },
        },
    })
}

/// Bytes as they are sent: each the character of the same number.
pub fn text_of(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| char::from(*byte)).collect()
}

fn bytes_of(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| u8::try_from(u32::from(c)).expect("a byte"))
        .collect()
}
