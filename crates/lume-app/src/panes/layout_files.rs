//! Tabs opened from layout files, and what follows when a file changes.
//!
//! A layout file's tabs are put into a window whole, their panes without sessions:
//! the window's page starts those as it starts the panes of a workspace read from its
//! file. Each tab remembers the file it came from. When the file is saved again, the
//! tab is given the file's new shape, at once or once its window's user says so, as
//! the settings ask. A pane the file still names keeps its session; a pane it no longer
//! has closes, unless something still runs in it, in which case it is put away.

use lume_config::layouts::{Layout, Layouts, assign_keys, layout_id, layout_text, split_origin};
use lume_config::schema::FilesReapply;
use lume_layout::{
    Group, GroupId, Node, Shelved, Tab, TabColor, TabId, reshape_leaves, tuning::UNITS,
};

use super::launch::expand_home;
use super::*;

/// What opening a layout made: the tab, or the group of its tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LayoutOpened {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab: Option<TabId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<GroupId>,
}

/// What applying a layout file to a tab again did to the panes that had no place in
/// it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LayoutReapplied {
    /// The panes that closed: nothing ran in them.
    pub closed: Vec<PaneId>,
    /// The panes that were put away, their programs still running.
    pub put_away: Vec<PaneId>,
}

/// Where a tab was saved as a layout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LayoutSaved {
    pub id: String,
    pub file: String,
}

/// A layout's tab as it is opened: with `~` in its directories made the home directory.
fn prepared(tab: &Tab) -> Tab {
    let mut tab = tab.clone();
    expand_dirs(&mut tab.root);
    tab
}

fn expand_dirs(node: &mut Node) {
    match node {
        Node::Pane(pane) => {
            if let PaneContent::Terminal(terminal) = &mut pane.content
                && let Some(cwd) = &terminal.cwd
            {
                terminal.cwd = Some(expand_home(cwd));
            }
        }
        Node::Split(split) => split.children.iter_mut().for_each(expand_dirs),
    }
}

/// Whether a tab already has the shape a layout's tab gives it: the same splits, the
/// same keys in the same places, and shares no further apart than a file's whole
/// percentages put them.
fn fits(tab: &Tab, wanted: &Tab) -> bool {
    let keys = |tab: &Tab| -> Vec<Option<String>> {
        tab.root
            .panes()
            .into_iter()
            .map(|pane| pane.key.clone())
            .collect()
    };
    let close = tab
        .root
        .splits()
        .iter()
        .zip(wanted.root.splits())
        .all(|(is, was)| {
            is.ratios
                .iter()
                .zip(&was.ratios)
                .all(|(a, b)| a.abs_diff(*b) <= UNITS / 100)
        });
    tab.root.same_shape(&wanted.root)
        && keys(tab) == keys(wanted)
        && close
        && tab.title == wanted.title
        && tab.color == wanted.color
}

/// The layout and the tab of it a tab was opened from.
fn source_of<'a>(layouts: &'a Layouts, tab: &Tab) -> Option<(&'a Layout, &'a Tab)> {
    let origin = tab.origin.as_deref()?;
    let (id, _) = split_origin(origin)?;
    let layout = layouts.get(id)?;
    Some((layout, layout.tab_for(origin)?))
}

impl Panes {
    /// Opens the tabs of the layout file `id` in `window`, or in the window last
    /// worked in.
    pub async fn open_layout(
        &self,
        id: &str,
        window: Option<WindowId>,
    ) -> Result<LayoutOpened, CommandError> {
        self.own_workspace("opening a layout")?;
        let layouts = self.config.current().layouts.clone();
        let layout = layouts
            .get(id)
            .ok_or_else(|| CommandError::not_found(format!("there is no layout `{id}`")))?;
        let workspace = self.workspace();
        let window = window
            .or(workspace.focused_window)
            .or_else(|| workspace.windows.first().map(|window| window.id))
            .ok_or_else(|| CommandError::bad_request("there is no window to open it in"))?;
        let tabs: Vec<Tab> = layout.tabs.iter().map(prepared).collect();
        let intent = match <[Tab; 1]>::try_from(tabs) {
            Ok([tab]) => Intent::TabGraft {
                window,
                tab: Box::new(tab),
                index: None,
            },
            Err(tabs) => Intent::GroupGraft {
                window,
                group: Box::new(Shelved {
                    group: Group {
                        id: GroupId::new(0),
                        name: Some(layout.name.clone()),
                        color: layout.color.unwrap_or(TabColor::Blue),
                        icon: None,
                        collapsed: false,
                        kept: false,
                        muted: false,
                    },
                    tabs,
                }),
            },
        };
        let created = self.apply_core(intent).await?.created;
        Ok(LayoutOpened {
            tab: created.tab,
            group: created.group,
        })
    }

