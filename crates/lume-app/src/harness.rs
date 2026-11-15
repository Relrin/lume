//! Running a test suite in the webview.
//!
//! `lume-gui --harness <suite>` shows the harness page in place of the terminal. The page
//! runs the suite in the real webview, which is the only place its engine, its GPU
//! paths and its fonts can be tested, and reports here. The report is printed, written
//! to a file if one was named, and the process exits with the suite's verdict.
//!
//! The page may read reference files and write result files. Both are confined to the
//! two directories named on the command line, and to plain file names within them.
//!
//! The commands here are granted to the page only in a harness run.

use std::path::{Component, Path, PathBuf};

use tauri::ipc::{CapabilityBuilder, InvokeBody, Request, Response};
use tauri::{App, AppHandle, Manager, State};

use crate::args::{Args, Mode};
use crate::error::CommandError;
use crate::tuning::{HARNESS_FILE_MAX_BYTES, HARNESS_TIMEOUT};

/// The exit status when the suite failed.
const EXIT_FAILED: i32 = 1;

/// The exit status when the page never reported.
const EXIT_SILENT: i32 = 3;

/// The request header that names the file a raw body is to be written to.
const FILE_HEADER: &str = "lume-file";

/// The request header that says which directory the file belongs in.
const ROOT_HEADER: &str = "lume-root";

/// Lets the page call the harness commands, and ends the run if the page never reports.
pub fn start(app: &App) -> tauri::Result<()> {
    app.add_capability(
        CapabilityBuilder::new("harness")
            .window(crate::window::LABEL)
            // A suite's page starts and ends the sessions it measures itself.
            .permission("allow-carrier-open")
            .permission("allow-carrier-send")
            .permission("allow-session-spawn")
            .permission("allow-session-attach")
            .permission("allow-session-kill")
            .permission("allow-session-list")
            .permission("allow-page-reply")
            .permission("allow-harness-done")
            .permission("allow-harness-save")
            .permission("allow-harness-read"),
    )?;
    let handle = app.handle().clone();
    std::thread::spawn(move || {
        std::thread::sleep(HARNESS_TIMEOUT);
        // Still running: the page did not load, or stopped before it reported.
        let report = serde_json::json!({
            "ok": false,
            "error": format!(
                "the harness page did not report within {} seconds",
                HARNESS_TIMEOUT.as_secs()
            ),
        });
        finish(&handle, &report, EXIT_SILENT);
    });
    Ok(())
}

/// Prints the report, writes it where it was asked for, and ends the process.
fn finish(app: &AppHandle, report: &serde_json::Value, status: i32) {
    println!("{report}");
    let args = app.state::<Args>();
    if let Some(path) = &args.report
        && let Err(error) = write_file(path, report.to_string().as_bytes())
    {
        eprintln!(
            "lume-gui: the report could not be written to {}: {error}",
            path.display()
        );
    }
    app.exit(status);
}

fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, bytes)
}

/// Called by the harness page when its suite is finished.
#[tauri::command]
pub fn harness_done(report: serde_json::Value, app: AppHandle, args: State<'_, Args>) {
    if !matches!(args.mode, Mode::Harness(_)) {
        return;
    }
    let passed = report.get("ok").and_then(serde_json::Value::as_bool) == Some(true);
    finish(&app, &report, if passed { 0 } else { EXIT_FAILED });
}

/// The directories the page may reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Root {
    /// What the suite produces: pictures it drew, to be looked at when one differs.
    Out,
    /// What the suite compares against.
    References,
}

impl Root {
    fn named(name: &str) -> Option<Self> {
        match name {
            "out" => Some(Self::Out),
            "references" => Some(Self::References),
            _ => None,
        }
    }

    fn dir(self, args: &Args) -> Result<&Path, CommandError> {
        let (dir, option) = match self {
            Self::Out => (&args.out, "--out"),
            Self::References => (&args.references, "--references"),
        };
        dir.as_deref().ok_or_else(|| {
            CommandError::new(
                "unsupported",
                format!("this run was started without {option}"),
            )
        })
    }
}

