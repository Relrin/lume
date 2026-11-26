//! The rows of a plugin's list panels as they are kept and checked, and the other small
//! pure helpers the plugins' host uses: no plugin runs here.

use super::*;

/// The plugin's commands a panel's rows may run, once the panel is known to be one of
/// its manifest's.
pub(super) fn panel_commands(loaded: &Loaded, panel: &str) -> Result<BTreeSet<String>, ApiError> {
    let manifest = loaded
        .manifest()
        .ok_or_else(|| api_error(ErrorCode::Stopped, "the plugin is not installed"))?;
    if !manifest
        .contributes
        .panels
        .iter()
        .any(|known| known.id == panel)
    {
        return Err(api_error(
            ErrorCode::NotFound,
            format!("the plugin's manifest has no panel {panel}"),
        ));
    }
    Ok(manifest
        .contributes
        .commands
        .iter()
        .map(|command| command.id.clone())
        .collect())
}

/// The row at the end of a path of ids, among rows as they are kept.
pub(super) fn row_at<'a>(rows: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
    let (first, rest) = path.split_first()?;
    let row = rows
        .as_array_mut()?
        .iter_mut()
        .find(|row| row.get("id").and_then(Value::as_str) == Some(first.as_str()))?;
    if rest.is_empty() {
        Some(row)
    } else {
        row_at(row.get_mut("children")?, rest)
    }
}

/// How many rows there are among rows as they are kept, at every depth.
pub(super) fn count_rows(rows: &Value) -> usize {
    rows.as_array().map_or(0, |rows| {
        rows.iter()
            .map(|row| 1 + row.get("children").map_or(0, count_rows))
            .sum()
    })
}

/// Everything the windows that are not minimised have on screen.
pub(super) fn seen(inner: &Inner) -> BTreeSet<String> {
    inner
        .visible
        .iter()
        .filter(|(window, _)| !inner.minimised.contains(*window))
        .flat_map(|(_, items)| items.iter().cloned())
        .collect()
}

/// Checks the rows a plugin gives a list panel: how many, how deep, how long their
/// words, and that their actions run the plugin's own commands.
pub(super) fn check_rows(
    rows: &[PanelRow],
    depth: usize,
    count: &mut usize,
    commands: &BTreeSet<String>,
) -> Result<(), String> {
    if depth > PANEL_DEPTH_MAX {
        return Err(format!("a panel's rows go {PANEL_DEPTH_MAX} deep at most"));
    }
    let mut ids = BTreeSet::new();
    for row in rows {
        *count += 1;
        if *count > PANEL_ROWS_MAX {
            return Err(format!("a panel holds {PANEL_ROWS_MAX} rows at most"));
        }
        if !ids.insert(row.id.as_str()) {
            return Err(format!("two rows beside each other are called {}", row.id));
        }
        for text in [
            Some(&row.id),
            Some(&row.label),
            row.detail.as_ref(),
            row.badge.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if text.chars().count() > PANEL_TEXT_MAX || text.chars().any(char::is_control) {
                return Err(format!(
                    "a row's words are one line of {PANEL_TEXT_MAX} characters at most"
                ));
            }
        }
        for action in row.actions.iter().flatten() {
            if !commands.contains(&action.command) {
                return Err(format!(
                    "an action runs {}, which is not one of the plugin's commands",
                    action.command
                ));
            }
        }
        if let Some(children) = &row.children {
            check_rows(children, depth + 1, count, commands)?;
        }
    }
    Ok(())
}

/// Gives the actions of rows their commands' full ids, and marks the rows whose rows
/// are given when they are opened and that came with them as having them.
pub(super) fn full_commands(rows: &mut Value, local: &str) {
    let Some(rows) = rows.as_array_mut() else {
        return;
    };
    for row in rows {
        if row.get("lazy").and_then(Value::as_bool) == Some(true) && row.get("children").is_some() {
            row["loaded"] = json!(true);
        }
        if let Some(actions) = row.get_mut("actions").and_then(Value::as_array_mut) {
            for action in actions {
                if let Some(command) = action.get("command").and_then(Value::as_str) {
                    let full = full_id(local, command);
                    action["command"] = json!(full);
                }
            }
        }
        if let Some(children) = row.get_mut("children") {
            full_commands(children, local);
        }
    }
}

/// Whether two paths name the same file, as a watcher may report one written another
/// way.
pub(super) fn same_file(one: &Path, other: &Path) -> bool {
    one == other
        || one.file_name() == other.file_name()
            && one.parent().map(Path::file_name) == other.parent().map(Path::file_name)
}
