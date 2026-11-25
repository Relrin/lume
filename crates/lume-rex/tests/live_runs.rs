//! Programs run on the machine of a Rex server that is really running, on this machine or
//! another.
//!
//! Not run with the other tests: it makes and ends a session of its own there, and starts
//! programs (git, and node on Windows or perl elsewhere). `LUME_REX_LIVE_RUNS` names the
//! server, as an address that a person would type.
//!
//! cargo test -p lume-rex --test live_runs -- --ignored --nocapture

mod common;

use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::client;
use lume_proto::BackendId;
use lume_rex::{
    Control, Endpoint, Events, Options, Ran, Reach, RexBackend, RunAsked, RunError, RunLimits,
    RunReader,
};
use lume_session::SessionBackend as _;
use serde_json::{Value, json};
use tokio::time::{sleep, timeout};

const ADDRESS: &str = "LUME_REX_LIVE_RUNS";
const LABEL: &str = "lume-live-runs";

/// A program that does as its first argument says, for node and for perl.
const NODE: &str = r#"
const [mode, ...a] = process.argv.slice(1);
const n = Number(a[0]);
const bytes = () => Uint8Array.from({ length: n }, (_, i) => (i * 7 + (i >> 8)) & 255);
const child = (stdio) => require("child_process").spawn(process.execPath, ["-e", "setTimeout(() => {}, n)".replace("n", a[0] ?? 600000)], { stdio, detached: true });
if (mode === "out") process.stdout.write(bytes());
else if (mode === "ends") { process.stdout.write("to-out\n"); process.stderr.write("to-err\r\n"); process.exitCode = n; }
else if (mode === "args") process.stdout.write(a.map((x) => Buffer.from(x).toString("hex")).join("\n"));
else if (mode === "cat") process.stdin.pipe(process.stdout);
else if (mode === "env") process.stdout.write(process.env[a[0]] ?? "<unset>");
else if (mode === "cwd") process.stdout.write(process.cwd());
else if (mode === "tree") { const c = require("child_process").spawn(process.execPath, ["-e", "setTimeout(() => {}, 600000)"], { stdio: "ignore" }); process.stdout.write(process.pid + " " + c.pid + "\n"); setTimeout(() => {}, 600000); }
else if (mode === "held") { child(["ignore", "inherit", "inherit"]).unref(); process.stdout.write("done"); }
else if (mode === "ticks") { let i = 0; const t = setInterval(() => { process.stdout.write("tick " + i + "\n"); if (++i === n) clearInterval(t); }, 200); }
"#;
const PERL: &str = r#"
use Cwd; binmode STDOUT; binmode STDIN; $| = 1;
my ($mode, @a) = @ARGV;
if ($mode eq "out") { my $s = ""; for (my $i = 0; $i < $a[0]; $i++) { $s .= chr(($i * 7 + ($i >> 8)) & 255); if (length($s) > 65536) { print $s; $s = "" } } print $s }
elsif ($mode eq "ends") { print "to-out\n"; print STDERR "to-err\r\n"; exit $a[0] }
elsif ($mode eq "args") { print join("\n", map { unpack("H*", $_) } @a) }
elsif ($mode eq "cat") { my $b; while (read(STDIN, $b, 65536)) { print $b } }
elsif ($mode eq "env") { print exists $ENV{$a[0]} ? $ENV{$a[0]} : "<unset>" }
elsif ($mode eq "cwd") { print getcwd() }
elsif ($mode eq "tree") { my $p = fork(); if (!$p) { open STDOUT, ">/dev/null"; open STDERR, ">/dev/null"; sleep 600; exit } print "$$ $p\n"; sleep 600 }
elsif ($mode eq "held") { if (!fork()) { select undef, undef, undef, $a[0] / 1000; exit } print "done" }
elsif ($mode eq "ticks") { for (0 .. $a[0] - 1) { print "tick $_\n"; select undef, undef, undef, 0.2 } }
"#;