    /// The panes of `tab` that applying its layout file again would take out of it,
    /// and which of those still run something.
    pub fn layout_leaving(&self, tab: TabId) -> Option<(Vec<PaneId>, Vec<PaneId>)> {
        let workspace = self.workspace();
        let current = workspace.tab(tab)?;
        let layouts = &self.config.current().layouts;
        let (_, wanted) = source_of(layouts, current)?;
        let leaving = reshape_leaves(current, wanted);
        let busy = leaving
            .iter()
            .copied()
            .filter(|pane| self.busy(*pane))
            .collect();
        Some((leaving, busy))
    }

    /// Gives a tab opened from a layout file the shape the file has now. Panes that
    /// have no place in it close, or are put away while something runs in them.
    pub async fn reapply_layout(&self, tab: TabId) -> Result<LayoutReapplied, CommandError> {
        self.own_workspace("applying a layout")?;
        let workspace = self.workspace();
        let current = workspace
            .tab(tab)
            .ok_or_else(|| CommandError::not_found(format!("there is no tab {tab}")))?;
        let layouts = self.config.current().layouts.clone();
        let (_, wanted) = source_of(&layouts, current).ok_or_else(|| {
            CommandError::not_found("the tab was not opened from a layout file there still is")
        })?;
        let leaving = reshape_leaves(current, wanted);
        let (put_away, closed): (Vec<PaneId>, Vec<PaneId>) =
            leaving.into_iter().partition(|pane| self.busy(*pane));
        self.apply_core(Intent::TabReshape {
            tab,
            wanted: Box::new(prepared(wanted)),
            put_away: put_away.clone(),
        })
        .await?;
        Ok(LayoutReapplied { closed, put_away })
    }

    /// Lets a tab go of the layout file it was opened from: a change to the file no
    /// longer reaches it.
    pub async fn detach_layout(&self, tab: TabId) -> Result<(), CommandError> {
        self.apply_core(Intent::TabOrigin {
            tab,
            origin: None,
            keys: Vec::new(),
        })
        .await?;
        Ok(())
    }

    /// Writes a tab as a layout file called `name`, and ties the tab to the file. A
    /// file of that name is replaced only when `replace` says so.
    pub async fn save_tab_as_layout(
        &self,
        tab: TabId,
        name: &str,
        replace: bool,
    ) -> Result<LayoutSaved, CommandError> {
        let name = name.trim();
        let id = layout_id(name).ok_or_else(|| {
            CommandError::bad_request(format!(
                "`{name}` has nothing in it a file could be called by"
            ))
        })?;
        let workspace = self.workspace();
        let found = workspace
            .tab(tab)
            .ok_or_else(|| CommandError::not_found(format!("there is no tab {tab}")))?
            .clone();
        let file = self.config.dirs().layout_file(&id);
        let text = layout_text(name, &[&found]);
        let keys: Vec<(PaneId, String)> = assign_keys(&found.root).into_iter().collect();
        let written = file.clone();
        tokio::task::spawn_blocking(move || -> Result<(), CommandError> {
            if written.exists() && !replace {
                return Err(CommandError::new(
                    "exists",
                    format!("there is already a layout file {}", written.display()),
                ));
            }
            write_whole(&written, &text)
                .map_err(|error| CommandError::internal(format!("{}: {error}", written.display())))
        })
        .await
        .map_err(|error| CommandError::internal(error.to_string()))??;
        self.apply_core(Intent::TabOrigin {
            tab,
            origin: Some(format!("{id}#1")),
            keys,
        })
        .await?;
        Ok(LayoutSaved {
            id,
            file: file.display().to_string(),
        })
    }

