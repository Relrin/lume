//! Everything a plugin may not do, each as its own case: every service without its grant,
//! paths that climb or link out of what was granted, Lume's own folders and programs,
//! and addresses of this computer reached by name, by redirect or by a grant too wide.
//!
//! Nothing here reaches past this computer: servers listen on the loopback address, and
//! names are looked up by a made-up lookup.

mod common;

use std::time::Duration;

use common::{Layout, Server, answer, exe, get, grants, lookup, redirect};
use lume_services::fs::{self, Access, WriteMode};
use lume_services::net::{self, FetchRequest};
use lume_services::process;
use lume_services::secrets::{Keychain as _, OsKeychain};
use lume_services::streams::Streams;
use lume_services::{ErrorKind, ServiceError};

fn kind<T>(result: Result<T, ServiceError>) -> ErrorKind {
    match result {
        Ok(_) => panic!("the call is allowed, but should be refused"),
        Err(error) => error.kind,
    }
}

// Files.

#[test]
fn every_file_service_refuses_without_its_grant() {
    let layout = Layout::new("no-grant");
    layout.dir.write("ws/notes.md", b"notes");
    let places = &layout.places;
    let none: Vec<String> = Vec::new();
    let path = "${workspace}/notes.md";
    assert_eq!(
        kind(fs::check(places, &none, path, Access::Read)),
        ErrorKind::Denied
    );
    assert_eq!(
        kind(fs::check(places, &none, path, Access::Write)),
        ErrorKind::Denied
    );
    assert_eq!(kind(fs::read(places, &none, path, 100)), ErrorKind::Denied);
    assert_eq!(kind(fs::open_read(places, &none, path)), ErrorKind::Denied);
    assert_eq!(kind(fs::stat(places, &none, path)), ErrorKind::Denied);
    assert_eq!(
        kind(fs::list(places, &none, "${workspace}", 100)),
        ErrorKind::Denied
    );
    assert_eq!(
        kind(fs::write(places, &none, path, b"x", WriteMode::Replace)),
        ErrorKind::Denied
    );
    assert_eq!(
        kind(fs::watch(
            places,
            &none,
            "${workspace}",
            true,
            Box::new(|_| {})
        )),
        ErrorKind::Denied
    );
    assert_eq!(std::fs::read(layout.path("ws/notes.md")).unwrap(), b"notes");
}

#[test]
fn a_grant_to_read_does_not_cover_writing_elsewhere() {
    let layout = Layout::new("read-not-write");
    let write = grants(&["${workspace}/out/**"]);
    layout.dir.folder("ws/out");
    let error = fs::write(
        &layout.places,
        &write,
        "${workspace}/a.txt",
        b"x",
        WriteMode::Replace,
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Denied);
    assert!(error.message.contains("fs.write"), "{}", error.message);
    assert!(!layout.path("ws/a.txt").exists());
}

#[test]
fn a_path_that_climbs_out_of_the_workspace_is_refused() {
    let layout = Layout::new("climb");
    layout.dir.write("outside/secret.txt", b"secret");
    layout.dir.folder("ws/sub");
    let all = grants(&["${workspace}/**"]);
    for path in [
        "${workspace}/../outside/secret.txt",
        "${workspace}/sub/../../outside/secret.txt",
        "${workspace}/..",
    ] {
        assert_eq!(
            kind(fs::read(&layout.places, &all, path, 100)),
            ErrorKind::Invalid,
            "{path}"
        );
        assert_eq!(
            kind(fs::write(
                &layout.places,
                &all,
                path,
                b"x",
                WriteMode::Replace
            )),
            ErrorKind::Invalid,
            "{path}"
        );
    }
    assert_eq!(
        std::fs::read(layout.path("outside/secret.txt")).unwrap(),
        b"secret"
    );
}

