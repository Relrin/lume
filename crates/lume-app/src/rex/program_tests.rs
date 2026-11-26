//! A plugin's programs on the machine of a server elsewhere: the pretend server plays the
//! script that runs them, as a real one runs it.

use common::programs::{Played, Programs};
use lume_plugin_api::{ApiError, Call, ErrorCode};
use lume_proto::SessionEvent;

use super::*;
use crate::carrier::SessionKey;
use crate::plugins::elsewhere_tests::{
    PLUGIN, allow_on, entry, git, install, machines_of, release, run, told,
};

/// What every terminal of the server is in: a folder of a Windows machine.
const THERE: &str = r"C:\Users\me";

/// The recorded server, its terminals in `THERE`, running programs as `programs` play.
fn world(programs: &Programs) -> World {
    let mut world = World::recorded().running(programs);
    world.twist = Some(Arc::new(|method, mut answer| {
        if method == "com.superlogical.terminal.process" {
            answer["foreground"]["cwd"] = json!(THERE);
            answer["child"]["cwd"] = json!(THERE);
        }
        Some(answer)
    }));
    world
}

/// What a program plays, by its first argument.
fn programs() -> Programs {
    Programs::new(|job| {
        let words = job.words();
        match words.get(1).map(String::as_str) {
            Some("status") => Played::says(format!("in {}", job.told["folder"])),
            Some("nowhere") => Played::failed("chdir: no such folder", true),
            Some("stays") => Played::stays("begun"),
            Some("log") => Played::says("one\ntwo\n"),
            _ => Played::failed("exec: \"git\": executable file not found in %PATH%", false),
        }
    })
}

struct Served {
    app: Running,
    server: Server,
    programs: Programs,
    /// The machine's id.
    id: String,
}

impl Served {
    /// The application with the plugin, and the server elsewhere, not yet connected.
    async fn start(name: &str) -> Self {
        let programs = programs();
        let server = Server::start(world(&programs)).await;
        let app = Running::with(name, &server.url()).await;
        app.services.plugins.start(&app.services);
        install(&app.services, app.dir.path()).await;
        let id = format!("rex:{}", server.url());
        Self {
            app,
            server,
            programs,
            id,
        }
    }

    fn services(&self) -> &Services {
        &self.app.services
    }

    async fn open(&self) {
        self.app
            .outside("lume.rex.open", json!({ "host": HOST }))
            .await
            .unwrap();
    }

    /// The first session of the server, as the application knows it.
    async fn session(&self) -> SessionKey {
        let listed = self.services().state.list_all().await.unwrap();
        let listed = listed
            .iter()
            .find(|listed| listed.backend.0 == self.id)
            .expect("a session of the server");
        SessionKey {
            backend: listed.backend.clone(),
            session: listed.info.id,
        }
    }

    fn git(&self, args: &[&str], folder: Option<&str>) -> Call {
        let mut call = git(args, Some(&self.id));
        if let Call::ProcessRun(run) = &mut call {
            run.options.cwd = folder.map(str::to_owned);
        }
        call
    }

    async fn run(&self, args: &[&str], folder: Option<&str>) -> Result<Value, ApiError> {
        run(self.services(), self.git(args, folder)).await
    }

    fn machines(&self) -> Vec<(String, String, bool)> {
        machines_of(self.services())
            .into_iter()
            .map(|entry| (entry.machine, entry.label, entry.allowed))
            .collect()
    }

    async fn stop(self) {
        self.app.stop().await;
    }
}

