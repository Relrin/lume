//! The calls that reach the machine: files, programs, servers, the keychain, streams of
//! bytes, and what the programs of a pane use.
//!
//! Each is carried out by the services within the places the plugin's grant covers,
//! away from the threads that serve the windows. The grant that allowed a call, or the
//! path, program or server that was refused, is written to the audit log; never what
//! was read or written. What a plugin has open (streams, watches, programs it started)
//! is its own, and goes when its code stops.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use lume_plugin::broker::grant_of;
use lume_plugin::permissions::Kind;
use lume_plugin_api::encoding::{decode, encode};
use lume_plugin_api::types::{
    Encoding, EntryKind, FetchInit, FetchResult, FetchStreamResult, FsChange, FsEntry, FsStat,
    PaneStats, ProcessExit, ProcessResult, Spawned, WriteMode,
};
use lume_plugin_api::{ApiError, Call, ErrorCode};
use lume_services::fs as files;
use lume_services::net::{self, FetchHead, FetchRequest};
use lume_services::process::{self, Program, RunOptions, SpawnOptions};
use lume_services::secrets::{Keychain, OsKeychain};
use lume_services::streams::Streams;
use lume_services::{ErrorKind, Places, ServiceError, procs};
use serde_json::{Value, json};

use super::{Plugins, api_error};

/// How many streams a plugin may hold open at once.
const STREAMS_MAX: usize = 64;

/// How many watches and programs a plugin may hold at once.
const WATCHES_MAX: usize = 32;
const PROGRAMS_MAX: usize = 16;

/// What one read of a file or a stream gives, if the plugin does not say, and at most.
const READ_DEFAULT: u64 = 16 * 1024 * 1024;
const READ_MAX: u64 = 64 * 1024 * 1024;
const STREAM_READ_DEFAULT: usize = 64 * 1024;
const STREAM_READ_MAX: usize = 1024 * 1024;

/// How many entries of a folder a listing gives at most.
const LIST_MAX: usize = 10_000;

/// How long a program may run, and how much of its output is kept.
const RUN_DEFAULT: Duration = Duration::from_secs(30);
const RUN_MAX: Duration = Duration::from_secs(10 * 60);
const OUTPUT_DEFAULT: u64 = 4 * 1024 * 1024;
const OUTPUT_MAX: u64 = 16 * 1024 * 1024;

/// How long a request to a server may take, and how large its answer may be.
const FETCH_DEFAULT: Duration = Duration::from_secs(30);
const FETCH_MAX: Duration = Duration::from_secs(5 * 60);

/// How often a program that is waited for is looked at.
const WAIT_LOOK: Duration = Duration::from_millis(50);

/// A program a plugin started, and the plugin.
struct Started {
    local: String,
    child: Arc<process::Child>,
}

/// What plugins' code has open on the machine, and what is kept between its calls.
pub(super) struct Held {
    streams: Arc<Streams>,
    watches: Mutex<HashMap<u64, (String, files::Watcher)>>,
    programs: Mutex<HashMap<u64, Started>>,
    keychain: Arc<dyn Keychain>,
    /// For each pane, when its programs' processor time was last read, and what it was.
    cpu: Mutex<HashMap<String, (Instant, Duration)>>,
    next: AtomicU64,
}

impl Default for Held {
    fn default() -> Self {
        Self {
            streams: Arc::new(Streams::new(STREAMS_MAX)),
            watches: Mutex::new(HashMap::new()),
            programs: Mutex::new(HashMap::new()),
            keychain: Arc::new(OsKeychain),
            cpu: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
        }
    }
}

