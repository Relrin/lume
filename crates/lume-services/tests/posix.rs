//! Programs run by the `sh` of another machine, against a real one.
//!
//! On Linux and macOS it is this machine's own, reached as the SSH client reaches one: by
//! a program that can be killed and leaves the other side to find that its input ended.
//! It is given the words of a command once, and once one line that a shell reads. On
//! Windows it is that of the Linux distribution that `LUME_TEST_DISTRO` names, reached
//! directly and in the same way; without it the tests do not run.

use std::io::Read as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lume_services::posix::{Asked, Door, FOLDER_MISSING, Limits, PROGRAM_MISSING, Shell};
use lume_services::process::{RunOutput, Running, Stop};
use lume_services::{ErrorKind, ServiceError};

/// What starts the rest of its words away from here: in a session of its own, which a
/// signal to whoever started it does not reach, with the same input.
const AWAY: &str = "exec 3<&0; perl -e 'use POSIX; POSIX::setsid(); exec(@ARGV)' \
     \"$@\" <&3 & p=$!; exec 3<&-; wait $p";

/// The doors to a shell there are here.
fn doors() -> Vec<Door> {
    let door = |program: PathBuf, args: &[&str], one_line| Door {
        program,
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        env: Vec::new(),
        one_line,
        label: "box".to_owned(),
    };
    if cfg!(windows) {
        let Ok(distro) = std::env::var("LUME_TEST_DISTRO") else {
            eprintln!("LUME_TEST_DISTRO names no distribution: nothing is tested");
            return Vec::new();
        };
        let program = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32")
            .join("wsl.exe");
        let near = ["-d", &distro, "--cd", "/", "--exec"];
        let far = [&near[..], &["sh", "-c", AWAY, "away"]].concat();
        return vec![
            door(program.clone(), &near, false),
            door(program, &far, false),
        ];
    }
    let sh = || PathBuf::from("/bin/sh");
    vec![
        door(sh(), &["-c", AWAY, "away"], false),
        door(sh(), &["-c", AWAY, "away", "sh", "-c"], true),
    ]
}

fn limits() -> Limits {
    Limits {
        stdin: Vec::new(),
        timeout: Duration::from_secs(30),
        max_output: 16 * 1024 * 1024,
        stop: Arc::new(Stop::default()),
    }
}

fn asked(program: &str, args: &[&str]) -> Asked {
    Asked {
        program: program.to_owned(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        folder: Some("/".to_owned()),
        env: Vec::new(),
    }
}

/// What a line of `sh` comes to over there.
fn sh(shell: &Shell, line: &str) -> Result<RunOutput, ServiceError> {
    shell.run(&asked("sh", &["-c", line]), limits())
}

fn text(shell: &Shell, line: &str) -> String {
    String::from_utf8(sh(shell, line).unwrap().stdout).unwrap()
}

/// Whether a process is still there, asked of another connection.
fn there(door: &Door, pid: &str) -> bool {
    let other = Shell::new(door.clone());
    let state = text(&other, &format!("ps -o stat= -p {pid}"));
    !state.trim().is_empty() && !state.trim_start().starts_with('Z')
}

fn until(what: &str, mut done: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < end, "{what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn what_a_program_reads_and_writes_goes_there_and_back_as_it_is() {
    for door in doors() {
        let shell = Shell::new(door);
        assert_eq!(shell.name(), None);
        // Every byte, three times over and then some.
        let bytes: Vec<u8> = (0..1_000_003_u32)
            .map(|n| (n * 7 + (n >> 8)) as u8)
            .collect();
        for input in [&bytes[..], b"", b"one line without an end"] {
            let output = shell
                .run(
                    &asked("cat", &[]),
                    Limits {
                        stdin: input.to_vec(),
                        ..limits()
                    },
                )
                .unwrap();
            assert!(output.stdout == input, "{} bytes", input.len());
            assert_eq!((output.code, output.stderr.len()), (Some(0), 0));
            assert!(!output.timed_out && !output.truncated);
        }
        assert!(shell.name().is_some_and(|name| !name.is_empty()));

        let output = sh(&shell, "printf out; printf 'err\\r\\n' >&2; exit 37").unwrap();
        assert_eq!(output.code, Some(37));
        assert_eq!(
            (&output.stdout[..], &output.stderr[..]),
            (&b"out"[..], &b"err\r\n"[..])
        );
    }
}

#[test]
fn words_reach_a_program_whatever_is_in_them() {
    let words = [
        "a b",
        "q\"uo\"te",
        "back\\slash\\",
        "$HOME `id` $(id)",
        "%PATH%",
        "new\nline",
        "åäö 漢字 🙂",
        "",
        "'single'",
        "it's",
        "*",
        "a;b|c&d>e<f^g!h",
        "--",
        "-n",
        "tab\there",
        r"C:\x\y",
        r"\\host\share",
        "end\\",
        "~",
        "#x",
    ];
    for door in doors() {
        let shell = Shell::new(door);
        // Each on a line of its own, its line ends marked.
        let mut args = vec![
            "-c",
            "for word; do printf '<%s>|' \"$word\"; done; printf '%s' \"$LUME_X\"; pwd",
            "sh",
        ];
        args.extend(words);
        let output = shell
            .run(
                &Asked {
                    env: vec![("LUME_X".to_owned(), "värde '1' $HOME \\".to_owned())],
                    ..asked("sh", &args)
                },
                limits(),
            )
            .unwrap();
        let expected: String = words.iter().map(|word| format!("<{word}>|")).collect();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{expected}värde '1' $HOME \\/\n")
        );
        assert_eq!(text(&shell, "printf \"${LUME_X-unset}\""), "unset");
    }
}