#[test]
fn an_absolute_path_outside_the_grants_is_refused() {
    let layout = Layout::new("absolute");
    layout.dir.write("outside/secret.txt", b"secret");
    let all = grants(&["${workspace}/**", "${home}/**"]);
    let path = layout.asked("outside/secret.txt");
    assert_eq!(
        kind(fs::read(&layout.places, &all, &path, 100)),
        ErrorKind::Denied
    );
    assert_eq!(
        kind(fs::stat(&layout.places, &all, &path)),
        ErrorKind::Denied
    );
    assert_eq!(
        kind(fs::write(
            &layout.places,
            &all,
            &path,
            b"x",
            WriteMode::Append
        )),
        ErrorKind::Denied
    );
    assert_eq!(
        std::fs::read(layout.path("outside/secret.txt")).unwrap(),
        b"secret"
    );
}

#[test]
fn a_path_outside_the_grants_is_refused_before_it_is_said_not_to_exist() {
    let layout = Layout::new("existence");
    let all = grants(&["${workspace}/**"]);
    for path in [
        layout.asked("outside/nothing.txt"),
        layout.asked("nowhere/at/all.txt"),
    ] {
        assert_eq!(
            kind(fs::read(&layout.places, &all, &path, 100)),
            ErrorKind::Denied,
            "{path}"
        );
        assert_eq!(
            kind(fs::stat(&layout.places, &all, &path)),
            ErrorKind::Denied,
            "{path}"
        );
    }
    // Inside the grants, what is not there is said not to be.
    assert_eq!(
        kind(fs::read(
            &layout.places,
            &all,
            "${workspace}/nothing.txt",
            100
        )),
        ErrorKind::NotFound
    );
}

#[test]
fn a_relative_path_or_an_unknown_variable_is_refused() {
    let layout = Layout::new("relative");
    let all = grants(&["${workspace}/**"]);
    for path in ["notes.md", "./notes.md", "${secrets}/x", "${workspace}x"] {
        assert_eq!(
            kind(fs::read(&layout.places, &all, path, 100)),
            ErrorKind::Invalid,
            "{path}"
        );
    }
}

#[test]
fn a_symlink_out_of_the_workspace_leads_nowhere() {
    let layout = Layout::new("symlink");
    layout.dir.write("outside/secret.txt", b"secret");
    let made =
        common::folder_link(&layout.path("outside"), &layout.path("ws/out")).and_then(|()| {
            common::file_link(
                &layout.path("outside/secret.txt"),
                &layout.path("ws/secret-link.txt"),
            )
        });
    if let Err(why) = made {
        println!("skipped: {why}");
        return;
    }
    let all = grants(&["${workspace}/**"]);
    let places = &layout.places;
    for path in [
        "${workspace}/out/secret.txt",
        "${workspace}/secret-link.txt",
    ] {
        assert_eq!(
            kind(fs::read(places, &all, path, 100)),
            ErrorKind::Denied,
            "{path}"
        );
        assert_eq!(
            kind(fs::stat(places, &all, path)),
            ErrorKind::Denied,
            "{path}"
        );
        assert_eq!(
            kind(fs::write(places, &all, path, b"x", WriteMode::Replace)),
            ErrorKind::Denied,
            "{path}"
        );
    }
    assert_eq!(
        kind(fs::list(places, &all, "${workspace}/out", 10)),
        ErrorKind::Denied
    );
    assert_eq!(
        kind(fs::write(
            places,
            &all,
            "${workspace}/out/new.txt",
            b"x",
            WriteMode::CreateNew
        )),
        ErrorKind::Denied
    );
    assert!(!layout.path("outside/new.txt").exists());
    assert_eq!(
        std::fs::read(layout.path("outside/secret.txt")).unwrap(),
        b"secret"
    );
    // The link itself is listed as a link, not as what it leads to.
    let listed = fs::list(places, &all, "${workspace}", 10).unwrap().value;
    let out = listed.iter().find(|entry| entry.name == "out").unwrap();
    assert_eq!(out.kind, fs::EntryKind::Other);
}

#[test]
fn a_link_to_nothing_is_not_written_through() {
    let layout = Layout::new("dangling");
    let made = common::file_link(
        &layout.path("outside/not-yet.txt"),
        &layout.path("ws/dangling.txt"),
    );
    if let Err(why) = made {
        println!("skipped: {why}");
        return;
    }
    let all = grants(&["${workspace}/**"]);
    for mode in [WriteMode::Replace, WriteMode::Append, WriteMode::CreateNew] {
        assert_eq!(
            kind(fs::write(
                &layout.places,
                &all,
                "${workspace}/dangling.txt",
                b"x",
                mode
            )),
            ErrorKind::Denied
        );
    }
    assert!(!layout.path("outside/not-yet.txt").exists());
}

