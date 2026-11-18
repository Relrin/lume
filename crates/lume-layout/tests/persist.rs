//! Workspaces are kept in files and come back as they were, and a file that cannot be
//! used is moved aside, never lost and never fatal.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use common::*;
use lume_layout::tuning::{CORRUPT_KEPT, WORKSPACE_FILE_VERSION};
use lume_layout::{
    Area, Dir, Intent, Loaded, PanelContent, PanelId, Store, StoreError, Workspace, WorkspaceFile,
};

static NEXT: AtomicU32 = AtomicU32::new(1);

/// A directory of its own for one test, removed when the test ends.
struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new() -> Self {
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("lume-layout-test-{}-{number}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        // The store's own directory is one level down, and is not made here: making
        // it is the store's to do.
        Self { path }
    }

    fn store(&self) -> Store {
        Store::new(self.path.join("workspaces"))
    }

    fn file(&self, name: &str) -> PathBuf {
        self.path.join("workspaces").join(name)
    }

    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.path.join("workspaces"))
            .map(|entries| {
                entries
                    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// A workspace with something of everything in it: tabs, splits, a floating pane, a
/// docked panel, the scratch terminal put away, and sessions.
fn furnished(name: &str) -> Workspace {
    let (mut workspace, window, _, a) = one_pane();
    workspace.name = name.to_owned();
    let b = split(&mut workspace, a, Dir::Right);
    split(&mut workspace, b, Dir::Down);
    let (_, d) = open_tab(&mut workspace, window);
    let e = split(&mut workspace, d, Dir::Left);
    must(
        &mut workspace,
        Intent::Float {
            pane: e,
            rect: None,
        },
    );
    must(
        &mut workspace,
        Intent::PanelSet {
            window,
            panel: PanelId::new("lume.tabs").unwrap(),
            content: PanelContent::TabList {},
            area: Some(Area::Left),
            index: 0,
        },
    );
    must_with(
        &mut workspace,
        Intent::ScratchShow {
            window,
            content: Some(terminal()),
        },
        Some(started(9)),
    );
    must(&mut workspace, Intent::ScratchHide {});
    must(
        &mut workspace,
        Intent::SessionSet {
            pane: a,
            session: lume_proto::SessionId::new(4),
            backend: None,
            cwd: Some("/projects/lume".to_owned()),
        },
    );
    workspace
}

fn loaded(store: &Store, name: &str) -> Workspace {
    match store.load(name, 1_000).unwrap() {
        Loaded::Workspace(workspace) => *workspace,
        other => panic!("`{name}` was not read: {other:?}"),
    }
}

fn set_aside(store: &Store, name: &str, now: u64) -> (PathBuf, String) {
    match store.load(name, now).unwrap() {
        Loaded::SetAside { to, reason } => (to, reason),
        other => panic!("`{name}` was not set aside: {other:?}"),
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn a_workspace_comes_back_as_it_was_saved_with_its_sessions() {
    let dir = TestDir::new();
    let store = dir.store();
    let workspace = furnished("default");
    assert_eq!(workspace.sessions().len(), 2);

    store.save(&workspace, "1.2.3", 42).unwrap();
    let back = loaded(&store, "default");
    assert_eq!(back, workspace);
    assert_eq!(back.sessions().len(), 2);
    // The directory a terminal was started in is kept: it is started there again.
    assert!(format!("{back:?}").contains("/projects/lume"));

    // Nothing is left beside the file from writing it.
    assert_eq!(dir.names(), vec!["default.json"]);
    let text = fs::read_to_string(dir.file("default.json")).unwrap();
    let file: WorkspaceFile = serde_json::from_str(&text).unwrap();
    assert_eq!(file.version, WORKSPACE_FILE_VERSION);
    assert_eq!(file.app_version, "1.2.3");
    assert_eq!(file.saved_unix_ms, 42);
    assert!(text.ends_with('\n'));
}

#[test]
fn saving_again_replaces_the_file() {
    let dir = TestDir::new();
    let store = dir.store();
    let mut workspace = furnished("default");
    store.save(&workspace, "1", 1).unwrap();

    let pane = workspace.panes()[0].id;
    must(&mut workspace, Intent::Close { pane });
    store.save(&workspace, "1", 2).unwrap();
    assert_eq!(loaded(&store, "default"), workspace);
}

#[test]
fn a_workspace_that_was_never_saved_is_missing() {
    let dir = TestDir::new();
    let store = dir.store();
    assert_eq!(store.load("default", 0).unwrap(), Loaded::Missing);
    assert_eq!(store.list().unwrap(), Vec::<String>::new());
    assert!(!store.contains("default").unwrap());
}

#[test]
fn a_file_that_is_not_a_workspace_is_moved_aside_and_reported() {
    let dir = TestDir::new();
    let store = dir.store();
    write(&dir.file("default.json"), "{ this is not json");

    let (to, reason) = set_aside(&store, "default", 5_000);
    assert_eq!(to, dir.file("default.json.corrupt-5000"));
    assert!(reason.contains("not JSON"), "{reason}");
    assert_eq!(
        fs::read_to_string(&to).unwrap(),
        "{ this is not json",
        "the file is kept as it was"
    );
    // The name is free again: the application starts afresh under it.
    assert_eq!(store.load("default", 5_001).unwrap(), Loaded::Missing);
    assert_eq!(store.list().unwrap(), Vec::<String>::new());
}

#[test]
fn a_file_that_is_not_text_is_moved_aside() {
    let dir = TestDir::new();
    let store = dir.store();
    fs::create_dir_all(dir.file("")).unwrap();
    fs::write(dir.file("default.json"), [0xff, 0xfe, 0x00, 0x80]).unwrap();
    let (to, _) = set_aside(&store, "default", 7);
    assert!(to.is_file());
}

#[test]
fn a_layout_that_is_not_sound_is_moved_aside() {
    let dir = TestDir::new();
    let store = dir.store();
    let workspace = furnished("default");
    store.save(&workspace, "1", 1).unwrap();
    let path = dir.file("default.json");

    // The same file with a split whose shares no longer add up.
    let text = fs::read_to_string(&path).unwrap();
    let broken = text.replacen("500000", "400000", 1);
    assert_ne!(broken, text);
    fs::write(&path, broken).unwrap();
    let (_, reason) = set_aside(&store, "default", 10);
    assert!(reason.contains("not sound"), "{reason}");

    // And with a field this version does not know.
    store.save(&workspace, "1", 1).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        text.replacen("\"name\"", "\"colour\": 1, \"name\"", 1),
    )
    .unwrap();
    let (_, reason) = set_aside(&store, "default", 11);
    assert!(reason.contains("does not describe a workspace"), "{reason}");
}

#[test]
fn only_the_latest_few_unusable_files_are_kept() {
    let dir = TestDir::new();
    let store = dir.store();
    let rounds = CORRUPT_KEPT as u64 + 3;
    for round in 1..=rounds {
        write(&dir.file("default.json"), "broken");
        set_aside(&store, "default", round * 100);
    }
    let kept = dir.names();
    assert_eq!(kept.len(), CORRUPT_KEPT);
    assert!(kept.contains(&format!("default.json.corrupt-{}", rounds * 100)));
    assert!(!kept.contains(&"default.json.corrupt-100".to_owned()));
}

#[test]
fn a_file_from_a_later_version_is_set_aside_under_its_version() {
    let dir = TestDir::new();
    let store = dir.store();
    let later = format!(
        r#"{{ "version": {}, "written": "by a later Lume" }}"#,
        WORKSPACE_FILE_VERSION + 4
    );
    write(&dir.file("default.json"), &later);

    let (to, reason) = set_aside(&store, "default", 1);
    let expected = format!("default.v{}.json.bak", WORKSPACE_FILE_VERSION + 4);
    assert_eq!(to, dir.file(&expected));
    assert!(reason.contains("newer version"), "{reason}");
    assert_eq!(fs::read_to_string(to).unwrap(), later);
}

#[test]
fn a_file_is_the_workspace_its_name_says() {
    let dir = TestDir::new();
    let store = dir.store();
    store.save(&furnished("default"), "1", 1).unwrap();
    // Copied by hand under another name.
    fs::copy(dir.file("default.json"), dir.file("copy.json")).unwrap();
    assert_eq!(loaded(&store, "copy").name, "copy");
}

#[test]
fn workspaces_are_listed_renamed_and_deleted() {
    let dir = TestDir::new();
    let store = dir.store();
    for name in ["work", "Api v2", "default"] {
        store.save(&furnished(name), "1", 1).unwrap();
    }
    // Files that are not workspaces are not listed.
    write(&dir.file("default.json.corrupt-12"), "x");
    write(&dir.file("notes.txt"), "x");
    write(&dir.file("con.json"), "x");
    assert_eq!(store.list().unwrap(), vec!["Api v2", "default", "work"]);
    assert!(store.contains("WORK").unwrap());

    store.rename("work", "play").unwrap();
    assert_eq!(store.list().unwrap(), vec!["Api v2", "default", "play"]);
    assert_eq!(loaded(&store, "play").name, "play");

    // Two workspaces may not differ only in the case of their letters, but one may
    // have the case of its own name changed.
    let taken = store.rename("play", "DEFAULT").unwrap_err();
    assert!(matches!(taken, StoreError::Exists(name) if name == "default"));
    store.rename("play", "Play").unwrap();
    assert!(store.list().unwrap().contains(&"Play".to_owned()));

    let missing = store.rename("nobody", "somebody").unwrap_err();
    assert!(matches!(missing, StoreError::Missing(name) if name == "nobody"));

    store.delete("Play").unwrap();
    assert_eq!(store.list().unwrap(), vec!["Api v2", "default"]);
    assert!(matches!(
        store.delete("Play").unwrap_err(),
        StoreError::Missing(_)
    ));
}

#[test]
fn a_name_that_is_not_a_plain_file_name_reaches_no_file() {
    let dir = TestDir::new();
    let store = dir.store();
    let mut workspace = furnished("default");
    for name in ["../outside", "a/b", "..", "", "nul", "trailing."] {
        assert!(
            matches!(store.path(name), Err(StoreError::Name(_))),
            "{name}"
        );
        assert!(matches!(store.load(name, 0), Err(StoreError::Name(_))));
        assert!(matches!(store.delete(name), Err(StoreError::Name(_))));
        assert!(matches!(
            store.rename("default", name),
            Err(StoreError::Name(_))
        ));
        workspace.name = name.to_owned();
        assert!(matches!(
            store.save(&workspace, "1", 1),
            Err(StoreError::Name(_))
        ));
    }
    assert!(!dir.path.join("outside.json").exists());
    assert!(dir.names().is_empty());
}

#[test]
fn groups_and_what_was_put_away_come_back_and_a_first_version_file_still_reads() {
    let dir = TestDir::new();
    let store = dir.store();
    let mut workspace = furnished("default");
    let window = workspace.windows[0].id;
    let tabs: Vec<_> = workspace.windows[0]
        .centre
        .tabs
        .iter()
        .map(|tab| tab.id)
        .collect();
    let kept = must(&mut workspace, Intent::GroupNew { tab: tabs[0] })
        .1
        .group
        .unwrap();
    must(
        &mut workspace,
        Intent::GroupSet {
            group: kept,
            name: Some(Some("Kept".to_owned())),
            color: Some(lume_layout::TabColor::Purple),
            icon: Some(Some(lume_layout::GroupIcon::Server)),
            collapsed: None,
            kept: Some(true),
        },
    );
    must(&mut workspace, Intent::GroupClose { group: kept });
    must(&mut workspace, Intent::GroupNew { tab: tabs[1] });
    must(
        &mut workspace,
        Intent::TabLock {
            tab: tabs[1],
            on: true,
        },
    );
    must(
        &mut workspace,
        Intent::TabColor {
            tab: tabs[1],
            color: Some(lume_layout::TabColor::Red),
        },
    );
    assert_eq!(workspace.shelf.len(), 1);
    assert_eq!(workspace.window(window).unwrap().centre.groups.len(), 1);

    store.save(&workspace, "1", 1).unwrap();
    assert_eq!(loaded(&store, "default"), workspace);

    // A file written before groups existed says version 1 and has none of their fields.
    let (old, ..) = one_pane();
    let text = serde_json::json!({
        "version": 1,
        "app_version": "0.1.0",
        "saved_unix_ms": 1,
        "workspace": old,
    });
    write(&dir.file("old.json"), &text.to_string());
    let mut expected = old.clone();
    expected.name = "old".to_owned();
    assert_eq!(loaded(&store, "old"), expected);
}
