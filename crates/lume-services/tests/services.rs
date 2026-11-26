//! What the services do once a call is allowed: files read, written, listed and watched,
//! programs run and stopped with what they started, and fetches made and followed.
//!
//! Nothing here reaches past this computer.

mod common;

use std::io::Read as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Layout, Server, answer, get, grants, redirect};
use lume_services::fs::{self, Access, EntryKind, WatchKind, WriteMode};
use lume_services::net::{self, FetchRequest};
use lume_services::process::{self, Program, RunOptions, SpawnOptions};
use lume_services::{ErrorKind, procs};

// Files.

#[test]
fn a_file_is_replaced_appended_to_and_made_only_once() {
    let layout = Layout::new("write-modes");
    let all = grants(&["${workspace}/**"]);
    let places = &layout.places;
    let path = "${workspace}/notes.txt";
    let made = fs::write(places, &all, path, b"one", WriteMode::Replace).unwrap();
    assert_eq!(made.grant, "${workspace}/**");
    fs::write(places, &all, path, b"+two", WriteMode::Append).unwrap();
    assert_eq!(fs::read(places, &all, path, 100).unwrap().value, b"one+two");
    fs::write(places, &all, path, b"three", WriteMode::Replace).unwrap();
    assert_eq!(fs::read(places, &all, path, 100).unwrap().value, b"three");
    let again = fs::write(places, &all, path, b"x", WriteMode::CreateNew).unwrap_err();
    assert_eq!(again.kind, ErrorKind::Failed);
    fs::write(
        places,
        &all,
        "${workspace}/fresh.txt",
        b"new",
        WriteMode::CreateNew,
    )
    .unwrap();
    assert_eq!(std::fs::read(layout.path("ws/fresh.txt")).unwrap(), b"new");
    fs::write(
        places,
        &all,
        "${workspace}/log.txt",
        b"first",
        WriteMode::Append,
    )
    .unwrap();
    assert_eq!(std::fs::read(layout.path("ws/log.txt")).unwrap(), b"first");
    // A file in a folder that is not there is not found; the folder is not made.
    assert_eq!(
        fs::write(
            places,
            &all,
            "${workspace}/missing/x.txt",
            b"x",
            WriteMode::Replace
        )
        .unwrap_err()
        .kind,
        ErrorKind::NotFound
    );
}

#[test]
fn a_file_past_the_limit_is_too_large() {
    let layout = Layout::new("read-limit");
    layout.dir.write("ws/big.bin", &[7u8; 64]);
    let all = grants(&["${workspace}/**"]);
    let places = &layout.places;
    assert_eq!(
        fs::read(places, &all, "${workspace}/big.bin", 64)
            .unwrap()
            .value
            .len(),
        64
    );
    assert_eq!(
        fs::read(places, &all, "${workspace}/big.bin", 63)
            .unwrap_err()
            .kind,
        ErrorKind::TooLarge
    );
    let folder = fs::read(places, &all, "${workspace}", 63).unwrap_err();
    assert_eq!(folder.kind, ErrorKind::Invalid);
    let mut file = fs::open_read(places, &all, "${workspace}/big.bin")
        .unwrap()
        .value;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes.len(), 64);
}

#[test]
fn a_check_gives_the_canonical_path_and_the_grant() {
    let layout = Layout::new("check");
    layout.dir.write("ws/src/main.rs", b"fn main() {}");
    let sources = grants(&["${workspace}/README.md", "${workspace}/src/**/*.rs"]);
    let checked = fs::check(
        &layout.places,
        &sources,
        "${workspace}/src/main.rs",
        Access::Read,
    )
    .unwrap();
    assert_eq!(checked.grant, "${workspace}/src/**/*.rs");
    assert_eq!(
        checked.value,
        std::fs::canonicalize(layout.path("ws/src/main.rs"))
            .map(|path| {
                let text = path.display().to_string();
                std::path::PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
            })
            .unwrap()
    );
    // A file not made yet is checked by its folder and its name.
    let new = fs::check(
        &layout.places,
        &sources,
        "${workspace}/src/new.rs",
        Access::Write,
    );
    assert_eq!(new.unwrap().grant, "${workspace}/src/**/*.rs");
    let wrong = fs::check(
        &layout.places,
        &sources,
        "${workspace}/src/new.txt",
        Access::Write,
    );
    assert_eq!(wrong.unwrap_err().kind, ErrorKind::Denied);
}