#[cfg(windows)]
#[test]
fn a_junction_out_of_the_workspace_leads_nowhere() {
    let layout = Layout::new("junction");
    layout.dir.write("outside/secret.txt", b"secret");
    common::junction(&layout.path("outside"), &layout.path("ws/out"));
    let all = grants(&["${workspace}/**"]);
    let places = &layout.places;
    assert_eq!(
        kind(fs::read(places, &all, "${workspace}/out/secret.txt", 100)),
        ErrorKind::Denied
    );
    assert_eq!(
        kind(fs::list(places, &all, "${workspace}/out", 10)),
        ErrorKind::Denied
    );
    assert_eq!(
        kind(fs::write(
            places,
            &all,
            "${workspace}/out/secret.txt",
            b"x",
            WriteMode::Replace
        )),
        ErrorKind::Denied
    );
    assert_eq!(
        kind(fs::write(
            places,
            &all,
            "${workspace}/out/new.txt",
            b"x",
            WriteMode::CreateNew
        )),
        ErrorKind::Denied
    );
    assert!(!layout.path("outside/new.txt").exists());
    assert_eq!(
        std::fs::read(layout.path("outside/secret.txt")).unwrap(),
        b"secret"
    );
}

#[cfg(windows)]
#[test]
fn a_junction_to_lumes_state_leads_nowhere_even_with_the_whole_disk_granted() {
    let layout = Layout::new("junction-state");
    layout.dir.write("state/lock.toml", b"lock");
    common::junction(&layout.path("state"), &layout.path("ws/state"));
    let everything = grants(&[&format!("{}/**", layout.asked(""))]);
    let places = &layout.places;
    assert_eq!(
        kind(fs::read(
            places,
            &everything,
            "${workspace}/state/lock.toml",
            100
        )),
        ErrorKind::Denied
    );
    assert_eq!(
        kind(fs::write(
            places,
            &everything,
            "${workspace}/state/lock.toml",
            b"x",
            WriteMode::Replace
        )),
        ErrorKind::Denied
    );
    assert_eq!(
        std::fs::read(layout.path("state/lock.toml")).unwrap(),
        b"lock"
    );
}

#[test]
fn a_write_through_a_hard_link_is_refused() {
    let layout = Layout::new("hard-link");
    let outside = layout.dir.write("outside/secret.txt", b"secret");
    std::fs::hard_link(&outside, layout.path("ws/linked.txt")).unwrap();
    let all = grants(&["${workspace}/**"]);
    for mode in [WriteMode::Replace, WriteMode::Append] {
        let error = fs::write(
            &layout.places,
            &all,
            "${workspace}/linked.txt",
            b"changed",
            mode,
        )
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Denied);
        assert!(error.message.contains("hard links"), "{}", error.message);
    }
    assert_eq!(std::fs::read(&outside).unwrap(), b"secret");
}

#[test]
fn the_workspace_unset_reaches_nothing() {
    let mut layout = Layout::new("no-workspace");
    layout.dir.write("ws/notes.md", b"notes");
    layout.places.workspace = None;
    let all = grants(&["${workspace}/**"]);
    let places = &layout.places;
    assert_eq!(
        kind(fs::read(places, &all, "${workspace}/notes.md", 100)),
        ErrorKind::Denied
    );
    // The folder that would be the workspace, asked for by its path, is not covered either.
    assert_eq!(
        kind(fs::read(places, &all, &layout.asked("ws/notes.md"), 100)),
        ErrorKind::Denied
    );
    assert_eq!(
        kind(fs::write(
            places,
            &all,
            "${workspace}/new.txt",
            b"x",
            WriteMode::Replace
        )),
        ErrorKind::Denied
    );
}

