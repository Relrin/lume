//! Opening a link the user asked for.
//!
//! A link in a terminal comes from a program's output, so it is whatever a file or a
//! remote machine wanted it to be. The webview shows the target and asks before it calls
//! this, but the webview is not trusted to have done so: everything is decided again
//! here, from the text alone.
//!
//! Web addresses are handed to the system to open in the browser. A `file` link never
//! starts anything: a folder is opened in the file manager, and a file is shown there,
//! selected, for the user to open or not. Nothing is ever passed through a shell.

use std::path::{Path, PathBuf};
use std::process::Command;

use url::Url;

use crate::error::CommandError;
use crate::tuning::LINK_MAX_BYTES;

/// What a link turned out to be.
#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    /// An address for the browser.
    Web(Url),
    /// A folder on this machine.
    Directory(PathBuf),
    /// A file on this machine.
    File(PathBuf),
}

/// Decides what a link is, or refuses it.
pub fn resolve(link: &str) -> Result<Target, CommandError> {
    if link.len() > LINK_MAX_BYTES {
        return Err(CommandError::bad_request("the link is too long"));
    }
    let url = Url::parse(link)
        .map_err(|error| CommandError::bad_request(format!("not a link: {error}")))?;
    match url.scheme() {
        "http" | "https" => {
            if url.host_str().is_none_or(str::is_empty) {
                return Err(CommandError::bad_request("the link names no host"));
            }
            Ok(Target::Web(url))
        }
        "file" => {
            // A file on another machine would be fetched over the network on the way to
            // being shown, which tells that machine who asked.
            if !matches!(url.host_str(), None | Some("" | "localhost")) {
                return Err(CommandError::new(
                    "unsupported",
                    "links to files on other machines are not opened",
                ));
            }
            let mut local = url.clone();
            let _ = local.set_host(None);
            let path = local.to_file_path().map_err(|()| {
                CommandError::bad_request("the link is not a path on this machine")
            })?;
            let found = std::fs::metadata(&path)
                .map_err(|_| CommandError::new("not_found", "the link leads nowhere"))?;
            Ok(if found.is_dir() {
                Target::Directory(path)
            } else {
                Target::File(path)
            })
        }
        other => Err(CommandError::new(
            "unsupported",
            format!("`{other}` links are not opened"),
        )),
    }
}

/// Opens a link, if it is one that may be opened.
pub fn open(link: &str) -> Result<(), CommandError> {
    let failed = |error: std::io::Error| {
        CommandError::internal(format!("the link could not be opened: {error}"))
    };
    match resolve(link)? {
        Target::Web(url) => open_in_browser(&url).map_err(failed),
        Target::Directory(path) => open_directory(&path).map_err(failed),
        Target::File(path) => reveal_file(&path).map_err(failed),
    }
}

/// Shows a folder of this machine in the file manager.
pub fn open_folder(path: &Path) -> Result<(), CommandError> {
    if !path.is_dir() {
        return Err(CommandError::not_found(format!(
            "{} is not a folder that is there",
            path.display()
        )));
    }
    open_directory(path)
        .map_err(|error| CommandError::internal(format!("the folder could not be opened: {error}")))
}

/// The extension of the files a user configures the application with.
const SETTINGS_EXTENSION: &str = "toml";

/// Decides whether `file` is one of the user's settings files in `dir`, and returns
/// its full path. Nothing outside that folder and nothing but a settings file is ever
/// opened for editing, whatever a page asks for.
pub fn settings_file(dir: &Path, file: &Path) -> Result<PathBuf, CommandError> {
    let missing = |_| CommandError::not_found(format!("{} does not exist", file.display()));
    let dir = dir.canonicalize().map_err(missing)?;
    let found = file.canonicalize().map_err(missing)?;
    let settings = found.is_file()
        && found.starts_with(&dir)
        && found
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case(SETTINGS_EXTENSION));
    if settings {
        Ok(found)
    } else {
        Err(CommandError::forbidden(format!(
            "{} is not one of the settings files",
            file.display()
        )))
    }
}