#[test]
fn a_folder_lists_only_what_the_grants_cover() {
    let layout = Layout::new("list");
    layout.dir.write("ws/b.md", b"bb");
    layout.dir.write("ws/a.md", b"a");
    layout.dir.write("ws/secret.txt", b"secret");
    layout.dir.folder("ws/docs.md");
    let notes = grants(&["${workspace}", "${workspace}/*.md"]);
    let listed = fs::list(&layout.places, &notes, "${workspace}", 10)
        .unwrap()
        .value;
    let names: Vec<(&str, EntryKind, u64)> = listed
        .iter()
        .map(|entry| (entry.name.as_str(), entry.kind, entry.size))
        .collect();
    assert_eq!(
        names,
        [
            ("a.md", EntryKind::File, 1),
            ("b.md", EntryKind::File, 2),
            ("docs.md", EntryKind::Dir, 0)
        ]
    );
    let capped = fs::list(&layout.places, &notes, "${workspace}", 2).unwrap_err();
    assert_eq!(capped.kind, ErrorKind::TooLarge);
}

#[test]
fn a_file_and_a_folder_are_described() {
    let layout = Layout::new("stat");
    layout.dir.write("ws/a.txt", b"abc");
    let all = grants(&["${workspace}/**"]);
    let file = fs::stat(&layout.places, &all, "${workspace}/a.txt")
        .unwrap()
        .value;
    assert_eq!(
        (file.kind, file.size, file.readonly),
        (EntryKind::File, 3, false)
    );
    assert!(file.modified_ms.is_some_and(|ms| ms > 1_600_000_000_000));
    let folder = fs::stat(&layout.places, &all, "${workspace}")
        .unwrap()
        .value;
    assert_eq!((folder.kind, folder.size), (EntryKind::Dir, 0));
    let json = serde_json::to_value(&file).unwrap();
    assert_eq!(json["kind"], "file");
}

#[test]
fn a_watch_tells_of_files_made_changed_and_removed() {
    let layout = Layout::new("watch-events");
    let all = grants(&["${workspace}/**"]);
    let (tell, told) = std::sync::mpsc::channel();
    let watcher = fs::watch(
        &layout.places,
        &all,
        "${workspace}",
        true,
        Box::new(move |event| {
            let _ = tell.send(event);
        }),
    )
    .unwrap();
    let path = layout.dir.write("ws/a.txt", b"one");
    std::fs::remove_file(&path).unwrap();
    let mut kinds = Vec::new();
    let until = Instant::now() + Duration::from_secs(5);
    while !kinds.contains(&WatchKind::Removed) {
        let Ok(event) = told.recv_timeout(until.saturating_duration_since(Instant::now())) else {
            break;
        };
        assert!(event.path.ends_with("a.txt"), "{}", event.path);
        kinds.push(event.kind);
    }
    assert!(kinds.contains(&WatchKind::Created), "{kinds:?}");
    assert!(kinds.contains(&WatchKind::Removed), "{kinds:?}");
    drop(watcher);
}

// Programs.

/// The system's shell, and the arguments that have it run a line.
fn shell(layout: &Layout) -> (Program, Vec<&'static str>) {
    let (name, args) = if cfg!(windows) {
        ("cmd", vec!["/d", "/c"])
    } else {
        ("sh", vec!["-c"])
    };
    let program = process::resolve(&layout.places, name, Some(&common::system_path())).unwrap();
    (program, args)
}

fn options(timeout: Duration) -> RunOptions {
    RunOptions {
        cwd: None,
        env: Vec::new(),
        stdin: None,
        timeout,
        max_output: 64 * 1024,
        stop: Arc::default(),
    }
}

fn line(args: &[&str], line: &str) -> Vec<String> {
    let mut all: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    all.push(line.to_owned());
    all
}