#[test]
fn lumes_own_folders_are_refused_whatever_is_granted() {
    let layout = Layout::new("own-folders");
    layout.dir.write("config/config.toml", b"[look]");
    layout.dir.write("state/plugins.lock", b"lock");
    let everything = grants(&[&format!("{}/**", layout.asked(""))]);
    let places = &layout.places;
    for path in [
        layout.asked("config/config.toml"),
        layout.asked("state/plugins.lock"),
        layout.asked(&format!("install/{}", exe("lume"))),
    ] {
        assert_eq!(
            kind(fs::read(places, &everything, &path, 100)),
            ErrorKind::Denied,
            "{path}"
        );
        assert_eq!(
            kind(fs::stat(places, &everything, &path)),
            ErrorKind::Denied,
            "{path}"
        );
        assert_eq!(
            kind(fs::write(
                places,
                &everything,
                &path,
                b"x",
                WriteMode::Replace
            )),
            ErrorKind::Denied,
            "{path}"
        );
    }
    for folder in ["config", "state", "install"] {
        assert_eq!(
            kind(fs::list(places, &everything, &layout.asked(folder), 10)),
            ErrorKind::Denied
        );
    }
    assert_eq!(
        kind(fs::write(
            places,
            &everything,
            &layout.asked("state/new.txt"),
            b"x",
            WriteMode::CreateNew
        )),
        ErrorKind::Denied
    );
    assert!(!layout.path("state/new.txt").exists());
    // Listing the folder above them leaves them out.
    let names: Vec<String> = fs::list(places, &everything, &layout.asked(""), 10)
        .unwrap()
        .value
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(names, ["home", "outside", "ws"]);
}

#[test]
fn lumes_own_folders_are_refused_however_their_names_are_cased() {
    let layout = Layout::new("own-folders-case");
    layout.dir.write("state/plugins.lock", b"lock");
    let everything = grants(&[&format!("{}/**", layout.asked(""))]);
    let path = layout.asked("STATE/plugins.lock");
    let refused = fs::write(&layout.places, &everything, &path, b"x", WriteMode::Replace);
    assert!(refused.is_err());
    if cfg!(windows) {
        assert_eq!(refused.unwrap_err().kind, ErrorKind::Denied);
    }
    assert_eq!(
        std::fs::read(layout.path("state/plugins.lock")).unwrap(),
        b"lock"
    );
}

#[test]
fn the_plugins_own_data_folder_is_reached_inside_lumes_state() {
    let layout = Layout::new("plugin-data");
    layout.dir.write("state/plugin-data/b.y/token", b"theirs");
    let own = grants(&["${plugin-data}/**"]);
    let places = &layout.places;
    fs::write(
        places,
        &own,
        "${plugin-data}/cache.json",
        b"{}",
        WriteMode::Replace,
    )
    .unwrap();
    assert_eq!(
        fs::read(places, &own, "${plugin-data}/cache.json", 100)
            .unwrap()
            .value,
        b"{}"
    );
    assert_eq!(
        fs::list(places, &own, "${plugin-data}", 10)
            .unwrap()
            .value
            .len(),
        1
    );
    // Another plugin's data folder lies in Lume's state like any other part of it.
    let everything = grants(&[&format!("{}/**", layout.asked(""))]);
    assert_eq!(
        kind(fs::read(
            places,
            &everything,
            &layout.asked("state/plugin-data/b.y/token"),
            100
        )),
        ErrorKind::Denied
    );
}

#[test]
fn a_folder_is_listed_only_when_the_folder_itself_is_granted() {
    let layout = Layout::new("list-folder");
    layout.dir.write("ws/a.md", b"a");
    let notes = grants(&["${workspace}/*.md"]);
    assert_eq!(
        kind(fs::list(&layout.places, &notes, "${workspace}", 10)),
        ErrorKind::Denied
    );
}