fn bytes(count: usize) -> Vec<u8> {
    (0..count).map(|nth| (nth * 7 + (nth >> 8)) as u8).collect()
}

fn limits() -> RunLimits {
    RunLimits {
        stdin: Vec::new(),
        timeout: Duration::from_secs(60),
        max_output: 16 * 1024 * 1024,
    }
}

fn median(mut took: Vec<Duration>) -> (u128, u128) {
    took.sort();
    let at = |share: usize| took[(took.len() * share / 100).min(took.len() - 1)].as_millis();
    (at(50), at(95))
}

/// Whoever else is connected to the server, with what the connection goes through.
async fn another(endpoint: &Endpoint) -> (Control, (Events, Reach)) {
    let reach = Reach::new(endpoint.clone(), options());
    let dialed = reach.dial().await.unwrap();
    let name = endpoint.to_string();
    let opened = Control::open_at(dialed.io, &name, &dialed.authority, &client(), None).await;
    let (control, events) = opened.unwrap();
    (control, (events, reach))
}

/// The address is typed by whoever runs the test, who agrees to what it names.
fn options() -> Options {
    Options::new().plain_http_agreed(true)
}

async fn connected(endpoint: &Endpoint) -> Arc<RexBackend> {
    let backend = RexBackend::connect_with(
        BackendId("rex".to_owned()),
        endpoint.clone(),
        client(),
        options(),
    );
    timeout(Duration::from_secs(90), backend.ready())
        .await
        .expect("is a Rex server running?")
        .unwrap();
    backend
}

struct Live {
    endpoint: Endpoint,
    backend: Arc<RexBackend>,
    other: Control,
    session: String,
    windows: bool,
}

impl Live {
    fn tool(&self, words: &[&str]) -> RunAsked {
        let (program, script) = if self.windows {
            ("node", NODE)
        } else {
            ("perl", PERL)
        };
        let mut args = vec!["-e".to_owned(), script.to_owned()];
        args.extend(words.iter().map(|word| (*word).to_owned()));
        RunAsked {
            program: program.to_owned(),
            args,
            ..RunAsked::default()
        }
    }

    async fn run(&self, asked: RunAsked) -> Ran {
        self.run_with(asked, limits()).await.unwrap()
    }

    async fn run_with(&self, asked: RunAsked, limits: RunLimits) -> Result<Ran, RunError> {
        timeout(Duration::from_secs(120), self.backend.run(&asked, limits))
            .await
            .expect("it ended")
    }

    /// Which of these processes there are still, by another run.
    async fn left(&self, backend: &RexBackend, processes: &str) -> Vec<String> {
        let mut left = Vec::new();
        for process in processes.split_whitespace() {
            let asked = if self.windows {
                RunAsked {
                    program: "tasklist".to_owned(),
                    args: vec!["/NH".into(), "/FI".into(), format!("PID eq {process}")],
                    ..RunAsked::default()
                }
            } else {
                RunAsked {
                    program: "ps".to_owned(),
                    args: vec!["-o".into(), "pid=".into(), "-p".into(), process.into()],
                    ..RunAsked::default()
                }
            };
            let ran = backend.run(&asked, limits()).await.unwrap();
            if String::from_utf8_lossy(&ran.stdout)
                .split_whitespace()
                .any(|word| word == process)
            {
                left.push(process.to_owned());
            }
        }
        left
    }

    async fn all_gone(&self, backend: &RexBackend, processes: &str, within: Duration) {
        let began = Instant::now();
        loop {
            let left = self.left(backend, processes).await;
            if left.is_empty() {
                println!("    gone after {} ms", began.elapsed().as_millis());
                return;
            }
            assert!(began.elapsed() < within, "still there: {left:?}");
            sleep(Duration::from_millis(200)).await;
        }
    }