#[test]
fn a_program_runs_with_its_input_variables_and_folder() {
    let layout = Layout::new("run");
    let (program, args) = shell(&layout);
    assert!(program.path.is_absolute());
    let script = if cfg!(windows) {
        "echo %LUME_TEST%& cd& exit /b 3"
    } else {
        "echo $LUME_TEST; pwd; exit 3"
    };
    let output = process::run(
        &program,
        &line(&args, script),
        RunOptions {
            cwd: Some(layout.path("ws")),
            env: vec![("LUME_TEST".to_owned(), "hello".to_owned())],
            ..options(Duration::from_secs(10))
        },
    )
    .unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.code, Some(3), "{text}");
    assert!(text.contains("hello"), "{text}");
    assert!(text.contains("ws"), "{text}");
    assert!(!output.timed_out && !output.truncated);

    // An odd number of bytes: Windows' `sort` takes input it can read as UTF-16 for
    // UTF-16, and whether it does with an even number depends on the machine.
    let sorted = process::run(
        &program,
        &line(&args, "sort"),
        RunOptions {
            stdin: Some(b"beta\nalpha\n".to_vec()),
            ..options(Duration::from_secs(10))
        },
    )
    .unwrap();
    let text = String::from_utf8_lossy(&sorted.stdout).replace("\r\n", "\n");
    assert_eq!(text, "alpha\nbeta\n");
}

#[test]
fn output_past_the_limit_is_cut() {
    let layout = Layout::new("run-cut");
    let (program, args) = shell(&layout);
    let script = if cfg!(windows) {
        "for /l %i in (1,1,500) do @echo line %i"
    } else {
        "i=0; while [ $i -lt 500 ]; do echo line $i; i=$((i+1)); done"
    };
    let output = process::run(
        &program,
        &line(&args, script),
        RunOptions {
            max_output: 100,
            ..options(Duration::from_secs(20))
        },
    )
    .unwrap();
    assert_eq!(output.stdout.len(), 100);
    assert!(output.truncated);
    assert_eq!(output.code, Some(0));
}

/// A line that starts a program of its own that runs for half a minute, writing to the
/// same output.
fn long_line() -> &'static str {
    if cfg!(windows) {
        "ping -n 30 127.0.0.1"
    } else {
        "sleep 30; echo done"
    }
}

