//! Programs: finding them, and running them within what `process.run` grants.
//!
//! A plugin names a program by its name alone, as in `git`. The name is looked up once
//! along the PATH, never in the current folder, and the program found is pinned by its
//! canonical path, so what runs is what was checked. A grant names the program the same
//! way. Lume's own programs, and anything in Lume's own folders, are never run.
//!
//! On Windows a batch file is run by `cmd.exe`, so finding `npm.cmd` for `npm` means the
//! plugin needs the `cmd` grant. Its arguments are escaped for `cmd.exe` by the standard
//! library, which refuses those it cannot pass safely. No console window opens for a
//! program run here.
//!
//! A program runs with everything it starts: when it is stopped (by a timeout or by
//! [`Child::kill`]), the programs it started are stopped too, through a job object on
//! Windows and a process group elsewhere.

use std::ffi::OsStr;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use lume_plugin::permissions::check_program;

use crate::paths::{self, Fence, IGNORE_CASE, Parts};
use crate::scope::{Places, ServiceError};

/// How long the output of a program that has ended is still read, for what is left in
/// its pipes. A program it started that keeps them open is not waited for past this.
pub const OUTPUT_GRACE: Duration = Duration::from_secs(2);

/// How long after its output ended a program is first asked after again, if it had not
/// ended by then; the wait doubles from there.
const AFTER_OUTPUT: Duration = Duration::from_micros(100);

/// A program found on the PATH.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    /// The name a `process.run` grant must hold to run it: the name asked for, or `cmd`
    /// for a batch file on Windows.
    pub grant: String,
    /// Where it is: absolute and canonical.
    pub path: PathBuf,
}

/// The kinds of file Windows runs as programs, of those the PATHEXT variable lists.
#[cfg(windows)]
const RUNNABLE: [&str; 4] = [".com", ".exe", ".bat", ".cmd"];

/// Finds a program by its name alone along `path_var` (the application's own PATH when
/// `None`), once, as a shell would: on Windows with the extensions PATHEXT lists, and
/// never in the current folder or in a relative folder of the PATH. A program in Lume's
/// own folders, or one of Lume's own programs, is refused.
pub fn resolve(
    places: &Places,
    name: &str,
    path_var: Option<&OsStr>,
) -> Result<Program, ServiceError> {
    let name = check_program(name).map_err(ServiceError::invalid)?;
    let own;
    let path_var = match path_var {
        Some(path_var) => path_var,
        None => {
            own = std::env::var_os("PATH").unwrap_or_default();
            own.as_os_str()
        }
    };
    let found = search(&name, path_var)
        .ok_or_else(|| ServiceError::not_found(format!("`{name}` was not found on the PATH")))?;
    let path = paths::canonical(&found).map_err(|error| {
        ServiceError::failed(format!("`{name}` could not be looked at: {error}"))
    })?;
    let parts = Parts::of(&path)
        .ok_or_else(|| ServiceError::denied(format!("`{name}` cannot be reached")))?;
    if Fence::new(places).within_closed(&parts) {
        return Err(ServiceError::denied(format!(
            "`{name}` lies in Lume's own folders, whose programs plugins cannot run"
        )));
    }
    let lume = places.own_programs.iter().any(|program| {
        Parts::of(&paths::canonical_lenient(program))
            .is_some_and(|program| program.same(&parts, IGNORE_CASE))
    });
    if lume {
        return Err(ServiceError::denied(format!(
            "`{name}` is one of Lume's own programs, which plugins cannot run"
        )));
    }
    let grant = grant_for(&name, &path);
    Ok(Program { grant, path })
}

/// The first program of that name along the PATH.
fn search(name: &str, path_var: &OsStr) -> Option<PathBuf> {
    for folder in std::env::split_paths(path_var) {
        if !folder.is_absolute() {
            continue;
        }
        if let Some(found) = candidates(name)
            .into_iter()
            .map(|file| folder.join(file))
            .find(|path| runnable(path))
        {
            return Some(found);
        }
    }
    None
}

/// The file names a program's name can have: on Windows the name as written if it has an
/// extension that runs, then the name with each extension PATHEXT lists.
#[cfg(windows)]
fn candidates(name: &str) -> Vec<String> {
    let listed = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
    let extensions: Vec<String> = listed
        .split(';')
        .map(str::to_ascii_lowercase)
        .filter(|extension| RUNNABLE.contains(&extension.as_str()))
        .collect();
    let mut names = Vec::new();
    let lower = name.to_ascii_lowercase();
    if extensions
        .iter()
        .any(|extension| lower.ends_with(extension.as_str()))
    {
        names.push(name.to_owned());
    }
    names.extend(
        extensions
            .iter()
            .map(|extension| format!("{name}{extension}")),
    );
    names
}