fn why(result: Result<Value, ApiError>) -> (ErrorCode, Option<String>) {
    let error = result.unwrap_err();
    (error.code, error.kind)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plugin_runs_its_programs_on_a_servers_machine_once_the_user_allowed_it_there() {
    let served = Served::start("rex-programs").await;
    let not_connected = (ErrorCode::Unavailable, Some("not_connected".to_owned()));
    let not_allowed = (ErrorCode::Denied, Some("machine_not_allowed".to_owned()));
    // Nobody is on it yet, and nothing was agreed to.
    assert_eq!(served.machines(), []);
    assert_eq!(why(served.run(&["status"], None).await), not_allowed);

    // Connected, but with no session open in a window, it is not one to run on yet.
    served.app.rex().connect(HOST, false, None).await.unwrap();
    assert_eq!(served.machines(), []);
    served.open().await;
    let key = served.session().await;
    let place = entry(served.services(), &key).await.place.unwrap();
    assert_eq!(
        (
            place.machine.as_str(),
            place.label.as_str(),
            place.folder.as_deref()
        ),
        (served.id.as_str(), HOST, Some(THERE))
    );
    assert_eq!(
        served.machines(),
        [(served.id.clone(), HOST.to_owned(), false)]
    );
    let refused = served.run(&["status"], Some(THERE)).await.unwrap_err();
    assert_eq!(
        (refused.code, refused.kind.as_deref()),
        (ErrorCode::Denied, Some("machine_not_allowed"))
    );
    let shown = format!(
        "{HOST}, {} (Rex server)",
        served.id.trim_start_matches("rex:")
    );
    assert_eq!(refused.data.unwrap()["label"], shown.as_str());
    assert!(served.programs.seen(|seen| seen.blocks.is_empty()));

    allow_on(served.services(), &served.id, &["git"]);
    assert!(served.machines()[0].2);
    let answer = served.run(&["status"], Some(THERE)).await.unwrap();
    assert_eq!(
        answer["stdout"],
        format!("in \"{}\"", THERE.replace('\\', "\\\\"))
    );
    assert_eq!(answer["code"], 0);
    let job = served.programs.seen(|seen| seen.jobs[0].told.clone());
    assert_eq!(job["argv"], json!(["git", "status"]));

    // What cannot be run there is not asked of it.
    let blocks = served.programs.seen(|seen| seen.blocks.len());
    for folder in ["/home/me", r"\\server\share", "${home}"] {
        let wrong = why(served.run(&["status"], Some(folder)).await);
        assert_eq!(wrong.0, ErrorCode::Invalid, "{folder}");
    }
    assert_eq!(served.programs.seen(|seen| seen.blocks.len()), blocks);
    // What is not there is told apart.
    let gone = served
        .run(&["nowhere"], Some(r"C:\gone"))
        .await
        .unwrap_err();
    assert_eq!(
        (gone.code, gone.kind.as_deref()),
        (ErrorCode::NotFound, Some("folder_missing"))
    );
    assert_eq!(gone.data.unwrap()["machine"], served.id.as_str());
    let missing = why(served.run(&["--version"], None).await);
    assert_eq!(
        missing,
        (ErrorCode::NotFound, Some("program_missing".to_owned()))
    );

    // Read as it comes.
    let spawn = json!({ "program": "git", "args": ["log"], "options": { "machine": served.id } });
    let spawned = run(
        served.services(),
        Call::decode("process.spawn", spawn).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(spawned["pid"], 0);
    let mut read = String::new();
    loop {
        let asked = json!({ "stream": spawned["stdout"] });
        let piece = run(
            served.services(),
            Call::decode("streams.read", asked).unwrap(),
        )
        .await
        .unwrap();
        match piece.as_str() {
            Some(piece) => read.push_str(piece),
            None => break,
        }
    }
    assert_eq!(read, "one\ntwo\n");
    let wait = json!({ "process": spawned["process"] });
    let ended = run(
        served.services(),
        Call::decode("process.wait", wait).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(ended["code"], 0);

    // Called otherwise, it is the same machine and the same yes.
    let renamed = configured(&served.server.url()).replace(HOST, "Renamed");
    std::fs::write(served.app.dir.dirs().config_file(), renamed).unwrap();
    served.services().config.reload().unwrap();
    served.app.rex().config_changed();
    until("it is called otherwise", || {
        served.machines() == [(served.id.clone(), "Renamed".to_owned(), true)]
    })
    .await;
    assert_eq!(
        entry(served.services(), &key).await.place.unwrap().label,
        "Renamed"
    );
    assert!(served.run(&["status"], None).await.is_ok());

    served
        .services()
        .plugins
        .revoke_machine(PLUGIN, &served.id)
        .await
        .unwrap();
    assert_eq!(why(served.run(&["status"], None).await), not_allowed);

    // Gone with the connection, and the yes kept for when it is back.
    allow_on(served.services(), &served.id, &["git"]);
    served
        .app
        .outside("lume.rex.disconnect", json!({ "host": "Renamed" }))
        .await
        .unwrap();
    assert_eq!(served.machines(), []);
    assert_eq!(why(served.run(&["status"], None).await), not_connected);
    assert!(served.services().state.machine_of(&key).is_none());
    served.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn what_runs_on_a_server_ends_with_its_plugin_and_with_the_connection() {
    let served = Served::start("rex-programs-end").await;
    served.open().await;
    allow_on(served.services(), &served.id, &["git"]);
    let start = || {
        let services = Arc::clone(&served.app.services);
        let call = served.git(&["stays"], None);
        tokio::spawn(async move { run(&services, call).await })
    };
    let running = start();
    until("it runs", || {
        served.programs.seen(|seen| !seen.jobs.is_empty())
    })
    .await;
    release(served.services());
    let ended = timeout(STEP, running).await.unwrap().unwrap().unwrap();
    assert!(ended["code"].is_null(), "{ended}");
    until("its block is closed", || {
        served.programs.seen(|seen| !seen.closed.is_empty())
    })
    .await;

    let running = start();
    until("it runs again", || {
        served.programs.seen(|seen| seen.jobs.len() == 2)
    })
    .await;
    served.server.last(|served| served.tell(Tell::Drop));
    let error = timeout(STEP, running).await.unwrap().unwrap().unwrap_err();
    assert_eq!(error.kind.as_deref(), Some("not_connected"));
    served.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plugins_hear_where_a_servers_session_is_when_that_changes() {
    let served = Served::start("rex-programs-place").await;
    served.open().await;
    let key = served.session().await;
    let services = served.services();
    let foreground = || {
        SessionEvent::Foreground(lume_proto::Foreground {
            pid: 1000,
            name: "cmd.exe".to_owned(),
        })
    };
    let places = |told: Vec<(String, Value)>| -> Vec<Option<String>> {
        told.into_iter()
            .filter(|(kind, _)| kind == "place")
            .map(|(_, place)| place["folder"].as_str().map(str::to_owned))
            .collect()
    };
    assert_eq!(
        places(told(services, &key, foreground())),
        [Some(THERE.to_owned())]
    );
    // Told once while it stays, whatever is printed meanwhile.
    assert_eq!(places(told(services, &key, foreground())), []);
    let printed = SessionEvent::Cwd {
        url: "file://box/C:/Printed".to_owned(),
    };
    assert_eq!(places(told(services, &key, printed)), []);
    let all = told(services, &key, SessionEvent::Cwd { url: String::new() });
    assert!(all.is_empty(), "{all:?}");

    // The server says it moved.
    let view = World::recorded().views[common::SESSION].clone();
    let block = view["windows"][0]["layers"][0]["blocks"][0]["block_id"].clone();
    let moved = json!({ "foreground": { "pid": 1000, "name": "cmd.exe", "cwd": r"D:\code" } });
    served.server.last(|served| {
        served.tell(Tell::SessionEvent(
            common::SESSION.to_owned(),
            "block_event".to_owned(),
            json!({
                "block_id": block, "creator_name": "com.superlogical.terminal",
                "name": "process_changed", "payload": moved,
            }),
        ));
    });
    until("it has moved", || {
        services.state.machine_of(&key).is_some_and(|machine| {
            machine.place(key.session, None).folder.as_deref() == Some(r"D:\code")
        })
    })
    .await;
    assert_eq!(
        places(told(services, &key, foreground())),
        [Some(r"D:\code".to_owned())]
    );
    // A whole state tells it again.
    let state = SessionEvent::State {
        state: lume_proto::SessionState::default(),
    };
    assert_eq!(
        places(told(services, &key, state)),
        [Some(r"D:\code".to_owned())]
    );
    served.stop().await;
}