#[test]
fn a_program_runs_in_its_folder_or_not_at_all() {
    for door in doors() {
        let shell = Shell::new(door);
        let made = text(&shell, "mktemp -d").trim_end().to_owned();
        let odd = format!("{made}/å 'x\" $y");
        shell.run(&asked("mkdir", &[&odd]), limits()).unwrap();
        let within = Asked {
            folder: Some(odd.clone()),
            ..asked("pwd", &[])
        };
        let output = shell.run(&within, limits()).unwrap();
        assert!(String::from_utf8_lossy(&output.stdout).ends_with("/å 'x\" $y\n"));
        shell.run(&asked("rm", &["-r", &made]), limits()).unwrap();
        let gone = shell.run(&within, limits()).unwrap_err();
        assert_eq!(
            (gone.kind, gone.why),
            (ErrorKind::NotFound, Some(FOLDER_MISSING))
        );
        assert!(gone.message.contains("folder"), "{gone}");
        let gone = shell
            .spawn(&within, &Stop::default())
            .map(|_| ())
            .unwrap_err();
        assert_eq!(
            (gone.kind, gone.why),
            (ErrorKind::NotFound, Some(FOLDER_MISSING))
        );
        assert!(gone.message.contains("folder"), "{gone}");
        // The home folder, when none is named.
        let home = Asked {
            folder: None,
            ..asked("sh", &["-c", "[ \"$PWD\" = \"$HOME\" ] && echo home"])
        };
        assert_eq!(shell.run(&home, limits()).unwrap().stdout, b"home\n");

        let missing = asked("lume-no-such-program", &[]);
        let error = shell.run(&missing, limits()).unwrap_err();
        assert_eq!(
            (error.kind, error.why),
            (ErrorKind::NotFound, Some(PROGRAM_MISSING))
        );
        assert!(error.message.contains("was not found on box"), "{error}");
        let error = shell
            .spawn(&missing, &Stop::default())
            .map(|_| ())
            .unwrap_err();
        assert!(error.message.contains("was not found on box"), "{error}");
    }
}

