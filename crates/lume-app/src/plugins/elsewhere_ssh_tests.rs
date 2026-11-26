//! A plugin's programs on the machine an SSH profile signs in to, through a stand-in for
//! the SSH client that runs what it is given with a real `sh`: this machine's own on
//! Linux and macOS, on Windows that of the distribution `LUME_TEST_DISTRO` names. The
//! tests do nothing where there is neither, or where the stand-in was not built.

use std::path::PathBuf;

use lume_plugin_api::Call;
use lume_proto::{ExitStatus, SessionEvent};
use lume_session::Launcher;

use super::tests::{PLUGIN, allow_on, install, machines_of, run, told};
use super::*;
use crate::plugins::calls::tests::running;
use crate::services::Services;
use crate::testing::TempDir;

const MACHINE: &str = "ssh:me@devbox";

/// The stand-in, where it was built beside the tests.
fn stand_in() -> Option<PathBuf> {
    let name = format!("stand_in_ssh{}", std::env::consts::EXE_SUFFIX);
    let found = std::env::current_exe()
        .ok()?
        .ancestors()
        .map(|dir| dir.join("examples").join(&name))
        .find(|candidate| candidate.is_file());
    if found.is_none() {
        eprintln!("the stand-in for the SSH client was not built: nothing is tested");
    }
    found
}

/// What runs the command the client is given, as a machine's shell would.
fn shell() -> Option<Vec<String>> {
    if cfg!(windows) {
        let Ok(distro) = std::env::var("LUME_TEST_DISTRO") else {
            eprintln!("LUME_TEST_DISTRO names no distribution: nothing is tested");
            return None;
        };
        let wsl = PathBuf::from(std::env::var_os("SystemRoot")?)
            .join("System32")
            .join("wsl.exe");
        return Some(
            [
                &wsl.display().to_string(),
                "-d",
                &distro,
                "--cd",
                "/",
                "--exec",
                "sh",
                "-c",
            ]
            .map(str::to_owned)
            .to_vec(),
        );
    }
    Some(vec!["/bin/sh".to_owned(), "-c".to_owned()])
}

/// The machine the stand-in plays, and what it was asked.
struct Far {
    script: PathBuf,
    log: PathBuf,
    client: PathBuf,
}

impl Far {
    /// A machine that asks for a password when `asks`, and runs commands with `shell`.
    fn new(dir: &TempDir, asks: bool, shell: Vec<String>) -> Option<Self> {
        let client = stand_in()?;
        let script = dir.path().join("far.json");
        let log = dir.path().join("far.log");
        let questions = if asks {
            json!([{
                "words": "me@devbox's password: ",
                "right": "horse",
                "tries": 3,
                "again": "Permission denied, please try again.",
                "failure": "me@devbox: Permission denied (publickey,password).",
            }])
        } else {
            json!([])
        };
        let far = json!({ "asks": questions, "shell": shell, "log": log });
        std::fs::write(&script, far.to_string()).unwrap();
        Some(Self {
            script,
            log,
            client,
        })
    }

    fn launcher(&self) -> Launcher {
        Launcher::Ssh {
            host: "devbox".to_owned(),
            user: Some("me".to_owned()),
            port: None,
            extra_args: vec!["-F".to_owned(), self.script.display().to_string()],
            client: Some(self.client.display().to_string()),
        }
    }

