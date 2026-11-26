use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stored_values_changed_together_are_all_kept() {
    use crate::marks::Marks;
    use crate::testing::TempDir;

    let dir = TempDir::new("kv-together");
    std::fs::create_dir_all(&dir.dirs().config).unwrap();
    let state = AppState::new(
        tokio::runtime::Handle::current(),
        lume_session::Router::new(),
        Vec::new(),
        Duration::from_secs(600),
        Arc::new(Marks::start()),
    );
    let config = ConfigService::load(dir.dirs(), None);
    let plugins = Plugins::new(
        config.dirs(),
        Arc::new(crate::builtins::registry()),
        state,
        Arc::clone(&config),
    );
    // Two changes made in one turn are carried out side by side; neither is lost.
    let set = |key: &'static str| {
        let plugins = Arc::clone(&plugins);
        tokio::spawn(async move {
            plugins
                .kv("acme.kept", move |map| {
                    map.insert(key.to_owned(), json!(true));
                    Ok(Value::Null)
                })
                .await
        })
    };
    let (one, other) = (set("one"), set("other"));
    one.await.unwrap().unwrap();
    other.await.unwrap().unwrap();
    let keys = plugins
        .kv("acme.kept", |map| {
            Ok(json!(map.keys().cloned().collect::<Vec<_>>()))
        })
        .await
        .unwrap();
    assert_eq!(keys, json!(["one", "other"]));

    // What is written is read back by the next run of the plugin.
    plugins.release("acme.kept");
    let file = plugins.store.data_dir("acme.kept").join(KV_FILE);
    let written: Value = serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
    assert_eq!(written, json!({ "one": true, "other": true }));
}

#[test]
fn a_session_that_is_not_of_this_machine_names_no_folder_and_no_process() {
    use lume_proto::{ExitStatus, Foreground, SessionEvent, SessionState};

    let foreground = Foreground {
        pid: 4711,
        name: "vim".to_owned(),
    };
    let cwd = SessionNews::Event(SessionEvent::Cwd {
        url: "file:///C:/secret".to_owned(),
    });
    let running = SessionNews::Event(SessionEvent::Foreground(foreground.clone()));
    let whole = SessionNews::Event(SessionEvent::State {
        state: SessionState {
            title: Some("a title".to_owned()),
            cwd: Some("file:///C:/secret".to_owned()),
            foreground: Some(foreground),
            ..SessionState::default()
        },
    });
    let told = |what: &SessionNews, here| {
        report_of(what, here).map(|(kind, details)| (kind, details.to_string()))
    };

    assert_eq!(told(&cwd, false), None);
    let (kind, details) = told(&running, false).unwrap();
    assert_eq!(
        (kind.as_str(), details.as_str()),
        ("foreground", r#"{"name":"vim"}"#)
    );
    let (kind, details) = told(&whole, false).unwrap();
    assert_eq!(kind, "state");
    assert!(details.contains("a title") && details.contains("vim"));
    assert!(!details.contains("secret") && !details.contains("4711"));

    // Of this machine everything is told, as it was said.
    let (kind, details) = report_of(&cwd, true).unwrap();
    assert_eq!(kind, "cwd");
    assert_eq!(details, json!({ "url": "file:///C:/secret" }));
    for what in [&running, &whole] {
        assert!(told(what, true).unwrap().1.contains("4711"));
    }
    let bell = SessionNews::Event(SessionEvent::Bell);
    let exit = SessionNews::Exit(ExitStatus::Code(3));
    for what in [&bell, &exit] {
        assert_eq!(report_of(what, false), report_of(what, true));
    }
}

fn rows(value: Value) -> Vec<PanelRow> {
    serde_json::from_value(value).unwrap()
}

#[test]
fn a_panels_rows_are_held_to_their_limits_and_run_the_plugins_own_commands() {
    let commands: BTreeSet<String> = ["open".to_owned()].into_iter().collect();
    let good = rows(json!([
        { "id": "a", "label": "A", "children": [
            { "id": "a1", "label": "A1", "actions": [{ "command": "open", "title": "Open" }] }
        ] },
        { "id": "b", "label": "B" },
    ]));
    let mut count = 0;
    assert_eq!(check_rows(&good, 1, &mut count, &commands), Ok(()));
    assert_eq!(count, 3);

    let foreign = rows(json!([
        { "id": "a", "label": "A", "actions": [{ "command": "lume.app.quit", "title": "Quit" }] }
    ]));
    assert!(
        check_rows(&foreign, 1, &mut 0, &commands)
            .unwrap_err()
            .contains("lume.app.quit")
    );
    let twice = rows(json!([{ "id": "a", "label": "A" }, { "id": "a", "label": "B" }]));
    assert!(
        check_rows(&twice, 1, &mut 0, &commands)
            .unwrap_err()
            .contains("called a")
    );
    let long = rows(json!([{ "id": "a", "label": "x".repeat(PANEL_TEXT_MAX + 1) }]));
    assert!(check_rows(&long, 1, &mut 0, &commands).is_err());
    let mut deep = json!({ "id": "leaf", "label": "leaf" });
    for level in 0..PANEL_DEPTH_MAX {
        deep = json!({ "id": format!("l{level}"), "label": "x", "children": [deep] });
    }
    assert!(
        check_rows(&rows(json!([deep])), 1, &mut 0, &commands)
            .unwrap_err()
            .contains("deep")
    );
    let many: Vec<Value> = (0..=PANEL_ROWS_MAX)
        .map(|at| json!({ "id": format!("r{at}"), "label": "r" }))
        .collect();
    assert!(
        check_rows(&rows(Value::Array(many)), 1, &mut 0, &commands)
            .unwrap_err()
            .contains("rows at most")
    );
}

#[test]
fn a_row_is_found_by_the_ids_down_to_it_and_rows_are_counted_at_every_depth() {
    let mut held = json!([
        { "id": "a", "label": "A", "lazy": true, "children": [
            { "id": "b", "label": "B", "children": [{ "id": "c", "label": "C" }] }
        ] },
        { "id": "d", "label": "D" },
    ]);
    assert_eq!(count_rows(&held), 4);
    let path = |ids: &[&str]| ids.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        row_at(&mut held, &path(&["a", "b", "c"])).unwrap()["label"],
        "C"
    );
    assert_eq!(row_at(&mut held, &path(&["d"])).unwrap()["label"], "D");
    assert!(row_at(&mut held, &path(&["a", "x"])).is_none());
    assert!(row_at(&mut held, &path(&["d", "e"])).is_none());
    assert!(row_at(&mut held, &[]).is_none());

    // A row that came with the rows it would otherwise be asked for has them.
    full_commands(&mut held, "acme.tools");
    assert_eq!(held[0]["loaded"], true);
    assert!(held[1].get("loaded").is_none());
}

