//! Programs run on another machine that has a POSIX shell, reached by a program of this
//! one: `wsl.exe` for a Linux distribution, the SSH client for a machine it signs in to.
//!
//! Signing in takes time and may take a person, so one connection is kept open to a
//! small loop in the other side's `sh`. Each run is sent to it as a script with what the
//! program reads after it, by length; runs go on beside each other there, and each is
//! answered whole: how it ended, what it wrote, what it wrote of errors. A program whose
//! output is wanted as it comes has a connection of its own, unless the connection there
//! is took a person to open: then it runs over that one, and its output comes whole, at
//! its end.
//!
//! Nothing of a run is read by a shell that this module does not know: the loop holds no
//! quote, backslash or line end, so whatever shell a user signs in with hands it on as it
//! is, and the scripts are read by `sh` alone, with every word quoted for it.
//!
//! The other side ends every program, with all it started, when it is told to and when
//! the connection ends, whoever ended it. What the other side says is not trusted: it is
//! read within the limits of what was asked, and an answer that does not fit ends the
//! connection.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{ChildStdin, ChildStdout};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use lume_plugin::permissions::check_program;

use crate::process::{self, Child, OUTPUT_GRACE, Program, RunOutput, SpawnOptions, Stop};
use crate::scope::ServiceError;

/// How long the other side may take to say that it is there, and to take in one run.
const GREETING_WAIT: Duration = Duration::from_secs(20);
const TAKEN_WAIT: Duration = Duration::from_secs(60);

/// How much may be written on the output before the loop's own first line: what the
/// other side's start-up files print.
const CHATTER_MAX: usize = 64 * 1024;

/// How long a line of the loop's own may be.
const LINE_MAX: u64 = 1024;

/// How much of what the connection says on its error output is kept.
const SAID_MAX: usize = 4 * 1024;

/// How often a run that is waited for looks whether it is to stop.
const LOOK: Duration = Duration::from_millis(50);

/// How long a program that was told to end may take to be answered for.
const END_WAIT: Duration = Duration::from_secs(5);

/// How long a client whose output ended before it said anything may take to end, so
/// that how it ended can be told.
const ENDED_WAIT: Duration = Duration::from_secs(2);

/// How much of a started program's output is kept when it runs over the one connection,
/// and how long it may run there.
const WHOLE_MAX: usize = 16 * 1024 * 1024;
const WHOLE_TIME: Duration = Duration::from_secs(24 * 60 * 60);

/// The longest script of a run. The arguments are in it.
const SCRIPT_MAX: usize = 1024 * 1024;

/// What the loop's first line begins with, and the first line of a program of its own.
const GREETING: &str = "lume-there";
const STARTED: &str = "lume-started";
const NOT_STARTED: &str = "lume-not-started";

/// How a program is made the leader of a group of its own, so that all it starts can be
/// ended with it: `setsid` where it is, else perl, else not at all.
const LEADER: &str = "if command -v setsid >/dev/null 2>&1; then G=setsid; \
     elif command -v perl >/dev/null 2>&1; then G=\"perl -e setpgrp;exec(@ARGV)\"; \
     else G=; fi";

/// Something said to a side that is gone, as a shell says of a program that it was killed,
/// would end the shell before it has tidied up. Caught and not ignored: what it starts
/// would ignore it too, and write for ever.
const PIPE: &str = "trap : PIPE";

/// The loop. `kill -s KILL --` is the one spelling every shell takes: dash answers
/// `kill -9 --` with an error that ends the script. A script that waits for its turn to
/// answer looks whether the folder is still there, or it would wait for ever.
fn looping() -> String {
    [
        "d=$(mktemp -d \"${TMPDIR:-/tmp}/lume.XXXXXXXX\") || exit 120",
        "stop() { for p in \"$d\"/*.pid; do [ -f \"$p\" ] && \
         kill -s KILL -- \"-$(cat \"$p\" 2>/dev/null)\" 2>/dev/null; done; rm -rf \"$d\"; }",
        "trap \"stop; exit 143\" HUP TERM INT",
        PIPE,
        LEADER,
        "export G d",
        &format!("echo \"{GREETING} $(uname -n)\""),
        "while read -r w i n t; do case $w in \
         run) head -c \"$t\" > \"$d/$i.all\"; echo \"taken $i\"; \
         dd if=\"$d/$i.all\" of=\"$d/$i.sh\" bs=\"$n\" count=1 2>/dev/null; \
         tail -c \"+$((n + 1))\" \"$d/$i.all\" > \"$d/$i.in\"; rm -f \"$d/$i.all\"; \
         sh \"$d/$i.sh\" < /dev/null & ;; \
         end) : > \"$d/$i.end\"; [ -f \"$d/$i.pid\" ] && \
         kill -s KILL -- \"-$(cat \"$d/$i.pid\")\" 2>/dev/null ;; \
         esac; done",
        "stop",
    ]
    .join("; ")
}