    /// The blocks of the session that are in no window.
    async fn detached(&self) -> Vec<Value> {
        let asked = json!({ "session_id": self.session });
        let listed = self.other.call("session.list_blocks", asked).await.unwrap();
        let blocks = listed.payload["blocks"].as_array().unwrap().clone();
        blocks
            .into_iter()
            .filter(|block| block["placement"] != "placed")
            .collect()
    }

    async fn no_blocks_left(&self) {
        let began = Instant::now();
        while !self.detached().await.is_empty() {
            assert!(
                began.elapsed() < Duration::from_secs(10),
                "left: {:?}",
                self.detached().await
            );
            sleep(Duration::from_millis(100)).await;
        }
    }
}

/// Reads the first line of an output.
async fn first_line(mut from: RunReader) -> (String, RunReader) {
    tokio::task::spawn_blocking(move || {
        let mut line = Vec::new();
        let mut byte = [0];
        while from.read(&mut byte).unwrap() == 1 && byte[0] != b'\n' {
            line.push(byte[0]);
        }
        (String::from_utf8(line).unwrap(), from)
    })
    .await
    .unwrap()
}

async fn all_of(mut from: RunReader) -> Vec<u8> {
    tokio::task::spawn_blocking(move || {
        let mut all = Vec::new();
        from.read_to_end(&mut all).unwrap();
        all
    })
    .await
    .unwrap()
}

async fn input_and_output(live: &Live) {
    for count in [0, 256, 65_536, 1 << 20, 8 << 20] {
        let began = Instant::now();
        let ran = live.run(live.tool(&["out", &count.to_string()])).await;
        assert!(ran.stdout == bytes(count), "{count}: {}", ran.stdout.len());
        println!("  {count} bytes: {} ms", began.elapsed().as_millis());
    }
    for count in [1, 1 << 20] {
        let began = Instant::now();
        let with = RunLimits {
            stdin: bytes(count),
            ..limits()
        };
        let ran = live.run_with(live.tool(&["cat"]), with).await.unwrap();
        assert!(ran.stdout == bytes(count), "{count}: {}", ran.stdout.len());
        println!(
            "  {count} bytes in and out: {} ms",
            began.elapsed().as_millis()
        );
    }
    let cut = RunLimits {
        max_output: 100_000,
        ..limits()
    };
    let ran = live
        .run_with(live.tool(&["out", "3000000"]), cut)
        .await
        .unwrap();
    assert!(ran.truncated && ran.stdout == bytes(100_000));
    assert_eq!(ran.code, Some(0), "it ended as it would have");

    let ran = live.run(live.tool(&["ends", "37"])).await;
    assert_eq!(ran.code, Some(37));
    assert_eq!(
        (ran.stdout, ran.stderr),
        (b"to-out\n".to_vec(), b"to-err\r\n".to_vec())
    );
    // With nothing to read it is at the end at once.
    assert_eq!(live.run(live.tool(&["cat"])).await.stdout, b"");
}

