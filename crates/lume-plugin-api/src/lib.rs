//! The API Lume offers plugins' code, defined once.
//!
//! Every call a plugin can make is a row of [`api!`] below: its name, the arguments it
//! takes in JavaScript, the type of what it sends and of what it gets back, the
//! permission it needs, and what it does. From that one table come:
//!
//! - [`Call`], what the runtime reads a call into, and which the application's
//!   dispatcher matches on, so a call with no handler does not build;
//! - the functions on the `lume` object, written into the prelude the runtime loads
//!   before a plugin's code;
//! - the TypeScript typings authors write against (`@lume/plugin-api`);
//! - the reference in the documentation.
//!
//! A few parts of the API are not plain calls (commands, timers, listening for events,
//! logging); they are written by hand, in the prelude and the typings' templates beside
//! this file, next to the generated parts.

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde_json::Value;

pub mod encoding;
pub mod generate;
mod ts;
pub mod types;

pub use types::{ApiError, ErrorCode};
use types::{
    ClipboardWrite, CommandExecute, ContextSet, FetchResult, FetchStreamResult, FsEntry, FsPath,
    FsRead, FsStat, FsUnwatch, FsWatch, FsWrite, Grant, KvKey, KvSet, LayoutApply, MetricsSample,
    MetricsSubscribe, MetricsUnsubscribe, NetFetch, Nothing, PaneInfo, PaneRef, PaneStats,
    PaneWrite, PanelId, PanelsSet, PanelsSetChildren, PermissionsRequest, ProcessExit, ProcessId,
    ProcessResult, ProcessRun, ProcessSpawn, SecretKey, SecretSet, ServicesCall, SessionEntry,
    SessionsSpawn, SettingsGet, Spawned, StatusClear, StatusSet, StreamId, StreamRead, StreamWrite,
    Subscribe, UiConfirm, UiPick, UiToast, ViewInstance, ViewsInstances, ViewsPost, WorkspaceInfo,
};

/// The major version of the API, which a manifest's `api` names.
pub const API_MAJOR: u32 = 1;

/// The calls that only change what a plugin shows. The code is not answered for each:
/// its promise is settled at once, and a failure goes to the plugin's log. What they
/// show is shown together at the end of the turn that made them in any case.
pub const TOLD: &[&str] = &[
    "status.set",
    "status.clear",
    "context.set",
    "panels.set",
    "panels.clear",
    "panels.setChildren",
    "views.post",
];

/// One call of the API.
pub struct Method {
    /// Its name: the namespace and the function, as in `status.set`.
    pub name: &'static str,
    /// The arguments the function takes in JavaScript, in order. Each is a field of
    /// what the call sends.
    pub args: &'static [&'static str],
    /// The permission it needs, by the name a manifest writes; none for the plugin's
    /// own things.
    pub permission: Option<&'static str>,
    /// What it does.
    pub doc: &'static str,
    /// Whether the function is reached only through the hand-written part of the
    /// prelude, not as `lume.<name>`.
    pub internal: bool,
    pub params: fn(&mut SchemaGenerator) -> Schema,
    pub result: fn(&mut SchemaGenerator) -> Schema,
}

/// Why what a plugin's code sent is not a call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// No call has the name.
    Unknown(String),
    /// The arguments are not what the call takes.
    Invalid { name: &'static str, message: String },
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(name) => write!(f, "there is no call `{name}`"),
            Self::Invalid { name, message } => write!(f, "{name}: {message}"),
        }
    }
}

fn schema_of<T: JsonSchema>(generator: &mut SchemaGenerator) -> Schema {
    generator.subschema_for::<T>()
}