/// What starts one program with its output as it comes. The script comes first on the
/// input; what follows only tells that this side is still there.
fn loader() -> String {
    [
        "d=$(mktemp -d \"${TMPDIR:-/tmp}/lume.XXXXXXXX\") || exit 120",
        "p=0",
        "stop() { [ \"$p\" = 0 ] || kill -s KILL -- \"-$p\" 2>/dev/null; rm -rf \"$d\"; }",
        "trap \"stop; exit 143\" HUP TERM INT",
        PIPE,
        "read -r n",
        "head -c \"$n\" > \"$d/s\"",
        LEADER,
        "exec 3<&0",
        "$G sh \"$d/s\" < /dev/null & p=$!",
        "{ cat <&3 > /dev/null; kill -s KILL -- \"-$p\"; } 2>/dev/null & w=$!",
        "wait \"$p\"; c=$?",
        "kill \"$w\" 2>/dev/null",
        "stop",
        "exit \"$c\"",
    ]
    .join("; ")
}

/// A word as `sh` takes it, whatever is in it.
fn quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// What a connection that could not be opened failed for, by how the program that was to
/// open it ended (`None` where it was ended from here) and what it said last on its
/// error output: one of the kinds below, or nothing that this program can tell.
pub type Unreached = fn(Option<i32>, &[u8]) -> Option<&'static str>;

/// How the other side's shell is reached.
#[derive(Clone, Debug)]
pub struct Door {
    /// The program of this machine, as it was found.
    pub program: PathBuf,
    /// What it is given before what is to be run on the other side.
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Whether the other side is given one line, which the user's own shell there reads,
    /// or the words of a command.
    pub one_line: bool,
    /// What the machine is called, for messages.
    pub label: String,
    /// What tells why a connection was not opened, where the program has a way to.
    pub unreached: Option<Unreached>,
}

impl Door {
    /// Starts `sh` over there with a script that holds no quote of its own.
    fn start(&self, script: &str) -> Result<process::Spawned, ServiceError> {
        let mut args = self.args.clone();
        if self.one_line {
            args.push(format!("exec sh -c '{script}'"));
        } else {
            args.extend(["sh".to_owned(), "-c".to_owned(), script.to_owned()]);
        }
        let program = Program {
            grant: self
                .program
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: self.program.clone(),
        };
        process::spawn(
            &program,
            &args,
            SpawnOptions {
                cwd: None,
                env: self.env.clone(),
            },
        )
        .map_err(|error| ServiceError::unavailable(error.message))
    }
}

/// What a run fails for, beside what fails for anything: its folder is not there, or no
/// such program is.
pub const FOLDER_MISSING: &str = "folder_missing";
pub const PROGRAM_MISSING: &str = "program_missing";

/// What a connection fails for: the machine wants the user to sign in, or to be told that
/// it is the one it says; it is not reached; it has no shell to run programs with.
pub const SIGN_IN: &str = "sign_in";
pub const NOT_CONNECTED: &str = "not_connected";
pub const CANNOT_RUN: &str = "cannot_run";

/// How a machine spells its paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spelling {
    Posix,
    Windows,
}

impl Spelling {
    /// Whether a path begins at the root, or at a drive. A share of a network is neither:
    /// it is another machine's again.
    fn is_absolute(self, path: &str) -> bool {
        match self {
            Self::Posix => path.starts_with('/'),
            Self::Windows => {
                let mut chars = path.chars();
                chars
                    .next()
                    .is_some_and(|drive| drive.is_ascii_alphabetic())
                    && chars.next() == Some(':')
                    && matches!(chars.next(), Some('\\' | '/'))
            }
        }
    }
}

/// A program to be run on another machine.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Asked {
    /// Its name alone, which the other side looks up on its own path.
    pub program: String,
    pub args: Vec<String>,
    /// The folder it runs in, absolute over there; the user's home folder when `None`.
    pub folder: Option<String>,
    pub env: Vec<(String, String)>,
}

impl Asked {
    /// Whether it can be run at all on a machine that spells its paths so.
    pub fn check(&self, spelling: Spelling) -> Result<(), ServiceError> {
        check_program(&self.program).map_err(ServiceError::invalid)?;
        if self.args.iter().any(|arg| arg.contains('\0')) {
            return Err(ServiceError::invalid("an argument holds a NUL character"));
        }
        if let Some(folder) = &self.folder
            && (!spelling.is_absolute(folder) || folder.contains('\0'))
        {
            return Err(ServiceError::invalid(format!(
                "`{folder}` is not an absolute path"
            )));
        }
        for (key, value) in &self.env {
            let name = !key.is_empty()
                && !key.starts_with(|c: char| c.is_ascii_digit())
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !name || value.contains('\0') {
                return Err(ServiceError::invalid(format!(
                    "`{key}` is not an environment variable that can be set"
                )));
            }
        }
        Ok(())
    }

