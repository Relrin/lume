//! Programs run on a server's machine, against a pretend server that plays the script's
//! part: what is asked of it, what comes of it, and that nothing of a run is left.

mod common;

use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use common::programs::{Played, Programs, SCRIPT, request};
use common::{Listener, SESSION, Served, Tell, World, client, serve, streams, until};
use lume_rex::tuning::{
    RUN_CAME_WAIT, RUN_GRACE, RUN_INPUT_MAX_BYTES, RUN_PART_BYTES, RUN_SILENT, RUN_STREAMS_AT_ONCE,
    RUNS_AT_ONCE,
};
use lume_rex::{Ran, RexBackend, RunAsked, RunError, RunLimits, RunReader, RunStarted};
use serde_json::{Value, json};
use tokio::time::{Instant, timeout};

const SOON: Duration = Duration::from_secs(20);

/// More than any output here.
const WHOLE: usize = 16 * 1024 * 1024;

struct Setup {
    backend: Arc<RexBackend>,
    served: Served,
    listener: Listener,
    programs: Programs,
}

impl Setup {
    async fn new(programs: Programs) -> Self {
        let (dialer, mut listener) = streams();
        let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
        let world = World::recorded().running(&programs);
        let served = serve(listener.accept().await, world);
        backend.ready().await.unwrap();
        Self {
            backend,
            served,
            listener,
            programs,
        }
    }

    /// With the one session open.
    async fn opened(programs: Programs) -> Self {
        let setup = Self::new(programs).await;
        setup.backend.open_session(SESSION).await.unwrap();
        setup
    }

    async fn run(&self, asked: RunAsked, limits: RunLimits) -> Result<Ran, RunError> {
        self.backend.run(&asked, limits).await
    }

    fn closed(&self) -> Vec<String> {
        self.programs.seen(|seen| seen.closed.clone())
    }

    /// Runs something in the background.
    fn begin(&self, asked: RunAsked) -> tokio::task::JoinHandle<Result<Ran, RunError>> {
        let backend = Arc::clone(&self.backend);
        tokio::spawn(async move { backend.run(&asked, limits()).await })
    }
}