/// Defines the calls. Each row is
/// `Variant = "name"(js, args): Params => Result [, needs "permission"] [, internal = true];`.
macro_rules! api {
    ($(
        $(#[doc = $doc:literal])*
        $variant:ident = $name:literal ( $($arg:ident),* ) : $params:ty => $result:ty
        $(, needs $permission:literal)? $(, internal = $internal:literal)? ;
    )*) => {
        /// A call a plugin's code made, read.
        #[derive(Clone, Debug, PartialEq)]
        pub enum Call {
            $($variant($params),)*
        }

        impl Call {
            /// Reads a call from its name and the JSON its code sent.
            pub fn decode(name: &str, args: Value) -> Result<Self, DecodeError> {
                match name {
                    $($name => serde_json::from_value::<$params>(args)
                        .map(Self::$variant)
                        .map_err(|error| DecodeError::Invalid {
                            name: $name,
                            message: error.to_string(),
                        }),)*
                    other => Err(DecodeError::Unknown(other.to_owned())),
                }
            }

            /// The call's name.
            pub fn name(&self) -> &'static str {
                match self {
                    $(Self::$variant(_) => $name,)*
                }
            }

            /// The permission the call needs, if any.
            pub fn permission(&self) -> Option<&'static str> {
                method(self.name()).and_then(|method| method.permission)
            }
        }

        /// Every call of the API.
        pub static METHODS: &[Method] = &[
            $(Method {
                name: $name,
                args: &[$(stringify!($arg)),*],
                permission: api!(@permission $($permission)?),
                doc: concat!($($doc, "\n",)* ""),
                internal: api!(@internal $($internal)?),
                params: schema_of::<$params>,
                result: schema_of::<$result>,
            },)*
        ];
    };
    (@permission) => { None };
    (@permission $permission:literal) => { Some($permission) };
    (@internal) => { false };
    (@internal $internal:literal) => { $internal };
}