    /// The script that goes to the folder and becomes the program. `told` is what it
    /// does with the word for why it cannot.
    fn script(&self, told: impl Fn(&str) -> String) -> String {
        let mut script = match &self.folder {
            Some(folder) => format!("cd -- {} 2>/dev/null", quote(folder)),
            None => "cd".to_owned(),
        };
        script.push_str(&format!(" || {{ {}; exit 125; }}\n", told("folder")));
        for (key, value) in &self.env {
            script.push_str(&format!("export {key}={}\n", quote(value)));
        }
        let program = quote(&self.program);
        script.push_str(&format!(
            "command -v {program} >/dev/null 2>&1 || {{ {}; exit 127; }}\n",
            told("program")
        ));
        script
    }

    fn exec(&self) -> String {
        let mut line = format!("exec {}", quote(&self.program));
        for arg in &self.args {
            line.push(' ');
            line.push_str(&quote(arg));
        }
        line.push('\n');
        line
    }

    /// Why the other side could not run it, from its word for that.
    fn failed(&self, why: &str, label: &str) -> ServiceError {
        match why {
            "folder" => ServiceError::not_found(format!(
                "the folder `{}` does not exist on {label}",
                self.folder.as_deref().unwrap_or("~")
            ))
            .because(FOLDER_MISSING),
            "program" => {
                ServiceError::not_found(format!("`{}` was not found on {label}", self.program))
                    .because(PROGRAM_MISSING)
            }
            _ => ServiceError::failed(format!("{label} could not run `{}`", self.program)),
        }
    }
}

/// The script of run `id`, whose answer holds at most `max` bytes of each output and one
/// more, which tells that there were more. What is past that is read and dropped, so the
/// program ends as it would have; `cat` keeps it, as what it reads is written at once. Once it has ended, what is left of its output is read
/// for as long as [`OUTPUT_GRACE`], in steps of a hundredth of a second: something it
/// started may keep the output open.
fn run_script(id: u64, asked: &Asked, max: usize) -> String {
    let keep = max.saturating_add(1);
    // Of 512 bytes, or of 1024 to some shells.
    let blocks = keep / 512 + 1;
    let steps = OUTPUT_GRACE.as_millis() / 10;
    let inner = format!(
        "echo $$ > \"$d/{id}.pid\"\n[ -e \"$d/{id}.end\" ] && exit 137\n{}{}",
        asked.script(|why| format!("echo {why} > \"$d/{id}.why\"")),
        asked.exec()
    );
    format!(
        "mkfifo \"$d/{id}.pipe\" || {{ echo \"done {id} 1 0 0 failed\"; exit 1; }}\n\
         {{ (ulimit -f {blocks}; exec cat > \"$d/{id}.out\"); cat > /dev/null; \
         : > \"$d/{id}.read\"; }} < \"$d/{id}.pipe\" 2> /dev/null &\n\
         $G sh -c {} < \"$d/{id}.in\" > \"$d/{id}.pipe\" 2> \"$d/{id}.err\"\n\
         c=$?; s=ran; n=0\n\
         until [ -e \"$d/{id}.read\" ]; do \
         [ \"$n\" -ge {steps} ] && {{ s=held; break; }}; \
         [ -d \"$d\" ] || exit 0; sleep 0.01; n=$((n + 1)); done\n\
         until mkdir \"$d/lock\" 2>/dev/null; do [ -d \"$d\" ] || exit 0; sleep 0.01; done\n\
         [ -f \"$d/{id}.why\" ] && s=$(cat \"$d/{id}.why\")\n\
         o=$(($(wc -c < \"$d/{id}.out\"))); e=$(($(wc -c < \"$d/{id}.err\")))\n\
         [ \"$o\" -gt {keep} ] && o={keep}\n\
         [ \"$e\" -gt {keep} ] && e={keep}\n\
         echo \"done {id} $c $o $e $s\"\n\
         head -c \"$o\" \"$d/{id}.out\"; head -c \"$e\" \"$d/{id}.err\"\n\
         rmdir \"$d/lock\"; rm -f \"$d/{id}\".*\n",
        quote(&inner)
    )
}

/// The other side's answer to a run.
struct Answer {
    code: i32,
    out: Vec<u8>,
    err: Vec<u8>,
    why: String,
}