    /// Follows a change to the layout files: each tab opened from a file whose tab
    /// changed is given the new shape, or its window is asked, as the settings say.
    pub async fn layouts_changed(&self, before: Layouts, now: Layouts) {
        if self.served_by().is_some() {
            return;
        }
        let mode = self.config.current().config.layout.files_reapply;
        if mode == FilesReapply::Never {
            return;
        }
        let workspace = self.workspace();
        for window in &workspace.windows {
            for tab in &window.centre.tabs {
                let Some((layout, wanted)) = source_of(&now, tab) else {
                    // A file that is gone, or cannot be read for now, changes nothing.
                    continue;
                };
                // A file that was not there before is one just written: whoever wrote
                // it has the tab in mind as it is.
                let Some((_, was)) = source_of(&before, tab) else {
                    continue;
                };
                if was == wanted || fits(tab, wanted) {
                    continue;
                }
                match mode {
                    FilesReapply::Always => {
                        if let Err(error) = self.reapply_layout(tab.id).await {
                            tracing::warn!(%error, tab = %tab.id, "a layout file's change was not applied");
                        }
                    }
                    FilesReapply::Ask => {
                        let (leaving, busy) = self.layout_leaving(tab.id).unwrap_or_default();
                        let notice = json!({
                            "type": "layout_file_changed",
                            "tab": tab.id,
                            "layout": layout.id,
                            "name": layout.name,
                            "leaving": leaving,
                            "busy": busy,
                        });
                        if let Some(page) = self
                            .state
                            .upgrade()
                            .and_then(|state| state.page_of(&label_of(window.id)))
                        {
                            page.notify(&notice);
                        }
                    }
                    FilesReapply::Never => {}
                }
            }
        }
    }
}

/// Writes a file whole: to a file beside it first, then moved into place, so that the
/// watcher of the folder never reads half of it.
fn write_whole(file: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let beside = file.with_extension(format!("toml.{}.tmp", std::process::id()));
    std::fs::write(&beside, text)?;
    std::fs::rename(&beside, file).inspect_err(|_| {
        let _ = std::fs::remove_file(&beside);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lume_config::layouts::parse_layout;

    fn layout(text: &str) -> Layout {
        let (layout, problems) = parse_layout("dev", text, Path::new("dev.toml")).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        layout
    }

    #[test]
    fn a_tab_fits_a_file_whose_shares_are_its_own_rounded() {
        let saved = layout(
            "[[tab]]\nsplit = \"row\"\nsizes = [33, 67]\n[[tab.pane]]\nkey = \"a\"\n[[tab.pane]]\nkey = \"b\"\n",
        );
        let mut tab = saved.tabs[0].clone();
        if let Node::Split(split) = &mut tab.root {
            split.ratios = vec![UNITS / 3, UNITS - UNITS / 3];
        }
        assert!(fits(&tab, &saved.tabs[0]));

        let moved = layout(
            "[[tab]]\nsplit = \"row\"\nsizes = [33, 67]\n[[tab.pane]]\nkey = \"b\"\n[[tab.pane]]\nkey = \"a\"\n",
        );
        assert!(!fits(&tab, &moved.tabs[0]));
    }

    #[test]
    fn a_home_directory_is_spelt_out_before_a_pane_starts() {
        let opened = prepared(&layout("cwd = \"~/src\"\n[[tab]]\n").tabs[0]);
        let PaneContent::Terminal(terminal) = &opened.root.panes()[0].content else {
            panic!("a terminal")
        };
        let cwd = terminal.cwd.as_deref().unwrap();
        assert!(!cwd.starts_with('~'), "{cwd}");
        assert!(cwd.ends_with("src"));
    }
}