#[test]
fn a_program_past_its_timeout_is_stopped_with_what_it_started() {
    let layout = Layout::new("run-timeout");
    let (program, args) = shell(&layout);
    let started = Instant::now();
    let output = process::run(
        &program,
        &line(&args, long_line()),
        options(Duration::from_millis(700)),
    )
    .unwrap();
    assert!(output.timed_out);
    assert_eq!(output.code, None);
    // The program it started held the output too; it was stopped with it, so the output
    // ended at once rather than after the grace for what is left in the pipes.
    assert!(
        !output.truncated,
        "the started program kept the output open"
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_program_that_is_no_longer_wanted_is_stopped_with_what_it_started() {
    let layout = Layout::new("run-stop");
    let (program, args) = shell(&layout);
    let options = options(Duration::from_secs(20));
    let stop = Arc::clone(&options.stop);
    let started = Instant::now();
    let asker = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        stop.ask();
    });
    let output = process::run(&program, &line(&args, long_line()), options).unwrap();
    asker.join().unwrap();
    assert_eq!((output.code, output.timed_out), (None, false));
    assert!(
        !output.truncated,
        "the started program kept the output open"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn a_program_that_ends_while_something_it_started_holds_its_output_is_not_waited_past_its_end() {
    let layout = Layout::new("run-held");
    let (program, args) = shell(&layout);
    // What it starts writes to the same output for a few seconds more.
    let script = if cfg!(windows) {
        "start /b ping -n 5 127.0.0.1 & exit /b 4"
    } else {
        "(sleep 4; echo late) & exit 4"
    };
    let started = Instant::now();
    let output = process::run(
        &program,
        &line(&args, script),
        options(Duration::from_secs(20)),
    )
    .unwrap();
    // Its end is seen though its output has not ended, and what is still to be written
    // is waited for no longer than the grace.
    assert_eq!(output.code, Some(4));
    assert!(!output.timed_out);
    assert!(output.truncated, "the output was still open");
    assert!(
        started.elapsed() < process::OUTPUT_GRACE + Duration::from_millis(1500),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_spawned_program_is_measured_and_stopped_with_what_it_started() {
    let layout = Layout::new("spawn");
    let (program, args) = shell(&layout);
    let mut spawned =
        process::spawn(&program, &line(&args, long_line()), SpawnOptions::default()).unwrap();
    let id = spawned.child.id();
    let until = Instant::now() + Duration::from_secs(10);
    let usage = loop {
        let usage = procs::tree_usage(id).expect("the program is there");
        if usage.processes >= 2 || Instant::now() > until {
            break usage;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(usage.processes >= 2, "{usage:?}");
    assert!(usage.memory_bytes > 0);
    assert_eq!(spawned.child.try_wait(), None);
    spawned.child.kill().unwrap();
    assert_eq!(
        spawned.child.wait(),
        if cfg!(windows) { Some(1) } else { None }
    );
    // With everything stopped the output ends.
    let mut stdout = spawned.stdout.take().unwrap();
    let (tell, ended) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut rest = Vec::new();
        let _ = stdout.read_to_end(&mut rest);
        let _ = tell.send(());
    });
    assert!(ended.recv_timeout(Duration::from_secs(5)).is_ok());
    assert_eq!(procs::tree_usage(id), None);
    spawned.child.kill().unwrap();
}

// The network.

#[test]
fn a_fetch_sends_the_method_headers_and_body() {
    let server = Server::new(vec![answer("201 Created", &["X-Answer: yes"], b"made")]);
    let granted = grants(&[&format!("127.0.0.1:{}", server.port)]);
    let request = FetchRequest {
        method: "post".to_owned(),
        headers: vec![("Content-Type".to_owned(), "text/plain".to_owned())],
        body: Some(b"payload".to_vec()),
        ..get(&server.url("/items?x=1"))
    };
    let response = net::fetch(&granted, request).unwrap().value;
    assert_eq!(response.head.status, 201);
    assert_eq!(response.body, b"made");
    assert!(
        response
            .head
            .headers
            .iter()
            .any(|(name, value)| name == "x-answer" && value == "yes")
    );
    let got = &server.got()[0];
    assert!(
        got.head.starts_with("POST /items?x=1 HTTP/1.1"),
        "{}",
        got.head
    );
    assert_eq!(got.body, b"payload");
    assert_eq!(got.header("user-agent").as_deref(), Some("Lume"));
    assert_eq!(got.header("content-type").as_deref(), Some("text/plain"));
}

#[test]
fn a_redirect_is_followed_and_a_post_becomes_a_get_after_see_other() {
    let server = Server::new(vec![
        redirect("303 See Other", "/done"),
        answer("200 OK", &[], b"done"),
    ]);
    let granted = grants(&[&format!("127.0.0.1:{}", server.port)]);
    let request = FetchRequest {
        method: "POST".to_owned(),
        headers: vec![("Content-Type".to_owned(), "text/plain".to_owned())],
        body: Some(b"form".to_vec()),
        ..get(&server.url("/submit"))
    };
    let response = net::fetch(&granted, request).unwrap().value;
    assert_eq!(response.body, b"done");
    assert_eq!(response.head.url, server.url("/done"));
    let got = server.got();
    assert!(got[1].head.starts_with("GET /done "), "{}", got[1].head);
    assert!(got[1].body.is_empty());
    assert_eq!(got[1].header("content-type"), None);
}

#[test]
fn a_response_is_streamed() {
    let body: Vec<u8> = (0..20_000u32).map(|n| (n % 251) as u8).collect();
    let server = Server::new(vec![answer("200 OK", &[], &body)]);
    let granted = grants(&[&format!("127.0.0.1:{}", server.port)]);
    let (head, mut reader) = net::fetch_stream(&granted, get(&server.url("/")))
        .unwrap()
        .value;
    assert_eq!(head.status, 200);
    let mut read = Vec::new();
    reader.read_to_end(&mut read).unwrap();
    assert_eq!(read, body);
}

#[test]
fn a_server_that_does_not_answer_in_time_times_out() {
    // A listener that takes the connection and never answers.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let holder = std::thread::spawn(move || {
        let held = listener.accept();
        std::thread::sleep(Duration::from_secs(2));
        drop(held);
    });
    let granted = grants(&[&format!("127.0.0.1:{port}")]);
    let request = FetchRequest {
        timeout: Duration::from_millis(300),
        ..get(&format!("http://127.0.0.1:{port}/"))
    };
    let error = net::fetch(&granted, request).unwrap_err();
    assert_eq!(error.kind, ErrorKind::TimedOut, "{}", error.message);
    holder.join().unwrap();
}