/// How a run is waited for.
#[derive(Clone, Debug)]
pub struct Limits {
    /// What the program reads, all of it.
    pub stdin: Vec<u8>,
    pub timeout: Duration,
    /// How many bytes of its output and of its error output are kept, each.
    pub max_output: usize,
    /// What has it end before its time.
    pub stop: Arc<Stop>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Waiting {
    max: usize,
    answer: mpsc::Sender<Answer>,
}

/// What the reading thread shares with those who wait.
#[derive(Default)]
struct Shared {
    /// The runs on their way. `None` once the connection has ended.
    runs: Mutex<Option<HashMap<u64, Waiting>>>,
    /// The end of what the connection said on its error output.
    said: Mutex<Vec<u8>>,
    /// Whether all of it has been read.
    said_all: AtomicBool,
}

/// One connection to the loop.
struct Line {
    label: String,
    child: Child,
    /// What is sent, and the word that the last run was taken in.
    send: Mutex<(ChildStdin, mpsc::Receiver<u64>)>,
    shared: Arc<Shared>,
    next: AtomicU64,
    /// What the other side calls itself.
    name: String,
    /// When a run last ended.
    used: Mutex<Instant>,
    /// Whether opening it took a person, so that another is not opened beside it.
    signed_in: bool,
}

impl Drop for Line {
    fn drop(&mut self) {
        // Its input closes with it, which has the other side end everything; a client
        // that does not go of itself is ended, with nobody kept waiting for it.
        let _ = self.child.kill();
    }
}

/// Reads the loop's answers until the connection ends.
fn read_answers(mut out: BufReader<ChildStdout>, shared: &Shared, taken: &mpsc::Sender<u64>) {
    let mut line = String::new();
    loop {
        line.clear();
        match (&mut out).take(LINE_MAX).read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) if !line.ends_with('\n') => return,
            Ok(_) => {}
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        match words[..] {
            ["taken", id] => {
                let Ok(id) = id.parse() else { return };
                if taken.send(id).is_err() {
                    return;
                }
            }
            ["done", id, code, out_len, err_len, why] => {
                let (Ok(id), Ok(code), Ok(out_len), Ok(err_len)) = (
                    id.parse::<u64>(),
                    code.parse::<i32>(),
                    out_len.parse::<usize>(),
                    err_len.parse::<usize>(),
                ) else {
                    return;
                };
                let Some(waiting) = lock(&shared.runs)
                    .as_mut()
                    .and_then(|runs| runs.remove(&id))
                else {
                    return;
                };
                let most = waiting.max.saturating_add(1);
                if out_len > most || err_len > most {
                    return;
                }
                let mut answer = Answer {
                    code,
                    out: vec![0; out_len],
                    err: vec![0; err_len],
                    why: why.to_owned(),
                };
                if out.read_exact(&mut answer.out).is_err()
                    || out.read_exact(&mut answer.err).is_err()
                {
                    return;
                }
                // Whoever waited may have given up.
                let _ = waiting.answer.send(answer);
            }
            _ => return,
        }
    }
}

/// Keeps the end of what is written to an error output.
fn keep_said(mut from: impl Read + Send + 'static, shared: Arc<Shared>) {
    thread::spawn(move || {
        let mut buffer = [0u8; 1024];
        while let Ok(read) = from.read(&mut buffer)
            && read > 0
        {
            let mut said = lock(&shared.said);
            said.extend_from_slice(&buffer[..read]);
            let over = said.len().saturating_sub(SAID_MAX);
            said.drain(..over);
        }
        shared.said_all.store(true, Ordering::Release);
    });
}

/// Waits a little for the rest of what a connection that ended said of errors.
fn said_all(shared: &Shared) {
    let until = Instant::now() + ENDED_WAIT;
    while !shared.said_all.load(Ordering::Acquire) && Instant::now() < until {
        thread::sleep(Duration::from_millis(10));
    }
}

/// The last line that says something, made plain, of what a connection said of errors.
fn last_said(said: &[u8]) -> Option<String> {
    String::from_utf8_lossy(said)
        .lines()
        .rev()
        .map(|line| {
            line.chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .take(300)
                .collect::<String>()
                .trim()
                .to_owned()
        })
        .find(|line| !line.is_empty())
}

/// The first line that begins with one of `words`, of what the other side writes, read
/// byte by byte so that nothing after it is taken. What comes before is passed over.
fn first_line(from: &mut impl Read, words: &[&str]) -> Option<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    for _ in 0..CHATTER_MAX {
        match from.read(&mut byte) {
            Ok(1) => {}
            Ok(_) | Err(_) => return None,
        }
        if byte[0] != b'\n' {
            line.push(byte[0]);
            continue;
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches('\r');
        if words.iter().any(|word| {
            text.strip_prefix(word)
                .is_some_and(|rest| rest.starts_with(' '))
        }) {
            return Some(text.to_owned());
        }
        line.clear();
    }
    None
}

/// Waits for the other side's first line on a thread of its own, so that a side that
/// says nothing keeps nobody longer than it may.
fn greeted(
    mut out: ChildStdout,
    words: &'static [&'static str],
    stop: &Stop,
    wait: Duration,
) -> Result<(String, ChildStdout), Option<RecvTimeoutError>> {
    let (tell, told) = mpsc::channel();
    thread::spawn(move || {
        let line = first_line(&mut out, words);
        let _ = tell.send(line.map(|line| (line, out)));
    });
    let until = Instant::now() + wait;
    loop {
        match told.recv_timeout(LOOK) {
            Ok(Some(there)) => return Ok(there),
            Ok(None) => return Err(None),
            Err(RecvTimeoutError::Timeout) if Instant::now() < until && !stop.asked() => {}
            Err(error) => return Err(Some(error)),
        }
    }
}