#[test]
fn output_past_the_limit_is_cut_and_the_program_ends_as_it_would_have() {
    for door in doors() {
        let shell = Shell::new(door);
        let run = |line: &str, max_output| {
            shell
                .run(
                    &asked("sh", &["-c", line]),
                    Limits {
                        max_output,
                        ..limits()
                    },
                )
                .unwrap()
        };
        let many = "i=0; while [ $i -lt 3000 ]; do echo line $i; echo err $i >&2; i=$((i+1)); done; exit 3";
        let output = run(many, 100);
        assert_eq!((output.stdout.len(), output.stderr.len()), (100, 100));
        assert!(output.truncated);
        assert_eq!(output.code, Some(3));
        let exact = run("printf 12345", 5);
        assert_eq!((&exact.stdout[..], exact.truncated), (&b"12345"[..], false));
        let over = run("printf 123456", 5);
        assert_eq!((&over.stdout[..], over.truncated), (&b"12345"[..], true));
    }
}

#[test]
fn runs_go_beside_each_other_and_each_has_its_own_answer() {
    for door in doors() {
        let shell = Arc::new(Shell::new(door));
        let started = Instant::now();
        let runs: Vec<_> = (0..8)
            .map(|n| {
                let shell = Arc::clone(&shell);
                std::thread::spawn(move || {
                    // Large, and done at the same time: a part of one answer within
                    // another would be seen.
                    let line = format!("sleep 1; head -c 2000000 /dev/zero | tr '\\000' {n}");
                    text(&shell, &line)
                })
            })
            .collect();
        for (n, run) in runs.into_iter().enumerate() {
            assert!(
                run.join().unwrap() == n.to_string().repeat(2_000_000),
                "{n}"
            );
        }
        assert!(
            started.elapsed() < Duration::from_secs(6),
            "{:?}",
            started.elapsed()
        );
    }
}

#[test]
fn a_program_past_its_time_or_no_longer_wanted_is_ended_with_what_it_started() {
    for door in doors() {
        let shell = Arc::new(Shell::new(door.clone()));
        let tree = "sleep 600 & echo $$ $!; sleep 600";
        let output = shell
            .run(
                &asked("sh", &["-c", tree]),
                Limits {
                    timeout: Duration::from_millis(1500),
                    ..limits()
                },
            )
            .unwrap();
        assert_eq!((output.code, output.timed_out), (None, true));
        let pids = String::from_utf8(output.stdout).unwrap();
        for pid in pids.split_whitespace() {
            assert!(!there(&door, pid), "{pid} of {pids}");
        }

        let stop = Arc::new(Stop::default());
        let asking = Arc::clone(&stop);
        let asker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1000));
            asking.ask();
        });
        let output = shell
            .run(&asked("sh", &["-c", tree]), Limits { stop, ..limits() })
            .unwrap();
        asker.join().unwrap();
        assert_eq!((output.code, output.timed_out), (None, false));
        // The connection goes on.
        assert_eq!(text(&shell, "echo still here"), "still here\n");
    }
}

#[test]
fn a_program_that_leaves_something_holding_its_output_is_not_waited_past_its_end() {
    for door in doors() {
        let shell = Shell::new(door);
        sh(&shell, "true").unwrap();
        let started = Instant::now();
        let output = sh(&shell, "(sleep 6; echo late) & echo early; exit 4").unwrap();
        assert_eq!((output.code, output.timed_out), (Some(4), false));
        assert_eq!(output.stdout, b"early\n");
        assert!(output.truncated, "the output was still open");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }
}

#[test]
fn nothing_is_left_when_the_connection_is_let_go_of() {
    for door in doors() {
        let shell = Arc::new(Shell::new(door.clone()));
        let running = Arc::clone(&shell);
        let run = std::thread::spawn(move || sh(&running, "sleep 600"));
        let dir = text(&shell, "echo \"$d\"").trim().to_owned();
        let pid = || {
            text(&shell, "cat \"$d\"/*.pid | head -n 1")
                .trim()
                .to_owned()
        };
        until("the program runs", || !pid().is_empty());
        let pid = pid();
        assert!(there(&door, &pid));
        shell.close();
        let error = run.join().unwrap().unwrap_err();
        assert_eq!(error.kind, ErrorKind::Unavailable);
        until("the program is ended", || !there(&door, &pid));
        let other = Shell::new(door.clone());
        until("its folder is gone", || {
            text(&other, &format!("[ -e {dir} ] || echo gone")) == "gone\n"
        });
        // Closed is closed.
        assert_eq!(sh(&shell, "true").unwrap_err().kind, ErrorKind::Unavailable);
    }
}