impl Held {
    fn next(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A failure of the services, as a plugin's code is told it.
fn from_service(error: ServiceError) -> ApiError {
    let code = match error.kind {
        ErrorKind::Denied => ErrorCode::Denied,
        ErrorKind::NotFound => ErrorCode::NotFound,
        ErrorKind::Invalid => ErrorCode::Invalid,
        ErrorKind::TooLarge => ErrorCode::TooLarge,
        ErrorKind::Unavailable => ErrorCode::Unavailable,
        ErrorKind::TimedOut => ErrorCode::TimedOut,
        ErrorKind::Failed => ErrorCode::Failed,
    };
    api_error(code, error.message)
}

/// Runs blocking work away from the async threads.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ServiceError> + Send + 'static,
) -> Result<T, ServiceError> {
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|error| {
            Err(ServiceError {
                kind: ErrorKind::Failed,
                message: error.to_string(),
            })
        })
}

/// A command line as the user is shown it to approve: each argument that holds a space
/// or a quote in double quotes.
fn command_line(program: &Program, args: &[String]) -> String {
    let mut line = program.path.display().to_string();
    for arg in args {
        line.push(' ');
        if arg.is_empty()
            || arg
                .chars()
                .any(|c| c.is_whitespace() || c == '"' || c == '\'')
        {
            line.push('"');
            line.push_str(&arg.replace('"', "\\\""));
            line.push('"');
        } else {
            line.push_str(arg);
        }
    }
    line
}

fn kind_of(kind: files::EntryKind) -> EntryKind {
    match kind {
        files::EntryKind::File => EntryKind::File,
        files::EntryKind::Dir => EntryKind::Dir,
        files::EntryKind::Other => EntryKind::Other,
    }
}

/// A server's headers by their names in lower case, repeated ones joined.
fn headers_of(head: &FetchHead) -> std::collections::BTreeMap<String, String> {
    let mut headers = std::collections::BTreeMap::<String, String>::new();
    for (name, value) in &head.headers {
        headers
            .entry(name.to_ascii_lowercase())
            .and_modify(|held| {
                held.push_str(", ");
                held.push_str(value);
            })
            .or_insert_with(|| value.clone());
    }
    headers
}