#[cfg(not(windows))]
fn candidates(name: &str) -> Vec<String> {
    vec![name.to_owned()]
}

/// Whether a path is a file this system would run.
fn runnable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

/// A program's name without a written `.exe`, on Windows.
fn without_exe(name: &str) -> &str {
    if !cfg!(windows) {
        return name;
    }
    let cut = name.len().saturating_sub(4);
    match name.get(cut..) {
        Some(end) if cut > 0 && end.eq_ignore_ascii_case(".exe") => &name[..cut],
        _ => name,
    }
}

/// The name a grant must hold to run what was found for `name`.
fn grant_for(name: &str, path: &Path) -> String {
    if cfg!(windows) {
        let batch = path
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("bat") || extension.eq_ignore_ascii_case("cmd")
            });
        if batch {
            return "cmd".to_owned();
        }
    }
    without_exe(name).to_owned()
}

/// The grant among `granted` (the names of `process.run`) that allows a program, if
/// any. On Windows names compare without regard to case, and a written `.exe` is the
/// same as none.
pub fn allowed(granted: &[String], program: &Program) -> Option<String> {
    granted
        .iter()
        .find(|grant| {
            let (grant, wanted) = (without_exe(grant), without_exe(&program.grant));
            if cfg!(windows) {
                grant.eq_ignore_ascii_case(wanted)
            } else {
                grant == wanted
            }
        })
        .cloned()
}

/// How a program is run to its end.
#[derive(Clone, Debug)]
pub struct RunOptions {
    /// The folder it runs in; the application's own when `None`.
    pub cwd: Option<PathBuf>,
    /// Variables set on top of the application's environment.
    pub env: Vec<(String, String)>,
    /// What it reads, all of it; nothing when `None`.
    pub stdin: Option<Vec<u8>>,
    /// How long it may run before it is stopped.
    pub timeout: Duration,
    /// How many bytes of its output and of its error output are kept, each.
    pub max_output: usize,
}

/// What a program did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunOutput {
    /// Its exit code. `None` when it was stopped (by the timeout, or on Linux and macOS
    /// by a signal).
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Whether it ran past the timeout and was stopped.
    pub timed_out: bool,
    /// Whether output was cut: past `max_output`, or still coming from a program it
    /// started when it ended.
    pub truncated: bool,
}

/// How a program is started to run alongside.
#[derive(Clone, Debug, Default)]
pub struct SpawnOptions {
    /// The folder it runs in; the application's own when `None`.
    pub cwd: Option<PathBuf>,
    /// Variables set on top of the application's environment.
    pub env: Vec<(String, String)>,
}

/// A program started, with its pipes.
#[derive(Debug)]
pub struct Spawned {
    pub stdin: Option<std::process::ChildStdin>,
    pub stdout: Option<std::process::ChildStdout>,
    pub stderr: Option<std::process::ChildStderr>,
    pub child: Child,
}

/// The command for a program, checked.
fn command(
    program: &Program,
    args: &[String],
    cwd: Option<&Path>,
    env: &[(String, String)],
) -> Result<Command, ServiceError> {
    if args.iter().any(|arg| arg.contains('\0')) {
        return Err(ServiceError::invalid("an argument holds a NUL character"));
    }
    for (key, value) in env {
        if key.is_empty() || key.contains(['=', '\0']) || value.contains('\0') {
            return Err(ServiceError::invalid(format!(
                "`{key}` is not an environment variable that can be set"
            )));
        }
    }
    if let Some(cwd) = cwd
        && !cwd.is_dir()
    {
        return Err(ServiceError::not_found(format!(
            "the folder `{}` to run in does not exist",
            cwd.display()
        )));
    }
    let mut command = Command::new(&program.path);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    command.envs(env.iter().map(|(key, value)| (key, value)));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A program learns what it was called as a shell would tell it, which matters
        // to programs that are one file under many names.
        command.arg0(&program.grant);
        command.process_group(0);
    }
    Ok(command)
}