/// Why a connection was not opened. `ended` is how the program that was to open it
/// ended, where it ended by itself, which `late` says it did not.
fn unreached(door: &Door, late: bool, ended: Option<i32>, said: &[u8]) -> ServiceError {
    let label = &door.label;
    let error = ServiceError::unavailable(match last_said(said) {
        Some(said) => format!("{label} could not be reached: {said}"),
        None if late => format!("{label} did not answer in time"),
        None => format!("{label} has no shell that programs can be run with"),
    });
    if late {
        return error;
    }
    // A program that ended before the loop said it was there: the way in failed, as the
    // program that went there can tell, or there is no `sh` over there to run it.
    match door.unreached.and_then(|why| why(ended, said)) {
        Some(why) => error.because(why),
        None => error.because(CANNOT_RUN),
    }
}

/// How a program that may be ending by itself ended, waited for a little while.
fn ended_by_itself(child: &Child) -> Option<i32> {
    let until = Instant::now() + ENDED_WAIT;
    loop {
        if let Some(code) = child.try_wait() {
            return code;
        }
        if Instant::now() >= until {
            return None;
        }
        thread::sleep(LOOK);
    }
}

impl Line {
    /// Opens it through `door`, waiting as long as `wait` for the other side to be there.
    /// `signed_in` says that a person was to be asked on the way.
    fn open(
        door: &Door,
        stop: &Stop,
        wait: Duration,
        signed_in: bool,
    ) -> Result<Self, ServiceError> {
        let spawned = door.start(&looping())?;
        let missing = || ServiceError::failed("the connection's pipes were not opened");
        let shared = Arc::new(Shared::default());
        keep_said(spawned.stderr.ok_or_else(missing)?, Arc::clone(&shared));
        let stdin = spawned.stdin.ok_or_else(missing)?;
        let child = spawned.child;
        let (line, out) = greeted(spawned.stdout.ok_or_else(missing)?, &[GREETING], stop, wait)
            .map_err(|why| {
                let late = why == Some(RecvTimeoutError::Timeout);
                let ended = if late { None } else { ended_by_itself(&child) };
                // Ended first, so that what it said last is there to be read.
                let _ = child.kill();
                said_all(&shared);
                unreached(door, late, ended, &lock(&shared.said))
            })?;
        let name = line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_owned();
        *lock(&shared.runs) = Some(HashMap::new());
        let (tell, taken) = mpsc::channel();
        let reading = Arc::clone(&shared);
        thread::spawn(move || {
            read_answers(BufReader::new(out), &reading, &tell);
            // Everyone who waits hears that there is nothing more to wait for.
            lock(&reading.runs).take();
        });
        Ok(Self {
            label: door.label.clone(),
            child,
            send: Mutex::new((stdin, taken)),
            shared,
            next: AtomicU64::new(1),
            name,
            used: Mutex::new(Instant::now()),
            signed_in,
        })
    }

    fn ended(&self) -> bool {
        lock(&self.shared.runs).is_none()
    }

    fn lost(&self) -> ServiceError {
        ServiceError::unavailable(match last_said(&lock(&self.shared.said)) {
            Some(said) => format!("the connection to {} was lost: {said}", self.label),
            None => format!("the connection to {} was lost", self.label),
        })
    }

    /// Ends the connection, when the other side can no longer be followed.
    fn give_up(&self) {
        lock(&self.shared.runs).take();
        let _ = self.child.kill();
    }

    fn run(&self, asked: &Asked, limits: Limits) -> Result<RunOutput, ServiceError> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let script = run_script(id, asked, limits.max_output);
        if script.len() > SCRIPT_MAX {
            return Err(ServiceError::too_large(
                "the arguments are longer than can be sent to another machine",
            ));
        }
        let (tell, told) = mpsc::channel();
        match lock(&self.shared.runs).as_mut() {
            Some(runs) => {
                runs.insert(
                    id,
                    Waiting {
                        max: limits.max_output,
                        answer: tell,
                    },
                );
            }
            None => return Err(self.lost()),
        }
        let until = Instant::now() + limits.timeout;
        self.send(id, &script, &limits.stdin)?;
        let mut ended = None;
        loop {
            match told.recv_timeout(LOOK) {
                Ok(answer) => {
                    *lock(&self.used) = Instant::now();
                    return self.output(asked, answer, limits.max_output, ended);
                }
                Err(RecvTimeoutError::Disconnected) => return Err(self.lost()),
                Err(RecvTimeoutError::Timeout) => {}
            }
            let now = Instant::now();
            match ended {
                None if now >= until || limits.stop.asked() => {
                    ended = Some((now, now >= until));
                    let mut send = lock(&self.send);
                    if writeln!(send.0, "end {id}").is_err() || send.0.flush().is_err() {
                        drop(send);
                        self.give_up();
                        return Err(self.lost());
                    }
                }
                Some((at, _)) if now.duration_since(at) >= END_WAIT => {
                    self.give_up();
                    return Err(ServiceError::unavailable(format!(
                        "{} did not say that the program ended",
                        self.label
                    )));
                }
                _ => {}
            }
        }
    }

    /// Sends a run, and waits until it was taken in: some programs that read a number of
    /// bytes over there read on past it, and what they took would be lost.
    fn send(&self, id: u64, script: &str, stdin: &[u8]) -> Result<(), ServiceError> {
        let mut send = lock(&self.send);
        let (to, taken) = &mut *send;
        let total = script.len() + stdin.len();
        let written = writeln!(to, "run {id} {} {total}", script.len())
            .and_then(|()| to.write_all(script.as_bytes()))
            .and_then(|()| to.write_all(stdin))
            .and_then(|()| to.flush());
        let said = written
            .is_ok()
            .then(|| taken.recv_timeout(TAKEN_WAIT).ok())
            .flatten();
        drop(send);
        if said == Some(id) {
            return Ok(());
        }
        self.give_up();
        Err(self.lost())
    }

    fn output(
        &self,
        asked: &Asked,
        mut answer: Answer,
        max: usize,
        ended: Option<(Instant, bool)>,
    ) -> Result<RunOutput, ServiceError> {
        let held = answer.why == "held";
        if !held && answer.why != "ran" {
            return Err(asked.failed(&answer.why, &self.label));
        }
        let truncated = held || answer.out.len() > max || answer.err.len() > max;
        answer.out.truncate(max);
        answer.err.truncate(max);
        Ok(RunOutput {
            code: ended.is_none().then_some(answer.code),
            stdout: answer.out,
            stderr: answer.err,
            timed_out: ended.is_some_and(|(_, late)| late),
            truncated,
        })
    }
}