    fn log(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

/// `git` with these words, run on the machine in a folder that is there.
fn git(args: &[&str]) -> Call {
    let run = json!({ "program": "git", "args": args,
        "options": { "cwd": "/tmp", "machine": MACHINE } });
    Call::decode("process.run", run).unwrap()
}

/// Has the user sign in to a machine, as a press in the plugin's view does.
async fn sign_in(services: &Services, machine: &str) -> Result<bool, ApiError> {
    let asked = MachinesSignIn {
        machine: machine.to_owned(),
    };
    services.plugins.sign_in_machine(PLUGIN, asked).await
}

/// The application with the plugin, and a session of an SSH profile to `far`.
async fn with_profile(dir: &TempDir, far: &Far) -> (Arc<Services>, SessionKey) {
    let (services, key) = running(dir).await;
    install(&services, dir.path()).await;
    services
        .state
        .reports()
        .started(key.clone(), far.launcher(), None);
    (services, key)
}

fn why(result: Result<Value, ApiError>) -> Option<String> {
    result.unwrap_err().kind
}

/// What plugins are told of where a session is, when it prints its folder.
fn place_told(services: &Services, key: &SessionKey, url: &str) -> Option<Value> {
    told(
        services,
        key,
        SessionEvent::Cwd {
            url: url.to_owned(),
        },
    )
    .into_iter()
    .find_map(|(kind, details)| (kind == "place").then_some(details))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plugin_runs_its_programs_where_an_ssh_profile_signs_in_once_allowed() {
    let dir = TempDir::new("plugin-ssh");
    let Some(far) = shell().and_then(|shell| Far::new(&dir, false, shell)) else {
        return;
    };
    let (services, key) = with_profile(&dir, &far).await;
    let version = || git(&["--version"]);

    // Known, not allowed, and nothing is reached before the user's yes.
    let listed = machines_of(&services);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(
        (
            listed[0].machine.as_str(),
            listed[0].kind,
            listed[0].allowed,
            listed[0].sign_in
        ),
        (MACHINE, MachineKind::Ssh, false, false)
    );
    assert_eq!(
        why(run(&services, version()).await).as_deref(),
        Some(NOT_ALLOWED)
    );
    let printed = "file://somewhere/home/me";
    let place = place_told(&services, &key, printed).unwrap();
    assert_eq!(
        (&place["machine"], &place["folder"]),
        (&json!(MACHINE), &Value::Null)
    );
    let news = crate::sessions::News {
        key: key.clone(),
        what: lume_proto::app::SessionNews::Event(SessionEvent::Cwd {
            url: printed.to_owned(),
        }),
    };
    assert!(services.plugins.to_learn(&news).is_none());
    assert!(far.log().is_empty(), "{:?}", far.log());

    allow_on(&services, MACHINE, &["git"]);
    let ran = run(&services, version()).await.unwrap();
    assert_eq!(ran["code"], 0, "{ran}");
    assert!(
        ran["stdout"].as_str().unwrap().starts_with("git version"),
        "{ran}"
    );
    // Asked of the client that asks nothing.
    assert_eq!(far.log()[0], "quiet");

    // What it called itself is what a folder it prints has to name.
    let machine = services.state.machine_of(&key).unwrap();
    let runner = services
        .plugins
        .elsewhere
        .machines
        .runner(&machine)
        .unwrap();
    let own = runner.name().expect("it said what it is called");
    let place = place_told(&services, &key, &format!("file://{own}/tmp")).unwrap();
    assert_eq!(place["folder"], "/tmp", "{place}");
    let place = place_told(&services, &key, "file://further-box/srv").unwrap();
    assert_eq!(place["folder"], Value::Null, "{place}");

    // Where it runs, and what it is told when its folder is gone.
    let mut status = json!({ "program": "git", "args": ["rev-parse", "--show-toplevel"],
        "options": { "machine": MACHINE, "cwd": "/no such folder" } });
    let call = Call::decode("process.run", status.clone()).unwrap();
    assert_eq!(
        why(run(&services, call).await).as_deref(),
        Some("folder_missing")
    );
    status["options"]["cwd"] = json!("/tmp");
    let call = Call::decode("process.run", status).unwrap();
    let ran = run(&services, call).await.unwrap();
    assert_ne!(ran["code"], 0, "{ran}");

    // The session gone, its connection is let go of.
    assert!(services.plugins.elsewhere.machines.holds(MACHINE));
    services.plugins.reports_of(&crate::sessions::News {
        key: key.clone(),
        what: lume_proto::app::SessionNews::Exit(ExitStatus::Code(0)),
    });
    assert!(!services.plugins.elsewhere.machines.holds(MACHINE));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_machine_that_asks_for_a_password_wants_the_user_to_sign_in_and_asks_nobody() {
    let dir = TempDir::new("plugin-ssh-asks");
    let Some(far) = shell().and_then(|shell| Far::new(&dir, true, shell)) else {
        return;
    };
    let (services, key) = with_profile(&dir, &far).await;
    allow_on(&services, MACHINE, &["git"]);
    let version = || git(&["--version"]);
    assert_eq!(
        why(run(&services, version()).await).as_deref(),
        Some("sign_in")
    );
    assert_eq!(far.log(), ["quiet", "refused"]);
    assert!(machines_of(&services)[0].sign_in);

    // Nobody is reached to find out its name while it wants a sign-in.
    let news = crate::sessions::News {
        key: key.clone(),
        what: lume_proto::app::SessionNews::Event(SessionEvent::Cwd {
            url: "file://devbox/home/me".to_owned(),
        }),
    };
    assert!(services.plugins.to_learn(&news).is_none());

    // With no window, nobody is asked.
    let error = sign_in(&services, MACHINE).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Failed, "{error:?}");
    assert_eq!(far.log(), ["quiet", "refused"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_machine_whose_shell_runs_no_sh_cannot_run_programs() {
    let dir = TempDir::new("plugin-ssh-no-sh");
    let Some(mut shell) = shell() else {
        return;
    };
    // A shell that understands nothing it is given.
    shell.truncate(shell.len() - 2);
    shell.push("false".to_owned());
    let Some(far) = Far::new(&dir, false, shell) else {
        return;
    };
    let (services, _key) = with_profile(&dir, &far).await;
    allow_on(&services, MACHINE, &["git"]);
    let version = git(&["--version"]);
    assert_eq!(
        why(run(&services, version).await).as_deref(),
        Some(CANNOT_RUN)
    );
    assert!(!machines_of(&services)[0].sign_in);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signing_in_is_only_for_a_machine_the_plugin_is_allowed_on_that_wants_it() {
    let dir = TempDir::new("plugin-ssh-sign-in");
    let Some(far) = shell().and_then(|shell| Far::new(&dir, false, shell)) else {
        return;
    };
    let (services, _key) = with_profile(&dir, &far).await;
    let call = Call::decode("machines.signIn", json!({ "machine": MACHINE })).unwrap();
    assert_eq!(call.permission(), Some("process.run-elsewhere"));
    let refused = sign_in(&services, MACHINE).await.unwrap_err();
    assert_eq!(refused.kind.as_deref(), Some(NOT_ALLOWED));
    allow_on(&services, MACHINE, &["git"]);
    // It does not want it: there is nothing to do.
    assert!(sign_in(&services, MACHINE).await.unwrap());
    let nowhere = sign_in(&services, "ssh:nobody").await.unwrap_err();
    assert_eq!(nowhere.kind.as_deref(), Some(NOT_CONNECTED));
    assert!(far.log().is_empty(), "{:?}", far.log());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn what_a_machine_calls_itself_is_found_out_once_and_a_changed_profile_is_reached_anew() {
    let dir = TempDir::new("plugin-ssh-name");
    let Some(far) = shell().and_then(|shell| Far::new(&dir, false, shell)) else {
        return;
    };
    let (services, key) = with_profile(&dir, &far).await;
    allow_on(&services, MACHINE, &["git"]);
    let printed = |url: &str| crate::sessions::News {
        key: key.clone(),
        what: lume_proto::app::SessionNews::Event(SessionEvent::Cwd {
            url: url.to_owned(),
        }),
    };
    let plugins = &services.plugins;
    // Nothing is found out of what is printed by other means.
    let state = SessionEvent::Title {
        title: "file://devbox".to_owned(),
    };
    let other = crate::sessions::News {
        key: key.clone(),
        what: lume_proto::app::SessionNews::Event(state),
    };
    assert!(plugins.to_learn(&other).is_none());
    let machine = plugins
        .to_learn(&printed("file://devbox/tmp"))
        .expect("to be found out");
    // Once at a time.
    assert!(plugins.to_learn(&printed("file://devbox/tmp")).is_none());
    plugins.learn_name(machine).await;
    assert!(plugins.elsewhere.machines.knows_name(MACHINE));
    assert!(plugins.to_learn(&printed("file://devbox/tmp")).is_none());

    let machine = services.state.machine_of(&key).unwrap();
    let first = plugins.elsewhere.machines.runner(&machine).unwrap();
    assert!(Arc::ptr_eq(
        &first,
        &plugins.elsewhere.machines.runner(&machine).unwrap()
    ));
    // The profile now reaches it otherwise: the old connection is let go of.
    let Launcher::Ssh {
        host,
        user,
        port,
        mut extra_args,
        client,
    } = far.launcher()
    else {
        unreachable!()
    };
    extra_args.extend(["-o".to_owned(), "Compression=yes".to_owned()]);
    let changed = Launcher::Ssh {
        host,
        user,
        port,
        extra_args,
        client,
    };
    services.state.reports().started(key.clone(), changed, None);
    let machine = services.state.machine_of(&key).unwrap();
    assert_eq!(machine.id, MACHINE);
    let second = plugins.elsewhere.machines.runner(&machine).unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    assert!(first.name().is_none(), "the old connection is closed");
    // A profile that is gone takes its connection with it.
    let ran = run(&services, git(&["--version"])).await.unwrap();
    assert_eq!(ran["code"], 0, "{ran}");
    assert!(second.name().is_some());
    services.state.reports().forget(&key);
    plugins.config_changed();
    assert!(!plugins.elsewhere.machines.holds(MACHINE));
    assert!(second.name().is_none(), "the connection is closed");
    assert!(!plugins.elsewhere.machines.knows_name(MACHINE));
}

/// Every file under `dir`, but what plugins keep of their own and the stand-in's.
fn files_but_plugins(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            let path = entry.path();
            // What the stand-in keeps is the other machine's.
            let far = path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("far."));
            if path.ends_with("plugins") || far {
                continue;
            }
            if path.is_dir() {
                folders.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn whatever_the_machine_of_an_ssh_profile_says_nothing_is_started_or_changed_here() {
    let Some(shell) = shell() else {
        return;
    };
    // Each plays the other side's shell, whatever it is given to run.
    let sides = [
        // Says too much before it says it is there.
        r"head -c 70000 /dev/zero | tr '\0' a; echo; echo lume-there box; cat > /dev/null",
        // Calls itself what names no machine, and answers with more than it may.
        concat!(
            r"printf 'lume-there \033]0;x\007\n'; while read -r w i n t; do ",
            r#"head -c "$t" > /dev/null; echo "taken $i"; echo "done $i 0 999999999 0 ran"; done"#,
        ),
        // Answers a run nobody made, then one that is not a number.
        concat!(
            r#"echo lume-there ../etc; while read -r w i n t; do head -c "$t" > /dev/null; "#,
            r#"echo "taken $i"; echo "done 77 0 0 0 ran"; echo "done x y z w ran"; done"#,
        ),
        // Says it took what it did not.
        "echo lume-there box; read -r w i n t; echo taken 99; cat > /dev/null",
    ];
    for side in sides {
        let dir = TempDir::new("plugin-ssh-hostile");
        let mut played = shell.clone();
        played.extend([side.to_owned(), "lume".to_owned()]);
        let far = Far::new(&dir, false, played).unwrap();
        let (services, key) = with_profile(&dir, &far).await;
        allow_on(&services, MACHINE, &["git"]);
        let before = files_but_plugins(dir.path());
        let ran = tokio::time::timeout(Duration::from_secs(90), run(&services, git(&["status"])))
            .await
            .expect("an answer in time");
        let error = ran.expect_err(side);
        assert_eq!(error.code, ErrorCode::Unavailable, "{side}: {error:?}");
        // What it called itself names no machine, so nothing it prints is a folder.
        for url in [
            "file://box/etc",
            "file://../etc/passwd",
            "file://\u{1b}]0;x\u{7}/etc",
            "file:///etc",
            "file://localhost/etc",
        ] {
            if let Some(place) = place_told(&services, &key, url) {
                let folder = place["folder"].as_str();
                assert!(
                    folder.is_none() || side.contains("lume-there box"),
                    "{side}: {url} gave {place}"
                );
            }
        }
        assert_eq!(files_but_plugins(dir.path()), before, "{side}");
        assert!(
            far.log()
                .iter()
                .all(|line| line == "quiet" || line == "runs"),
            "{side}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_flood_of_folders_printed_on_the_machine_of_an_ssh_profile_is_told_within_bounds() {
    let dir = TempDir::new("plugin-ssh-folders");
    let Some(far) = shell().and_then(|shell| Far::new(&dir, false, shell)) else {
        return;
    };
    let (services, key) = with_profile(&dir, &far).await;
    allow_on(&services, MACHINE, &["git"]);
    let ran = run(&services, git(&["--version"])).await.unwrap();
    assert_eq!(ran["code"], 0, "{ran}");
    let machine = services.state.machine_of(&key).unwrap();
    let own = services
        .plugins
        .elsewhere
        .machines
        .runner(&machine)
        .unwrap()
        .name()
        .unwrap();
    let urls = [
        format!("file://{own}/home/me"),
        format!("file://{own}/home/me"),
        format!("file://{own}/{}", "x".repeat(5000)),
        format!("file://{own}/a%0Ab"),
        format!("file://{own}/a%zz"),
        "file://another/home".to_owned(),
        format!("file://{own}/C:/Windows"),
        format!("file://{own}/srv/%E2%80%AEevil"),
    ];
    let mut told = 0;
    for round in 0..250 {
        for url in &urls {
            let Some(place) = place_told(&services, &key, url) else {
                continue;
            };
            told += 1;
            if let Some(folder) = place["folder"].as_str() {
                assert!(folder.starts_with('/') && folder.len() <= 4096, "{folder}");
                assert!(!folder.chars().any(char::is_control), "{folder:?}");
            }
            assert_eq!(place["machine"], MACHINE, "round {round}");
        }
    }
    // Told when it changed, and never twice the same in a row.
    assert!(told <= 250 * urls.len(), "{told}");
    let repeat = place_told(&services, &key, &urls[7]);
    assert!(repeat.is_none(), "{repeat:?}");
}