#[test]
fn changes_to_paths_no_grant_covers_are_not_told() {
    let layout = Layout::new("watch");
    let watched = grants(&["${workspace}", "${workspace}/allowed.txt"]);
    let (tell, told) = std::sync::mpsc::channel();
    let _watcher = fs::watch(
        &layout.places,
        &watched,
        "${workspace}",
        false,
        Box::new(move |event| {
            let _ = tell.send(event.path);
        }),
    )
    .unwrap();
    layout.dir.write("ws/secret.txt", b"secret");
    layout.dir.write("ws/allowed.txt", b"allowed");
    let mut seen = Vec::new();
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while let Ok(path) =
        told.recv_timeout(until.saturating_duration_since(std::time::Instant::now()))
    {
        seen.push(path);
        if seen.iter().any(|path| path.ends_with("allowed.txt")) {
            break;
        }
    }
    // Whatever is still on its way is in by now.
    while let Ok(path) = told.recv_timeout(Duration::from_millis(300)) {
        seen.push(path);
    }
    assert!(
        seen.iter().any(|path| path.ends_with("allowed.txt")),
        "{seen:?}"
    );
    assert!(!seen.iter().any(|path| path.contains("secret")), "{seen:?}");
}

// Programs.

#[test]
fn running_a_program_without_its_grant_is_refused() {
    let layout = Layout::new("no-run-grant");
    let shell = if cfg!(windows) { "cmd" } else { "sh" };
    let program = process::resolve(&layout.places, shell, Some(&common::system_path())).unwrap();
    assert_eq!(process::allowed(&[], &program), None);
    assert_eq!(process::allowed(&grants(&["git", "rg"]), &program), None);
}

#[test]
fn a_program_named_with_a_folder_is_refused() {
    let layout = Layout::new("program-path");
    for name in [
        "bin/sh",
        "../sh",
        "/bin/sh",
        "C:\\Windows\\System32\\cmd.exe",
        "",
        "-sh",
    ] {
        assert_eq!(
            kind(process::resolve(
                &layout.places,
                name,
                Some(&common::system_path())
            )),
            ErrorKind::Invalid,
            "{name}"
        );
    }
}

#[test]
fn lumes_own_programs_are_refused() {
    let layout = Layout::new("own-programs");
    let install = std::env::join_paths([layout.path("install")]).unwrap();
    assert_eq!(
        kind(process::resolve(&layout.places, "lume", Some(&install))),
        ErrorKind::Denied
    );
    // One of Lume's programs outside its folders is refused too.
    let mut places = layout.places.clone();
    let helper = layout
        .dir
        .write(&format!("outside/{}", exe("lume-helper")), b"");
    make_runnable(&helper);
    places.own_programs.push(helper);
    let outside = std::env::join_paths([layout.path("outside")]).unwrap();
    assert_eq!(
        kind(process::resolve(&places, "lume-helper", Some(&outside))),
        ErrorKind::Denied
    );
}

#[test]
fn a_program_in_lumes_folders_is_refused_even_in_the_plugins_data() {
    let layout = Layout::new("program-in-state");
    let tool = layout
        .dir
        .write(&format!("state/plugin-data/a.x/{}", exe("tool")), b"");
    make_runnable(&tool);
    let data = std::env::join_paths([layout.path("state/plugin-data/a.x")]).unwrap();
    assert_eq!(
        kind(process::resolve(&layout.places, "tool", Some(&data))),
        ErrorKind::Denied
    );
}

#[test]
fn a_program_reached_by_a_link_is_judged_by_where_it_is() {
    let layout = Layout::new("program-link");
    let link = layout.path(&format!("outside/{}", exe("innocent")));
    let made = common::file_link(&layout.places.own_programs[0], &link);
    if let Err(why) = made {
        println!("skipped: {why}");
        return;
    }
    let outside = std::env::join_paths([layout.path("outside")]).unwrap();
    assert_eq!(
        kind(process::resolve(&layout.places, "innocent", Some(&outside))),
        ErrorKind::Denied
    );
}

fn make_runnable(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let _ = path;
}