/// A program started on the other side, with its output as it comes.
pub struct Started {
    pub stdout: Box<dyn Read + Send>,
    pub stderr: Box<dyn Read + Send>,
    pub child: Arc<dyn process::Running>,
}

/// The output of a program that runs over the one connection, which comes whole at its
/// end. Reading waits until then.
#[derive(Default)]
struct Whole {
    ended: Mutex<Option<WholeEnd>>,
    told: std::sync::Condvar,
}

struct WholeEnd {
    code: Option<i32>,
    out: std::io::Cursor<Vec<u8>>,
    err: std::io::Cursor<Vec<u8>>,
}

impl Whole {
    fn end(&self, end: WholeEnd) {
        *lock(&self.ended) = Some(end);
        self.told.notify_all();
    }

    fn read(&self, buffer: &mut [u8], err: bool) -> std::io::Result<usize> {
        let mut ended = lock(&self.ended);
        loop {
            if let Some(end) = ended.as_mut() {
                let cursor = if err { &mut end.err } else { &mut end.out };
                return cursor.read(buffer);
            }
            ended = self
                .told
                .wait(ended)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

struct WholeOutput {
    whole: Arc<Whole>,
    err: bool,
}

impl Read for WholeOutput {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.whole.read(buffer, self.err)
    }
}

/// A program that runs over the one connection, as it is waited for and ended from here.
struct OverLine {
    whole: Arc<Whole>,
    stop: Arc<Stop>,
}

impl process::Running for OverLine {
    fn id(&self) -> u32 {
        0
    }

    fn try_wait(&self) -> Option<Option<i32>> {
        lock(&self.whole.ended).as_ref().map(|end| end.code)
    }

    fn kill(&self) -> Result<(), ServiceError> {
        self.stop.ask();
        Ok(())
    }
}

/// The program as it is waited for and ended from here. Its input is what tells the
/// other side that this one is still there, and is written no more.
#[derive(Debug)]
pub struct Held {
    child: Child,
    there: Mutex<Option<ChildStdin>>,
}

impl process::Running for Held {
    fn id(&self) -> u32 {
        // Its process is not one of this machine's.
        0
    }

    fn try_wait(&self) -> Option<Option<i32>> {
        self.child.try_wait()
    }

    fn kill(&self) -> Result<(), ServiceError> {
        lock(&self.there).take();
        self.child.kill()
    }
}

/// The shell of another machine: a connection that is opened when it is first needed,
/// and again when it was lost.
pub struct Shell {
    door: Door,
    line: Mutex<Option<Arc<Line>>>,
    closed: AtomicBool,
}

impl Shell {
    pub fn new(door: Door) -> Self {
        Self {
            door,
            line: Mutex::new(None),
            closed: AtomicBool::new(false),
        }
    }

    /// What the machine calls itself, while there is a connection to it.
    pub fn name(&self) -> Option<String> {
        lock(&self.line).as_ref().map(|line| line.name.clone())
    }

    fn line(&self, stop: &Stop) -> Result<Arc<Line>, ServiceError> {
        let mut held = lock(&self.line);
        if self.closed.load(Ordering::Acquire) {
            return Err(ServiceError::unavailable(format!(
                "the connection to {} was closed",
                self.door.label
            )));
        }
        if let Some(line) = held.as_ref().filter(|line| !line.ended()) {
            return Ok(Arc::clone(line));
        }
        *held = None;
        let line = Arc::new(Line::open(&self.door, stop, GREETING_WAIT, false)?);
        *held = Some(Arc::clone(&line));
        Ok(line)
    }

    /// Opens the connection through `door`, which may ask the user what signing in takes,
    /// and keeps it in place of any other. Programs whose output is wanted as it comes run
    /// over it from then on, so that nobody is asked again for as long as it is open. It
    /// waits as long as `wait` for the other side, or until `stop` is asked.
    pub fn sign_in(&self, door: &Door, stop: &Stop, wait: Duration) -> Result<(), ServiceError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(ServiceError::unavailable(format!(
                "the connection to {} was closed",
                self.door.label
            )));
        }
        let line = Arc::new(Line::open(door, stop, wait, true)?);
        let mut held = lock(&self.line);
        if self.closed.load(Ordering::Acquire) {
            line.give_up();
            return Err(ServiceError::unavailable(format!(
                "the connection to {} was closed",
                self.door.label
            )));
        }
        if let Some(old) = held.replace(line) {
            // Runs on their way there end with it, and say that the connection was lost.
            old.give_up();
        }
        Ok(())
    }

