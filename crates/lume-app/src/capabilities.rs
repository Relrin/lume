//! The capability files: which of the application's commands each window's page may
//! call.
//!
//! They are written from the command table (`command_table.rs`) by the build step,
//! which Tauri then reads, and a test holds the files in the repository equal to what
//! this writes. Besides the commands, a capability may grant a permission of one of
//! Tauri's plugins, such as the folder picker.
//!
//! This file is compiled twice: into the build script and into the application.

/// A capability: the windows it is for and what their pages may call besides the
/// commands the table grants them.
pub struct Capability {
    /// Its identifier, which is also the name the table grants commands by and the name
    /// of its file.
    pub name: &'static str,
    pub description: &'static str,
    /// The labels of the windows it is for, which may end in `*`.
    pub windows: &'static [&'static str],
    /// Permissions of Tauri's plugins it grants too.
    pub also: &'static [&'static str],
}

/// The capabilities kept as files. The harness's is added at run time instead, only
/// when the application runs a test suite (see `harness.rs`).
pub const CAPABILITIES: [Capability; 3] = [
    Capability {
        name: "terminal",
        description: "What the page of a window that shows terminals may call, with the system's folder picker for choosing a folder to open as a workspace. Nothing from Tauri's core plugins is granted.",
        windows: &["w-*"],
        also: &["dialog:allow-open"],
    },
    Capability {
        name: "settings",
        description: "What the page of the settings window may call: the configuration, what there is to choose from, its own window, and the plugins it manages, with a folder picker to install one from, and fetching one from a repository. On a plugin's page it shows the view the plugin is set up in, and passes that view's calls to the plugin's code. It lists the commands and the keys bound to them, says what the application is, and puts that on the clipboard when asked. It lists the Rex servers, and adds, tries and takes out what terminals can be opened on. It starts no session, sees no layout and runs no command. Nothing else from Tauri's core plugins is granted.",
        windows: &["settings"],
        also: &["dialog:allow-open"],
    },
    Capability {
        name: "probe",
        description: "What the page of the window that reports on the webview, or runs a test suite, may call. A test suite is granted more when the application is started to run one.",
        windows: &["main"],
        also: &[],
    },
];

/// The permission that lets a page call a command.
pub fn permission(command: &str) -> String {
    format!("allow-{}", command.replace('_', "-"))
}

/// The commands the table grants the pages named `page`, in the table's order.
pub fn granted<'a>(table: &[(&'a str, &[&str])], page: &str) -> Vec<&'a str> {
    table
        .iter()
        .filter(|(_, pages)| pages.contains(&page))
        .map(|(command, _)| *command)
        .collect()
}

/// The path of a capability's file, from the crate's directory.
pub fn file_path(capability: &Capability) -> String {
    format!("capabilities/{}.json", capability.name)
}

/// A string as JSON writes it.
fn quoted(text: &str) -> String {
    let mut out = String::from('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A list of strings as one line, or one item to a line when the line would be longer
/// than 100 characters, as the repository's formatter writes JSON.
fn list(key: &str, items: &[String]) -> String {
    let items: Vec<String> = items.iter().map(|item| quoted(item)).collect();
    let line = format!("  {}: [{}]", quoted(key), items.join(", "));
    if line.len() <= 100 {
        return line;
    }
    let rows: Vec<String> = items.iter().map(|item| format!("    {item}")).collect();
    format!("  {}: [\n{}\n  ]", quoted(key), rows.join(",\n"))
}

/// The text of a capability's file.
pub fn file_text(capability: &Capability, table: &[(&str, &[&str])]) -> String {
    let mut permissions: Vec<String> = granted(table, capability.name)
        .into_iter()
        .map(permission)
        .collect();
    permissions.extend(capability.also.iter().map(|also| (*also).to_owned()));
    let windows: Vec<String> = capability.windows.iter().map(|w| (*w).to_owned()).collect();
    let lines = [
        format!(
            "  \"$schema\": {}",
            quoted("../gen/schemas/desktop-schema.json")
        ),
        format!("  \"identifier\": {}", quoted(capability.name)),
        format!("  \"description\": {}", quoted(capability.description)),
        list("windows", &windows),
        list("permissions", &permissions),
    ];
    format!("{{\n{}\n}}\n", lines.join(",\n"))
}