fn git(args: &[&str]) -> RunAsked {
    RunAsked {
        program: "git".to_owned(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        folder: Some("/home/me".to_owned()),
        env: vec![("GIT_PAGER".to_owned(), "cat".to_owned())],
    }
}

fn limits() -> RunLimits {
    RunLimits {
        stdin: Vec::new(),
        timeout: Duration::from_secs(30),
        max_output: 4 * 1024 * 1024,
    }
}

/// Every byte there is, again and again.
fn bytes(count: usize) -> Vec<u8> {
    (0..count).map(|nth| (nth * 7 + (nth >> 8)) as u8).collect()
}

/// What a program gives back that says what it was run as.
fn says_itself() -> Programs {
    Programs::new(|job| Played::says(job.words().join(" ")))
}

#[tokio::test]
async fn a_program_is_run_in_a_block_of_its_own_with_nothing_secret_in_its_arguments() {
    let setup = Setup::opened(says_itself()).await;
    let ran = setup.run(git(&["status"]), limits()).await.unwrap();
    assert_eq!(
        ran,
        Ran {
            code: Some(0),
            stdout: b"git status".to_vec(),
            stderr: Vec::new(),
            timed_out: false,
            truncated: false,
        }
    );

    let (made, secret, job) = setup.programs.seen(|seen| {
        let job = seen.jobs[0].told.clone();
        (seen.blocks[0].1.clone(), seen.secret(0), job)
    });
    assert_eq!(made["session_id"], SESSION);
    assert_eq!(made["label"], "Lume");
    let options = &made["options"];
    assert_eq!(options["shell"], "none");
    assert_eq!(
        options["exit"],
        json!({ "on_completion": true, "quick_exit_threshold_ms": 0 })
    );
    // Windows' terminals end a line as a keyboard does.
    assert_eq!(options["initial_input"], format!("{secret}\r"));
    assert!(secret.len() == 32 && secret.chars().all(|c| c.is_ascii_hexdigit()));
    let command: Vec<String> = serde_json::from_value(options["command"].clone()).unwrap();
    assert_eq!(
        command[..6],
        [
            "rex",
            "--autostart=false",
            "-S",
            "npipe:////./pipe/pretend",
            "do",
            "-e"
        ]
    );
    let script = &command[6];
    assert_eq!(command.len(), 7);
    assert!(script.contains("local CLIENT = \"client:") && !script.contains("@CLIENT@"));
    assert!(!command.iter().any(|word| word.contains(&secret)));

    assert_eq!(job["argv"], json!(["git", "status"]));
    assert_eq!(job["folder"], "/home/me");
    assert_eq!(job["env"], json!({ "GIT_PAGER": "cat" }));
    assert_eq!(job["timeout"], 30.0);
    assert_eq!(
        (&job["input"], &job["stream"]),
        (&json!(false), &json!(false))
    );
    assert_eq!(job["max"], 4 * 1024 * 1024 + 1);

    // Where a server's own program finds it is asked once for a connection.
    setup.run(git(&["log"]), limits()).await.unwrap();
    assert_eq!(setup.served.asked("server.status").len(), 1);
    let (first, second) = setup.programs.seen(|seen| (seen.secret(0), seen.secret(1)));
    assert_ne!(first, second);
    // A program that ended had its block go by itself.
    assert_eq!(setup.closed(), Vec::<String>::new());
    assert_eq!(setup.served.asked("block.close"), Vec::<Value>::new());
}

#[tokio::test]
async fn what_a_program_reads_and_writes_is_handed_on_to_the_byte() {
    let programs = Programs::new(|job| Played {
        out: job.input.iter().rev().copied().collect(),
        err: b"to-err\r\n\xff\x00".to_vec(),
        end: Some(json!({ "code": 37, "timed_out": false })),
    });
    let setup = Setup::opened(programs).await;
    for count in [1, 255, 256, RUN_PART_BYTES, RUN_PART_BYTES * 2 + 7] {
        let input = bytes(count);
        let with = RunLimits {
            stdin: input.clone(),
            ..limits()
        };
        let ran = setup.run(git(&[]), with).await.unwrap();
        let back: Vec<u8> = ran.stdout.iter().rev().copied().collect();
        assert!(back == input, "{count}");
        assert_eq!(ran.stderr, b"to-err\r\n\xff\x00");
        assert_eq!((ran.code, ran.truncated), (Some(37), false));
    }
}

#[tokio::test]
async fn output_is_kept_up_to_what_was_asked_for() {
    let programs = Programs::new(|job| Played {
        out: bytes(job.words()[1].parse().unwrap()),
        err: bytes(job.words()[2].parse().unwrap()),
        ..Played::code(0)
    });
    let setup = Setup::opened(programs).await;
    for (out, err, cut) in [
        ("1000", "1000", false),
        ("1001", "0", true),
        ("0", "1001", true),
        ("90000", "90000", true),
    ] {
        let with = RunLimits {
            max_output: 1000,
            ..limits()
        };
        let ran = setup.run(git(&[out, err]), with).await.unwrap();
        assert_eq!(ran.truncated, cut, "{out} {err}");
        let kept = |asked: &str| bytes(asked.parse::<usize>().unwrap().min(1000));
        assert!(ran.stdout == kept(out) && ran.stderr == kept(err));
    }
}

#[tokio::test]
async fn how_a_program_ended_is_told_apart() {
    let end = |word: &str| match word {
        "negative" => json!({ "code": 4_294_967_295_i64 }),
        "signal" => json!({ "code": -1, "signal": "SIGSEGV", "timed_out": false }),
        "late" => json!({ "code": -1, "signal": "SIGTERM", "timed_out": true }),
        "odd" => json!({ "code": "seven" }),
        "folder" => json!({ "failed": "git: working directory: gone", "folder": true }),
        "program" => json!({ "failed": "git: executable file not found in PATH", "folder": false }),
        "denied" => json!({ "failed": "fork/exec: permission\u{202e}\n denied", "folder": false }),
        "nothing" => json!({ "failed": {}, "folder": false }),
        _ => json!({ "code": 3 }),
    };
    let programs = Programs::new(move |job| Played {
        end: Some(end(&job.words()[1])),
        ..Played::says("so far")
    });
    let setup = Setup::opened(programs).await;
    let run = async |word: &str| setup.run(git(&[word]), limits()).await;
    let ended = async |word: &str| {
        let ran = run(word).await.unwrap();
        assert_eq!(ran.stdout, b"so far");
        (ran.code, ran.timed_out)
    };
    assert_eq!(ended("three").await, (Some(3), false));
    assert_eq!(ended("negative").await, (Some(-1), false));
    assert_eq!(ended("signal").await, (None, false));
    assert_eq!(ended("late").await, (None, true));
    assert_eq!(ended("odd").await, (None, false));
    assert_eq!(run("folder").await, Err(RunError::NoFolder));
    assert_eq!(run("program").await, Err(RunError::NoProgram));
    // A server's words are made plain.
    let said = RunError::Failed("fork/exec: permission  denied".to_owned());
    assert_eq!(run("denied").await, Err(said));
    assert_eq!(run("nothing").await, Err(RunError::Failed(String::new())));
}

#[tokio::test]
async fn programs_run_beside_each_other_and_only_so_many_at_once() {
    let setup = Setup::opened(Programs::new(|job| match job.words()[1].as_str() {
        "wait" => Played::stays(""),
        word => Played::says(word),
    }))
    .await;
    let answers = futures_util::future::join_all((0..RUNS_AT_ONCE * 2).map(|nth| {
        let setup = &setup;
        async move {
            let ran = setup.run(git(&[&nth.to_string()]), limits()).await.unwrap();
            assert_eq!(ran.stdout, nth.to_string().into_bytes());
        }
    }));
    timeout(SOON, answers).await.unwrap();

    let before = setup.programs.seen(|seen| seen.blocks.len());
    let waiting: Vec<_> = (0..RUNS_AT_ONCE)
        .map(|_| setup.begin(git(&["wait"])))
        .collect();
    let made = || setup.programs.seen(|seen| seen.blocks.len()) - before;
    until("they run", || made() == RUNS_AT_ONCE).await;
    let late = setup.begin(git(&["late"]));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(made(), RUNS_AT_ONCE);
    // Whoever stops waiting makes room, and leaves nothing running.
    waiting[0].abort();
    assert_eq!(
        timeout(SOON, late).await.unwrap().unwrap().unwrap().stdout,
        b"late"
    );
    for run in &waiting {
        run.abort();
    }
    until("their blocks are closed", || {
        setup.closed().len() == RUNS_AT_ONCE
    })
    .await;
}

#[tokio::test]
async fn a_run_nobody_waits_for_is_ended_however_far_it_got() {
    let setup = Setup::opened(Programs::new(|_| Played::stays("hello"))).await;
    // Before its block was made known.
    let run = setup.begin(git(&[]));
    until("it is asked for", || {
        !setup.programs.seen(|seen| seen.blocks.is_empty())
    })
    .await;
    run.abort();
    until("its block is closed", || setup.closed() == ["block:run-0"]).await;
    // Once it runs.
    let run = setup.begin(git(&[]));
    until("it runs", || {
        setup.programs.seen(|seen| seen.jobs.len()) == 2
    })
    .await;
    run.abort();
    until("its block is closed too", || {
        setup.closed().contains(&"block:run-1".to_owned())
    })
    .await;
}

#[tokio::test]
async fn a_run_ends_with_its_block_its_session_and_its_connection() {
    let mut setup = Setup::opened(Programs::new(|_| Played::stays(""))).await;
    let run = setup.begin(git(&[]));
    until("it runs", || {
        setup.programs.seen(|seen| seen.jobs.len()) == 1
    })
    .await;
    setup.served.tell(Tell::SessionEvent(
        SESSION.to_owned(),
        "block_closed".to_owned(),
        json!({ "block_id": "block:run-0" }),
    ));
    let error = timeout(SOON, run).await.unwrap().unwrap().unwrap_err();
    assert!(matches!(error, RunError::Failed(_)), "{error:?}");

    let run = setup.begin(git(&[]));
    until("another runs", || {
        setup.programs.seen(|seen| seen.jobs.len()) == 2
    })
    .await;
    let other = Programs::new(|_| Played::says("again"));
    setup.served.tell(Tell::Drop);
    let error = timeout(SOON, run).await.unwrap().unwrap().unwrap_err();
    assert_eq!(error, RunError::NotConnected);
    assert_eq!(
        setup.run(git(&[]), limits()).await,
        Err(RunError::NotConnected)
    );

    // Connected again, what was left there of it is closed; what is the user's is not.
    let world = World::recorded().running(&other);
    let served = serve(setup.listener.accept().await, world);
    until("the block is closed", || {
        !served.asked("block.close").is_empty()
    })
    .await;
    assert_eq!(
        served.asked("block.close"),
        [json!({ "session_id": SESSION, "block_id": "block:run-1" })]
    );
    setup.backend.ready().await.unwrap();
    let ran = setup.run(git(&[]), limits()).await.unwrap();
    assert_eq!(ran.stdout, b"again");
    // It is not closed a second time.
    assert_eq!(served.asked("block.close").len(), 1);

    setup.backend.shutdown();
    assert_eq!(
        setup.run(git(&[]), limits()).await,
        Err(RunError::NotConnected)
    );
}

#[tokio::test]
async fn a_program_that_ends_at_once_with_no_word_is_not_waited_for() {
    let mut programs = says_itself();
    programs.closes_early = true;
    let setup = Setup::opened(programs).await;
    let ended = timeout(Duration::from_secs(5), setup.run(git(&[]), limits())).await;
    let error = ended.expect("it is waited for").unwrap_err();
    assert!(matches!(error, RunError::Failed(_)), "{error:?}");
}

#[tokio::test]
async fn nothing_is_run_where_no_session_is_open_or_the_server_cannot() {
    let setup = Setup::new(says_itself()).await;
    let nowhere = setup.run(git(&[]), limits()).await;
    assert_eq!(nowhere, Err(RunError::NotConnected));
    setup.backend.open_session(SESSION).await.unwrap();
    setup.run(git(&[]), limits()).await.unwrap();
    setup.backend.close_session(SESSION).await.unwrap();
    let left = setup.run(git(&[]), limits()).await;
    assert_eq!(left, Err(RunError::NotConnected));
    assert_eq!(setup.programs.seen(|seen| seen.blocks.len()), 1);

    for primary in [None, Some(""), Some("--listen"), Some("unix:///a\u{202e}b")] {
        let mut programs = says_itself();
        programs.primary = primary.map(str::to_owned);
        let setup = Setup::opened(programs).await;
        let error = setup.run(git(&[]), limits()).await.unwrap_err();
        assert!(matches!(error, RunError::CannotRun(_)), "{primary:?}");
        let error = setup.backend.start(&git(&[])).await.map(drop).unwrap_err();
        assert!(matches!(error, RunError::CannotRun(_)), "{primary:?}");
        assert!(setup.programs.seen(|seen| seen.blocks.is_empty()));
    }
}

#[tokio::test]
async fn what_is_too_large_to_send_is_refused_before_anything_is_made() {
    let setup = Setup::opened(says_itself()).await;
    let long = "x".repeat(600 * 1024);
    let error = setup.run(git(&[&long]), limits()).await.unwrap_err();
    assert!(matches!(error, RunError::TooLarge(_)));
    let with = RunLimits {
        stdin: vec![0; RUN_INPUT_MAX_BYTES + 1],
        ..limits()
    };
    let error = setup.run(git(&[]), with).await.unwrap_err();
    assert!(matches!(error, RunError::TooLarge(_)));
    assert!(setup.programs.seen(|seen| seen.blocks.is_empty()));
}

#[tokio::test]
async fn what_would_be_something_else_in_the_script_is_not_put_into_it() {
    for id in [
        "client:\"..os.exit()..\"",
        "client:
",
        "",
        "client:é",
    ] {
        let (dialer, mut listener) = streams();
        let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
        let mut welcome = common::Recorded::load().welcome();
        welcome["client"]["id"] = json!(id);
        let conn = common::Conn::accept(listener.stream().await, welcome).await;
        let programs = says_itself();
        let _served = serve(conn, World::recorded().running(&programs));
        backend.ready().await.unwrap();
        backend.open_session(SESSION).await.unwrap();
        let error = backend.run(&git(&[]), limits()).await.unwrap_err();
        assert!(matches!(error, RunError::CannotRun(_)), "{id:?}: {error:?}");
        assert!(programs.seen(|seen| seen.blocks.is_empty()));
    }
}

#[tokio::test]
async fn a_server_that_takes_less_is_sent_less_at_a_time() {
    let (dialer, mut listener) = streams();
    let backend = RexBackend::over(common::backend_id(), "test".to_owned(), dialer, client());
    let mut welcome = common::Recorded::load().welcome();
    welcome["limits"]["max_message_bytes"] = json!(65_536);
    let conn = common::Conn::accept(listener.stream().await, welcome).await;
    let programs = Programs::new(|job| Played::says(job.input.clone()));
    let _served = serve(conn, World::recorded().running(&programs));
    backend.ready().await.unwrap();
    backend.open_session(SESSION).await.unwrap();
    let with = RunLimits {
        // Written as JSON each of these is six bytes.
        stdin: vec![0; 70_000],
        ..limits()
    };
    let error = backend.run(&git(&[]), with).await.unwrap_err();
    // The pretend script sends longer parts than it was told it may.
    assert!(matches!(error, RunError::Failed(_)), "{error:?}");
    let job = programs.seen(|seen| seen.jobs[0].clone());
    assert_eq!(job.input.len(), 70_000);
    assert_eq!(job.told["part"], 65_536 / 8);
    let error = backend
        .run(&git(&[&"x".repeat(64 * 1024)]), limits())
        .await
        .unwrap_err();
    assert!(matches!(error, RunError::TooLarge(_)));
}

#[tokio::test(start_paused = true)]
async fn a_script_that_does_not_come_or_falls_silent_is_not_waited_for() {
    let mut never = says_itself();
    never.comes = false;
    let setup = Setup::opened(never).await;
    let asked = Instant::now();
    let ended = timeout(RUN_CAME_WAIT * 4, setup.run(git(&[]), limits())).await;
    let error = ended.expect("it is waited for").unwrap_err();
    assert!(matches!(error, RunError::Failed(_)), "{error:?}");
    let took = asked.elapsed();
    assert!(
        (RUN_CAME_WAIT..RUN_CAME_WAIT * 2).contains(&took),
        "{took:?}"
    );
    until("its block is closed", || setup.closed().len() == 1).await;

    let setup = Setup::opened(Programs::new(|_| Played::stays(""))).await;
    let asked = Instant::now();
    let error = setup.run(git(&[]), limits()).await.unwrap_err();
    assert!(matches!(error, RunError::Failed(_)), "{error:?}");
    let took = asked.elapsed();
    assert!(
        (RUN_SILENT..RUN_SILENT + Duration::from_secs(5)).contains(&took),
        "{took:?}"
    );
    until("its block is closed", || setup.closed().len() == 1).await;
}

#[tokio::test(start_paused = true)]
async fn a_program_the_server_does_not_end_in_time_is_ended_from_here() {
    let setup = Setup::opened(Programs::new(|_| Played::stays(""))).await;
    let short = RunLimits {
        timeout: Duration::from_secs(25),
        ..limits()
    };
    let asked = Instant::now();
    let running = setup.run(git(&[]), short);
    let beats = async {
        // It says that it is there, and goes on.
        for nth in 0..1_000 {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let Some(secret) = setup
                .programs
                .seen(|seen| (!seen.blocks.is_empty()).then(|| seen.secret(0)))
            else {
                continue;
            };
            let args = json!({ "secret": secret });
            let asked = request(&format!("beat-{nth}"), SCRIPT, "lume.run.alive", args);
            setup
                .served
                .tell(Tell::Event("client_perform_requested".to_owned(), asked));
        }
    };
    let error = tokio::select! {
        ran = running => ran.unwrap_err(),
        () = beats => unreachable!(),
    };
    assert!(matches!(error, RunError::Failed(_)), "{error:?}");
    let most = RUN_CAME_WAIT + Duration::from_secs(25) + RUN_GRACE;
    let took = asked.elapsed();
    assert!(
        (most..most + Duration::from_secs(3)).contains(&took),
        "{took:?}"
    );
    until("its block is closed", || setup.closed().len() == 1).await;
}

/// Reads an output to its end, or to so many bytes. One that never comes fails the test:
/// the thread it is read on is not one that is waited for.
async fn read(mut from: RunReader, most: usize) -> (Vec<u8>, RunReader) {
    let (tell, told) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let mut got = vec![0; most];
        let mut at = 0;
        while at < most {
            match from.read(&mut got[at..]).unwrap() {
                0 => break,
                read => at += read,
            }
        }
        got.truncate(at);
        let _ = tell.send((got, from));
    });
    timeout(SOON, told).await.expect("it ends").unwrap()
}