impl Plugins {
    /// The places a plugin's paths are read against, and what no grant reaches.
    fn places(&self, local: &str) -> Places {
        let dirs = self.config.dirs();
        let plugin_data = self.store.data_dir(local);
        let _ = std::fs::create_dir_all(&plugin_data);
        let mut forbidden = vec![
            dirs.config.clone(),
            dirs.state.clone(),
            self.store.root().to_path_buf(),
        ];
        let mut own_programs = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(folder) = exe.parent() {
                forbidden.push(folder.to_path_buf());
                for name in ["lume", "lume-gui", "lume-host"] {
                    own_programs
                        .push(folder.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)));
                }
            }
            own_programs.push(exe);
        }
        Places {
            workspace: self
                .config
                .current()
                .workspace
                .root
                .clone()
                .map(PathBuf::from),
            home: std::env::home_dir().unwrap_or_default(),
            plugin_data,
            forbidden,
            own_programs,
        }
    }

    /// The scope a plugin holds of a kind.
    fn scope_of(&self, local: &str, kind: Kind) -> Vec<String> {
        self.inner()
            .loaded
            .get(local)
            .and_then(|loaded| grant_of(&loaded.grants, kind).map(|grant| grant.scope.clone()))
            .unwrap_or_default()
    }

    /// Writes how a call with a scope went: the grant that allowed it, or what was
    /// refused.
    fn audit<T>(
        &self,
        local: &str,
        kind: Kind,
        asked: &str,
        outcome: &Result<lume_services::Allowed<T>, ServiceError>,
    ) {
        match outcome {
            Ok(allowed) => self.broker.record(local, kind, Some(&allowed.grant), true),
            Err(error) if error.kind == ErrorKind::Denied => {
                self.broker.record(local, kind, Some(asked), false);
            }
            Err(_) => {}
        }
    }

    /// A program a plugin asked to run, found and checked against its grant. The name is
    /// checked before it is looked for, so what is refused does not depend on what is
    /// installed; a batch file found for it is checked again, as `cmd`.
    async fn program(&self, local: &str, name: &str) -> Result<Program, ApiError> {
        let named = Program {
            grant: name.to_owned(),
            path: PathBuf::new(),
        };
        if process::allowed(&self.scope_of(local, Kind::ProcessRun), &named).is_none() {
            self.broker
                .record(local, Kind::ProcessRun, Some(name), false);
            return Err(api_error(
                ErrorCode::Denied,
                format!("{local} has no permission to run {name}"),
            ));
        }
        let places = self.places(local);
        let owned = name.to_owned();
        let program = blocking(move || {
            process::resolve(&places, &owned, std::env::var_os("PATH").as_deref())
        })
        .await;
        let program = match program {
            Ok(program) => program,
            Err(error) => {
                if error.kind == ErrorKind::Denied {
                    self.broker
                        .record(local, Kind::ProcessRun, Some(name), false);
                }
                return Err(from_service(error));
            }
        };
        let granted = self.scope_of(local, Kind::ProcessRun);
        match process::allowed(&granted, &program) {
            Some(grant) => {
                self.broker
                    .record(local, Kind::ProcessRun, Some(&grant), true);
                Ok(program)
            }
            None => {
                self.broker
                    .record(local, Kind::ProcessRun, Some(&program.grant), false);
                Err(api_error(
                    ErrorCode::Denied,
                    format!(
                        "{local} has no permission to run {}",
                        if program.grant == name {
                            name.to_owned()
                        } else {
                            format!("{name}, which needs `{}`", program.grant)
                        }
                    ),
                ))
            }
        }
    }

    /// The folder a program starts in: one the plugin names, or its own data folder.
    fn start_folder(&self, local: &str, asked: Option<String>) -> Result<PathBuf, ApiError> {
        let places = self.places(local);
        let Some(asked) = asked else {
            return Ok(places.plugin_data);
        };
        let expanded = if let Some(rest) = asked.strip_prefix("${workspace}") {
            let root = places.workspace.ok_or_else(|| {
                api_error(ErrorCode::NotFound, "the workspace has no root folder")
            })?;
            format!("{}{rest}", root.display())
        } else if let Some(rest) = asked.strip_prefix("${home}") {
            format!("{}{rest}", places.home.display())
        } else if let Some(rest) = asked.strip_prefix("${plugin-data}") {
            format!("{}{rest}", places.plugin_data.display())
        } else {
            asked
        };
        let folder = PathBuf::from(expanded);
        if !folder.is_absolute() || !folder.is_dir() {
            return Err(api_error(
                ErrorCode::NotFound,
                format!("`{}` is not a folder that is there", folder.display()),
            ));
        }
        Ok(folder)
    }

    /// Carries out a call that reaches the machine.
    pub(super) async fn serve_system(
        self: &Arc<Self>,
        local: &str,
        call: Call,
    ) -> Result<Value, ApiError> {
        match call {
            Call::FsRead(read) => {
                let places = self.places(local);
                let granted = self.scope_of(local, Kind::FsRead);
                let max = read.options.max_bytes.unwrap_or(READ_DEFAULT).min(READ_MAX);
                let path = read.path.clone();
                let outcome = blocking(move || files::read(&places, &granted, &path, max)).await;
                self.audit(local, Kind::FsRead, &read.path, &outcome);
                let bytes = outcome.map_err(from_service)?.value;
                Ok(Value::String(encode(
                    bytes,
                    read.options.encoding.unwrap_or_default(),
                )?))
            }
            Call::FsWrite(write) => {
                let places = self.places(local);
                let granted = self.scope_of(local, Kind::FsWrite);
                let bytes = decode(&write.data, write.options.encoding.unwrap_or_default())?;
                let mode = match write.options.mode.unwrap_or_default() {
                    WriteMode::Replace => files::WriteMode::Replace,
                    WriteMode::Append => files::WriteMode::Append,
                    WriteMode::Create => files::WriteMode::CreateNew,
                };
                let path = write.path.clone();
                let outcome =
                    blocking(move || files::write(&places, &granted, &path, &bytes, mode)).await;
                self.audit(local, Kind::FsWrite, &write.path, &outcome);
                outcome.map_err(from_service)?;
                Ok(Value::Null)
            }
            Call::FsStat(asked) => {
                let places = self.places(local);
                let granted = self.scope_of(local, Kind::FsRead);
                let path = asked.path.clone();
                let outcome = blocking(move || files::stat(&places, &granted, &path)).await;
                self.audit(local, Kind::FsRead, &asked.path, &outcome);
                let stat = outcome.map_err(from_service)?.value;
                Ok(json!(FsStat {
                    kind: kind_of(stat.kind),
                    size: stat.size,
                    modified: stat.modified_ms,
                    readonly: stat.readonly,
                }))
            }
            Call::FsList(asked) => {
                let places = self.places(local);
                let granted = self.scope_of(local, Kind::FsRead);
                let path = asked.path.clone();
                let outcome =
                    blocking(move || files::list(&places, &granted, &path, LIST_MAX)).await;
                self.audit(local, Kind::FsRead, &asked.path, &outcome);
                let entries: Vec<FsEntry> = outcome
                    .map_err(from_service)?
                    .value
                    .into_iter()
                    .map(|entry| FsEntry {
                        name: entry.name,
                        kind: kind_of(entry.kind),
                        size: entry.size,
                    })
                    .collect();
                Ok(json!(entries))
            }
            Call::FsWatch(asked) => self.watch(local, &asked.path, asked.recursive).await,
            Call::FsUnwatch(asked) => {
                let mut watches = lock(&self.system.watches);
                if watches
                    .get(&asked.watch)
                    .is_some_and(|(owner, _)| owner == local)
                {
                    watches.remove(&asked.watch);
                }
                Ok(Value::Null)
            }
            Call::FsOpenStream(asked) => {
                let places = self.places(local);
                let granted = self.scope_of(local, Kind::FsRead);
                let path = asked.path.clone();
                let outcome = blocking(move || files::open_read(&places, &granted, &path)).await;
                self.audit(local, Kind::FsRead, &asked.path, &outcome);
                let file = outcome.map_err(from_service)?.value;
                let id = self
                    .system
                    .streams
                    .add(local, Some(Box::new(file)), None)
                    .map_err(from_service)?;
                Ok(json!(id))
            }
            Call::StreamsRead(asked) => {
                let max = asked
                    .options
                    .max
                    .and_then(|max| usize::try_from(max).ok())
                    .unwrap_or(STREAM_READ_DEFAULT)
                    .clamp(1, STREAM_READ_MAX);
                let streams = Arc::clone(&self.system.streams);
                let owner = local.to_owned();
                let read = blocking(move || streams.read(&owner, asked.stream, max))
                    .await
                    .map_err(from_service)?;
                match read {
                    Some(bytes) => Ok(Value::String(encode(
                        bytes,
                        asked.options.encoding.unwrap_or_default(),
                    )?)),
                    None => Ok(Value::Null),
                }
            }
            Call::StreamsWrite(asked) => {
                let bytes = decode(&asked.data, asked.encoding.unwrap_or_default())?;
                let streams = Arc::clone(&self.system.streams);
                let owner = local.to_owned();
                blocking(move || streams.write(&owner, asked.stream, &bytes))
                    .await
                    .map_err(from_service)?;
                Ok(Value::Null)
            }
            Call::StreamsClose(asked) => {
                self.system
                    .streams
                    .close(local, asked.stream)
                    .map_err(from_service)?;
                Ok(Value::Null)
            }
            Call::ProcessRun(asked) => {
                let program = self.program(local, &asked.program).await?;
                self.approve_use(
                    local,
                    Kind::ProcessRun,
                    &command_line(&program, &asked.args),
                    None,
                )
                .await?;
                let options = asked.options;
                let encoding = options.encoding.unwrap_or_default();
                let stdin = options
                    .stdin
                    .as_deref()
                    .map(|text| decode(text, encoding))
                    .transpose()?;
                let run = RunOptions {
                    cwd: Some(self.start_folder(local, options.cwd)?),
                    env: options.env.unwrap_or_default().into_iter().collect(),
                    stdin,
                    timeout: options
                        .timeout_ms
                        .map_or(RUN_DEFAULT, Duration::from_millis)
                        .min(RUN_MAX),
                    max_output: usize::try_from(
                        options.max_output.unwrap_or(OUTPUT_DEFAULT).min(OUTPUT_MAX),
                    )
                    .unwrap_or(usize::MAX),
                };
                let args = asked.args;
                let output = blocking(move || process::run(&program, &args, run))
                    .await
                    .map_err(from_service)?;
                let text = |bytes: Vec<u8>| match encoding {
                    Encoding::Utf8 => String::from_utf8_lossy(&bytes).into_owned(),
                    Encoding::Base64 => lume_plugin_api::encoding::to_base64(&bytes),
                };
                Ok(json!(ProcessResult {
                    code: output.code,
                    stdout: text(output.stdout),
                    stderr: text(output.stderr),
                    timed_out: output.timed_out,
                    truncated: output.truncated,
                }))
            }
            Call::ProcessSpawn(asked) => {
                if lock(&self.system.programs)
                    .values()
                    .filter(|started| started.local == local)
                    .count()
                    >= PROGRAMS_MAX
                {
                    return Err(api_error(
                        ErrorCode::TooLarge,
                        format!("a plugin runs {PROGRAMS_MAX} programs at most at once"),
                    ));
                }
                let program = self.program(local, &asked.program).await?;
                self.approve_use(
                    local,
                    Kind::ProcessRun,
                    &command_line(&program, &asked.args),
                    None,
                )
                .await?;
                let options = SpawnOptions {
                    cwd: Some(self.start_folder(local, asked.options.cwd)?),
                    env: asked.options.env.unwrap_or_default().into_iter().collect(),
                };
                let args = asked.args;
                let spawned = blocking(move || process::spawn(&program, &args, options))
                    .await
                    .map_err(from_service)?;
                let streams = &self.system.streams;
                let missing =
                    || api_error(ErrorCode::Failed, "the program's streams were not opened");
                let stdin = streams
                    .add(
                        local,
                        None,
                        Some(Box::new(spawned.stdin.ok_or_else(missing)?)),
                    )
                    .map_err(from_service)?;
                let stdout = streams
                    .add(
                        local,
                        Some(Box::new(spawned.stdout.ok_or_else(missing)?)),
                        None,
                    )
                    .map_err(from_service)?;
                let stderr = streams
                    .add(
                        local,
                        Some(Box::new(spawned.stderr.ok_or_else(missing)?)),
                        None,
                    )
                    .map_err(from_service)?;
                let pid = spawned.child.id();
                let id = self.system.next();
                lock(&self.system.programs).insert(
                    id,
                    Started {
                        local: local.to_owned(),
                        child: Arc::new(spawned.child),
                    },
                );
                Ok(json!(Spawned {
                    process: id,
                    pid,
                    stdin,
                    stdout,
                    stderr,
                }))
            }
            Call::ProcessWait(asked) => {
                let child = self.started(local, asked.process)?;
                loop {
                    if let Some(code) = child.try_wait() {
                        lock(&self.system.programs).remove(&asked.process);
                        return Ok(json!(ProcessExit { code }));
                    }
                    tokio::time::sleep(WAIT_LOOK).await;
                }
            }
            Call::ProcessKill(asked) => {
                let child = self.started(local, asked.process)?;
                child.kill().map_err(from_service)?;
                Ok(Value::Null)
            }
            Call::NetFetch(asked) => {
                let granted = self.scope_of(local, Kind::Net);
                let encoding = asked.init.encoding.unwrap_or_default();
                let request = fetch_request(&asked.url, asked.init)?;
                let url = asked.url.clone();
                let outcome = blocking(move || net::fetch(&granted, request)).await;
                self.audit(local, Kind::Net, &url, &outcome);
                let response = outcome.map_err(from_service)?.value;
                let headers = headers_of(&response.head);
                Ok(json!(FetchResult {
                    status: response.head.status,
                    headers,
                    url: response.head.url,
                    body: encode(response.body, encoding)?,
                }))
            }
            Call::NetFetchStream(asked) => {
                let granted = self.scope_of(local, Kind::Net);
                let request = fetch_request(&asked.url, asked.init)?;
                let url = asked.url.clone();
                let outcome = blocking(move || net::fetch_stream(&granted, request)).await;
                self.audit(local, Kind::Net, &url, &outcome);
                let (head, body) = outcome.map_err(from_service)?.value;
                let stream = self
                    .system
                    .streams
                    .add(local, Some(body), None)
                    .map_err(from_service)?;
                Ok(json!(FetchStreamResult {
                    status: head.status,
                    headers: headers_of(&head),
                    url: head.url,
                    stream,
                }))
            }
            Call::SecretsGet(asked) => {
                let keychain = Arc::clone(&self.system.keychain);
                let owner = local.to_owned();
                let secret = blocking(move || keychain.get(&owner, &asked.key))
                    .await
                    .map_err(from_service)?;
                Ok(json!(secret))
            }
            Call::SecretsSet(asked) => {
                let keychain = Arc::clone(&self.system.keychain);
                let owner = local.to_owned();
                blocking(move || keychain.set(&owner, &asked.key, &asked.secret))
                    .await
                    .map_err(from_service)?;
                Ok(Value::Null)
            }
            Call::SecretsDelete(asked) => {
                let keychain = Arc::clone(&self.system.keychain);
                let owner = local.to_owned();
                blocking(move || keychain.delete(&owner, &asked.key))
                    .await
                    .map_err(from_service)?;
                Ok(Value::Null)
            }
            Call::PanesStats(asked) => self.pane_stats(&asked.pane).await,
            other => Err(api_error(
                ErrorCode::Invalid,
                format!("{} is not carried out here", other.name()),
            )),
        }
    }

    async fn watch(
        self: &Arc<Self>,
        local: &str,
        path: &str,
        recursive: bool,
    ) -> Result<Value, ApiError> {
        if lock(&self.system.watches)
            .values()
            .filter(|(owner, _)| owner == local)
            .count()
            >= WATCHES_MAX
        {
            return Err(api_error(
                ErrorCode::TooLarge,
                format!("a plugin watches {WATCHES_MAX} paths at most at once"),
            ));
        }
        let plugin = self
            .inner()
            .loaded
            .get(local)
            .and_then(|loaded| loaded.running.as_ref())
            .map(|running| Arc::clone(&running.plugin))
            .ok_or_else(|| api_error(ErrorCode::Stopped, format!("{local} is not running")))?;
        let id = self.system.next();
        let places = self.places(local);
        let granted = self.scope_of(local, Kind::FsRead);
        let asked = path.to_owned();
        let outcome = blocking(move || {
            files::watch(
                &places,
                &granted,
                &asked,
                recursive,
                Box::new(move |event| {
                    let kind = match event.kind {
                        files::WatchKind::Created => FsChange::Created,
                        files::WatchKind::Changed => FsChange::Changed,
                        files::WatchKind::Removed => FsChange::Removed,
                        files::WatchKind::Other => FsChange::Other,
                    };
                    plugin.event(
                        "fs",
                        json!({ "watch": id, "path": event.path, "kind": kind }),
                    );
                }),
            )
        })
        .await;
        self.audit(local, Kind::FsRead, path, &outcome);
        let watcher = outcome.map_err(from_service)?.value;
        lock(&self.system.watches).insert(id, (local.to_owned(), watcher));
        Ok(json!(id))
    }

    /// A program the plugin started and has not seen end.
    fn started(&self, local: &str, id: u64) -> Result<Arc<process::Child>, ApiError> {
        lock(&self.system.programs)
            .get(&id)
            .filter(|started| started.local == local)
            .map(|started| Arc::clone(&started.child))
            .ok_or_else(|| api_error(ErrorCode::NotFound, format!("there is no program {id}")))
    }

    /// What the programs of a pane use: the one its session started and all it started.
    async fn pane_stats(&self, pane: &str) -> Result<Value, ApiError> {
        let (_, key) = self.pane_session(pane)?;
        let listed = self
            .state
            .list_all()
            .await
            .map_err(|error| api_error(ErrorCode::Failed, error.message))?;
        let pid = listed
            .iter()
            .find(|listed| listed.backend == key.backend && listed.info.id == key.session)
            .and_then(|listed| listed.info.pid)
            .ok_or_else(|| {
                api_error(
                    ErrorCode::Unavailable,
                    format!("what pane {pane} runs cannot be seen from here"),
                )
            })?;
        let usage = tokio::task::spawn_blocking(move || procs::tree_usage(pid))
            .await
            .map_err(|error| api_error(ErrorCode::Failed, error.to_string()))?
            .ok_or_else(|| {
                api_error(
                    ErrorCode::NotFound,
                    format!("the program of pane {pane} has ended"),
                )
            })?;
        let now = Instant::now();
        let cores = std::thread::available_parallelism().map_or(1, |cores| cores.get());
        let before = lock(&self.system.cpu).insert(pane.to_owned(), (now, usage.cpu_time));
        let cpu = before.and_then(|(at, time)| {
            let wall = now.duration_since(at).as_secs_f64() * cores as f64;
            (wall > 0.0)
                .then(|| (usage.cpu_time.saturating_sub(time).as_secs_f64() / wall).clamp(0.0, 1.0))
        });
        Ok(json!(PaneStats {
            processes: usage.processes,
            memory: usage.memory_bytes,
            cpu,
        }))
    }

    /// Whether a plugin has something open on the machine: a stream, a watch, or a
    /// program it started.
    pub(super) fn holds_open(&self, local: &str) -> bool {
        self.system.streams.count(local) > 0
            || lock(&self.system.watches)
                .values()
                .any(|(owner, _)| owner == local)
            || lock(&self.system.programs)
                .values()
                .any(|started| started.local == local)
    }

    /// Lets go of what a plugin had open: its streams close, its watches stop, and the
    /// programs it started are stopped.
    pub(super) fn release(&self, local: &str) {
        // Its stored values are read again from the file when it next runs: the file may
        // change while it does not, as when it is removed with its data.
        lock(&self.kv).remove(local);
        self.system.streams.close_all(local);
        lock(&self.system.watches).retain(|_, (owner, _)| owner != local);
        let ended: Vec<Started> = {
            let mut programs = lock(&self.system.programs);
            let ids: Vec<u64> = programs
                .iter()
                .filter(|(_, started)| started.local == local)
                .map(|(id, _)| *id)
                .collect();
            ids.into_iter()
                .filter_map(|id| programs.remove(&id))
                .collect()
        };
        for started in ended {
            let _ = started.child.kill();
        }
    }
}

/// A request to a server as the services take it, from what the plugin sent.
fn fetch_request(url: &str, init: FetchInit) -> Result<FetchRequest, ApiError> {
    let method = init
        .method
        .unwrap_or_else(|| "GET".to_owned())
        .to_ascii_uppercase();
    if method.is_empty() || !method.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(api_error(
            ErrorCode::Invalid,
            format!("`{method}` is not a method"),
        ));
    }
    let encoding = init.encoding.unwrap_or_default();
    let body = init
        .body
        .as_deref()
        .map(|body| decode(body, encoding))
        .transpose()?;
    Ok(FetchRequest {
        method,
        url: url.to_owned(),
        headers: init.headers.unwrap_or_default().into_iter().collect(),
        body,
        timeout: init
            .timeout_ms
            .map_or(FETCH_DEFAULT, Duration::from_millis)
            .min(FETCH_MAX),
        max_bytes: init.max_bytes.unwrap_or(READ_DEFAULT).min(READ_MAX),
    })
}