async fn words_and_places(live: &Live) {
    let words = [
        "a b",
        "q\"uo\"te",
        "back\\slash\\",
        "mix\\\"x",
        "$HOME `id`",
        "%PATH%",
        "new\nline",
        "åäö 漢字 🙂",
        "",
        "'single'",
        "*",
        "a;b|c&d>e<f^g!h",
        "--",
        "tab\there",
        "end\\",
    ];
    let mut asked = vec!["args"];
    asked.extend(words);
    let ran = live.run(live.tool(&asked)).await;
    let said = String::from_utf8(ran.stdout).unwrap();
    let hex = |word: &str| {
        word.bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    assert_eq!(said, words.map(hex).join("\n"));

    let mut asked = live.tool(&["env", "LUME_X"]);
    asked.env = vec![("LUME_X".to_owned(), "värde '1' $HOME".to_owned())];
    let ran = live.run(asked).await;
    assert_eq!(String::from_utf8(ran.stdout).unwrap(), "värde '1' $HOME");

    // Where it runs: a folder that is there, the home folder, one that is not there.
    let home = live.run(live.tool(&["cwd"])).await.stdout;
    let home = String::from_utf8(home).unwrap();
    println!("  at home in {home}");
    let folder = if live.windows { "C:\\Windows" } else { "/etc" };
    let mut asked = live.tool(&["cwd"]);
    asked.folder = Some(folder.to_owned());
    let ran = live.run(asked.clone()).await;
    assert_eq!(String::from_utf8(ran.stdout).unwrap(), folder);
    asked.folder = Some(format!("{folder}/no-such-folder"));
    let ended = live.run_with(asked, limits()).await;
    assert_eq!(ended, Err(RunError::NoFolder));
    let mut asked = live.tool(&[]);
    asked.program = "lume-no-such-program".to_owned();
    let ended = live.run_with(asked.clone(), limits()).await;
    assert_eq!(ended, Err(RunError::NoProgram));
    let started = live.backend.start(&asked).await.map(drop);
    assert_eq!(started, Err(RunError::NoProgram));
}

/// A program in the folder something is run in is not taken for the one of its name.
async fn names_are_looked_up_on_the_path_alone(live: &Live) {
    if !live.windows || !matches!(live.endpoint, Endpoint::Local | Endpoint::Pipe(_)) {
        println!("  (skipped: only for a server on this Windows machine)");
        return;
    }
    let folder = std::env::temp_dir().join(format!("lume-live-runs-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    for name in ["git.exe", "lume-live-own.exe"] {
        std::fs::copy("C:\\Windows\\System32\\whoami.exe", folder.join(name)).unwrap();
    }
    std::fs::write(folder.join("lume-live-own.cmd"), "@echo own\r\n").unwrap();
    let asked = |program: &str| RunAsked {
        program: program.to_owned(),
        args: vec!["--version".to_owned()],
        folder: Some(folder.to_string_lossy().into_owned()),
        env: Vec::new(),
    };
    let ran = live.run(asked("git")).await;
    let own = live.run_with(asked("lume-live-own"), limits()).await;
    std::fs::remove_dir_all(&folder).unwrap();
    assert!(
        ran.stdout.starts_with(b"git version"),
        "{:?}",
        String::from_utf8_lossy(&ran.stdout)
    );
    assert_eq!(own, Err(RunError::NoProgram));
}

async fn ends(live: &Live) {
    // Its time is up, with everything it started.
    let short = RunLimits {
        timeout: Duration::from_millis(1500),
        ..limits()
    };
    let began = Instant::now();
    let ran = live.run_with(live.tool(&["tree"]), short).await.unwrap();
    assert!(ran.timed_out && ran.code.is_none(), "{ran:?}");
    println!("  its time was up after {} ms", began.elapsed().as_millis());
    let processes = String::from_utf8(ran.stdout).unwrap();
    live.all_gone(&live.backend, &processes, Duration::from_secs(5))
        .await;

    // Nobody waits for it any more.
    let began = Instant::now();
    let tree = live.tool(&["tree"]);
    let running = live.backend.run(&tree, limits());
    assert!(timeout(Duration::from_secs(2), running).await.is_err());
    live.no_blocks_left().await;
    println!("  left alone after {} ms", began.elapsed().as_millis());

    // It is killed while its output is read.
    let started = live.backend.start(&live.tool(&["tree"])).await.unwrap();
    let (processes, rest) = first_line(started.stdout).await;
    assert_eq!(
        live.left(&live.backend, &processes).await.len(),
        2,
        "{processes}"
    );
    started.run.kill();
    assert_eq!(all_of(rest).await, b"");
    println!("  killed");
    live.all_gone(&live.backend, &processes, Duration::from_secs(5))
        .await;

    // What it started holds its output open: it ends when that does.
    let began = Instant::now();
    let ran = live.run(live.tool(&["held", "3000"])).await;
    assert_eq!((ran.code, ran.stdout.as_slice()), (Some(0), &b"done"[..]));
    println!(
        "  output held open for 3 s: {} ms",
        began.elapsed().as_millis()
    );
    live.no_blocks_left().await;
}

async fn read_as_it_comes(live: &Live) {
    let began = Instant::now();
    let started = live
        .backend
        .start(&live.tool(&["ticks", "4"]))
        .await
        .unwrap();
    let (line, rest) = first_line(started.stdout).await;
    let first = began.elapsed();
    assert_eq!(line, "tick 0");
    assert_eq!(all_of(rest).await, b"tick 1\ntick 2\ntick 3\n");
    assert!(began.elapsed() > first + Duration::from_millis(300));
    println!("  first line after {} ms", first.as_millis());
    let until = Instant::now() + Duration::from_secs(5);
    while started.run.try_wait().is_none() && Instant::now() < until {
        sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(started.run.try_wait(), Some(Some(0)));

    // Not read for a while, and then all of it.
    let count = 12 << 20;
    let asked = live.tool(&["out", &count.to_string()]);
    let started = live.backend.start(&asked).await.unwrap();
    sleep(Duration::from_secs(4)).await;
    assert_eq!(started.run.try_wait(), None, "it did not wait");
    let began = Instant::now();
    assert!(all_of(started.stdout).await == bytes(count));
    println!(
        "  {count} bytes, read after a pause: {} ms",
        began.elapsed().as_millis()
    );

    // Not read any more.
    let started = live.backend.start(&asked).await.unwrap();
    drop(started.stdout);
    let until = Instant::now() + Duration::from_secs(10);
    while started.run.try_wait().is_none() && Instant::now() < until {
        sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(started.run.try_wait(), Some(None));
    live.no_blocks_left().await;
}

async fn how_fast(live: &Live) {
    let git = RunAsked {
        program: "git".to_owned(),
        args: vec!["--version".to_owned()],
        ..RunAsked::default()
    };
    let mut took = Vec::new();
    for _ in 0..20 {
        let began = Instant::now();
        live.run(git.clone()).await;
        took.push(began.elapsed());
    }
    let (p50, p95) = median(took);
    println!("  git --version: {p50} ms, at most {p95} ms of 95 in 100");
    assert!(p50 < 300);
    let began = Instant::now();
    futures_util::future::join_all((0..8).map(|_| live.run(git.clone()))).await;
    println!("  8 at once: {} ms", began.elapsed().as_millis());
}

async fn nothing_is_seen_of_it(live: &Live) {
    let view = || async {
        let asked = json!({ "session_id": live.session });
        live.other
            .call("session.view", asked)
            .await
            .unwrap()
            .payload
    };
    let before = view().await;
    live.run(live.tool(&["out", "10"])).await;
    let started = live.backend.start(&live.tool(&["tree"])).await.unwrap();
    assert_eq!(view().await, before, "while it runs");
    assert_eq!(live.detached().await.len(), 1);
    started.run.kill();
    live.no_blocks_left().await;
    assert_eq!(view().await, before);
}

async fn this_side_goes(live: &Live) {
    // The connection ends with no word of the run.
    let gone = connected(&live.endpoint).await;
    gone.open_session(&live.session).await.unwrap();
    let started = gone.start(&live.tool(&["tree"])).await.unwrap();
    let (processes, _rest) = first_line(started.stdout).await;
    gone.shutdown();
    println!("  this side goes");
    live.all_gone(&live.backend, &processes, Duration::from_secs(15))
        .await;
    live.no_blocks_left().await;

    // The session ends under it.
    let made = session(&live.other, live.windows).await;
    let backend = connected(&live.endpoint).await;
    backend.open_session(&made).await.unwrap();
    let started = backend.start(&live.tool(&["tree"])).await.unwrap();
    let (processes, rest) = first_line(started.stdout).await;
    end(&live.other, &made).await;
    assert_eq!(all_of(rest).await, b"");
    println!("  its session ends");
    live.all_gone(&live.backend, &processes, Duration::from_secs(15))
        .await;
    backend.shutdown();
}

/// Makes a session with nothing of the user's in it.
async fn session(control: &Control, windows: bool) -> String {
    let command: &[&str] = if windows {
        &["cmd.exe", "/d"]
    } else {
        &["/bin/cat"]
    };
    session_of(control, command).await.unwrap()
}

/// Makes a session of one terminal with a program in it, if the program is there.
async fn session_of(control: &Control, command: &[&str]) -> Option<String> {
    let block = json!({
        "flavor": "com.superlogical.terminal.shell",
        "options": { "shell": "none", "command": command },
    });
    let asked = json!({ "label": LABEL, "initial_windows": [{ "layout": { "block": block } }] });
    let made = control.call("session.create", asked).await.ok()?;
    let session = made.payload["session_id"].as_str().unwrap().to_owned();
    let attach = json!({ "session_id": session });
    control.call("session.attach", attach).await.unwrap();
    Some(session)
}

async fn end(control: &Control, session: &str) {
    let asked = json!({ "session_id": session });
    control.call("session.destroy", asked).await.unwrap();
}

/// A shell, what is typed into it, and the folder it is in after each, where that is
/// known. `None` at the start stands for any folder there is.
type Walk<'a> = (&'a [&'a str], &'a [(&'a str, Option<&'a str>)]);

const OSC_9: &str = r#"function prompt { "$([char]27)]9;9;$($PWD.ProviderPath)$([char]7)> " }"#;

/// What another program prints, with a machine's name as an SSH client would show it.
const PRINTED_THERE: &str = r#"node -e "process.stdout.write('\x1b]7;file://box/C:/Temp\x07')""#;

fn walks(windows: bool) -> Vec<Walk<'static>> {
    if !windows {
        // The shell keeps no history of this.
        return vec![(
            &[
                "/usr/bin/env",
                "HISTFILE=/dev/null",
                "/bin/bash",
                "--norc",
                "--noprofile",
            ],
            &[
                ("", Some("/root")),
                ("cd /usr", Some("/usr")),
                (r"printf '\033]7;file://box/etc\007'", Some("/usr")),
                ("cd /tmp && sleep 3", Some("/tmp")),
            ],
        )];
    }
    vec![
        (
            &["cmd.exe", "/d"],
            &[
                (r"cd /d C:\Windows\System32", Some(r"C:\Windows\System32")),
                ("ping -n 4 127.0.0.1 >nul", Some(r"C:\Windows\System32")),
            ],
        ),
        (
            &["powershell.exe", "-NoLogo", "-NoProfile"],
            &[
                (r"cd C:\Windows", None),
                (OSC_9, Some(r"C:\Windows")),
                (r"cd C:\Windows\System32", Some(r"C:\Windows\System32")),
                (PRINTED_THERE, Some(r"C:\Windows\System32")),
                (
                    r"cmd /d /c 'cd /d C:\ & ping -n 4 127.0.0.1 >nul'",
                    Some(r"C:\"),
                ),
                // It says where it is each time it asks for a line.
                ("", Some(r"C:\Windows\System32")),
                (
                    r"function prompt { '> ' }; cd C:\Users",
                    Some(r"C:\Windows\System32"),
                ),
                // Printed by another, with nothing that says it anew: it is not known.
                (PRINTED_THERE, None),
            ],
        ),
        (
            &[
                r"C:\Program Files\Git\bin\bash.exe",
                "--norc",
                "--noprofile",
            ],
            &[
                ("cd /c/Windows", Some(r"C:\Windows")),
                ("cd /c/", Some(r"C:\")),
            ],
        ),
    ]
}

async fn walk(backend: &RexBackend, session: &str, (command, steps): Walk<'_>) {
    backend.open_session(session).await.unwrap();
    let id = backend.list().await.unwrap()[0].id;
    sleep(Duration::from_secs(3)).await;
    for (typed, wanted) in steps {
        if !typed.is_empty() {
            backend.send(id, &format!("{typed}\r")).await.unwrap();
        }
        let began = Instant::now();
        // What is not known must stay so, and is looked at late.
        sleep(Duration::from_millis(if wanted.is_none() {
            2_500
        } else {
            900
        }))
        .await;
        while backend.folder_of(id).as_deref() != *wanted {
            assert!(
                began.elapsed() < Duration::from_secs(8),
                "{command:?}: {typed:?} leaves it in {:?}, not {wanted:?}",
                backend.folder_of(id)
            );
            sleep(Duration::from_millis(100)).await;
        }
        // Asked for anew it is the same.
        let told = backend.folder_of(id);
        backend.list().await.unwrap();
        assert_eq!(backend.folder_of(id), told, "{command:?}: {typed:?}");
    }
    backend.close_session(session).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a Rex server, named in LUME_REX_LIVE_RUNS"]
async fn the_folder_a_terminal_is_in_is_followed() {
    let Ok(address) = std::env::var(ADDRESS) else {
        eprintln!("skipped: {ADDRESS} is not set");
        return;
    };
    let endpoint = Endpoint::parse(&address).unwrap();
    let backend = connected(&endpoint).await;
    let (other, _held) = another(&endpoint).await;
    let mut failed = Vec::new();
    for steps in walks(backend.on_windows()) {
        let Some(session) = session_of(&other, steps.0).await else {
            println!("not there: {:?}", steps.0);
            continue;
        };
        println!("{:?}", steps.0);
        let (walking, its) = (Arc::clone(&backend), session.clone());
        let walked = tokio::spawn(async move { walk(&walking, &its, steps).await }).await;
        end(&other, &session).await;
        failed.extend(walked.err());
    }
    backend.shutdown();
    other.close();
    assert!(failed.is_empty(), "{failed:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a Rex server, named in LUME_REX_LIVE_RUNS"]
async fn programs_are_run_on_the_machine_of_a_real_server() {
    let Ok(address) = std::env::var(ADDRESS) else {
        eprintln!("skipped: {ADDRESS} is not set");
        return;
    };
    let endpoint = Endpoint::parse(&address).unwrap();
    let backend = connected(&endpoint).await;
    let version = backend.ready().await.unwrap();
    println!("Rex {} on {}", version.rex_version, version.platform);
    let windows = version.platform.starts_with("windows");
    let (other, _held) = another(&endpoint).await;
    let session = session(&other, windows).await;
    backend.open_session(&session).await.unwrap();
    let live = Arc::new(Live {
        endpoint: endpoint.clone(),
        backend,
        other,
        session: session.clone(),
        windows,
    });
    let running = Arc::clone(&live);
    let done = tokio::spawn(async move {
        let live = &*running;
        println!("input and output");
        input_and_output(live).await;
        println!("words and places");
        words_and_places(live).await;
        println!("names");
        names_are_looked_up_on_the_path_alone(live).await;
        println!("ends");
        ends(live).await;
        println!("read as it comes");
        read_as_it_comes(live).await;
        println!("how fast");
        how_fast(live).await;
        println!("what is seen");
        nothing_is_seen_of_it(live).await;
        println!("this side goes");
        this_side_goes(live).await;
    })
    .await;
    end(&live.other, &session).await;
    live.backend.shutdown();
    live.other.close();
    sleep(Duration::from_millis(300)).await;
    // Over a connection of its own, since what is left would be left by this.
    let (other, _held) = another(&endpoint).await;
    let listed = other.call("session.list", Value::Null).await.unwrap();
    let ours = listed.payload["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|session| session["label"] == LABEL)
        .count();
    assert_eq!(ours, 0, "a session of this test is left");
    done.unwrap();
}