#[tokio::test]
async fn a_started_programs_output_is_read_as_it_comes_and_its_end_is_known() {
    let programs = Programs::new(|job| match job.words()[1].as_str() {
        "stays" => Played::stays("first"),
        "missing" => Played::failed("git: executable file not found in PATH", false),
        _ => Played {
            out: bytes(200_000),
            err: b"words".to_vec(),
            ..Played::code(4)
        },
    });
    let setup = Setup::opened(programs).await;
    let start = async |word: &str| setup.backend.start(&git(&[word])).await;

    let RunStarted {
        stdout,
        stderr,
        run,
    } = start("log").await.unwrap();
    assert!(read(stdout, WHOLE).await.0 == bytes(200_000));
    assert_eq!(read(stderr, 100).await.0, b"words");
    until("its end is known", || run.try_wait() == Some(Some(4))).await;
    let job = setup.programs.seen(|seen| seen.jobs[0].told.clone());
    assert_eq!(
        (&job["stream"], &job["timeout"]),
        (&json!(true), &Value::Null)
    );

    assert_eq!(start("missing").await.map(drop), Err(RunError::NoProgram));

    // One that runs on is there until it is killed, and its output ends then.
    let started = start("stays").await.unwrap();
    let (got, rest) = read(started.stdout, 5).await;
    assert_eq!(got, b"first");
    assert_eq!(started.run.try_wait(), None);
    started.run.kill();
    assert_eq!(read(rest, 10).await.0, b"");
    until("it has ended", || started.run.try_wait() == Some(None)).await;
    until("its block is closed", || setup.closed() == ["block:run-2"]).await;

    // What is not held any more leaves the program running.
    let started = start("stays").await.unwrap();
    let (_, _stdout) = read(started.stdout, 5).await;
    drop((started.run, started.stderr));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(setup.closed().len(), 1);
}