    /// Whether the connection there is now took a person to open.
    pub fn signed_in(&self) -> bool {
        lock(&self.line)
            .as_ref()
            .is_some_and(|line| line.signed_in && !line.ended())
    }

    /// Runs a program to its end, or until its time is up or it is told to stop, and
    /// gives what it wrote. It waits, so it belongs on a thread that may.
    pub fn run(&self, asked: &Asked, limits: Limits) -> Result<RunOutput, ServiceError> {
        asked.check(Spelling::Posix)?;
        self.line(&limits.stop)?.run(asked, limits)
    }

    /// Starts a program and gives its output as it comes. It reads nothing.
    pub fn spawn(&self, asked: &Asked, stop: &Stop) -> Result<Started, ServiceError> {
        asked.check(Spelling::Posix)?;
        let signed_in = lock(&self.line)
            .as_ref()
            .filter(|line| line.signed_in && !line.ended())
            .cloned();
        if let Some(line) = signed_in {
            return Ok(Self::spawn_over(&line, asked));
        }
        let script = format!(
            "{}echo \"{STARTED} \"\n{}",
            asked.script(|why| format!("echo \"{NOT_STARTED} {why}\"")),
            asked.exec()
        );
        if script.len() > SCRIPT_MAX {
            return Err(ServiceError::too_large(
                "the arguments are longer than can be sent to another machine",
            ));
        }
        let spawned = self.door.start(&loader())?;
        let missing = || ServiceError::failed("the program's pipes were not opened");
        let mut stdin = spawned.stdin.ok_or_else(missing)?;
        let stdout = spawned.stdout.ok_or_else(missing)?;
        let mut stderr = spawned.stderr.ok_or_else(missing)?;
        let child = spawned.child;
        let sent = writeln!(stdin, "{}", script.len())
            .and_then(|()| stdin.write_all(script.as_bytes()))
            .and_then(|()| stdin.flush());
        let answered = match sent {
            Ok(()) => greeted(stdout, &[STARTED, NOT_STARTED], stop, GREETING_WAIT),
            Err(_) => Err(None),
        };
        match answered {
            Ok((line, stdout)) if line.starts_with(STARTED) => Ok(Started {
                stdout: Box::new(stdout),
                stderr: Box::new(stderr),
                child: Arc::new(Held {
                    child,
                    there: Mutex::new(Some(stdin)),
                }),
            }),
            Ok((line, _)) => {
                let why = line.split_whitespace().nth(1).unwrap_or_default();
                Err(asked.failed(why, &self.door.label))
            }
            Err(why) => {
                let late = why == Some(RecvTimeoutError::Timeout);
                let ended = if late { None } else { ended_by_itself(&child) };
                let _ = child.kill();
                let mut said = Vec::new();
                let _ = (&mut stderr).take(SAID_MAX as u64).read_to_end(&mut said);
                Err(unreached(&self.door, late, ended, &said))
            }
        }
    }

    /// Runs a program over the one connection, its output given when it ends.
    fn spawn_over(line: &Arc<Line>, asked: &Asked) -> Started {
        let whole = Arc::new(Whole::default());
        let stop = Arc::new(Stop::default());
        let limits = Limits {
            stdin: Vec::new(),
            timeout: WHOLE_TIME,
            max_output: WHOLE_MAX,
            stop: Arc::clone(&stop),
        };
        let (line, asked, ending) = (Arc::clone(line), asked.clone(), Arc::clone(&whole));
        thread::spawn(move || {
            let end = match line.run(&asked, limits) {
                Ok(output) => WholeEnd {
                    code: output.code,
                    out: std::io::Cursor::new(output.stdout),
                    err: std::io::Cursor::new(output.stderr),
                },
                Err(error) => WholeEnd {
                    code: None,
                    out: std::io::Cursor::new(Vec::new()),
                    err: std::io::Cursor::new(error.message.into_bytes()),
                },
            };
            ending.end(end);
        });
        Started {
            stdout: Box::new(WholeOutput {
                whole: Arc::clone(&whole),
                err: false,
            }),
            stderr: Box::new(WholeOutput {
                whole: Arc::clone(&whole),
                err: true,
            }),
            child: Arc::new(OverLine { whole, stop }),
        }
    }