api! {
    /// Shows something in one of the plugin's status items. Fields left out keep what
    /// the item showed before.
    StatusSet = "status.set"(id, value, pane): StatusSet => ();
    /// Takes back what a status item showed, so it shows what its manifest says.
    StatusClear = "status.clear"(id, pane): StatusClear => ();
    /// Runs a command: one of the plugin's own, by its name, or one of the
    /// application's that plugins may run. Its arguments are a JSON object.
    CommandsExecute = "commands.execute"(id, args): CommandExecute => ();
    /// The value the plugin stored under a key, or null.
    KvGet = "kv.get"(key): KvKey => Value;
    /// Stores a value under a key. What a plugin stores is its own, and all of it
    /// together may be 1 MiB.
    KvSet = "kv.set"(key, value): KvSet => ();
    /// Removes what is stored under a key.
    KvDelete = "kv.delete"(key): KvKey => ();
    /// The keys the plugin has stored values under.
    KvList = "kv.list"(): Nothing => Vec<String>;
    /// One of the plugin's settings, by its key, or all of them as an object when no
    /// key is given. A setting the user has not set has its default.
    SettingsGet = "settings.get"(key): SettingsGet => Value;
    /// Sets a condition the plugin's keys and menu items can be shown under.
    ContextSet = "context.set"(key, value): ContextSet => ();
    /// Starts hearing about a topic.
    EventsSubscribe = "events.subscribe"(topic): Subscribe => (), needs "workspace.read", internal = true;
    /// Shows rows in one of the plugin's list panels, in place of those it showed. A
    /// panel holds 5000 rows at most, eight deep.
    PanelsSet = "panels.set"(id, rows): PanelsSet => ();
    /// Takes away the rows a list panel shows.
    PanelsClear = "panels.clear"(id): PanelId => ();
    /// Gives the rows beneath a row that is `lazy`, by the ids of the rows from the top
    /// down to it, in place of those it had. They count toward the panel's 5000 rows
    /// and eight levels. Setting the panel's rows anew takes them away with the rest.
    PanelsSetChildren = "panels.setChildren"(id, path, rows): PanelsSetChildren => ();
    /// The latest reading of the machine, or null before the first: how busy the
    /// processor and the graphics card are, how much memory is used, how fast the
    /// network goes.
    MetricsSnapshot = "metrics.snapshot"(): Nothing => Option<MetricsSample>, needs "system.metrics";
    /// Starts readings of the machine for the plugin's code.
    MetricsSubscribe = "metrics.subscribe"(kinds, visible): MetricsSubscribe => u64, needs "system.metrics", internal = true;
    /// Stops them.
    MetricsUnsubscribe = "metrics.unsubscribe"(subscription): MetricsUnsubscribe => (), needs "system.metrics", internal = true;
    /// What a file holds, as text or, when asked, base64. Only files the plugin may
    /// read, found after links are followed; a file larger than `maxBytes` is refused.
    FsRead = "fs.read"(path, options): FsRead => String, needs "fs.read";
    /// Writes a file the plugin may write. A file with more than one name (a hard
    /// link) is refused.
    FsWrite = "fs.write"(path, data, options): FsWrite => (), needs "fs.write";
    /// What is known of a file or folder the plugin may read.
    FsStat = "fs.stat"(path): FsPath => FsStat, needs "fs.read";
    /// What a folder the plugin may read holds: the entries it may read, 10 000 at most.
    FsList = "fs.list"(path): FsPath => Vec<FsEntry>, needs "fs.read";
    /// Starts watching a file or folder.
    FsWatch = "fs.watch"(path, recursive): FsWatch => u64, needs "fs.read", internal = true;
    /// Stops watching it.
    FsUnwatch = "fs.unwatch"(watch): FsUnwatch => (), internal = true;
    /// Opens a file the plugin may read as a stream, read with `lume.streams.read`.
    FsOpenStream = "fs.openStream"(path): FsPath => u64, needs "fs.read";
    /// The next bytes of one of the plugin's streams, or null once it has ended. Waits
    /// until there are some.
    StreamsRead = "streams.read"(stream, options): StreamRead => Option<String>;
    /// Writes to one of the plugin's streams, as a program's input.
    StreamsWrite = "streams.write"(stream, data, encoding): StreamWrite => ();
    /// Closes one of the plugin's streams. A program's input that is closed ends.
    StreamsClose = "streams.close"(stream): StreamId => ();
    /// Runs a program the plugin may run, found once on the system's path, and gives
    /// back how it ended and what it wrote. Where the permission says so, the user is
    /// asked first, shown the command.
    ProcessRun = "process.run"(program, args, options): ProcessRun => ProcessResult, needs "process.run";
    /// Starts a program the plugin may run and leaves it running, with its input and
    /// output as streams.
    ProcessSpawn = "process.spawn"(program, args, options): ProcessSpawn => Spawned, needs "process.run";
    /// Waits for a program the plugin started to end.
    ProcessWait = "process.wait"(process): ProcessId => ProcessExit;
    /// Stops a program the plugin started.
    ProcessKill = "process.kill"(process): ProcessId => ();
    /// Asks a server the plugin may reach. Redirects are followed while they stay
    /// within what it may reach; this computer's own addresses only where they are
    /// named.
    NetFetch = "net.fetch"(url, init): NetFetch => FetchResult, needs "net";
    /// The same, with the answer's body as a stream.
    NetFetchStream = "net.fetchStream"(url, init): NetFetch => FetchStreamResult, needs "net";
    /// One of the plugin's secrets from the system's keychain, or null.
    SecretsGet = "secrets.get"(key): SecretKey => Option<String>;
    /// Keeps a secret of the plugin's in the system's keychain. Where there is no
    /// keychain this fails as `unavailable`; it is never written anywhere else.
    SecretsSet = "secrets.set"(key, secret): SecretSet => ();
    /// Removes one of the plugin's secrets.
    SecretsDelete = "secrets.delete"(key): SecretKey => ();
    /// The text on the clipboard, or null.
    ClipboardRead = "clipboard.read"(): Nothing => Option<String>, needs "clipboard.read";
    /// Puts text on the clipboard.
    ClipboardWrite = "clipboard.write"(text): ClipboardWrite => (), needs "clipboard.write";
    /// The terminal panes of the workspace.
    PanesList = "panes.list"(): Nothing => Vec<PaneInfo>, needs "panes.read";
    /// What a terminal pane shows.
    PanesText = "panes.text"(pane): PaneRef => String, needs "panes.read";
    /// Types into a terminal pane. Where the permission says so, the user is asked
    /// first, shown the text.
    PanesWrite = "panes.write"(pane, text): PaneWrite => (), needs "panes.write";
    /// What the programs of a terminal pane use of the machine.
    PanesStats = "panes.stats"(pane): PaneRef => PaneStats, needs "panes.read";
    /// Opens a terminal from one of the user's profiles.
    SessionsSpawn = "sessions.spawn"(profile, options): SessionsSpawn => PaneInfo, needs "sessions.spawn";
    /// The sessions of the workspace.
    SessionsList = "sessions.list"(): Nothing => Vec<SessionEntry>, needs "workspace.read";
    /// The workspace in use.
    WorkspaceCurrent = "workspace.current"(): Nothing => WorkspaceInfo, needs "workspace.read";
    /// The workspace's root folder, which `${workspace}` stands for, or null.
    WorkspaceRoot = "workspace.root"(): Nothing => Option<String>, needs "workspace.read";
    /// Changes the layout: opens, moves, splits or closes tabs and panes, as a layout
    /// intent says. What only the application may do is refused.
    LayoutApply = "layout.apply"(intent): LayoutApply => Value, needs "layout.write";
    /// Shows a notice, under the plugin's name.
    UiToast = "ui.toast"(message, level): UiToast => ();
    /// Asks the user a question, under the plugin's name, and says whether they went
    /// ahead.
    UiConfirm = "ui.confirm"(title, message, options): UiConfirm => bool;
    /// Has the user choose one of a list, under the plugin's name: its id, or null if
    /// they chose none.
    UiPick = "ui.pick"(items, options): UiPick => Option<String>;
    /// Asks the user for a permission the manifest lists under
    /// `[permissions.optional]`, and says whether it was granted.
    PermissionsRequest = "permissions.request"(permission, scope, reason): PermissionsRequest => bool;
    /// The permissions the plugin holds.
    PermissionsList = "permissions.list"(): Nothing => Vec<Grant>;
    /// Calls a service another plugin offers, as its manifest declares it.
    ServicesCall = "services.call"(plugin, name, args): ServicesCall => Value, needs "services.use";
    /// Sends an event to the plugin's own open views: to one, by its instance, or to
    /// every open one of a view, by the view's name. A view hears it with
    /// `host.on(event, …)`; views that are not open miss it.
    ViewsPost = "views.post"(to, event, payload): ViewsPost => ();
    /// The plugin's views that are open now, in every window: all of them, or those of
    /// one view by its name.
    ViewsInstances = "views.instances"(view): ViewsInstances => Vec<ViewInstance>;
}