#[tokio::test]
async fn output_that_nobody_reads_has_the_program_wait() {
    let whole = bytes(6 * 1024 * 1024);
    let given = whole.clone();
    let programs = Programs::new(move |job| match job.words()[1].as_str() {
        "log" => Played::says(given.clone()),
        _ => Played::says("other"),
    });
    let setup = Setup::opened(programs).await;
    let started = setup.backend.start(&git(&["log"])).await.unwrap();
    until("the program waits", || {
        setup.programs.seen(|seen| seen.held > 10)
    })
    .await;
    // Meanwhile other programs are run.
    let ran = timeout(SOON, setup.run(git(&["other"]), limits())).await;
    assert_eq!(ran.unwrap().unwrap().stdout, b"other");
    assert_eq!(started.run.try_wait(), None);
    assert!(read(started.stdout, WHOLE).await.0 == whole);
    until("it ends", || started.run.try_wait() == Some(Some(0))).await;

    // Output that nobody can read any more ends the program.
    let started = setup.backend.start(&git(&["log"])).await.unwrap();
    drop(started.stdout);
    until("it has ended", || started.run.try_wait() == Some(None)).await;
    assert!(setup.programs.seen(|seen| !seen.refused.is_empty()));
}

#[tokio::test]
async fn only_so_many_are_read_at_once_and_they_take_no_place_of_the_others() {
    let setup = Setup::opened(Programs::new(|job| match job.words()[1].as_str() {
        "log" => Played::stays(""),
        _ => Played::says("other"),
    }))
    .await;
    let mut started = Vec::new();
    for _ in 0..RUN_STREAMS_AT_ONCE {
        started.push(setup.backend.start(&git(&["log"])).await.unwrap());
    }
    let more = setup.backend.start(&git(&["log"])).await.map(drop);
    assert_eq!(more, Err(RunError::Busy));
    setup.run(git(&["other"]), limits()).await.unwrap();
    started.pop().unwrap().run.kill();
    let backend = &setup.backend;
    let again = async {
        loop {
            match backend.start(&git(&["log"])).await {
                Ok(started) => return started,
                Err(error) => assert_eq!(error, RunError::Busy),
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    timeout(SOON, again).await.unwrap();
}

#[tokio::test]
async fn a_client_asked_for_something_else_says_that_it_does_not_do_that() {
    let setup = Setup::opened(Programs::new(|_| Played::stays(""))).await;
    for (nth, action) in ["pane.zoom", "lume.other", "", "lume.run."]
        .iter()
        .enumerate()
    {
        let asked = request(&format!("ask-{nth}"), "client:other", action, json!({}));
        setup
            .served
            .tell(Tell::Event("client_perform_requested".to_owned(), asked));
    }
    until("each is answered", || {
        setup.served.asked("client.reply").len() == 4
    })
    .await;
    let replies = setup.served.asked("client.reply");
    for (nth, reply) in replies.iter().enumerate() {
        assert_eq!(reply["call_id"], format!("ask-{nth}"));
        let code = if nth == 3 { "refused" } else { "not_supported" };
        assert_eq!(reply["error"]["code"], code, "{reply}");
        assert!(reply.get("result").is_none());
    }
}