#[test]
fn a_window_is_told_what_a_plugin_showed_as_one_and_only_the_rows_it_shows() {
    let mut told = Told::default();
    told.statuses
        .push(json!({ "item": "acme.cpu", "pane": null, "value": { "text": "4%" } }));
    told.panels.insert(
        "acme.procs".to_owned(),
        json!([{ "id": "a", "label": "A" }]),
    );
    told.panels.insert("acme.files".to_owned(), Value::Null);
    let on_screen: BTreeSet<String> = ["acme.procs".to_owned()].into_iter().collect();

    let seen = told.notice("acme", Some(&on_screen)).unwrap();
    assert_eq!(seen["type"], "plugin_frame");
    assert_eq!(seen["statuses"][0]["value"]["text"], "4%");
    assert_eq!(seen["panels"].as_object().unwrap().len(), 1);
    assert_eq!(seen["panels"]["acme.procs"][0]["id"], "a");

    // A window with nothing of it on screen still hears of the status items.
    let other = told.notice("acme", Some(&BTreeSet::new())).unwrap();
    assert!(other["panels"].as_object().unwrap().is_empty());

    // Rows alone are not told to a window that does not show their panel.
    let rows_only = Told {
        panels: told.panels.clone(),
        ..Told::default()
    };
    assert!(rows_only.notice("acme", Some(&BTreeSet::new())).is_none());
    assert_eq!(
        rows_only.notice("acme", None).unwrap()["panels"]
            .as_object()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn a_rows_actions_name_the_plugins_commands_in_full() {
    let mut value = json!([
        { "id": "a", "label": "A", "actions": [{ "command": "open", "title": "Open" }],
          "children": [{ "id": "b", "label": "B", "actions": [{ "command": "stop", "title": "Stop" }] }] }
    ]);
    full_commands(&mut value, "acme.tools");
    assert_eq!(value[0]["actions"][0]["command"], "acme.tools.open");
    assert_eq!(
        value[0]["children"][0]["actions"][0]["command"],
        "acme.tools.stop"
    );
}