/// The path of `name` inside `dir`, if `name` is a plain relative path: a few names of
/// letters, digits, dots, dashes and underscores, none of them leading out of `dir`.
fn inside(dir: &Path, name: &str) -> Result<PathBuf, CommandError> {
    let refused =
        || CommandError::bad_request(format!("{name} is not a file name the harness may use"));
    let plain = |part: &str| {
        !part.is_empty()
            && !part.starts_with('.')
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    };
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() > 3 || !parts.iter().all(|part| plain(part)) {
        return Err(refused());
    }
    let path = dir.join(parts.iter().collect::<PathBuf>());
    // What the check above allows cannot climb out; this holds if it is ever loosened.
    if path.strip_prefix(dir).is_ok_and(|rest| {
        rest.components()
            .all(|part| matches!(part, Component::Normal(_)))
    }) {
        Ok(path)
    } else {
        Err(refused())
    }
}

fn header<'a>(request: &'a Request<'_>, name: &str) -> Result<&'a str, CommandError> {
    request
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| CommandError::bad_request(format!("the request has no {name} header")))
}

/// Writes a file the suite produced. The body is the file; headers name it and say
/// which directory it belongs in. Reference files are written only in a run that was
/// started to update them.
#[tauri::command]
pub fn harness_save(request: Request<'_>, args: State<'_, Args>) -> Result<(), CommandError> {
    if !matches!(args.mode, Mode::Harness(_)) {
        return Err(CommandError::new(
            "unsupported",
            "this is not a harness run",
        ));
    }
    let root = Root::named(header(&request, ROOT_HEADER)?)
        .ok_or_else(|| CommandError::bad_request("there is no such directory"))?;
    if root == Root::References && !args.update {
        return Err(CommandError::new(
            "unsupported",
            "reference files are written only with --update",
        ));
    }
    let path = inside(root.dir(&args)?, header(&request, FILE_HEADER)?)?;
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err(CommandError::bad_request("the body is not bytes"));
    };
    if bytes.len() > HARNESS_FILE_MAX_BYTES {
        return Err(CommandError::new("limit_exceeded", "the file is too large"));
    }
    write_file(&path, bytes).map_err(|error| CommandError::internal(error.to_string()))
}

/// Reads a reference file. A file that is not there is answered with `not_found`: a
/// suite on a platform nobody has made references for says so rather than failing.
#[tauri::command]
pub fn harness_read(name: String, args: State<'_, Args>) -> Result<Response, CommandError> {
    if !matches!(args.mode, Mode::Harness(_)) {
        return Err(CommandError::new(
            "unsupported",
            "this is not a harness run",
        ));
    }
    let path = inside(Root::References.dir(&args)?, &name)?;
    match std::fs::read(&path) {
        Ok(bytes) => Ok(Response::new(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(CommandError::new(
            "not_found",
            format!("there is no reference called {name}"),
        )),
        Err(error) => Err(CommandError::internal(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_stay_inside_their_directory() {
        let dir = Path::new("refs");
        assert_eq!(inside(dir, "a.png").unwrap(), dir.join("a.png"));
        assert_eq!(
            inside(dir, "webview2/ascii-colors.webgpu.png").unwrap(),
            dir.join("webview2").join("ascii-colors.webgpu.png")
        );
        assert_eq!(
            inside(dir, "a/b/c_1.json").unwrap(),
            dir.join("a").join("b").join("c_1.json")
        );
    }

    #[test]
    fn names_that_could_leave_the_directory_are_refused() {
        let dir = Path::new("refs");
        for name in [
            "",
            "..",
            "../x.png",
            "a/../../x.png",
            "/etc/passwd",
            "C:/x.png",
            r"a\b.png",
            ".hidden",
            "a//b.png",
            "a/b/c/d.png",
            "name with space.png",
            "a/./b.png",
        ] {
            assert!(inside(dir, name).is_err(), "accepted: {name}");
        }
    }

    #[test]
    fn the_two_directories_have_their_names() {
        assert_eq!(Root::named("out"), Some(Root::Out));
        assert_eq!(Root::named("references"), Some(Root::References));
        assert_eq!(Root::named("home"), None);
    }
}