/// The call of this name.
pub fn method(name: &str) -> Option<&'static Method> {
    METHODS.iter().find(|method| method.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_call_is_read_from_its_name_and_arguments() {
        let call = Call::decode(
            "status.set",
            json!({ "id": "cpu", "value": { "value": 0.5, "text": "50%" } }),
        )
        .unwrap();
        assert_eq!(call.name(), "status.set");
        assert_eq!(call.permission(), None);
        let Call::StatusSet(set) = call else {
            panic!("not a status.set");
        };
        assert_eq!(set.value.value, Some(0.5));

        assert_eq!(
            Call::decode("events.subscribe", json!({ "topic": "session" }))
                .unwrap()
                .permission(),
            Some("workspace.read")
        );
        assert_eq!(
            Call::decode("status.blink", json!({})),
            Err(DecodeError::Unknown("status.blink".to_owned()))
        );
        assert!(matches!(
            Call::decode(
                "status.set",
                json!({ "id": "cpu", "value": {}, "colour": 1 })
            ),
            Err(DecodeError::Invalid {
                name: "status.set",
                ..
            })
        ));
        assert!(Call::decode("kv.list", json!({})).is_ok());
    }

    #[test]
    fn every_call_says_what_it_does_and_its_arguments_are_its_fields() {
        let mut generator = schemars::generate::SchemaSettings::draft2020_12().into_generator();
        for method in METHODS {
            assert!(!method.doc.trim().is_empty(), "{}", method.name);
            let reference = (method.params)(&mut generator);
            let name = reference
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|path| path.rsplit('/').next())
                .unwrap_or_default()
                .to_owned();
            let definitions = generator.definitions().clone();
            let fields = definitions
                .get(&name)
                .and_then(|schema| schema.get("properties"))
                .and_then(Value::as_object)
                .map(|properties| properties.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            for arg in method.args {
                assert!(
                    fields.contains(&(*arg).to_owned()),
                    "{}: {arg}",
                    method.name
                );
            }
        }
    }
}