/// Why a program could not be started, for the plugin.
fn start_error(error: &std::io::Error, program: &Program) -> ServiceError {
    let name = &program.grant;
    match error.kind() {
        std::io::ErrorKind::InvalidInput => ServiceError::invalid(format!(
            "an argument cannot be passed safely to `{}`",
            program.path.display()
        )),
        std::io::ErrorKind::NotFound => {
            ServiceError::not_found(format!("`{}` is no longer there", program.path.display()))
        }
        _ => ServiceError::failed(format!("`{name}` could not be started: {error}")),
    }
}

/// Runs a program to its end, or until the timeout stops it, and gives what it wrote.
/// It waits, so it belongs on a thread that may.
pub fn run(
    program: &Program,
    args: &[String],
    options: RunOptions,
) -> Result<RunOutput, ServiceError> {
    let mut command = command(program, args, options.cwd.as_deref(), &options.env)?;
    command
        .stdin(if options.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| start_error(&error, program))?;
    let tree = Tree::adopt(&child);
    if let (Some(mut pipe), Some(bytes)) = (child.stdin.take(), options.stdin) {
        // It ends when everything is written or the program stops reading.
        thread::spawn(move || {
            let _ = pipe.write_all(&bytes);
        });
    }
    let mut stdout = Capture::start(child.stdout.take(), options.max_output);
    let stderr = Capture::start(child.stderr.take(), options.max_output);
    let deadline = Instant::now() + options.timeout;
    let mut pause = Duration::from_millis(1);
    // A program's output ends as the program does, so its end is what is waited for,
    // and the program is asked after at once when it comes. A program that ends and
    // leaves its output open to something it started is still found, by looking now
    // and then; so is one that closes its output and runs on.
    let mut writing = true;
    let (code, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (status.code(), false),
            Ok(None) => {}
            Err(_) => {
                stop(&mut child, &tree);
                break (None, false);
            }
        }
        let now = Instant::now();
        if now >= deadline {
            stop(&mut child, &tree);
            break (None, true);
        }
        let wait = pause.min(deadline - now);
        if writing && stdout.ended_within(wait) {
            writing = false;
            pause = AFTER_OUTPUT;
            continue;
        }
        if !writing {
            thread::sleep(wait);
        }
        pause = (pause * 2).min(Duration::from_millis(20));
    };
    let until = Instant::now() + OUTPUT_GRACE;
    let (stdout, stdout_cut) = stdout.finish(until);
    let (stderr, stderr_cut) = stderr.finish(until);
    Ok(RunOutput {
        code,
        stdout,
        stderr,
        timed_out,
        truncated: stdout_cut || stderr_cut,
    })
}

/// Stops a program that has not been waited for, with everything it started.
fn stop(child: &mut std::process::Child, tree: &Tree) {
    tree.kill();
    let _ = child.kill();
    let _ = child.wait();
}

/// What a program writes to one pipe, read on a thread of its own.
struct Capture {
    kept: Arc<Mutex<(Vec<u8>, bool)>>,
    done: mpsc::Receiver<()>,
    /// Whether the pipe's end was seen.
    ended: bool,
}

impl Capture {
    fn start(pipe: Option<impl Read + Send + 'static>, max: usize) -> Self {
        let kept = Arc::new(Mutex::new((Vec::new(), false)));
        let (tell, done) = mpsc::channel();
        if let Some(mut pipe) = pipe {
            let kept = Arc::clone(&kept);
            thread::spawn(move || {
                let mut buffer = [0u8; 8192];
                loop {
                    let read = match pipe.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(read) => read,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    };
                    let mut kept = kept.lock().unwrap_or_else(PoisonError::into_inner);
                    let room = max.saturating_sub(kept.0.len());
                    // Past the limit the output is still read, so the program does not
                    // wait on a full pipe, but no longer kept.
                    if read > room {
                        kept.1 = true;
                    }
                    kept.0.extend_from_slice(&buffer[..read.min(room)]);
                }
                let _ = tell.send(());
            });
        }
        Self {
            kept,
            done,
            ended: false,
        }
    }

    /// Waits up to `wait` for the pipe's end, and says whether it came. A capture with no
    /// pipe has ended.
    fn ended_within(&mut self, wait: Duration) -> bool {
        if !self.ended {
            self.ended = !matches!(
                self.done.recv_timeout(wait),
                Err(mpsc::RecvTimeoutError::Timeout)
            );
        }
        self.ended
    }

    /// What was kept, and whether it was cut. Waits for the pipe's end until `until`; a
    /// pipe still open then counts as cut.
    fn finish(mut self, until: Instant) -> (Vec<u8>, bool) {
        let ended = self.ended_within(until.saturating_duration_since(Instant::now()));
        let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        (std::mem::take(&mut kept.0), kept.1 || !ended)
    }
}