#[test]
fn a_connection_that_ended_is_opened_again_and_one_not_used_is_let_go_of() {
    for door in doors() {
        let shell = Shell::new(door);
        // The loop is the parent of the script that runs a program.
        let dir = text(&shell, "echo \"$d\"").trim().to_owned();
        let error = sh(&shell, "kill -s KILL $(ps -o ppid= -p $PPID); sleep 5").unwrap_err();
        assert_eq!(error.kind, ErrorKind::Unavailable);
        assert_eq!(text(&shell, "echo again"), "again\n");
        // Killed, it could not take its folder with it.
        assert!(dir.contains("/lume."), "{dir}");
        shell.run(&asked("rm", &["-r", &dir]), limits()).unwrap();
        shell.close_idle(Duration::from_secs(60));
        assert!(shell.name().is_some());
        shell.close_idle(Duration::ZERO);
        assert!(shell.name().is_none());
        assert_eq!(text(&shell, "echo and again"), "and again\n");
    }
}

#[test]
fn a_started_program_gives_its_output_as_it_comes_and_waits_to_be_read() {
    for door in doors() {
        let shell = Shell::new(door.clone());
        let started = Instant::now();
        let line = "for n in 1 2 3; do echo $n; sleep 1; done; echo err >&2; exit 5";
        let mut run = shell
            .spawn(&asked("sh", &["-c", line]), &Stop::default())
            .unwrap();
        let mut first = [0; 2];
        run.stdout.read_exact(&mut first).unwrap();
        assert_eq!(&first, b"1\n");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        let mut rest = String::new();
        run.stdout.read_to_string(&mut rest).unwrap();
        assert_eq!(rest, "2\n3\n");
        let mut said = String::new();
        run.stderr.read_to_string(&mut said).unwrap();
        assert_eq!(said, "err\n");
        until("it ended", || run.child.try_wait() == Some(Some(5)));

        // One that writes without end, and one that says nothing: both ended from here.
        for line in [
            "echo $$; while :; do echo 0123456789012345678901234567890; done",
            "echo $$; sleep 600",
        ] {
            let mark = format!("mark-{}-{}", std::process::id(), line.len());
            let mut run = shell
                .spawn(&asked("sh", &["-c", line, &mark]), &Stop::default())
                .unwrap();
            let mut pid = Vec::new();
            let mut byte = [0; 1];
            while run.stdout.read(&mut byte).unwrap() == 1 && byte[0] != b'\n' {
                pid.push(byte[0]);
            }
            let pid = String::from_utf8(pid).unwrap();
            std::thread::sleep(Duration::from_millis(500));
            assert!(there(&door, &pid), "{line}");
            run.child.kill().unwrap();
            until("it is ended", || !there(&door, &pid));
            drop(run);
            // Its script was in a folder of its own.
            let other = Shell::new(door.clone());
            let find = format!("grep -ls '{mark}' \"${{TMPDIR:-/tmp}}\"/lume.*/s");
            until("its folder is gone", || text(&other, &find).is_empty());
        }
    }
}

#[test]
fn a_machine_that_is_not_there_is_unavailable_and_says_why() {
    for mut door in doors() {
        if cfg!(windows) {
            door.args = vec!["-d".into(), "lume-no-such-distro".into(), "--exec".into()];
        } else {
            door.program = PathBuf::from("/bin/sh");
            door.args = vec!["-c".into(), "echo no way in >&2; exit 255".into()];
        }
        let shell = Shell::new(door);
        let error = sh(&shell, "true").unwrap_err();
        assert_eq!(error.kind, ErrorKind::Unavailable);
        if cfg!(unix) {
            assert_eq!(error.message, "box could not be reached: no way in");
        }
        let error = shell
            .spawn(&asked("sh", &[]), &Stop::default())
            .map(|_| ())
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Unavailable);
    }
}