    /// Lets go of the connection, if nothing has used it for as long as `idle`.
    pub fn close_idle(&self, idle: Duration) {
        let mut held = lock(&self.line);
        let unused = held.as_ref().is_some_and(|line| {
            lock(&line.used).elapsed() >= idle
                && lock(&line.shared.runs)
                    .as_ref()
                    .is_none_or(HashMap::is_empty)
        });
        if unused {
            held.take();
        }
    }

    /// Ends the connection and opens no other.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        if let Some(line) = lock(&self.line).take() {
            line.give_up();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_users_own_shell_hands_on_holds_nothing_it_could_read() {
        for script in [looping(), loader()] {
            assert!(!script.contains(['\'', '\\', '\n']), "{script}");
        }
    }

    #[test]
    fn a_word_is_quoted_whatever_is_in_it() {
        assert_eq!(quote("plain"), "'plain'");
        assert_eq!(quote(""), "''");
        assert_eq!(quote("it's $HOME"), r"'it'\''s $HOME'");
    }

    #[test]
    fn what_cannot_be_run_is_refused_before_anything_is_started() {
        let shell = Shell::new(Door {
            program: PathBuf::from("no-such-program"),
            args: Vec::new(),
            env: Vec::new(),
            one_line: false,
            label: "box".to_owned(),
            unreached: None,
        });
        let asked = |program: &str| Asked {
            program: program.to_owned(),
            ..Asked::default()
        };
        let cases = [
            asked("/bin/git"),
            asked("git status"),
            asked("$(id)"),
            Asked {
                args: vec!["a\0b".to_owned()],
                ..asked("git")
            },
            Asked {
                folder: Some("relative".to_owned()),
                ..asked("git")
            },
            Asked {
                folder: Some(r"C:\Users".to_owned()),
                ..asked("git")
            },
            Asked {
                env: vec![("A;id".to_owned(), String::new())],
                ..asked("git")
            },
            Asked {
                env: vec![("1A".to_owned(), String::new())],
                ..asked("git")
            },
            Asked {
                env: vec![("A".to_owned(), "x\0y".to_owned())],
                ..asked("git")
            },
        ];
        for asked in cases {
            let error = shell
                .spawn(&asked, &Stop::default())
                .map(|_| ())
                .unwrap_err();
            assert_eq!(error.kind, crate::ErrorKind::Invalid, "{asked:?}");
        }
        let error = shell
            .spawn(&asked("git"), &Stop::default())
            .map(|_| ())
            .unwrap_err();
        assert_eq!(error.kind, crate::ErrorKind::Unavailable);
    }

    #[test]
    fn a_folder_is_absolute_as_its_machine_spells_paths() {
        let checked = |spelling, folder: &str| {
            let asked = Asked {
                program: "git".to_owned(),
                folder: Some(folder.to_owned()),
                ..Asked::default()
            };
            asked.check(spelling).is_ok()
        };
        for (folder, posix, windows) in [
            ("/home/me", true, false),
            ("/", true, false),
            (r"C:\Users", false, true),
            ("d:/code", false, true),
            ("C:", false, false),
            (r"C:code", false, false),
            (r"1:\code", false, false),
            (r"\\server\share", false, false),
            (r"\\?\C:\code", false, false),
            (r"\code", false, false),
            ("${home}", false, false),
            ("", false, false),
            ("/a\0", false, false),
            ("C:/a\0", false, false),
        ] {
            assert_eq!(checked(Spelling::Posix, folder), posix, "{folder:?}");
            assert_eq!(checked(Spelling::Windows, folder), windows, "{folder:?}");
        }
    }

    #[test]
    fn why_a_program_was_not_run_has_a_name_of_its_own() {
        let asked = Asked {
            program: "git".to_owned(),
            ..Asked::default()
        };
        let why = |word: &str| asked.failed(word, "box").why;
        assert_eq!(why("folder"), Some("folder_missing"));
        assert_eq!(why("program"), Some("program_missing"));
        assert_eq!(why("something else"), None);
    }

    #[test]
    fn what_comes_before_the_first_line_is_passed_over_and_what_comes_after_is_left() {
        let mut from: &[u8] = b"Welcome!\r\nlume-thereabouts\nlume-there box\nrest";
        let line = first_line(&mut from, &[GREETING]).unwrap();
        assert_eq!((line.as_str(), from), ("lume-there box", &b"rest"[..]));
        let endless = std::iter::repeat_n(b'x', CHATTER_MAX * 2).collect::<Vec<_>>();
        assert_eq!(first_line(&mut endless.as_slice(), &[GREETING]), None);
    }

    #[test]
    fn what_a_connection_said_last_is_made_plain() {
        assert_eq!(last_said(b""), None);
        assert_eq!(
            last_said(b"one\r\nssh: connect\x1b[31m: refused\r\n\r\n").as_deref(),
            Some("ssh: connect [31m: refused")
        );
    }
}