/// Starts a program to run alongside, with pipes to its input and from its output and
/// error output. The program is not stopped when its [`Child`] is dropped.
pub fn spawn(
    program: &Program,
    args: &[String],
    options: SpawnOptions,
) -> Result<Spawned, ServiceError> {
    let mut command = command(program, args, options.cwd.as_deref(), &options.env)?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| start_error(&error, program))?;
    let tree = Tree::adopt(&child);
    Ok(Spawned {
        stdin: child.stdin.take(),
        stdout: child.stdout.take(),
        stderr: child.stderr.take(),
        child: Child {
            id: child.id(),
            inner: Mutex::new(child),
            tree,
        },
    })
}

/// A program started with [`spawn`]. It can be waited for on one thread and stopped from
/// another. Dropping it leaves the program running.
pub struct Child {
    id: u32,
    inner: Mutex<std::process::Child>,
    tree: Tree,
}

impl std::fmt::Debug for Child {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Child")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Child {
    /// The system's id of the process.
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Waits for the program to end and gives its exit code: `None` when it was
    /// stopped, or could not be waited for.
    pub fn wait(&self) -> Option<i32> {
        let mut pause = Duration::from_millis(1);
        loop {
            if let Some(code) = self.try_wait() {
                return code;
            }
            thread::sleep(pause);
            pause = (pause * 2).min(Duration::from_millis(50));
        }
    }

    /// `None` while the program runs; once it has ended, its exit code as [`Child::wait`]
    /// gives it.
    pub fn try_wait(&self) -> Option<Option<i32>> {
        let mut child = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        match child.try_wait() {
            Ok(Some(status)) => Some(status.code()),
            Ok(None) => None,
            Err(_) => Some(None),
        }
    }

    /// Stops the program, with everything it started. A program that has ended already
    /// is left as it is.
    pub fn kill(&self) -> Result<(), ServiceError> {
        let mut child = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if !matches!(child.try_wait(), Ok(None)) {
            return Ok(());
        }
        self.tree.kill();
        match child.kill() {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => Ok(()),
            Err(error) => Err(ServiceError::failed(format!(
                "the program could not be stopped: {error}"
            ))),
        }
    }
}

/// A program and everything it starts: a job object on Windows. Programs are added as
/// they start, so one started in the moment before the program joins is not in it.
#[cfg(windows)]
struct Tree {
    job: Option<std::os::windows::io::OwnedHandle>,
}

#[cfg(windows)]
impl Tree {
    fn adopt(child: &std::process::Child) -> Self {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW};
        // SAFETY: no attributes and no name: a new job of this process's own.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Self { job: None };
        }
        // SAFETY: a handle just made, owned from here on.
        let job = unsafe { OwnedHandle::from_raw_handle(job) };
        // SAFETY: the job above and the child's process handle, both open.
        let joined =
            unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.as_raw_handle()) };
        Self {
            job: (joined != 0).then_some(job),
        }
    }

    /// Stops every program in the job.
    fn kill(&self) {
        use std::os::windows::io::AsRawHandle;
        if let Some(job) = &self.job {
            // SAFETY: the job this tree owns.
            unsafe {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(job.as_raw_handle(), 1);
            }
        }
    }
}

/// A program and everything it starts: its process group elsewhere, which it leads.
#[cfg(unix)]
struct Tree {
    group: libc::pid_t,
}

#[cfg(unix)]
impl Tree {
    fn adopt(child: &std::process::Child) -> Self {
        Self {
            group: libc::pid_t::try_from(child.id()).unwrap_or(0),
        }
    }