#[cfg(windows)]
#[test]
fn a_batch_file_needs_the_cmd_grant() {
    let layout = Layout::new("batch");
    layout.dir.write("outside/tool.cmd", b"@echo [%1] [%2]\r\n");
    let folder = std::env::join_paths([layout.path("outside")]).unwrap();
    let program = process::resolve(&layout.places, "tool", Some(&folder)).unwrap();
    assert_eq!(program.grant, "cmd");
    assert_eq!(process::allowed(&grants(&["tool"]), &program), None);
    assert_eq!(
        process::allowed(&grants(&["cmd"]), &program).as_deref(),
        Some("cmd")
    );
    // Its arguments reach it as arguments, not as commands.
    let output = process::run(
        &program,
        &["a & echo INJECTED".to_owned(), "%PATH%".to_owned()],
        process::RunOptions {
            cwd: None,
            env: Vec::new(),
            stdin: None,
            timeout: Duration::from_secs(10),
            max_output: 4096,
            stop: Default::default(),
        },
    )
    .unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.code, Some(0), "{text}");
    assert!(
        !text.lines().any(|line| line.trim() == "INJECTED"),
        "{text}"
    );
    assert!(text.contains("PATH"), "{text}");
    assert!(!text.contains("System32"), "{text}");
    // An argument that cannot be passed to a batch file safely is refused.
    let refused = process::run(
        &program,
        &["two\nlines".to_owned()],
        process::RunOptions {
            cwd: None,
            env: Vec::new(),
            stdin: None,
            timeout: Duration::from_secs(10),
            max_output: 4096,
            stop: Default::default(),
        },
    );
    assert_eq!(kind(refused), ErrorKind::Invalid);
}

// The network.

fn ok_server() -> Server {
    Server::new(vec![answer("200 OK", &[], b"hello")])
}

#[test]
fn fetching_without_a_net_grant_is_refused() {
    let server = ok_server();
    let none: Vec<String> = Vec::new();
    assert_eq!(
        kind(net::fetch(&none, get(&server.url("/")))),
        ErrorKind::Denied
    );
    assert_eq!(
        kind(net::fetch_stream(&none, get(&server.url("/")))),
        ErrorKind::Denied
    );
    assert!(server.got().is_empty());
}

#[test]
fn a_grant_for_one_port_does_not_reach_another() {
    let server = ok_server();
    let other = grants(&[&format!("127.0.0.1:{}", server.port.wrapping_add(1).max(1))]);
    assert_eq!(
        kind(net::fetch(&other, get(&server.url("/")))),
        ErrorKind::Denied
    );
    let default_ports = grants(&["127.0.0.1"]);
    assert_eq!(
        kind(net::fetch(&default_ports, get(&server.url("/")))),
        ErrorKind::Denied
    );
    assert!(server.got().is_empty());
}

#[test]
fn a_grant_naming_the_loopback_address_reaches_it() {
    let server = ok_server();
    let named = grants(&[&format!("127.0.0.1:{}", server.port)]);
    let response = net::fetch(&named, get(&server.url("/"))).unwrap();
    assert_eq!(response.value.head.status, 200);
    assert_eq!(response.value.body, b"hello");
    assert_eq!(response.grant, format!("127.0.0.1:{}", server.port));
}

#[test]
fn a_grant_naming_localhost_reaches_this_computer_by_that_name() {
    let server = ok_server();
    let named = grants(&[&format!("localhost:{}", server.port)]);
    let names = lookup(&[("localhost", "127.0.0.1")]);
    let url = format!("http://localhost:{}/", server.port);
    let response = net::fetch_with(&names, &named, get(&url)).unwrap();
    assert_eq!(response.value.body, b"hello");
}

#[test]
fn loopback_is_refused_to_a_grant_for_any_server() {
    let server = ok_server();
    let any = grants(&["*:*"]);
    assert_eq!(
        kind(net::fetch(&any, get(&server.url("/")))),
        ErrorKind::Denied
    );
    let v6 = format!("http://[::1]:{}/", server.port);
    assert_eq!(kind(net::fetch(&any, get(&v6))), ErrorKind::Denied);
    let mapped = format!("http://[::ffff:127.0.0.1]:{}/", server.port);
    assert_eq!(kind(net::fetch(&any, get(&mapped))), ErrorKind::Denied);
    let names = lookup(&[("localhost", "127.0.0.1")]);
    let by_name = format!("http://localhost:{}/", server.port);
    assert_eq!(
        kind(net::fetch_with(&names, &any, get(&by_name))),
        ErrorKind::Denied
    );
    assert!(server.got().is_empty());
}

