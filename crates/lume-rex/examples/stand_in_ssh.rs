//! Stands in for the OpenSSH client in tests: it asks what it is told to ask the way the
//! client does, through the program its environment names, and then does what it was run
//! for, without another machine.
//!
//! What it does is in a JSON file, which it is given with `-F` or with
//! `STAND_IN_SSH` in its environment. What happens to it is written to the file's `log`,
//! one line each, without the answers.
//!
//! Run by itself with one word, it is the program that asks: it hands the word to the
//! test that listens at `STAND_IN_ASKPASS`, and prints what comes back.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

use serde::Deserialize;

/// The status the client ends with when it fails.
const FAILED: u8 = 255;
/// The status a shell ends with that has no such command.
const NOT_FOUND: u8 = 127;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Script {
    /// What is asked, in this order, at each run.
    #[serde(default)]
    asks: Vec<Question>,
    /// What the other machine prints when it is asked about its server; without it, it
    /// has no such command.
    #[serde(default)]
    status: Option<String>,
    /// Where the forwarded place leads, on this machine.
    #[serde(default)]
    target: Option<String>,
    /// A file that ends the client when it appears, as a lost connection would.
    #[serde(default)]
    cut: Option<PathBuf>,
    /// A file it goes on writing for as long as it forwards.
    alive: Option<PathBuf>,
    log: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Question {
    words: String,
    right: String,
    tries: u32,
    /// What the client says when no answer was right.
    failure: String,
    /// What it says when one answer was not.
    #[serde(default)]
    again: Option<String>,
}

impl Script {
    fn note(&self, line: &str) {
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log)
            .expect("the log opens");
        writeln!(log, "{line}").expect("the log is written");
    }
}

/// The program that asks: says the words to the test and prints its answer.
fn ask_the_test(words: &str) -> ExitCode {
    let Ok(address) = std::env::var("STAND_IN_ASKPASS") else {
        return ExitCode::FAILURE;
    };
    let mut answer = String::new();
    let asked = TcpStream::connect(address).and_then(|mut test| {
        test.write_all(words.as_bytes())?;
        test.shutdown(Shutdown::Write)?;
        test.read_to_string(&mut answer)
    });
    match (asked, answer.strip_prefix('+')) {
        (Ok(_), Some(answer)) => {
            println!("{answer}");
            ExitCode::SUCCESS
        }
        _ => ExitCode::FAILURE,
    }
}

/// Asks as the client does. A program that does not answer is taken for an empty answer,
/// as the client takes it.
fn ask(words: &str) -> String {
    let forced = std::env::var("SSH_ASKPASS_REQUIRE").is_ok_and(|require| require == "force");
    let Some(program) = std::env::var_os("SSH_ASKPASS").filter(|_| forced) else {
        return String::new();
    };
    let asked = Command::new(program)
        .arg(words)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output();
    match asked {
        Ok(asked) if asked.status.success() => String::from_utf8_lossy(&asked.stdout)
            .trim_end_matches(['\r', '\n'])
            .to_owned(),
        _ => String::new(),
    }
}

/// Asks everything. `false` when an answer was not right, which has been said.
fn signed_in(script: &Script, batch: bool) -> bool {
    for question in &script.asks {
        let mut right = false;
        for _ in 0..if batch { 0 } else { question.tries } {
            script.note("asks");
            let answer = ask(&question.words);
            right = answer == question.right;
            script.note(match (right, answer.is_empty()) {
                (true, _) => "right",
                (false, true) => "empty",
                (false, false) => "wrong",
            });
            if right {
                break;
            }
            if let Some(again) = &question.again {
                eprintln!("{again}");
            }
        }
        if !right {
            eprintln!("{}", question.failure);
            return false;
        }
    }
    true
}

/// Carries bytes between two streams until either ends.
fn carry<A, B>(near: A, far: B)
where
    A: Both,
    B: Both,
{
    let (mut near_in, mut far_out) = (near.again(), far.again());
    let (mut far_in, mut near_out) = (far, near);
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut near_in, &mut far_out);
        far_out.done();
    });
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut far_in, &mut near_out);
        near_out.done();
    });
}

/// A stream whose two sides can be held apart.
trait Both: Read + Write + Send + 'static {
    fn again(&self) -> Self;
    fn done(&self);
}

impl Both for TcpStream {
    fn again(&self) -> Self {
        self.try_clone().expect("a stream is held twice")
    }

    fn done(&self) {
        let _ = self.shutdown(Shutdown::Both);
    }
}

#[cfg(unix)]
impl Both for std::os::unix::net::UnixStream {
    fn again(&self) -> Self {
        self.try_clone().expect("a stream is held twice")
    }

    fn done(&self) {
        let _ = self.shutdown(Shutdown::Both);
    }
}

/// Forwards a place on this machine, written `PLACE:SOCKET`, for as long as it runs.
fn forward(script: &Script, what: &str) -> ExitCode {
    let target = script.target.clone().expect("a target to forward to");
    let place = what.split(":/").next().expect("a place to forward from");
    script.note("forwards");
    let reach = move || TcpStream::connect(&target);
    #[cfg(unix)]
    if what.starts_with('/') {
        let place = what.rsplit_once(":/").expect("a place to forward from").0;
        let listener = std::os::unix::net::UnixListener::bind(place).expect("the place is free");
        std::thread::spawn(move || {
            for near in listener.incoming().flatten() {
                if let Ok(far) = reach() {
                    carry(near, far);
                }
            }
        });
        return last(script);
    }
    let Ok(listener) = TcpListener::bind(place) else {
        eprintln!("bind [{place}]: Address already in use");
        return ExitCode::from(FAILED);
    };
    std::thread::spawn(move || {
        for near in listener.incoming().flatten() {
            if let Ok(far) = reach() {
                carry(near, far);
            }
        }
    });
    last(script)
}

/// Runs until it is ended, or its connection is.
fn last(script: &Script) -> ExitCode {
    loop {
        std::thread::sleep(std::time::Duration::from_millis(50));
        if let Some(alive) = &script.alive {
            let _ = std::fs::write(alive, std::process::id().to_string());
        }
        // Without the folder it belongs to nobody waits for it any more.
        if !script.log.exists() || script.cut.as_ref().is_some_and(|cut| cut.exists()) {
            eprintln!("client_loop: send disconnect: Connection reset");
            return ExitCode::from(FAILED);
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The client is always told its destination after this.
    let Some(end) = args.iter().position(|arg| arg == "--") else {
        return ask_the_test(args.first().map_or("", String::as_str));
    };
    let after = |option: &str| {
        let at = args[..end].iter().position(|arg| arg == option)?;
        args.get(at + 1).cloned()
    };
    let file = after("-F")
        .or_else(|| std::env::var("STAND_IN_SSH").ok())
        .expect("what it stands in for");
    let text = std::fs::read_to_string(file).expect("the script is read");
    let script: Script = serde_json::from_str(&text).expect("the script is understood");

    let batch = args[..end].iter().any(|arg| arg == "BatchMode=yes");
    script.note(if batch { "quiet" } else { "may ask" });
    if !signed_in(&script, batch) {
        script.note("refused");
        return ExitCode::from(FAILED);
    }
    if let Some(what) = after("-L") {
        return forward(&script, &what);
    }
    let said = args
        .get(end + 2)
        .is_some_and(|what| what.contains("status"));
    match script.status.as_ref().filter(|_| said) {
        Some(status) => {
            println!("{status}");
            script.note("told");
            ExitCode::SUCCESS
        }
        None if said => ExitCode::from(NOT_FOUND),
        None => ExitCode::SUCCESS,
    }
}