    /// Stops every program in the group. Called only while the program that leads it has
    /// not been waited for, so the group's id cannot belong to anyone else yet.
    fn kill(&self) {
        if self.group > 0 {
            // SAFETY: a signal to a process group this program leads.
            unsafe {
                libc::kill(-self.group, libc::SIGKILL);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(grant: &str) -> Program {
        Program {
            grant: grant.to_owned(),
            path: PathBuf::from("/x"),
        }
    }

    #[test]
    fn a_grant_names_a_program_as_this_system_would() {
        let granted = vec!["git".to_owned(), "rg.exe".to_owned()];
        assert_eq!(allowed(&granted, &program("git")).as_deref(), Some("git"));
        assert_eq!(allowed(&granted, &program("sh")), None);
        assert_eq!(allowed(&granted, &program("GIT")).is_some(), cfg!(windows));
        assert_eq!(allowed(&granted, &program("rg")).is_some(), cfg!(windows));
        assert_eq!(
            allowed(&["git".to_owned()], &program("git.exe")).is_some(),
            cfg!(windows)
        );
    }

    #[test]
    fn a_written_exe_is_dropped_from_the_name_on_windows() {
        assert_eq!(
            without_exe("git.exe"),
            if cfg!(windows) { "git" } else { "git.exe" }
        );
        assert_eq!(
            without_exe("git.EXE"),
            if cfg!(windows) { "git" } else { "git.EXE" }
        );
        assert_eq!(without_exe(".exe"), ".exe");
        assert_eq!(without_exe("git"), "git");
    }

    #[test]
    fn a_batch_file_needs_the_cmd_grant_on_windows() {
        assert_eq!(
            grant_for("npm", Path::new("/x/npm.cmd")),
            if cfg!(windows) { "cmd" } else { "npm" }
        );
        assert_eq!(
            grant_for("tool", Path::new("/x/tool.BAT")),
            if cfg!(windows) { "cmd" } else { "tool" }
        );
        assert_eq!(grant_for("git", Path::new("/x/git.exe")), "git");
    }

    #[cfg(windows)]
    #[test]
    fn windows_tries_the_name_then_each_runnable_extension() {
        let names = candidates("git");
        assert!(names.contains(&"git.exe".to_owned()));
        assert!(names.contains(&"git.cmd".to_owned()));
        assert!(!names.contains(&"git".to_owned()));
        assert_eq!(candidates("git.exe")[0], "git.exe");
    }

    /// Makes a file that this system would run, in a new folder.
    fn runnable_in(dir: &crate::testing::TempDir, folder: &str, file: &str) -> PathBuf {
        let path = dir.write(&format!("{folder}/{file}"), b"");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn places(dir: &crate::testing::TempDir) -> Places {
        Places {
            workspace: None,
            home: dir.path().join("home"),
            plugin_data: dir.path().join("state").join("data"),
            forbidden: vec![dir.path().join("state"), dir.path().join("install")],
            own_programs: Vec::new(),
        }
    }

    #[test]
    fn a_program_is_the_first_of_its_name_along_the_path_and_pinned_canonically() {
        let dir = crate::testing::TempDir::new("resolve");
        let file = if cfg!(windows) { "tool.exe" } else { "tool" };
        runnable_in(&dir, "first", "other");
        let wanted = runnable_in(&dir, "second", file);
        runnable_in(&dir, "third", file);
        let path_var = std::env::join_paths([
            Path::new("relative"),
            &dir.path().join("first"),
            &dir.path().join("second"),
            &dir.path().join("third"),
        ])
        .unwrap();
        let program = resolve(&places(&dir), "tool", Some(&path_var)).unwrap();
        assert_eq!(program.grant, "tool");
        assert_eq!(program.path, paths::canonical(&wanted).unwrap());
        assert!(program.path.is_absolute());

        let missing = resolve(&places(&dir), "nothing-here", Some(&path_var)).unwrap_err();
        assert_eq!(missing.kind, crate::ErrorKind::NotFound);
        let path = resolve(&places(&dir), "second/tool", Some(&path_var)).unwrap_err();
        assert_eq!(path.kind, crate::ErrorKind::Invalid);
    }

    #[test]
    fn a_folder_of_the_path_that_is_not_absolute_is_skipped() {
        let dir = crate::testing::TempDir::new("relative");
        let file = if cfg!(windows) { "tool.exe" } else { "tool" };
        runnable_in(&dir, "bin", file);
        // The tests run in the crate's folder; a relative folder there would be the
        // current folder's, which is never looked in.
        let relative = std::env::join_paths([Path::new("."), Path::new("bin")]).unwrap();
        assert!(search("tool", &relative).is_none());
        let absolute = std::env::join_paths([dir.path().join("bin")]).unwrap();
        assert!(search("tool", &absolute).is_some());
    }
}