#[test]
fn link_local_and_unspecified_addresses_are_refused() {
    let any = grants(&["*", "*:*"]);
    for url in [
        "http://169.254.169.254/latest/meta-data/",
        "http://[fe80::1]/",
        "http://0.0.0.0:8080/",
        "http://[::]/",
        "http://224.0.0.1/",
        "http://255.255.255.255/",
    ] {
        let error = net::fetch(&any, get(url)).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Denied, "{url}: {}", error.message);
    }
}

#[test]
fn a_name_that_leads_to_loopback_is_refused() {
    let server = ok_server();
    let names = lookup(&[
        ("rebind.test", "127.0.0.1"),
        ("mapped.test", "::ffff:127.0.0.1"),
        ("metadata.test", "169.254.169.254"),
    ]);
    for host in ["rebind.test", "mapped.test", "metadata.test"] {
        let granted = grants(&[&format!("{host}:{}", server.port), "*.test:*"]);
        let url = format!("http://{host}:{}/", server.port);
        let error = net::fetch_with(&names, &granted, get(&url)).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Denied, "{host}: {}", error.message);
    }
    assert!(server.got().is_empty());
}

#[test]
fn a_redirect_to_a_server_not_granted_is_refused() {
    let target = ok_server();
    let first = Server::new(vec![redirect("302 Found", &target.url("/"))]);
    let granted = grants(&[&format!("127.0.0.1:{}", first.port)]);
    assert_eq!(
        kind(net::fetch(&granted, get(&first.url("/")))),
        ErrorKind::Denied
    );
    assert_eq!(first.got().len(), 1);
    assert!(target.got().is_empty());
}

#[test]
fn a_redirect_to_a_name_that_leads_to_loopback_is_refused() {
    let target = ok_server();
    let first = Server::new(vec![redirect(
        "307 Temporary Redirect",
        &format!("http://rebind.test:{}/", target.port),
    )]);
    let names = lookup(&[("rebind.test", "127.0.0.1")]);
    let granted = grants(&[&format!("127.0.0.1:{}", first.port), "rebind.test:*"]);
    let error = net::fetch_with(&names, &granted, get(&first.url("/"))).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Denied, "{}", error.message);
    assert!(target.got().is_empty());
}

#[test]
fn a_redirect_to_link_local_or_loopback_under_a_wide_grant_is_refused() {
    for location in [
        "http://169.254.169.254/latest/meta-data/".to_owned(),
        "http://[::1]/".to_owned(),
    ] {
        let first = Server::new(vec![redirect("301 Moved Permanently", &location)]);
        let granted = grants(&[&format!("127.0.0.1:{}", first.port), "*", "*:*"]);
        let error = net::fetch(&granted, get(&first.url("/"))).unwrap_err();
        assert_eq!(
            error.kind,
            ErrorKind::Denied,
            "{location}: {}",
            error.message
        );
    }
}

#[test]
fn a_redirect_to_another_scheme_is_refused() {
    let first = Server::new(vec![redirect("302 Found", "file:///etc/passwd")]);
    let granted = grants(&["*:*", &format!("127.0.0.1:{}", first.port)]);
    assert_eq!(
        kind(net::fetch(&granted, get(&first.url("/")))),
        ErrorKind::Denied
    );
}

#[test]
fn too_many_redirects_are_refused() {
    let server = Server::new((0..6).map(|_| redirect("302 Found", "/again")).collect());
    let granted = grants(&[&format!("127.0.0.1:{}", server.port)]);
    let error = net::fetch(&granted, get(&server.url("/"))).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Failed);
    assert_eq!(server.got().len(), 6);
}