/// Opens one of the user's settings files in `dir` for editing, in whatever the system
/// edits such a file with.
pub fn open_settings_file(dir: &Path, file: &Path) -> Result<(), CommandError> {
    let found = settings_file(dir, file)?;
    edit_file(&found).map_err(|error| {
        CommandError::internal(format!(
            "{} could not be opened for editing: {error}",
            found.display()
        ))
    })
}

/// Starts a helper program and lets it go. It is waited for on a thread of its own, so
/// that it leaves nothing behind when it ends.
fn launch(mut command: Command) -> std::io::Result<()> {
    let mut child = command.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Has the shell carry out `verb` on `target`: an address or a file, never a command.
#[cfg(windows)]
fn shell_execute(verb: &str, target: &std::ffi::OsStr) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide = |text: &std::ffi::OsStr| -> Vec<u16> {
        text.encode_wide().chain(std::iter::once(0)).collect()
    };
    let verb = wide(std::ffi::OsStr::new(verb));
    let target = wide(target);
    // SAFETY: both strings are NUL-terminated and outlive the call; the other pointers
    // are optional and null.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // The function reports success as a value above 32, for reasons older than Windows.
    if result as usize > 32 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn open_in_browser(url: &Url) -> std::io::Result<()> {
    // The address as the parser wrote it back: it starts with `http:` or `https:`, so
    // the shell can only take it for an address, never for a program or a path.
    shell_execute("open", std::ffi::OsStr::new(url.as_str()))
}

/// Opens a settings file in the program the user edits such files with, or in the
/// system's own plain editor when no program is set for them.
#[cfg(windows)]
fn edit_file(path: &Path) -> std::io::Result<()> {
    shell_execute("open", path.as_os_str()).or_else(|_| {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let mut command = Command::new(Path::new(&root).join("System32").join("notepad.exe"));
        command.arg(path);
        launch(command)
    })
}

#[cfg(target_os = "macos")]
fn edit_file(path: &Path) -> std::io::Result<()> {
    let mut command = Command::new(OPEN);
    // As text, in the editor the user has chosen for text.
    command.arg("-t").arg(path);
    launch(command)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn edit_file(path: &Path) -> std::io::Result<()> {
    let mut command = Command::new("xdg-open");
    command.arg(path);
    launch(command)
}

/// The file manager, by its full path: a program of the same name elsewhere on the
/// search path must not be started in its place.
#[cfg(windows)]
fn explorer() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    Path::new(&root).join("explorer.exe")
}

#[cfg(windows)]
fn open_directory(path: &Path) -> std::io::Result<()> {
    let mut command = Command::new(explorer());
    command.arg(path);
    launch(command)
}

#[cfg(windows)]
fn reveal_file(path: &Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;

    let mut command = Command::new(explorer());
    // The file manager wants the path quoted after the comma, not the argument quoted
    // as a whole. A path cannot contain a quotation mark, so none needs escaping.
    command.raw_arg(format!("/select,\"{}\"", path.display()));
    launch(command)
}

#[cfg(target_os = "macos")]
const OPEN: &str = "/usr/bin/open";

#[cfg(target_os = "macos")]
fn open_in_browser(url: &Url) -> std::io::Result<()> {
    let mut command = Command::new(OPEN);
    command.arg(url.as_str());
    launch(command)
}

#[cfg(target_os = "macos")]
fn open_directory(path: &Path) -> std::io::Result<()> {
    let mut command = Command::new(OPEN);
    command.arg(path);
    launch(command)
}

#[cfg(target_os = "macos")]
fn reveal_file(path: &Path) -> std::io::Result<()> {
    let mut command = Command::new(OPEN);
    command.arg("-R").arg(path);
    launch(command)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_in_browser(url: &Url) -> std::io::Result<()> {
    let mut command = Command::new("xdg-open");
    command.arg(url.as_str());
    launch(command)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_directory(path: &Path) -> std::io::Result<()> {
    let mut command = Command::new("xdg-open");
    command.arg(path);
    launch(command)
}

/// There is no one way to select a file in a file manager here, so its folder is opened.
#[cfg(all(unix, not(target_os = "macos")))]
fn reveal_file(path: &Path) -> std::io::Result<()> {
    open_directory(path.parent().unwrap_or(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal(link: &str) -> String {
        resolve(link)
            .expect_err("the link was accepted")
            .code
            .into_owned()
    }

    #[test]
    fn web_addresses_are_for_the_browser() {
        for link in ["http://example.org/", "https://example.org/a?b=c#d"] {
            match resolve(link) {
                Ok(Target::Web(url)) => assert_eq!(url.as_str(), link),
                other => panic!("{link}: {other:?}"),
            }
        }
        // The scheme decides, however it is written.
        assert!(matches!(resolve("HTTPS://EXAMPLE.ORG"), Ok(Target::Web(_))));
    }

    #[test]
    fn a_web_address_without_a_host_is_refused() {
        assert_eq!(refusal("http://"), "bad_request");
        assert_eq!(refusal("https://"), "bad_request");
        assert_eq!(refusal("https:///"), "bad_request");
    }

    #[test]
    fn a_web_address_is_read_the_way_a_browser_reads_it() {
        // Extra slashes are skipped, so what follows them is the host. What is handed
        // to the system is the address written back out, with no room for surprises.
        match resolve("https:///example.org/x") {
            Ok(Target::Web(url)) => assert_eq!(url.as_str(), "https://example.org/x"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn other_kinds_of_link_are_refused() {
        for link in [
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "ftp://example.org/file",
            "mailto:someone@example.org",
            "ssh://example.org",
            "vbscript:msgbox(1)",
            "ms-settings:privacy",
            "calculator:",
        ] {
            assert_eq!(refusal(link), "unsupported", "{link}");
        }
    }

    #[test]
    fn text_that_is_not_a_link_is_refused() {
        for link in [
            "",
            "hello",
            "C:\\Windows\\System32\\calc.exe",
            "/bin/sh",
            "//example.org",
        ] {
            // A path with a drive letter parses as a link of an unknown kind.
            assert!(
                matches!(refusal(link).as_str(), "bad_request" | "unsupported"),
                "{link}"
            );
        }
    }

    #[test]
    fn a_link_longer_than_the_limit_is_refused() {
        let long = format!("https://example.org/{}", "a".repeat(LINK_MAX_BYTES));
        assert_eq!(refusal(&long), "bad_request");
    }

    #[test]
    fn a_file_link_to_another_machine_is_refused() {
        assert_eq!(refusal("file://server/share/file.txt"), "unsupported");
    }

    #[test]
    fn a_file_link_that_leads_nowhere_is_refused() {
        let missing = std::env::temp_dir().join("lume-no-such-file-anywhere");
        let link = Url::from_file_path(&missing).unwrap();
        assert_eq!(refusal(link.as_str()), "not_found");
    }

    #[test]
    fn a_file_link_is_a_folder_to_open_or_a_file_to_show() {
        let folder = std::env::temp_dir();
        let link = Url::from_file_path(&folder).unwrap();
        assert_eq!(
            resolve(link.as_str()),
            Ok(Target::Directory(folder.clone()))
        );

        let file = folder.join(format!("lume-opener-{}.txt", std::process::id()));
        std::fs::write(&file, b"x").unwrap();
        let link = Url::from_file_path(&file).unwrap();
        let resolved = resolve(link.as_str());
        // The same file through the name of this machine.
        let named = link.as_str().replacen("file:///", "file://localhost/", 1);
        let through_name = resolve(&named);
        std::fs::remove_file(&file).unwrap();
        assert_eq!(resolved, Ok(Target::File(file.clone())));
        assert_eq!(through_name, Ok(Target::File(file)));
    }
}