#[test]
fn an_authorization_header_is_not_sent_to_another_server() {
    let target = Server::new(vec![answer("200 OK", &[], b"done")]);
    let first = Server::new(vec![redirect("302 Found", &target.url("/next"))]);
    let granted = grants(&[
        &format!("127.0.0.1:{}", first.port),
        &format!("127.0.0.1:{}", target.port),
    ]);
    let mut request = get(&first.url("/"));
    request.headers = vec![
        ("Authorization".to_owned(), "Bearer token".to_owned()),
        ("Cookie".to_owned(), "session=1".to_owned()),
        ("X-Kept".to_owned(), "yes".to_owned()),
    ];
    let response = net::fetch(&granted, request).unwrap();
    assert_eq!(response.value.body, b"done");
    assert_eq!(response.grant, format!("127.0.0.1:{}", target.port));
    assert_eq!(
        first.got()[0].header("authorization").as_deref(),
        Some("Bearer token")
    );
    let second = &target.got()[0];
    assert_eq!(second.header("authorization"), None);
    assert_eq!(second.header("cookie"), None);
    assert_eq!(second.header("x-kept").as_deref(), Some("yes"));
}

#[test]
fn only_http_and_https_are_fetched() {
    let any = grants(&["*", "*:*"]);
    for url in [
        "ftp://example.org/",
        "file:///etc/passwd",
        "gopher://example.org/",
    ] {
        assert_eq!(
            kind(net::fetch(&any, get(url))),
            ErrorKind::Invalid,
            "{url}"
        );
    }
}

#[test]
fn headers_that_frame_the_connection_cannot_be_set() {
    let server = ok_server();
    let granted = grants(&[&format!("127.0.0.1:{}", server.port)]);
    for name in [
        "Host",
        "Connection",
        "Content-Length",
        "Transfer-Encoding",
        "Proxy-Authorization",
    ] {
        let mut request = get(&server.url("/"));
        request.headers = vec![(name.to_owned(), "x".to_owned())];
        assert_eq!(
            kind(net::fetch(&granted, request)),
            ErrorKind::Invalid,
            "{name}"
        );
    }
    assert!(server.got().is_empty());
}

#[test]
fn a_response_past_its_limit_is_refused() {
    let server = Server::new(vec![
        answer("200 OK", &[], &[b'x'; 100]),
        answer("200 OK", &[], &[b'x'; 100]),
    ]);
    let granted = grants(&[&format!("127.0.0.1:{}", server.port)]);
    let small = FetchRequest {
        max_bytes: 10,
        ..get(&server.url("/"))
    };
    assert_eq!(kind(net::fetch(&granted, small)), ErrorKind::TooLarge);
    let streamed = FetchRequest {
        max_bytes: 10,
        ..get(&server.url("/"))
    };
    let (_, mut reader) = net::fetch_stream(&granted, streamed).unwrap().value;
    let mut body = Vec::new();
    let error = std::io::Read::read_to_end(&mut reader, &mut body).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::FileTooLarge);
    assert!(body.len() <= 10);
}

// Secrets and streams.

#[test]
fn a_secret_under_a_key_that_is_not_one_is_refused_before_the_keychain_is_asked() {
    for key in ["", "a/b", "a b", "../other"] {
        assert_eq!(
            kind(OsKeychain.get("a.x", key)),
            ErrorKind::Invalid,
            "{key}"
        );
        assert_eq!(
            kind(OsKeychain.set("a.x", key, "x")),
            ErrorKind::Invalid,
            "{key}"
        );
        assert_eq!(
            kind(OsKeychain.delete("a.x", key)),
            ErrorKind::Invalid,
            "{key}"
        );
    }
    let large = "x".repeat(lume_services::secrets::SECRET_MAX + 1);
    assert_eq!(
        kind(OsKeychain.set("a.x", "token", &large)),
        ErrorKind::Invalid
    );
}

#[test]
fn another_plugins_stream_cannot_be_reached() {
    let streams = Streams::new(4);
    let id = streams
        .add(
            "a.x",
            Some(Box::new(std::io::Cursor::new(b"secret".to_vec()))),
            None,
        )
        .unwrap();
    assert_eq!(kind(streams.read("b.y", id, 10)), ErrorKind::NotFound);
    assert_eq!(kind(streams.write("b.y", id, b"x")), ErrorKind::NotFound);
    assert_eq!(kind(streams.close("b.y", id)), ErrorKind::NotFound);
    assert_eq!(
        streams.read("a.x", id, 10).unwrap().as_deref(),
        Some(&b"secret"[..])
    );
}
